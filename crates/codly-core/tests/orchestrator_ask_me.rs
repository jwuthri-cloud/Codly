//! "Ask me" end to end with fake agents in a temp git repo: a worker raises a blocking card
//! with codly_ask_user, its task parks as Blocked ("waiting for you") and is not gated or
//! merged; your answer resumes the same worker with a [User] prompt, and the plan finishes.
//! A heads-up card does not stop the worker, and dismissing a blocking card unblocks it too.
//! A task that waits holds no worker slot, and an answer for a worker that is gone (Codly
//! restarted) or that has no slot to go on in is kept for the worker's next run of the task.
//! A worker that takes the task over in a new session reads what you said in its brief.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::blockers::{Blocker, BlockerSeverity, BlockerStatus};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// `Core::shutdown`, and every run exited when it was told to stop. A fake agent that ignores
/// the stop fails here, in its own test: else the shutdown's last resort kills every process
/// group this test process started, the check commands of the tests beside it too.
async fn shut_down(core: &Core) {
    core.shutdown().await;
    assert_eq!(core.runs_given_up(), 0, "a run outlived the shutdown: its fake did not exit when told to stop");
}

/// Your own git settings (`~/.gitconfig`, the system file) reach no git of this test: neither
/// the core's (`core_env`) nor the test's own, which commits as a fixed author.
const NO_GIT_CONFIG: [(&str, &str); 2] = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")];

/// Who the test's own commits are by, whatever the repository says.
const TEST_AUTHOR: [(&str, &str); 4] =
    [("GIT_AUTHOR_NAME", "Test"), ("GIT_AUTHOR_EMAIL", "test@example.com"), ("GIT_COMMITTER_NAME", "Test"), ("GIT_COMMITTER_EMAIL", "test@example.com")];

