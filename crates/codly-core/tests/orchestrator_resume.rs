//! A resume that fails is never silent. With a fake agent in place of the CLI: a 1:1 agent's
//! session cannot be resumed, Codly starts a new one, and the conversation says so with the
//! reason. A resume that works says nothing. A task's worker says it the same way, and the
//! decision is recorded on its task.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
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

const GONE: &str = "claude exited during startup (code 1): No conversation found with session ID: sess-1";

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// How each run was asked to start, in order: "new", or "resume <session id>".
    starts: Vec<String>,
    /// Knob: a resume fails to start, as when the CLI no longer has the session.
    resume_fails: bool,
    /// Knob: a turn stays open until the agent is stopped.
    turns_stay_open: bool,
}

/// A fake agent whose sessions are "sess-1", "sess-2"… and whose turns end at once (or stay
/// open, with the knob).
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let session = {
                let mut s = seen.lock().unwrap();
                match &spec.session {
                    SessionMode::Resume { session_id } => {
                        s.starts.push(format!("resume {session_id}"));
                        if s.resume_fails {
                            return Err(anyhow::anyhow!(GONE));
                        }
                        session_id.clone()
                    }
                    SessionMode::New { .. } => {
                        s.starts.push("new".into());
                        format!("sess-{}", s.starts.iter().filter(|start| *start == "new").count())
                    }
                }
            };
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(session),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            let stays_open = {
                                let mut s = seen.lock().unwrap();
                                s.prompts.push(text.clone());
                                s.turns_stay_open
                            };
                            send(EventKind::TurnStarted { input: Some(text) });
                            if !stays_open {
                                send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                            }
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
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

async fn agent_now(core: &Core, id: &str) -> Agent {
    core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == id).expect("the agent is hired")
}

async fn task_now(core: &Core, id: &str) -> Task {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.id == id).expect("the task is on the board")
}

/// Send a message to the agent and wait until its turn is over.
async fn say(core: &Core, seen: &Arc<Mutex<Seen>>, id: &str, text: &str) {
    core.send_chat(&format!("agent:{id}"), text).await.unwrap();
    wait_for("the message to arrive", async || seen.lock().unwrap().prompts.iter().any(|p| p.ends_with(text)).then_some(())).await;
    wait_for("the agent to go idle", async || (agent_now(core, id).await.status == AgentStatus::Idle).then_some(())).await;
}

/// What Codly itself said in the agent's conversation.
async fn codly_said(core: &Core, id: &str) -> Vec<String> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.author == "codly" && message.thread == format!("agent:{id}") => Some(message.text),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resume_that_fails_says_so_and_goes_on_in_a_new_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let cfg = CoreConfig { data_dir: dir.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let id = agent.id.clone();

    say(&core, &seen, &id, "hello").await;
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-1"));

    // A resume that works: the same conversation, and nothing to say about it.
    core.stop_agent(&id).await.unwrap();
    say(&core, &seen, &id, "are you there?").await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"]);
    assert!(codly_said(&core, &id).await.is_empty());

    // A resume that fails: a new conversation takes the message, and the thread says why
    // the agent no longer knows the earlier ones.
    core.stop_agent(&id).await.unwrap();
    seen.lock().unwrap().resume_fails = true;
    say(&core, &seen, &id, "and now?").await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "resume sess-1", "new"]);
    let name = &agent.name;
    let notice = format!("{name}'s earlier conversation could not be resumed ({GONE}). A new one started: {name} does not know what was said before.");
    wait_for("the notice", async || codly_said(&core, &id).await.contains(&notice).then_some(())).await;
    assert_eq!(codly_said(&core, &id).await, [notice.clone()], "said once");
    let recorded = core.events(None, None, 2000).await.unwrap().into_iter().any(|e| match e.kind {
        EventKind::Decision { decision_type, summary, .. } => decision_type == "retry" && summary == notice,
        _ => false,
    });
    assert!(recorded, "the decision log has it too");
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-2"), "the next message resumes the new conversation");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_whose_resume_fails_says_so_and_the_decision_is_on_its_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen { turns_stay_open: true, ..Default::default() }));
    let cfg = CoreConfig { data_dir: dir.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let task = core.create_task("Write the greeting", "Create hello.txt containing hello").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();

    // The worker is in its first turn when you stop it: the task waits for it, with its session.
    wait_for("the worker's first turn", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    let worker = task_now(&core, &task.id).await.assignee.expect("the task has a worker");
    core.stop_agent(&worker).await.unwrap();
    wait_for("the task to wait for you", async || (task_now(&core, &task.id).await.status == TaskStatus::Blocked).then_some(())).await;

    // You retry the task and the session is gone: a new one takes the task, and the worker's
    // thread says why it starts over.
    seen.lock().unwrap().resume_fails = true;
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_for("the worker's new session", async || (seen.lock().unwrap().prompts.len() == 2).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "new"]);
    let name = agent_now(&core, &worker).await.name;
    let notice = format!("{name}'s earlier conversation could not be resumed ({GONE}). A new one started: {name} does not know what was said before.");
    wait_for("the notice", async || codly_said(&core, &worker).await.contains(&notice).then_some(())).await;
    assert_eq!(codly_said(&core, &worker).await, [notice.clone()], "said once, in the worker's own thread");
    let on_task = core.events(None, None, 2000).await.unwrap().into_iter().any(|e| match e.kind {
        EventKind::Decision { decision_type, summary, .. } => decision_type == "retry" && summary == notice && e.task_id.as_deref() == Some(task.id.as_str()),
        _ => false,
    });
    assert!(on_task, "the decision is on the task");
    core.shutdown().await;
}
