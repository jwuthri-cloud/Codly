//! Per-agent live controls end to end, with a fake agent in place of the CLI: block tools
//! (approvals denied), stop after this step (a steer note is held until the resume), kill.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::Store;
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    repo
}

fn claude() -> CliInfo {
    CliInfo {
        provider: Provider::Claude,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(Provider::Claude),
        default_model: None,
    }
}

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// What Codly answered to the fake agent's approval requests.
    answers: Vec<ApprovalChoice>,
    interrupts: u32,
    /// The run id of every spawn, in order.
    runs: Vec<String>,
    /// Knob: how long a spawn takes to hand back its agent.
    spawn_delay: Duration,
    /// Knob: the next spawn's CLI exits (code 1) right after its handshake, before Codly has the handle.
    dies_on_start: bool,
    /// Knob: a shutdown takes 300 ms and takes no more prompts, like the real adapters.
    slow_shutdown: bool,
}

/// A fake agent. "slow" takes 300 ms; "act" asks to run a command and records the answer.
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let send = {
                let (events, agent_id, run_id) = (events.clone(), spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            let (delay, dies) = {
                let mut s = seen.lock().unwrap();
                s.runs.push(spec.run_id.clone());
                (s.spawn_delay, std::mem::take(&mut s.dies_on_start))
            };
            tokio::time::sleep(delay).await;
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("sess".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            if dies {
                send(EventKind::Exited { code: Some(1) });
                drop(rx);
                tokio::time::sleep(Duration::from_millis(100)).await;
                return Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx });
            }
            let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            seen.lock().unwrap().prompts.push(text.clone());
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            if text.contains("slow") {
                                tokio::time::sleep(Duration::from_millis(300)).await;
                            }
                            if text.contains("act") {
                                let approval = Approval {
                                    id: new_id(),
                                    agent_id: agent_id.clone(),
                                    run_id: run_id.clone(),
                                    task_id: None,
                                    call_id: Some("c1".into()),
                                    subject: ApprovalSubject::Command { command: "cargo test".into(), cwd: None },
                                    reason: None,
                                    choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
                                    risks: vec![],
                                    status: ApprovalStatus::Pending,
                                    decided_by: None,
                                    created_at: now_ms(),
                                };
                                send(EventKind::ApprovalRequested { approval });
                                // The answer arrives as the next command.
                                if let Ok(Some(AgentCmd::Approval { decision, .. })) =
                                    tokio::time::timeout(Duration::from_secs(5), rx.recv()).await
                                {
                                    seen.lock().unwrap().answers.push(decision.choice);
                                }
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Interrupt => seen.lock().unwrap().interrupts += 1,
                        AgentCmd::Shutdown => {
                            let slow = seen.lock().unwrap().slow_shutdown;
                            if slow {
                                tokio::time::sleep(Duration::from_millis(300)).await;
                            }
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Approval { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A started core with one hired agent, not running yet.
async fn hired_agent(dir: &Path, seen: &Arc<Mutex<Seen>>) -> (Arc<Core>, String) {
    let repo = temp_repo(dir);
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    (core, agent.id)
}

/// A started core with one hired agent that is live (a first message spawned it).
async fn live_agent(dir: &Path, seen: &Arc<Mutex<Seen>>) -> (Arc<Core>, String) {
    let (core, id) = hired_agent(dir, seen).await;
    core.send_chat(&format!("agent:{id}"), "hello").await.unwrap();
    wait_for("the first turn", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    wait_for("the agent to go idle", async || {
        let a = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == id)?;
        (a.status == AgentStatus::Idle).then_some(())
    })
    .await;
    (core, id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_tools_are_denied_and_allowed_again() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    let state = core.set_agent_block_tools(&id, true).await.unwrap();
    assert!(state.block_tools);
    core.send_chat(&thread, "act now").await.unwrap();
    wait_for("the denial", async || (seen.lock().unwrap().answers.len() == 1).then_some(())).await;
    assert_eq!(seen.lock().unwrap().answers, vec![ApprovalChoice::Deny]);
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
            if decision_type == "agent_control" && summary.contains("cargo test"))),
        "the denial is recorded as a decision"
    );

    core.set_agent_block_tools(&id, false).await.unwrap();
    assert!(!core.agent_control(&id).await.unwrap().block_tools);
    assert!(core.set_agent_block_tools("nobody", true).await.is_err());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_after_step_holds_steer_notes_until_resume() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    core.send_chat(&thread, "slow step").await.unwrap();
    wait_for("the slow turn to start", async || (seen.lock().unwrap().prompts.len() == 2).then_some(())).await;

    let state = core.stop_agent_after_step(&id).await.unwrap();
    assert!(state.stop_requested && !state.paused, "{state:?}");
    let state = core.steer_agent(&id, "prefer the v2 client").await.unwrap();
    assert_eq!(state.held, 1, "the note waits instead of starting a turn behind the current one");

    let state = wait_for("the pause", async || {
        let s = core.agent_control(&id).await.unwrap();
        s.paused.then_some(s)
    })
    .await;
    assert!(!state.stop_requested);
    assert_eq!(seen.lock().unwrap().prompts.len(), 2, "nothing new started while paused");

    // Chat is held too: the agent stays still until you resume.
    core.send_chat(&thread, "one more thing").await.unwrap();
    assert_eq!(core.agent_control(&id).await.unwrap().held, 2);

    let state = core.resume_agent(&id).await.unwrap();
    assert!(!state.paused && state.held == 0);
    wait_for("the held prompts", async || (seen.lock().unwrap().prompts.len() == 4).then_some(())).await;
    let prompts = seen.lock().unwrap().prompts.clone();
    assert!(prompts[2].starts_with("[Steer note from the user"), "{prompts:?}");
    assert!(prompts[2].ends_with("prefer the v2 client"));
    assert!(core.resume_agent(&id).await.is_err(), "not paused any more");
    assert!(core.steer_agent(&id, "  ").await.is_err());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_agent_pauses_at_once_and_kill_clears_the_controls() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    core.set_agent_block_tools(&id, true).await.unwrap();
    let state = core.stop_agent_after_step(&id).await.unwrap();
    assert!(state.paused && state.block_tools, "{state:?}");

    core.stop_agent(&id).await.unwrap();
    wait_for("the controls to clear", async || core.agent_control(&id).await.unwrap().is_default().then_some(())).await;
    assert!(core.steer_agent(&id, "too late").await.is_err(), "a stopped agent takes no steer note");
    // The controls clear before their event is written: wait for it.
    wait_for("the cleared controls' event", async || {
        let events = core.events(Some(&id), None, 200).await.unwrap();
        events.iter().any(|e| matches!(&e.kind, EventKind::ControlChanged { control } if control.is_default())).then_some(())
    })
    .await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_dies_during_startup_leaves_no_ghost() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { dies_on_start: true, ..Default::default() }));
    let (core, id) = hired_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    assert!(core.send_chat(&thread, "hello").await.is_err(), "its CLI died before it could take the message");
    // The next message starts a new run: nothing is left that looks live with no process behind it.
    core.send_chat(&thread, "again").await.unwrap();
    wait_for("the second run's turn", async || seen.lock().unwrap().prompts.iter().any(|p| p.ends_with("again")).then_some(())).await;
    let runs = seen.lock().unwrap().runs.clone();
    assert_eq!(runs.len(), 2);
    let store = Store::open(&dir.path().join("data/codly.sqlite")).unwrap();
    assert_ne!(store.run(&runs[0]).unwrap().unwrap().status, "running", "the run that died is over");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_quick_messages_start_one_run_and_both_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { spawn_delay: Duration::from_millis(300), ..Default::default() }));
    let (core, id) = hired_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    let (one, two) = tokio::join!(core.send_chat(&thread, "one"), core.send_chat(&thread, "two"));
    assert!(one.is_ok() && two.is_ok(), "{one:?} {two:?}");
    let spawns = seen.lock().unwrap().runs.len();
    assert_eq!(spawns, 1, "the second message goes to the run the first one started");
    wait_for("both turns", async || {
        let prompts = seen.lock().unwrap().prompts.clone();
        ["one", "two"].iter().all(|t| prompts.iter().any(|p| p.ends_with(t))).then_some(())
    })
    .await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_sent_while_an_agent_starts_follows_the_one_that_started_it() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { spawn_delay: Duration::from_millis(300), ..Default::default() }));
    let (core, id) = hired_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    let first = tokio::spawn({
        let (core, thread) = (core.clone(), thread.clone());
        async move { core.send_chat(&thread, "one").await }
    });
    wait_for("the first message's spawn", async || (seen.lock().unwrap().runs.len() == 1).then_some(())).await;
    core.send_chat(&thread, "two").await.unwrap(); // its CLI is still starting
    first.await.unwrap().unwrap();
    wait_for("both turns", async || (seen.lock().unwrap().prompts.len() == 2).then_some(())).await;
    let prompts = seen.lock().unwrap().prompts.clone();
    assert!(prompts[0].ends_with("one") && prompts[1].ends_with("two"), "in the order you sent them: {prompts:?}");
    assert_eq!(seen.lock().unwrap().runs.len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_after_stop_starts_a_fresh_run() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { slow_shutdown: true, ..Default::default() }));
    let (core, id) = live_agent(dir.path(), &seen).await;

    core.stop_agent(&id).await.unwrap();
    // Its process is still shutting down and takes no more prompts.
    core.send_chat(&format!("agent:{id}"), "still there?").await.unwrap();
    wait_for("the message to arrive", async || seen.lock().unwrap().prompts.iter().any(|p| p.ends_with("still there?")).then_some(())).await;
    let spawns = seen.lock().unwrap().runs.len();
    assert_eq!(spawns, 2, "a new run took it");
    core.shutdown().await;
}