/// The environment the core runs with: this process's, without your git settings.
fn core_env() -> std::collections::HashMap<String, String> {
    std::env::vars().chain(NO_GIT_CONFIG.map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn sh(dir: &Path, args: &[&str]) {
    let out = Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.name", "Test"]);
    sh(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
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

#[derive(Clone)]
struct FakeTurn {
    spec: Arc<SpawnSpec>,
    prompt: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
}

impl FakeTurn {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.spec.agent_id.clone(), run_id: self.spec.run_id.clone(), kind });
    }

    fn say(&self, text: &str) {
        self.emit(EventKind::Message { item_id: new_id(), text: text.into(), phase: MessagePhase::Final });
    }

    /// Call a codly_* tool exactly like the stdio bridge does.
    async fn tool(&self, name: &str, args: Value) -> Value {
        let mcp = self.spec.mcp.as_ref().expect("Codly gives every agent its MCP server");
        let env = |k: &str| mcp.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
        let mut s = UnixStream::connect(env("CODLY_MCP_SOCK")).await.unwrap();
        let req = json!({ "token": env("CODLY_MCP_TOKEN"), "tool": name, "args": args });
        s.write_all(format!("{req}\n").as_bytes()).await.unwrap();
        let mut lines = BufReader::new(s).lines();
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
type Turns = Arc<Mutex<Vec<(String, String)>>>;

struct FakeSpawner {
    lead: Behavior,
    worker: Behavior,
    /// (agent id, prompt) of every turn.
    turns: Turns,
}

/// How long a turn still has to end once its run is told to stop: well inside the 4 s that
/// `Core::shutdown` gives every run.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Run a turn to its end, like a CLI: what comes meanwhile waits for that end. A stop gives the
/// turn `STOP_GRACE` more to end (the run then exits after it), else the run exits in the middle
/// of it: false. A turn that waits for the test and never looks at its run's commands would
/// keep the run alive through `Core::shutdown`, whose last resort kills every process group
/// this test process started, the check commands of the tests beside it too.
async fn end_turn(
    mut turn: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    rx: &mut mpsc::UnboundedReceiver<AgentCmd>,
    waiting: &mut VecDeque<AgentCmd>,
) -> bool {
    loop {
        tokio::select! {
            _ = &mut turn => return true,
            cmd = rx.recv() => match cmd {
                Some(AgentCmd::Shutdown) => break,
                Some(cmd) => waiting.push_back(cmd),
                // Nobody is left to tell it anything: the turn runs to its end.
                None => {
                    turn.await;
                    return true;
                }
            },
        }
    }
    let ended = tokio::time::timeout(STOP_GRACE, turn).await.is_ok();
    if ended {
        waiting.push_back(AgentCmd::Shutdown);
    }
    ended
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let behavior = if spec.role == Role::Lead { self.lead.clone() } else { self.worker.clone() };
        let turns = self.turns.clone();
        Box::pin(async move {
            let session = match &spec.session {
                SessionMode::New { session_id } => session_id.clone().unwrap_or_else(|| format!("fake-{}", spec.run_id)),
                SessionMode::Resume { session_id } => session_id.clone(),
            };
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let spec = Arc::new(spec);
            let send = {
                let (events, spec) = (events.clone(), spec.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind });
                }
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: Some("fake-model".into()),
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(session),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let spec2 = spec.clone();
            tokio::spawn(async move {
                // What came while a turn ran: read when the turn is over, in order.
                let mut waiting = VecDeque::new();
                loop {
                    let cmd = match waiting.pop_front() {
                        Some(cmd) => cmd,
                        None => match rx.recv().await {
                            Some(cmd) => cmd,
                            None => break,
                        },
                    };
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            turns.lock().unwrap().push((spec2.agent_id.clone(), text.clone()));
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            send(EventKind::StatusChanged { status: AgentStatus::Thinking });
                            if !end_turn(behavior(FakeTurn { spec: spec2.clone(), prompt: text, events: events.clone() }), &mut rx, &mut waiting).await {
                                send(EventKind::Exited { code: Some(0) });
                                break;
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(5) });
                            send(EventKind::StatusChanged { status: AgentStatus::Idle });
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

/// A task that creates one file.
fn file_task(id: &str, file: &str) -> Value {
    json!({
        "id": id, "title": format!("Create {file}"), "goal": format!("Create {file} containing the greeting"),
        "consultant": { "title": "Backend" }, "acceptance_criteria": [format!("{file} exists")], "write_scope": [file],
        "depends_on": [], "verify": [format!("test -f {file}")], "size": "M",
    })
}

/// A Lead that turns a request into a one-task plan writing hello.txt.
fn planning_lead() -> Behavior {
    lead_with(json!({ "summary": "Add a hello file", "tasks": [file_task("t1", "hello.txt")] }))
}

/// A Lead that submits this plan for every request.
fn lead_with(plan: Value) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        let plan = plan.clone();
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                turn.say("Noted.");
                return;
            }
            let reply = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(reply["ok"], true, "{reply}");
            turn.say("Planned.");
        })
    })
}

async fn start(root: &Path, spawner: FakeSpawner) -> Arc<Core> {
    start_with(root, spawner, vec![claude()]).await
}

async fn start_with(root: &Path, spawner: FakeSpawner, clis: Vec<CliInfo>) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(core_env()), clis: Some(clis), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

async fn wait_for<T>(what: &str, secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn open_card(core: &Core) -> Option<Blocker> {
    core.list_blockers().await.unwrap().into_iter().find(|b| b.status == BlockerStatus::Open && b.severity == BlockerSeverity::Blocking)
}

/// A worker that asks (heads-up first, then a blocking question), stops, and finishes when
/// the answer or the skip arrives.
fn asking_worker(question: &'static str) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("Answer to your question") || turn.prompt.contains("skipped your question") {
                std::fs::write(turn.spec.cwd.join("hello.txt"), "hello\n").unwrap();
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created hello.txt.", "files_changed": ["hello.txt"] })).await;
                return;
            }
            let fyi = turn
                .tool("codly_ask_user", json!({ "title": "Tests are slow", "question": "FYI the suite takes 4 minutes.", "severity": "heads_up" }))
                .await;
            assert_eq!(fyi["result"]["status"], "noted", "{fyi}");
            let r = turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": question })).await;
            assert_eq!(r["result"]["status"], "waiting_for_user", "{r}");
            // Asking the same thing twice is one card.
            turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": question })).await;
            turn.say("Waiting for the user's answer.");
        })
    })
}

