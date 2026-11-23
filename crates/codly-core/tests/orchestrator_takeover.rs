//! "Open in my terminal", with a fake agent in place of the CLI. An agent between turns goes to
//! your terminal: Codly stops its own process for the session, and starts none until you give
//! the agent back. Then its next message resumes the same session. An agent in a turn, one
//! whose process is starting, or one on a task that still runs, does not go.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::takeover::{TakeoverState, with_cd};
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc};

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
    /// Knob: a process that starts is up only after this is notified.
    start_waits: Option<Arc<Notify>>,
    /// Knob: a process asked to shut down exits only after this is notified.
    exit_waits: Option<Arc<Notify>>,
    /// The MCP socket and token of the Lead's run: what its CLI calls Codly's tools with.
    lead_mcp: Option<(String, String)>,
}

/// A fake agent whose sessions are "sess-1", "sess-2"… and whose turns end at once (or stay
/// open, with the knob).
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let (session, start_waits) = {
                let mut s = seen.lock().unwrap();
                if spec.role == Role::Lead {
                    let mcp = spec.mcp.as_ref().expect("Codly gives every agent its MCP server");
                    let env = |k: &str| mcp.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
                    s.lead_mcp = Some((env("CODLY_MCP_SOCK"), env("CODLY_MCP_TOKEN")));
                }
                let session = match &spec.session {
                    SessionMode::Resume { session_id } => {
                        s.starts.push(format!("resume {session_id}"));
                        session_id.clone()
                    }
                    SessionMode::New { .. } => {
                        s.starts.push("new".into());
                        format!("sess-{}", s.starts.iter().filter(|start| *start == "new").count())
                    }
                };
                (session, s.start_waits.clone())
            };
            if let Some(gate) = start_waits {
                gate.notified().await;
            }
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
                            let exit_waits = {
                                let mut s = seen.lock().unwrap();
                                s.shutdowns += 1;
                                s.exit_waits.clone()
                            };
                            if let Some(gate) = exit_waits {
                                gate.notified().await;
                            }
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

async fn start(data: &Path, seen: &Arc<Mutex<Seen>>, repo: &Path) -> Arc<Core> {
    start_with(data, seen, repo, vec![]).await
}

/// `start`, with a team.
async fn start_with(data: &Path, seen: &Arc<Mutex<Seen>>, repo: &Path, team: Vec<TeamMember>) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: data.to_path_buf(), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![cli(Provider::Claude), cli(Provider::Gemini)]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let settings = Settings { lead_followups: false, worker_providers: vec![Provider::Claude], team, ..Default::default() };
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

