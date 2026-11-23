//! A resume that fails is never silent. With a fake agent in place of the CLI: a 1:1 agent's
//! session cannot be resumed, Codly starts a new one, and the conversation says so with the
//! reason. A resume that works says nothing. A task's worker says it the same way, and the
//! decision is recorded on its task. A conversation the CLI no longer has is said as that, for
//! a 1:1 agent, a Lead and a team member. And when no new conversation starts either, the old
//! one is kept. A start that Codly itself refused says nothing against the conversation, and
//! no new one is tried: a worker keeps its session when the sandbox is not on, a team member
//! and a Lead keep theirs while the folder's own files are not trusted.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::claude::SandboxNotOn;
use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionGone, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::trust::TrustChoice;
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

fn claude() -> CliInfo {
    cli(Provider::Claude)
}

const GONE: &str = "claude exited during startup (code 1): No conversation found with session ID: sess-1";
const SIGNED_OUT: &str = "claude exited during startup (code 1): Not logged in · Please run /login";

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// How each run was asked to start, in order: "new", or "resume <session id>".
    starts: Vec<String>,
    /// The folder the last run was asked to start in.
    cwd: Option<PathBuf>,
    /// Knob: a resume fails to start, with the CLI's words only.
    resume_fails: bool,
    /// Knob: a resume is not started because Claude Code's sandbox is not on, as the Claude
    /// adapter reports it (`SandboxNotOn`).
    resume_refused: bool,
    /// Knob: a resume fails to start as the Claude adapter reports a conversation the CLI no
    /// longer has (`SessionGone`).
    resume_gone: bool,
    /// Knob: a new session does not start either (the CLI is signed out).
    new_fails: bool,
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
                s.cwd = Some(spec.cwd.clone());
                match &spec.session {
                    SessionMode::Resume { session_id } => {
                        s.starts.push(format!("resume {session_id}"));
                        if s.resume_refused {
                            return Err(SandboxNotOn("Claude Code did not turn its sandbox on, so Codly did not start this agent".into()).into());
                        }
                        if s.resume_gone {
                            return Err(anyhow::anyhow!(GONE).context(SessionGone));
                        }
                        if s.resume_fails {
                            return Err(anyhow::anyhow!(GONE));
                        }
                        session_id.clone()
                    }
                    SessionMode::New { .. } => {
                        s.starts.push("new".into());
                        if s.new_fails {
                            return Err(anyhow::anyhow!(SIGNED_OUT));
                        }
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
    codly_said_in(core, &format!("agent:{id}")).await
}

/// What Codly itself said in a thread.
async fn codly_said_in(core: &Core, thread: &str) -> Vec<String> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.author == "codly" && message.thread == thread => Some(message.text),
            _ => None,
        })
        .collect()
}

/// A core on a new repo, with the fake agent in place of the Claude CLI.
async fn core_with(dir: &Path, seen: &Arc<Mutex<Seen>>) -> Arc<Core> {
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
    core
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

/// The Claude adapter's typed start error: the CLI said that it no longer has the
/// conversation. The thread says exactly that, and the new conversation is the agent's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conversation_the_cli_no_longer_has_is_said_as_that() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = core_with(dir.path(), &seen).await;
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let id = agent.id.clone();
    say(&core, &seen, &id, "hello").await;
    core.stop_agent(&id).await.unwrap();

    seen.lock().unwrap().resume_gone = true;
    say(&core, &seen, &id, "and now?").await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "new"]);
    let name = &agent.name;
    let notice = format!("Claude Code no longer has {name}'s earlier conversation. A new one started: {name} does not know what was said before.");
    wait_for("the notice", async || codly_said(&core, &id).await.contains(&notice).then_some(())).await;
    assert_eq!(codly_said(&core, &id).await, [notice], "said once");
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-2"));
    core.shutdown().await;
}