async fn run_until_card(dir: &Path, worker: Behavior) -> (Arc<Core>, Turns, Blocker) {
    let repo = temp_repo(dir);
    let turns: Turns = Arc::new(Mutex::new(vec![]));
    let core = start(dir, FakeSpawner { lead: planning_lead(), worker, turns: turns.clone() }).await;
    let settings = Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let card = wait_for("the blocking card", 30, async || open_card(&core).await).await;
    (core, turns, card)
}

async fn task_of(core: &Core) -> Task {
    core.snapshot().await.unwrap().tasks.first().unwrap().clone()
}

async fn plan_done(core: &Core) {
    wait_for("the plan to finish", 30, async || core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(())).await;
}

fn turns_of(turns: &Turns, agent: Option<&str>) -> Vec<String> {
    turns.lock().unwrap().iter().filter(|(a, _)| Some(a.as_str()) == agent).map(|(_, t)| t.clone()).collect()
}

async fn task_by_key(core: &Core, key: &str) -> Option<Task> {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key)
}

/// Your answer, as the worker reads it.
const ANSWER: &str = "[User] Answer to your question \"Which greeting?\": casual please";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answering_a_blocking_card_resumes_the_worker_and_the_plan_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let (core, turns, card) = run_until_card(dir.path(), asking_worker("Formal or casual hello?")).await;

    // One heads-up and one blocking card (the repeat was folded), both attributed to the worker.
    let cards = core.list_blockers().await.unwrap();
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert!(cards.iter().any(|b| b.severity == BlockerSeverity::HeadsUp && b.title == "Tests are slow"));
    assert_eq!(card.body, "Formal or casual hello?");
    // The card is saved first, then its task is parked: waited for.
    let task = wait_for("the task to wait for you", 10, async || Some(task_of(&core).await).filter(|t| t.status == TaskStatus::Blocked)).await;
    assert_eq!(card.task_id.as_deref(), Some(task.id.as_str()));
    assert!(task.status_reason.as_deref().unwrap().starts_with("waiting for you:"), "{task:?}");

    // The worker's turn has ended, yet the gate must not commit, verify or merge a parked task.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(task_of(&core).await.status, TaskStatus::Blocked);
    assert!(!core.snapshot().await.unwrap().events.iter().any(|e| matches!(e.kind, EventKind::Committed { .. })));

    // Respond & unblock.
    let answered = core.answer_blocker(&card.id, "casual please").await.unwrap();
    assert_eq!((answered.status, answered.answer.as_deref()), (BlockerStatus::Answered, Some("casual please")));
    assert!(core.answer_blocker(&card.id, "again").await.is_err(), "a card is answered once");
    plan_done(&core).await;
    let resumed = turns_of(&turns, task.assignee.as_deref());
    assert!(resumed.iter().any(|t| t.contains("[User] Answer to your question \"Which greeting?\": casual please")), "{resumed:?}");

    // The heads-up stays on the list until you dismiss it; that sends the worker nothing.
    let fyi = core.list_blockers().await.unwrap().into_iter().find(|b| b.severity == BlockerSeverity::HeadsUp).unwrap();
    assert_eq!(fyi.status, BlockerStatus::Open);
    assert_eq!(core.dismiss_blocker(&fyi.id).await.unwrap().status, BlockerStatus::Dismissed);
    assert_eq!(turns_of(&turns, task.assignee.as_deref()).len(), resumed.len());
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skipping_a_blocking_card_tells_the_worker_to_use_its_best_judgment() {
    let dir = tempfile::tempdir().unwrap();
    let (core, turns, card) = run_until_card(dir.path(), asking_worker("Formal or casual hello?")).await;
    let task = task_of(&core).await;
    assert_eq!(core.dismiss_blocker(&card.id).await.unwrap().status, BlockerStatus::Dismissed);
    plan_done(&core).await;
    let resumed = turns_of(&turns, task.assignee.as_deref());
    assert!(resumed.iter().any(|t| t.contains("skipped your question") && t.contains("best judgment")), "{resumed:?}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retrying_a_task_that_waits_for_your_answer_restarts_it() {
    let dir = tempfile::tempdir().unwrap();
    // Asks; back on the task after your Retry, it does the work when the test lets it.
    let go = Arc::new(tokio::sync::Notify::new());
    let wait = go.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move {
            if turn.prompt.contains("You are back on task") {
                wait.notified().await;
                std::fs::write(turn.spec.cwd.join("hello.txt"), "hello\n").unwrap();
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created hello.txt.", "files_changed": ["hello.txt"] })).await;
            } else if turn.prompt.contains("Answer to your question") {
                turn.say("Thanks.");
            } else {
                turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": "Formal or casual hello?" })).await;
                turn.say("Waiting for the user's answer.");
            }
        })
    });
    let (core, turns, card) = run_until_card(dir.path(), worker).await;
    let task = wait_for("the task to wait for you", 10, async || Some(task_of(&core).await).filter(|t| t.status == TaskStatus::Blocked)).await;
    let worker_id = task.assignee.clone().unwrap();

    // Retry instead of an answer: the worker that waits lets go of the task and starts again.
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_for("the worker to be back on the task", 20, async || {
        turns_of(&turns, Some(&worker_id)).iter().any(|t| t.contains("You are back on task")).then_some(())
    })
    .await;
    assert_eq!(task_of(&core).await.status, TaskStatus::Running);

    // The card is still open: its answer reaches the worker as its next turn.
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    go.notify_one();
    plan_done(&core).await;
    let seen = turns_of(&turns, Some(&worker_id));
    assert!(seen.iter().any(|t| t.contains("[User] Answer to your question \"Which greeting?\": casual please")), "{seen:?}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_that_waits_for_your_answer_holds_no_worker_slot() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    // t1's worker asks you something; t2's does its work when the test lets it.
    let go = Arc::new(tokio::sync::Notify::new());
    let wait = go.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move {
            let file = if turn.prompt.contains("Answer to your question") {
                "hello.txt"
            } else if turn.prompt.contains("[Codly] Task t1:") {
                turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": "Formal or casual hello?" })).await;
                return;
            } else {
                wait.notified().await;
                "bye.txt"
            };
            std::fs::write(turn.spec.cwd.join(file), "hi\n").unwrap();
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": format!("Created {file}."), "files_changed": [file] })).await;
        })
    });
    let turns: Turns = Arc::new(Mutex::new(vec![]));
    let plan = json!({ "summary": "Add two files", "tasks": [file_task("t1", "hello.txt"), file_task("t2", "bye.txt")] });
    let core = start(dir.path(), FakeSpawner { lead: lead_with(plan), worker, turns: turns.clone() }).await;
    // One worker slot in all.
    let one_slot =
        Settings { auto_run_plans: true, lead_followups: false, max_parallel: 1, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(one_slot).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add two files").await.unwrap();
    let card = wait_for("the blocking card", 30, async || open_card(&core).await).await;

    // t1 waits for you and nobody works on it: the one slot goes to t2.
    let t2 = wait_for("t2 to start", 20, async || task_by_key(&core, "t2").await.filter(|t| t.status == TaskStatus::Running)).await;
    wait_for("t2's worker to get its brief", 20, async || (!turns_of(&turns, t2.assignee.as_deref()).is_empty()).then_some(())).await;
    let t1 = task_by_key(&core, "t1").await.unwrap();
    assert_eq!(t1.status, TaskStatus::Blocked);
    assert_ne!(t1.assignee, t2.assignee);

    // You answer while t2 has the slot: t1 does not run on top of it. It waits for the slot,
    // with your answer kept for it.
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    let waits = wait_for("t1 to wait for the slot", 20, async || {
        task_by_key(&core, "t1").await.filter(|t| t.status_reason.as_deref().is_some_and(|r| r.contains("worker slots are busy")))
    })
    .await;
    assert_eq!(waits.status, TaskStatus::Ready);
    assert!(turns_of(&turns, t1.assignee.as_deref()).iter().all(|t| !t.contains("Answer to your question")), "nothing ran above the limit");
    assert_eq!(task_by_key(&core, "t2").await.unwrap().status, TaskStatus::Running);

    // t2 finishes: t1's worker is back, and the first thing it reads is your answer.
    go.notify_one();
    plan_done(&core).await;
    let seen = turns_of(&turns, t1.assignee.as_deref());
    assert!(seen.len() == 2 && seen[1].starts_with(ANSWER), "the brief, then the answer: {seen:?}");
    assert_eq!(task_by_key(&core, "t1").await.unwrap().assignee, t1.assignee, "the same worker went on");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn your_answer_survives_a_restart_and_reaches_the_restarted_worker() {
    let dir = tempfile::tempdir().unwrap();
    let (core, turns, card) = run_until_card(dir.path(), asking_worker("Formal or casual hello?")).await;
    let task = task_of(&core).await;
    let worker = task.assignee.clone();
    let before = turns_of(&turns, worker.as_deref()).len();
    // You quit Codly with the question open, and answer it the next day.
    shut_down(&core).await;
    drop(core);
    let again = || FakeSpawner { lead: planning_lead(), worker: asking_worker("Formal or casual hello?"), turns: turns.clone() };
    let repo = dir.path().join("repo");
    let core = start(dir.path(), again()).await;
    // Paused, so the task cannot start before Codly is closed once more.
    core.update_settings(Settings { paused: true, ..core.settings() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    assert_eq!(task_of(&core).await.status, TaskStatus::Blocked, "the question is still open after the restart");
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    let answered = task_of(&core).await;
    assert_eq!(answered.status, TaskStatus::Ready, "{:?}", answered.status_reason);
    assert_eq!(answered.goal, task.goal, "the answer is not written into the task's goal");
    shut_down(&core).await;
    drop(core);

    // The answer was kept in the store: the worker that starts now reads it first, in its session.
    let core = start(dir.path(), again()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    // The answered task would start once nothing is paused, so the restart waits for your
    // Resume first (a pause is no reason not to ask).
    core.resume_work().await.unwrap();
    core.update_settings(Settings { paused: false, ..core.settings() }).await.unwrap();
    plan_done(&core).await;
    let seen = turns_of(&turns, worker.as_deref());
    assert!(seen.len() == before + 1 && seen[before].starts_with(ANSWER), "{seen:?}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn your_answer_reaches_the_worker_that_takes_the_task_over_in_a_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    // The Claude worker asks. It reads your answer, and its account runs out before the work is
    // done. The Codex worker that takes the task over never saw the question (`taken_over`:
    // the first prompt it gets).
    let taken_over = Arc::new(Mutex::new(None::<String>));
    let worker = |first: Arc<Mutex<Option<String>>>| -> Behavior {
        Arc::new(move |turn: FakeTurn| {
            let first = first.clone();
            Box::pin(async move {
                if turn.spec.provider == Provider::Codex {
                    let _ = first.lock().unwrap().get_or_insert_with(|| turn.prompt.clone());
                    std::fs::write(turn.spec.cwd.join("hello.txt"), "hello\n").unwrap();
                    turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created hello.txt.", "files_changed": ["hello.txt"] })).await;
                } else if turn.prompt.contains("Answer to your question") {
                    turn.emit(EventKind::StatusChanged { status: AgentStatus::RateLimited });
                } else {
                    turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": "Formal or casual hello?" })).await;
                }
            })
        })
    };
    let turns: Turns = Arc::new(Mutex::new(vec![]));
    let spawner = || FakeSpawner { lead: planning_lead(), worker: worker(taken_over.clone()), turns: turns.clone() };
    let clis = || vec![claude(), CliInfo { provider: Provider::Codex, ..claude() }];
    let core = start_with(dir.path(), spawner(), clis()).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude, Provider::Codex], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let card = wait_for("the blocking card", 30, async || open_card(&core).await).await;
    let asker = task_of(&core).await.assignee;

    // You quit Codly with the question open and answer it the next day: the worker that asked
    // is restarted and reads your answer. Nothing waits for the task's next run after that.
    shut_down(&core).await;
    drop(core);
    let core = start_with(dir.path(), spawner(), clis()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.answer_blocker(&card.id, "casual please").await.unwrap();

    // Its account is on its limit: the task goes to the other provider, in a new session.
    plan_done(&core).await;
    let asked = turns_of(&turns, asker.as_deref());
    assert!(asked.last().is_some_and(|t| t.starts_with(ANSWER)), "the worker that asked read your answer: {asked:?}");
    let brief = taken_over.lock().unwrap().clone().expect("the other provider took the task over");
    assert!(brief.contains("[Codly] Task t1:"), "a new session starts from the brief: {brief}");
    assert!(brief.contains("- Answer to your question \"Which greeting?\": casual please"), "your answer is not in it: {brief}");
    assert!(brief.contains("hit its usage limit"), "{brief}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_office_counts_what_waits_for_you() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _turns, card) = run_until_card(dir.path(), asking_worker("Formal or casual hello?")).await;
    let first = core.snapshot().await.unwrap().workspace.unwrap();
    let other = temp_repo(&dir.path().join("other"));
    let second = core.open_workspace(other.to_str().unwrap()).await.unwrap();
    let needs = |ws: &str| core.offices().unwrap().iter().find(|o| o.id == ws).map(|o| o.needs_you);

    // The blocking card counts (the heads-up next to it does not), until you answer it.
    assert_eq!(needs(&first.id), Some(1));
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    assert_eq!(needs(&first.id), Some(0));

    // A plan that waits for your go counts too.
    core.update_settings(Settings { auto_run_plans: false, ..core.settings() }).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let awaits = async || core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == PlanStatus::AwaitingApproval);
    wait_for("the plan to await your go", 30, async || awaits().await.then_some(())).await;
    core.open_workspace(&first.repo_path).await.unwrap();
    assert_eq!(needs(&second.id), Some(1));
    shut_down(&core).await;
}

/// A Lead that asks you a blocking question about the request, then takes your answer.
fn asking_lead() -> Behavior {
    Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("Answer to your question") {
                turn.say("Thanks.");
                return;
            }
            let r = turn.tool("codly_ask_user", json!({ "title": "Which greeting?", "question": "Formal or casual hello?" })).await;
            assert_eq!(r["result"]["status"], "waiting_for_user", "{r}");
            turn.say("Waiting for your answer.");
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answering_a_leads_question_reaches_its_office() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (temp_repo(&dir.path().join("a")), temp_repo(&dir.path().join("b")));
    let turns: Turns = Arc::new(Mutex::new(vec![]));
    let idle: Behavior = Arc::new(|_turn: FakeTurn| Box::pin(async {}));
    let core = start(dir.path(), FakeSpawner { lead: asking_lead(), worker: idle, turns: turns.clone() }).await;
    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let card = wait_for("the Lead's card", 30, async || open_card(&core).await).await;

    // Another office is open when you answer: the answer goes to the Lead that asked, in its office.
    core.open_workspace(b.to_str().unwrap()).await.unwrap();
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    let here = core.snapshot().await.unwrap();
    assert!(here.agents.iter().all(|a| a.role != Role::Lead), "the open office got a Lead for it: {:?}", here.agents);
    assert!(here.chats.is_empty(), "and the answer in its chat: {:?}", here.chats);
    wait_for("the answer to reach the Lead that asked", 10, async || {
        turns_of(&turns, Some(&card.agent_id)).iter().any(|t| t.contains("Answer to your question \"Which greeting?\": casual please")).then_some(())
    })
    .await;
    shut_down(&core).await;
}
