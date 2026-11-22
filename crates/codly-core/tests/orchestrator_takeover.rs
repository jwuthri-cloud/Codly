//! "Open in my terminal", with a fake agent in place of the CLI. An agent between turns goes to
//! your terminal: Codly stops its own process for the session, and starts none until you give
//! the agent back. Then its next message resumes the same session. An agent in a turn, or on a
//! task that still runs, does not go.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::takeover::{TakeoverState, with_cd};
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

fn cli(provider: Provider) -> CliInfo {
    CliInfo {
        provider,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(provider),
        default_model: None,
    }
}

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// How each run was asked to start, in order: "new", or "resume <session id>".
    starts: Vec<String>,
    /// How many runs Codly asked to shut down.
    shutdowns: usize,
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
                            seen.lock().unwrap().shutdowns += 1;
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

async fn start(data: &Path, seen: &Arc<Mutex<Seen>>, repo: &Path) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: data.to_path_buf(), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![cli(Provider::Claude), cli(Provider::Gemini)]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let settings = Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
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

/// Wait until the agent is between turns with a session: the moment it can go to your terminal.
async fn at_rest(core: &Core, id: &str) {
    wait_for("the agent to be between turns", async || (core.agent_takeover(id).await.unwrap() == TakeoverState::default()).then_some(())).await;
}