/// A resume fails and a new session does not start either (the CLI is signed out): nothing
/// took the place of the conversation, so it is kept, the message fails with the reason, and
/// the next message resumes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conversation_is_kept_when_no_new_one_starts_either() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = core_with(dir.path(), &seen).await;
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;
    say(&core, &seen, &id, "hello").await;
    core.stop_agent(&id).await.unwrap();

    {
        let mut s = seen.lock().unwrap();
        (s.resume_fails, s.new_fails) = (true, true);
    }
    let e = core.send_chat(&format!("agent:{id}"), "are you there?").await.unwrap_err();
    assert!(format!("{e:#}").contains("Not logged in"), "{e:#}");
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "new"]);
    assert_eq!(agent_now(&core, &id).await.session_id.as_deref(), Some("sess-1"), "the conversation is still the agent's");
    assert!(codly_said(&core, &id).await.is_empty(), "no new conversation started: nothing to say about one");

    // Signed in again: the same conversation goes on.
    {
        let mut s = seen.lock().unwrap();
        (s.resume_fails, s.new_fails) = (false, false);
    }
    say(&core, &seen, &id, "and now?").await;
    assert_eq!(seen.lock().unwrap().starts.last().map(String::as_str), Some("resume sess-1"));
    assert!(codly_said(&core, &id).await.is_empty());
    core.shutdown().await;
}

/// A Lead goes the same way as a 1:1 agent: its request thread says that its conversation is
/// gone, and a new one takes the message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lead_whose_conversation_is_gone_says_so_in_its_request_thread() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = core_with(dir.path(), &seen).await;
    core.update_settings(Settings { lead_followups: false, ..Default::default() }).await.unwrap();
    let lead = async || core.snapshot().await.unwrap().agents.into_iter().find(|a| a.role == Role::Lead);

    core.send_chat("lead", "hello").await.unwrap();
    wait_for("the Lead's first turn", async || (!seen.lock().unwrap().prompts.is_empty()).then_some(())).await;
    let first = wait_for("the Lead to rest", async || lead().await.filter(|l| l.status == AgentStatus::Idle)).await;
    assert_eq!(first.session_id.as_deref(), Some("sess-1"));
    core.stop_agent(&first.id).await.unwrap();
    wait_for("the Lead's process to end", async || lead().await.filter(|l| l.run_id.is_none())).await;

    seen.lock().unwrap().resume_gone = true;
    core.send_chat("lead", "are you there?").await.unwrap();
    wait_for("the Lead's new conversation", async || (seen.lock().unwrap().starts.len() == 3).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "new"]);
    let name = &first.name;
    let notice = format!("Claude Code no longer has {name}'s earlier conversation. A new one started: {name} does not know what was said before.");
    wait_for("the notice", async || codly_said_in(&core, "lead").await.contains(&notice).then_some(())).await;
    wait_for("the new conversation on the Lead", async || lead().await.filter(|l| l.session_id.as_deref() == Some("sess-2"))).await;
    core.shutdown().await;
}

/// A core on a new repo with these CLIs and settings, and the fake agent in place of each CLI.
async fn core_on(dir: &Path, seen: &Arc<Mutex<Seen>>, providers: &[Provider], settings: Settings) -> Arc<Core> {
    let repo = temp_repo(dir);
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(providers.iter().map(|p| cli(*p)).collect()),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { lead_followups: false, ..settings }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
}