/// Call one of Codly's tools the way the Lead's CLI does, over the MCP socket of its run.
async fn lead_tool(seen: &Arc<Mutex<Seen>>, name: &str, args: Value) -> Value {
    let (sock, token) = seen.lock().unwrap().lead_mcp.clone().expect("a Lead is running");
    let mut s = UnixStream::connect(sock).await.unwrap();
    let req = json!({ "token": token, "tool": name, "args": args });
    s.write_all(format!("{req}\n").as_bytes()).await.unwrap();
    let mut lines = BufReader::new(s).lines();
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
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
    // Asked again, the same hand-over. It is the one agent the window marks.
    assert_eq!(core.take_over_agent(&id).await.unwrap().takeover.as_ref(), Some(&took));
    assert_eq!(core.takeovers().await.unwrap(), [took.clone()]);
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
    assert_eq!(core.takeovers().await.unwrap(), [took.clone()]);
    assert_eq!(core.agent_takeover(&id).await.unwrap().takeover, Some(took));
    assert!(core.send_chat(&format!("agent:{id}"), "and now?").await.is_err());
    assert!(seen.lock().unwrap().starts.is_empty());

    // Give it back: the next message resumes the same session.
    assert_eq!(core.give_back_agent(&id).await.unwrap(), TakeoverState::default());
    assert!(core.takeovers().await.unwrap().is_empty());
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

    // A retry meanwhile starts nothing: the task waits for its worker, and says how to end that.
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let waits = format!("waiting: {name} (assigned) is in your terminal: give it back");
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

/// A start on its way is a turn on its way: the message it starts for is not lost to a
/// take-over, and the session never has Codly's new process and your terminal at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_whose_process_is_starting_stays_with_codly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;
    say(&core, &seen, &id, "hello").await;
    core.stop_agent(&id).await.unwrap();
    wait_for("its process to end", async || agent_now(&core, &id).await.run_id.is_none().then_some(())).await;
    let stopped = seen.lock().unwrap().shutdowns;

    // Your next message starts its process again, and the start takes a while.
    let up = Arc::new(Notify::new());
    seen.lock().unwrap().start_waits = Some(up.clone());
    let sending = tokio::spawn({
        let (core, thread) = (core.clone(), format!("agent:{id}"));
        async move { core.send_chat(&thread, "one more thing").await }
    });
    wait_for("the start to be on its way", async || (seen.lock().unwrap().starts.len() == 2).then_some(())).await;

    let refused = core.take_over_agent(&id).await.unwrap_err().to_string();
    assert!(refused.contains("is in the middle of a turn"), "{refused}");
    let state = core.agent_takeover(&id).await.unwrap();
    assert!(state.takeover.is_none() && state.blocked.as_deref() == Some(refused.as_str()), "{state:?}");
    assert!(core.takeovers().await.unwrap().is_empty());
    assert_eq!(seen.lock().unwrap().shutdowns, stopped, "the start is left alone");

    // Nothing was marked: the start goes on, on the same session, and the message arrives.
    up.notify_one();
    sending.await.unwrap().unwrap();
    wait_for("the message to arrive", async || seen.lock().unwrap().prompts.iter().any(|p| p.ends_with("one more thing")).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"]);
    core.shutdown().await;
}

/// The agent is in your terminal only when Codly's process for its session is gone. Until
/// then nothing says it is there and no terminal opens for it; if the process does not stop,
/// the agent stays with Codly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_is_in_your_terminal_only_once_its_process_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo).await;
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;
    say(&core, &seen, &id, "hello").await;
    at_rest(&core, &id).await;
    let name = agent_now(&core, &id).await.name;

    // Its process takes its time to exit.
    let exit = Arc::new(Notify::new());
    seen.lock().unwrap().exit_waits = Some(exit.clone());
    let taking = tokio::spawn({
        let (core, id) = (core.clone(), id.clone());
        async move { core.take_over_agent(&id).await }
    });
    wait_for("its process to be asked to stop", async || (seen.lock().unwrap().shutdowns == 1).then_some(())).await;

    // While it stops: not in your terminal yet, and nothing starts for it.
    let state = core.agent_takeover(&id).await.unwrap();
    assert_eq!(state, TakeoverState { takeover: None, blocked: Some(format!("{name} is stopping: one moment")) });
    assert!(core.takeovers().await.unwrap().is_empty());
    assert!(core.send_chat(&format!("agent:{id}"), "are you there?").await.is_err());

    // It did not stop in time: the agent stays with Codly, and nothing says it is in your terminal.
    let failed = taking.await.unwrap().unwrap_err().to_string();
    assert_eq!(failed, format!("{name} did not stop in time; try again"));
    assert!(core.agent_takeover(&id).await.unwrap().takeover.is_none());
    assert!(core.takeovers().await.unwrap().is_empty());

    // Its process is gone at last: now it goes.
    exit.notify_one();
    wait_for("its process to end", async || agent_now(&core, &id).await.run_id.is_none().then_some(())).await;
    let took = core.take_over_agent(&id).await.unwrap().takeover.expect("it is in your terminal");
    assert_eq!(core.takeovers().await.unwrap(), [took]);
    assert_eq!(seen.lock().unwrap().starts, ["new"]);
    core.shutdown().await;
}

/// The guard of every start, past the ones in front of it: a Lead's message to a team member
/// starts the member's process, and must not while the member's session is in your terminal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_leads_message_starts_nothing_for_a_member_in_your_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let team = vec![TeamMember { name: "Ana".into(), role: "Backend".into(), ..Default::default() }];
    let core = start_with(&dir.path().join("data"), &seen, &repo, team).await;
    let ana = wait_for("the team's agent", async || core.snapshot().await.unwrap().agents.into_iter().find(|a| a.member.is_some())).await.id;
    say(&core, &seen, &ana, "hello").await;
    assert_eq!(agent_now(&core, &ana).await.session_id.as_deref(), Some("sess-1"));
    // A Lead, for its tools.
    core.send_chat("lead", "hi").await.unwrap();
    wait_for("the Lead", async || seen.lock().unwrap().lead_mcp.clone()).await;

    at_rest(&core, &ana).await;
    core.take_over_agent(&ana).await.unwrap().takeover.expect("Ana is in your terminal");
    let starts = seen.lock().unwrap().starts.clone();
    assert_eq!(starts, ["new", "new"]);

    // The Lead's message is refused with the reason, and no process starts on Ana's session.
    let message = json!({ "to": "Ana", "text": "What did you change?" });
    let reply = lead_tool(&seen, "codly_message_member", message.clone()).await;
    let refused = reply["error"].as_str().expect("the tool call is refused").to_string();
    assert!(refused.contains("Ana is in your terminal: give it back first"), "{refused}");
    assert_eq!(seen.lock().unwrap().starts, starts);
    assert_eq!(agent_now(&core, &ana).await.session_id.as_deref(), Some("sess-1"), "its session is kept");

    // Given back, the same message resumes that session.
    core.give_back_agent(&ana).await.unwrap();
    let reply = lead_tool(&seen, "codly_message_member", message).await;
    assert_eq!(reply["ok"], true, "{reply}");
    wait_for("Ana to read it", async || seen.lock().unwrap().prompts.iter().any(|p| p.contains("What did you change?")).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "new", "resume sess-1"]);
    assert_eq!(agent_now(&core, &ana).await.session_id.as_deref(), Some("sess-1"));
    core.shutdown().await;
}