/// Everything said in the agent's conversation, by anyone.
async fn said_to(core: &Core, id: &str) -> Vec<String> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.thread == format!("agent:{id}") => Some(message.text),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_agent_goes_to_your_terminal_and_comes_back_on_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;

    // No conversation yet: there is nothing to open, and the sheet says why.
    let state = core.agent_takeover(&id).await.unwrap();
    assert!(state.takeover.is_none());
    assert!(state.blocked.as_deref().is_some_and(|why| why.contains("has no session yet")), "{state:?}");
    assert!(core.take_over_agent(&id).await.unwrap_err().to_string().contains("has no session yet"));

    say(&core, &seen, &id, "hello").await;
    let before = agent_now(&core, &id).await;
    assert_eq!(before.session_id.as_deref(), Some("sess-1"));
    assert!(before.run_id.is_some(), "its process stays up between turns");
    at_rest(&core, &id).await;

    // Take over: Codly's process for the session stops first, and the command is the CLI's own.
    let took = core.take_over_agent(&id).await.unwrap().takeover.expect("it is in your terminal");
    let worktree = before.worktree.clone().expect("a 1:1 agent has a worktree");
    assert_eq!((took.agent_id.as_str(), took.cwd.as_str(), took.command.as_str()), (id.as_str(), worktree.as_str(), "claude --resume sess-1"));
    assert_eq!(took.copy_command, with_cd(&worktree, "claude --resume sess-1"));
    assert_eq!(seen.lock().unwrap().shutdowns, 1);
    // No process of Codly's is on the session (its row follows the exit a moment later).
    wait_for("its row to have no run", async || agent_now(&core, &id).await.run_id.is_none().then_some(())).await;
    // Asked again, the same hand-over.
    assert_eq!(core.take_over_agent(&id).await.unwrap().takeover.as_ref(), Some(&took));
    let recorded = core.events(None, None, 2000).await.unwrap().into_iter().any(|e| match e.kind {
        EventKind::Decision { summary, .. } => summary.contains("is in your terminal") && e.agent_id.as_deref() == Some(id.as_str()),
        _ => false,
    });
    assert!(recorded, "the hand-over is in the agent's history");

    // While it is there a message is refused, not recorded, and starts nothing.
    let refused = core.send_chat(&format!("agent:{id}"), "are you there?").await.unwrap_err().to_string();
    assert!(refused.ends_with("is in your terminal: give it back first"), "{refused}");
    assert_eq!(said_to(&core, &id).await, ["hello"]);
    assert_eq!(seen.lock().unwrap().starts, ["new"]);
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-1"));

    // It outlives a restart of Codly: a terminal of your own may still have the session.
    core.shutdown().await;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    assert_eq!(core.agent_takeover(&id).await.unwrap().takeover, Some(took));
    assert!(core.send_chat(&format!("agent:{id}"), "and now?").await.is_err());
    assert!(seen.lock().unwrap().starts.is_empty());

    // Give it back: the next message resumes the same session.
    assert_eq!(core.give_back_agent(&id).await.unwrap(), TakeoverState::default());
    say(&core, &seen, &id, "welcome back").await;
    assert_eq!(seen.lock().unwrap().starts, ["resume sess-1"]);
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-1"));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_in_a_turn_stays_with_codly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen { turns_stay_open: true, ..Default::default() }));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;

    core.send_chat(&format!("agent:{id}"), "a long job").await.unwrap();
    wait_for("the turn to start", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    wait_for("its session", async || agent_now(&core, &id).await.session_id.map(|_| ())).await;

    let refused = core.take_over_agent(&id).await.unwrap_err().to_string();
    assert!(refused.contains("is in the middle of a turn"), "{refused}");
    let state = core.agent_takeover(&id).await.unwrap();
    assert!(state.takeover.is_none() && state.blocked.as_deref() == Some(refused.as_str()), "{state:?}");
    assert_eq!(seen.lock().unwrap().shutdowns, 0, "its turn goes on");
    // Nothing was marked: a second message still reaches it.
    core.send_chat(&format!("agent:{id}"), "one more thing").await.unwrap();

    // Stopped, it can go.
    core.stop_agent(&id).await.unwrap();
    wait_for("its process to end", async || agent_now(&core, &id).await.run_id.is_none().then_some(())).await;
    let took = core.take_over_agent(&id).await.unwrap().takeover.expect("it is in your terminal");
    assert_eq!(took.command, "claude --resume sess-1");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_goes_only_when_its_task_stopped_and_the_retry_waits_for_the_give_back() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen { turns_stay_open: true, ..Default::default() }));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let task = core.create_task("Write the greeting", "Create hello.txt containing hello").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_for("the worker's first turn", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    let worker = task_now(&core, &task.id).await.assignee.expect("the task has a worker");
    let name = agent_now(&core, &worker).await.name;
    wait_for("its session", async || agent_now(&core, &worker).await.session_id.map(|_| ())).await;

    // The task runs: it is Codly's to run.
    let refused = core.take_over_agent(&worker).await.unwrap_err().to_string();
    assert!(refused.contains("which is still running"), "{refused}");

    // You stop the worker: the task waits for you, and its session can go to your terminal.
    core.stop_agent(&worker).await.unwrap();
    wait_for("the task to wait for you", async || (task_now(&core, &task.id).await.status == TaskStatus::Blocked).then_some(())).await;
    wait_for("its process to end", async || agent_now(&core, &worker).await.run_id.is_none().then_some(())).await;
    let took = core.take_over_agent(&worker).await.unwrap().takeover.expect("it is in your terminal");
    assert_eq!(took.command, "claude --resume sess-1");
    assert_eq!(Some(took.cwd.as_str()), task_now(&core, &task.id).await.worktree.as_deref(), "the terminal opens where the work is");

    // A retry meanwhile starts nothing: the task waits for its worker.
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let waits = format!("waiting: {name} (assigned) is busy");
    wait_for("the task to wait for its worker", async || (task_now(&core, &task.id).await.status_reason.as_deref() == Some(waits.as_str())).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new"]);

    // Given back, the worker goes on in the same session.
    core.give_back_agent(&worker).await.unwrap();
    wait_for("the worker to go on", async || (seen.lock().unwrap().prompts.len() == 2).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cli_without_a_resume_command_stays_with_codly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let id = core.hire_agent(Provider::Gemini, None, None).await.unwrap().id;
    say(&core, &seen, &id, "hello").await;
    wait_for("its session", async || agent_now(&core, &id).await.session_id.map(|_| ())).await;

    let why = "command that opens one session by its id";
    let state = core.agent_takeover(&id).await.unwrap();
    assert!(state.takeover.is_none() && state.blocked.as_deref().is_some_and(|b| b.contains(why)), "{state:?}");
    assert!(core.take_over_agent(&id).await.unwrap_err().to_string().contains(why));
    assert_eq!(seen.lock().unwrap().shutdowns, 0, "its process is left alone");
    // Not marked: it still takes messages.
    say(&core, &seen, &id, "still here?").await;
    core.shutdown().await;
}