/// A task whose worker was in its first turn (session "sess-1") when you stopped it: the task
/// waits for you. Returns (task, worker id).
async fn stopped_task(core: &Core, seen: &Arc<Mutex<Seen>>) -> (Task, Id) {
    let task = core.create_task("Write the greeting", "Create hello.txt containing hello").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_for("the worker's first turn", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    let worker = task_now(core, &task.id).await.assignee.expect("the task has a worker");
    core.stop_agent(&worker).await.unwrap();
    let task = wait_for("the task to wait for you", async || Some(task_now(core, &task.id).await).filter(|t| t.status == TaskStatus::Blocked)).await;
    (task, worker)
}

/// A worker's resume that Codly itself did not let start (here: Claude Code's sandbox is not
/// on) says nothing against the session. No new one is tried, nothing is said about a lost
/// conversation, and the next start resumes the same session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_whose_resume_codly_refused_keeps_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { turns_stay_open: true, ..Default::default() }));
    let core = core_on(dir.path(), &seen, &[Provider::Claude], Settings::default()).await;
    let (task, worker) = stopped_task(&core, &seen).await;

    seen.lock().unwrap().resume_refused = true;
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let refused = |t: &Task| t.status_reason.as_deref().is_some_and(|r| r.contains("could not start the worker: Claude Code did not turn its sandbox on"));
    wait_for("the refused start on the task", async || Some(task_now(&core, &task.id).await).filter(refused)).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"], "no new session was tried");
    assert!(codly_said(&core, &worker).await.is_empty(), "no conversation was lost: nothing to say about one");

    // The sandbox is on again: the start Codly tries next resumes the same session.
    seen.lock().unwrap().resume_refused = false;
    wait_for("the worker's next turn", async || (seen.lock().unwrap().prompts.len() == 2).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "resume sess-1"]);
    assert!(codly_said(&core, &worker).await.is_empty());
    core.shutdown().await;
}

/// The team's agent called `name`.
async fn member(core: &Core, name: &str) -> Agent {
    wait_for("the team's agent", async || core.snapshot().await.unwrap().agents.into_iter().find(|a| a.member.is_some() && a.name == name)).await
}

/// A team member goes the same way as a 1:1 agent: its own thread says that its conversation
/// is gone, and a new one takes the message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_team_member_whose_conversation_is_gone_says_so_in_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let team = vec![TeamMember { name: "Ana".into(), role: "Backend".into(), ..Default::default() }];
    let core = core_on(dir.path(), &seen, &[Provider::Claude], Settings { team, ..Default::default() }).await;
    let ana = member(&core, "Ana").await.id;
    say(&core, &seen, &ana, "hello").await;
    assert_eq!(agent_now(&core, &ana).await.session_id.as_deref(), Some("sess-1"));
    core.stop_agent(&ana).await.unwrap();

    seen.lock().unwrap().resume_gone = true;
    say(&core, &seen, &ana, "and now?").await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1", "new"]);
    let notice = "Claude Code no longer has Ana's earlier conversation. A new one started: Ana does not know what was said before.".to_string();
    wait_for("the notice", async || codly_said(&core, &ana).await.contains(&notice).then_some(())).await;
    assert_eq!(codly_said(&core, &ana).await, [notice], "said once");
    assert_eq!(agent_now(&core, &ana).await.session_id.as_deref(), Some("sess-2"));
    core.shutdown().await;
}

/// A team member's CLI is not started in its worktree while the worktree has configuration
/// for it that you have not trusted. The member keeps its conversation: the message fails
/// with the reason, nothing is started, and once you trusted the files the same conversation
/// goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_team_member_keeps_its_conversation_while_its_folder_waits_for_your_trust() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let team = vec![TeamMember { name: "Gil".into(), role: "Backend".into(), provider: Provider::Gemini, ..Default::default() }];
    let core = core_on(dir.path(), &seen, &[Provider::Claude, Provider::Gemini], Settings { team, ..Default::default() }).await;
    let gil = member(&core, "Gil").await.id;
    say(&core, &seen, &gil, "hello").await;
    core.stop_agent(&gil).await.unwrap();
    let worktree = seen.lock().unwrap().cwd.clone().expect("the member's run started in its worktree");
    std::fs::create_dir_all(worktree.join(".gemini")).unwrap();
    std::fs::write(worktree.join(".gemini/settings.json"), r#"{ "mcpServers": {} }"#).unwrap();

    let refused = core.send_chat(&format!("agent:{gil}"), "are you there?").await.unwrap_err().to_string();
    assert!(refused.contains("Gemini CLI is not started in this folder"), "{refused}");
    assert_eq!(seen.lock().unwrap().starts, ["new"], "nothing was started, in the old conversation or in a new one");
    assert_eq!(agent_now(&core, &gil).await.session_id.as_deref(), Some("sess-1"), "the conversation is still the member's");

    let asked = core.workspace_trust(None).await.unwrap();
    assert_eq!(asked.worktrees.len(), 1, "the worktree's files wait for your choice");
    core.workspace_trust(Some(TrustChoice { trust: Some(asked.worktrees[0].fingerprint.clone()) })).await.unwrap();
    say(&core, &seen, &gil, "and now?").await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"]);
    core.shutdown().await;
}

/// A Lead on a CLI that Codly does not start in your folder while the folder has configuration
/// for it that you have not trusted. The Lead keeps its conversation: the message fails with
/// the reason (the request thread says it too), nothing is started, and once you trusted the
/// files the same conversation goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lead_keeps_its_conversation_while_your_folder_waits_for_your_trust() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let settings = Settings { lead_provider: Some(Provider::Gemini), ..Default::default() };
    let core = core_on(dir.path(), &seen, &[Provider::Claude, Provider::Gemini], settings).await;
    let lead = async || core.snapshot().await.unwrap().agents.into_iter().find(|a| a.role == Role::Lead);

    core.send_chat("lead", "hello").await.unwrap();
    wait_for("the Lead's first turn", async || (!seen.lock().unwrap().prompts.is_empty()).then_some(())).await;
    let first = wait_for("the Lead to rest", async || lead().await.filter(|l| l.status == AgentStatus::Idle)).await;
    assert_eq!((first.provider, first.session_id.as_deref()), (Provider::Gemini, Some("sess-1")));
    core.stop_agent(&first.id).await.unwrap();
    wait_for("the Lead's process to end", async || lead().await.filter(|l| l.run_id.is_none())).await;
    // In your folder the repo's own configuration is what git tracks. (`-f`: a machine's own
    // git ignore list may name the file.)
    let folder = seen.lock().unwrap().cwd.clone().expect("the Lead's run started in your folder");
    std::fs::create_dir_all(folder.join(".gemini")).unwrap();
    std::fs::write(folder.join(".gemini/settings.json"), r#"{ "mcpServers": {} }"#).unwrap();
    git(&folder, &["add", "-f", ".gemini/settings.json"]);
    git(&folder, &["commit", "-qm", "agent config"]);

    // The start error of a Lead comes back from the send call.
    let refused = core.send_chat("lead", "are you there?").await.unwrap_err().to_string();
    assert!(refused.contains("Gemini CLI is not started in this folder"), "{refused}");
    assert_eq!(seen.lock().unwrap().starts, ["new"], "nothing was started, in the old conversation or in a new one");
    assert_eq!(lead().await.expect("the Lead").session_id.as_deref(), Some("sess-1"), "the conversation is still the Lead's");
    let said = codly_said_in(&core, "lead").await;
    assert!(said.iter().any(|text| text.starts_with("The Lead could not start: Gemini CLI is not started in this folder")), "{said:?}");

    let asked = core.workspace_trust(None).await.unwrap();
    assert!(asked.fingerprint.is_some(), "your folder's files wait for your choice");
    core.workspace_trust(Some(TrustChoice { trust: asked.fingerprint })).await.unwrap();
    core.send_chat("lead", "and now?").await.unwrap();
    wait_for("the Lead's next start", async || (seen.lock().unwrap().starts.len() == 2).then_some(())).await;
    assert_eq!(seen.lock().unwrap().starts, ["new", "resume sess-1"]);
    let said = codly_said_in(&core, "lead").await;
    assert!(!said.iter().any(|text| text.contains("earlier conversation")), "no conversation was lost: {said:?}");
    core.shutdown().await;
}
