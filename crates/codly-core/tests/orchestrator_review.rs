//! Send back, from a task's diff, end to end with fake agents in a temp git repo:
//! - a worker that still runs the task reads your review comments after its current turn, and
//!   the gate checks the work only once it answered them;
//! - a review that is under way stops, and the work with your changes gets a review of its own;
//! - a task the gate stopped starts again with the comments, with new fix rounds;
//! - a done task takes none, and what you asked stays in the task's record and in the
//!   worker's conversation;
//! - a reviewer of an earlier round that reports late does not decide the next round.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc};

/// What the diff view sends: for each comment, the file and line, the quoted line, the comment.
const COMMENTS: &str = "Review comments (1):\n\n1. `hello.txt:1`\n   > +hello\n   Say that it was reviewed.";
/// Passes only once the worker has answered the comments.
const VERIFY: &str = "grep -q reviewed hello.txt";

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").arg("-C").arg(dir).args(args).output().unwrap()
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [&["init", "-q", "-b", "main"][..], &["config", "user.name", "Test"], &["config", "user.email", "test@example.com"]] {
        assert!(git(&repo, args).status.success());
    }
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    assert!(git(&repo, &["add", "-A"]).status.success());
    assert!(git(&repo, &["commit", "-qm", "init"]).status.success());
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
    fn say(&self, text: &str) {
        let kind = EventKind::Message { item_id: new_id(), text: text.into(), phase: MessagePhase::Final };
        let _ = self.events.send(AdapterEvent { agent_id: self.spec.agent_id.clone(), run_id: self.spec.run_id.clone(), kind });
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

    /// The worker's turn: hello.txt says "reviewed" only once the review comments reached it.
    fn work(&self) {
        let text = if self.prompt.contains("Say that it was reviewed.") { "hello, reviewed\n" } else { "hello\n" };
        std::fs::write(self.spec.cwd.join("hello.txt"), text).unwrap();
    }

    async fn done(&self) {
        self.tool("codly_submit_report", json!({ "status": "done", "summary": "Did it." })).await;
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
/// (role, agent id, prompt) of every turn.
type Turns = Arc<Mutex<Vec<(Role, String, String)>>>;

struct FakeSpawner {
    worker: Behavior,
    turns: Turns,
    /// Some: a reviewer's CLI is up only once this says so (a slow start).
    reviewer_up: Option<Arc<Notify>>,
    /// Some: a reviewer that is told to stop mid-turn closes its turn only once this says so
    /// (a CLI that is slow to stop).
    reviewer_stops_late: Option<Arc<Notify>>,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let behavior = if spec.role == Role::Lead { lead() } else { self.worker.clone() };
        let turns = self.turns.clone();
        let wait = self.reviewer_up.clone().filter(|_| spec.role == Role::Reviewer);
        let stops_late = self.reviewer_stops_late.clone().filter(|_| spec.role == Role::Reviewer);
        Box::pin(async move {
            if let Some(up) = wait {
                up.notified().await;
            }
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
                // What arrived during a turn waits for its end, in order.
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
                            turns.lock().unwrap().push((spec2.role, spec2.agent_id.clone(), text.clone()));
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            send(EventKind::StatusChanged { status: AgentStatus::Thinking });
                            let mut turn = behavior(FakeTurn { spec: spec2.clone(), prompt: text, events: events.clone() });
                            // Told to shut down mid-turn, a CLI does not finish the turn: it is
                            // closed as interrupted and the process exits.
                            let stopped = loop {
                                tokio::select! {
                                    _ = &mut turn => break false,
                                    cmd = rx.recv() => match cmd {
                                        Some(AgentCmd::Shutdown) | None => break true,
                                        Some(cmd) => waiting.push_back(cmd),
                                    },
                                }
                            };
                            if stopped {
                                if let Some(late) = &stops_late {
                                    late.notified().await;
                                }
                                send(EventKind::TurnCompleted { status: TurnStatus::Interrupted, result_text: None, duration_ms: Some(5) });
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

/// A Lead that plans one task for every request: create hello.txt, checked by `VERIFY`.
fn lead() -> Behavior {
    Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                turn.say("Noted.");
                return;
            }
            let task = json!({
                "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt", "consultant": { "title": "Backend" },
                "acceptance_criteria": ["hello.txt is right"], "write_scope": ["hello.txt"], "depends_on": [], "verify": [VERIFY], "size": "M",
            });
            let reply = turn.tool("codly_submit_plan", json!({ "summary": "Add a hello file", "tasks": [task] })).await;
            assert_eq!(reply["ok"], true, "{reply}");
            turn.say("Planned.");
        })
    })
}

/// A core with one open repo, where a request runs its plan at once.
async fn start(root: &Path, worker: Behavior, turns: Turns) -> Arc<Core> {
    start_with(root, FakeSpawner { worker, turns, reviewer_up: None, reviewer_stops_late: None }, false).await
}

/// `start` with your own agents, and with or without a review of each task before it merges.
async fn start_with(root: &Path, spawner: FakeSpawner, review_gate: bool) -> Arc<Core> {
    let repo = temp_repo(root);
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, review_gate, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please do it").await.unwrap();
    core
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

async fn the_task(core: &Core) -> Option<Task> {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "t1")
}

async fn wait_task(core: &Core, status: TaskStatus) -> Task {
    wait_for(&format!("t1 to be {status:?}"), 40, async || the_task(core).await.filter(|t| t.status == status)).await
}

async fn wait_plan_done(core: &Core) {
    wait_for("the plan to be done", 40, async || {
        core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(())
    })
    .await
}

/// Every event of the office, oldest first.
async fn events(core: &Core) -> Vec<EventRow> {
    let mut all = core.events(None, None, 4000).await.unwrap();
    all.sort_by_key(|e| e.id);
    all
}

/// Whether each of Codly's verify runs passed, oldest first.
fn verified(events: &[EventRow]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { ok, .. } => Some(*ok),
            _ => None,
        })
        .collect()
}

/// (summary, detail) of every decision of yours, oldest first.
fn your_decisions(events: &[EventRow]) -> Vec<(String, Value)> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { method, summary, detail, .. } if method == "human" => Some((summary.clone(), detail.clone())),
            _ => None,
        })
        .collect()
}

fn worker_prompts(turns: &Turns) -> Vec<String> {
    turns.lock().unwrap().iter().filter(|t| t.0 == Role::Worker).map(|t| t.2.clone()).collect()
}

/// What you asked, as the worker's own conversation shows it.
async fn asked_in_chat(core: &Core, worker: &str) -> Vec<String> {
    let chats = core.snapshot().await.unwrap().chats;
    chats.into_iter().filter(|c| c.thread == format!("agent:{worker}") && c.author == "user").map(|c| c.text).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comments_reach_a_worker_that_still_runs_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // The worker's first turn ends only when `go` says so.
    let go = Arc::new(Notify::new());
    let worker: Behavior = {
        let go = go.clone();
        Arc::new(move |turn: FakeTurn| {
            let go = go.clone();
            Box::pin(async move {
                turn.work();
                if !turn.prompt.contains("sent it back") {
                    go.notified().await;
                }
                turn.done().await;
            })
        })
    };
    let core = start(dir.path(), worker, turns.clone()).await;

    let t = wait_task(&core, TaskStatus::Running).await;
    wait_for("the worker's first turn", 40, async || (!worker_prompts(&turns).is_empty()).then_some(())).await;
    let sent = core.send_back_task(&t.id, &format!("  {COMMENTS}\n")).await.unwrap();
    assert_eq!(sent.status, TaskStatus::ChangesRequested);
    assert!(sent.status_reason.as_deref().is_some_and(|r| r.starts_with("you asked for changes")), "{:?}", sent.status_reason);
    go.notify_one();

    wait_plan_done(&core).await;
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Done);
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief, then your comments: {prompts:?}");
    assert!(prompts[1].contains("[Codly] The user reviewed your work on task t1") && prompts[1].contains("sent it back"), "{}", prompts[1]);
    assert!(prompts[1].contains(COMMENTS), "the comments as you wrote them: {}", prompts[1]);
    let all = events(&core).await;
    // The gate did not check the work before the worker answered the comments.
    assert_eq!(verified(&all), [true]);
    let yours = your_decisions(&all).into_iter().find(|d| d.0.contains("review comments")).expect("your review is on record");
    assert_eq!(yours.0, "you sent t1 back with review comments; its worker reads them now");
    assert_eq!(yours.1["comments"], COMMENTS);
    assert_eq!(asked_in_chat(&core, t.assignee.as_deref().unwrap()).await, [COMMENTS]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comments_start_a_stopped_task_again_and_a_done_task_takes_none() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.work();
            turn.done().await;
        })
    });
    let core = start(dir.path(), worker, turns.clone()).await;

    // The check fails until the comments say what to write: the gate gives up after its fix rounds.
    let t = wait_task(&core, TaskStatus::Failed).await;
    assert_eq!(worker_prompts(&turns).len(), 3, "the brief and two fix rounds");
    let e = core.send_back_task(&t.id, " \n").await.unwrap_err().to_string();
    assert!(e.contains("no comment to send"), "{e}");
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Failed, "nothing moved");

    let sent = core.send_back_task(&t.id, COMMENTS).await.unwrap();
    assert!(matches!(sent.status, TaskStatus::Ready | TaskStatus::Running), "{:?}", sent.status);
    wait_plan_done(&core).await;
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Done);
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 4, "one more turn, started with your comments: {prompts:?}");
    assert!(prompts[3].contains("sent it back") && prompts[3].contains(COMMENTS), "{}", prompts[3]);
    let all = events(&core).await;
    assert_eq!(verified(&all), [false, false, false, true]);
    let yours: Vec<String> = your_decisions(&all).into_iter().map(|d| d.0).collect();
    assert!(yours.contains(&"you sent t1 back with review comments; its next run starts with them".to_string()), "{yours:?}");
    assert_eq!(asked_in_chat(&core, t.assignee.as_deref().unwrap()).await, [COMMENTS]);

    // Its work is merged into the finished plan now: the comments go on the plan
    // (`send_back_plan`), and nothing is recorded.
    let e = core.send_back_task(&t.id, COMMENTS).await.unwrap_err().to_string();
    assert_eq!(e, "t1 is done; send your comments on the plan and they become one more task of it");
    assert_eq!(asked_in_chat(&core, t.assignee.as_deref().unwrap()).await.len(), 1);
    core.shutdown().await;
}

/// A worker whose first turn passes the check at once, so its work goes to review, and a
/// reviewer that approves. `slow`: the first review never ends by itself.
fn reviewed_work(slow: bool) -> Behavior {
    let reviews = Arc::new(AtomicUsize::new(0));
    Arc::new(move |turn: FakeTurn| {
        let reviews = reviews.clone();
        Box::pin(async move {
            if turn.spec.role == Role::Reviewer {
                if reviews.fetch_add(1, Ordering::SeqCst) == 0 && slow {
                    std::future::pending::<()>().await;
                }
                let criteria = json!([{ "criterion": "hello.txt is right", "met": true }]);
                turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] })).await;
                return;
            }
            let answer = turn.prompt.contains("sent it back");
            if answer {
                // A turn takes a moment, as a real one does: the task says Changes requested
                // and the old review is stopped long before this one ends.
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            std::fs::write(turn.spec.cwd.join("hello.txt"), if answer { "hello, reviewed twice\n" } else { "hello, reviewed\n" }).unwrap();
            turn.done().await;
        })
    })
}

/// The agent of every reviewer turn, oldest first.
fn reviewers(turns: &Turns) -> Vec<String> {
    turns.lock().unwrap().iter().filter(|t| t.0 == Role::Reviewer).map(|t| t.1.clone()).collect()
}

/// The verdict of every review that was read, oldest first.
fn verdicts(events: &[EventRow]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Reviewed { verdict, .. } => Some(verdict.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comments_sent_during_a_review_stop_it_and_the_new_work_gets_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let spawner = FakeSpawner { worker: reviewed_work(true), turns: turns.clone(), reviewer_up: None, reviewer_stops_late: None };
    let core = start_with(dir.path(), spawner, true).await;

    let t = wait_task(&core, TaskStatus::Reviewing).await;
    let first = wait_for("the first review", 40, async || reviewers(&turns).first().cloned()).await;
    let sent = core.send_back_task(&t.id, COMMENTS).await.unwrap();
    assert_eq!(sent.status, TaskStatus::ChangesRequested);
    // The review of the old work stops: nobody waits for a verdict on it.
    wait_for("the first review to stop", 40, async || {
        let stopped = |e: &EventRow| e.agent_id.as_deref() == Some(first.as_str()) && matches!(e.kind, EventKind::Exited { .. });
        events(&core).await.iter().any(stopped).then_some(())
    })
    .await;

    wait_plan_done(&core).await;
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Done);
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief, then your comments; the old review asked for nothing: {prompts:?}");
    assert!(prompts[1].contains("sent it back") && prompts[1].contains(COMMENTS), "{}", prompts[1]);
    assert_eq!(reviewers(&turns).len(), 2, "the work with your changes got a review of its own");
    let all = events(&core).await;
    assert_eq!(verified(&all), [true, true]);
    assert_eq!(verdicts(&all), ["approve"], "one verdict, on the work that has your changes");
    let yours: Vec<String> = your_decisions(&all).into_iter().map(|d| d.0).collect();
    assert!(
        yours.contains(&"you sent t1 back with review comments; its worker reads them now, and the review that was under way stopped".to_string()),
        "{yours:?}"
    );
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comments_wait_while_a_review_is_starting() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let up = Arc::new(Notify::new());
    let spawner = FakeSpawner { worker: reviewed_work(false), turns: turns.clone(), reviewer_up: Some(up.clone()), reviewer_stops_late: None };
    let core = start_with(dir.path(), spawner, true).await;

    let t = wait_task(&core, TaskStatus::Reviewing).await;
    wait_for("the reviewer to start", 40, async || {
        let agents = core.snapshot().await.unwrap().agents;
        agents.iter().any(|a| a.role == Role::Reviewer && a.status == AgentStatus::Starting).then_some(())
    })
    .await;
    // A reviewer that is starting cannot be stopped yet: nothing is sent, and you send again.
    let e = core.send_back_task(&t.id, COMMENTS).await.unwrap_err().to_string();
    assert_eq!(e, "a review of t1 is starting right now; send your comments again in a moment");
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Reviewing, "nothing moved");
    assert!(asked_in_chat(&core, t.assignee.as_deref().unwrap()).await.is_empty());

    up.notify_one();
    wait_plan_done(&core).await;
    assert_eq!(worker_prompts(&turns).len(), 1, "the worker read nothing of it");
    assert_eq!(verdicts(&events(&core).await), ["approve"]);
    core.shutdown().await;
}

/// A reviewer that was told to stop can report late: its turn ends when the next review round
/// is already under way. It read the work as it was before your comments, so its verdict is
/// not this round's: the round's own reviewer decides.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_verdict_of_an_earlier_round_does_not_count_for_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // `said`: the first reviewer gave its verdict. `late`: its turn ends, long after it was
    // told to stop. `second`: the second reviewer gives its verdict.
    let (said, late, second) = (Arc::new(Notify::new()), Arc::new(Notify::new()), Arc::new(Notify::new()));
    let reviews = Arc::new(AtomicUsize::new(0));
    let worker: Behavior = {
        let (said, second) = (said.clone(), second.clone());
        Arc::new(move |turn: FakeTurn| {
            let (reviews, said, second) = (reviews.clone(), said.clone(), second.clone());
            Box::pin(async move {
                if turn.spec.role == Role::Reviewer {
                    let criteria = json!([{ "criterion": "hello.txt is right", "met": true }]);
                    let approve = json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] });
                    if reviews.fetch_add(1, Ordering::SeqCst) == 0 {
                        // Its verdict is on record, and its turn does not end by itself.
                        turn.tool("codly_submit_review", approve).await;
                        said.notify_one();
                        return std::future::pending::<()>().await;
                    }
                    second.notified().await;
                    turn.tool("codly_submit_review", approve).await;
                    return;
                }
                let answer = turn.prompt.contains("sent it back");
                if answer {
                    tokio::time::sleep(Duration::from_millis(300)).await; // as in `reviewed_work`
                }
                std::fs::write(turn.spec.cwd.join("hello.txt"), if answer { "hello, reviewed twice\n" } else { "hello, reviewed\n" }).unwrap();
                turn.done().await;
            })
        })
    };
    let spawner = FakeSpawner { worker, turns: turns.clone(), reviewer_up: None, reviewer_stops_late: Some(late.clone()) };
    let core = start_with(dir.path(), spawner, true).await;

    let t = wait_task(&core, TaskStatus::Reviewing).await;
    said.notified().await;
    let first = reviewers(&turns)[0].clone();
    core.send_back_task(&t.id, COMMENTS).await.unwrap();
    // The worker answers, its work is checked, and a second review starts while the first
    // reviewer has still not closed its turn.
    let other = wait_for("the second review", 40, async || reviewers(&turns).get(1).cloned()).await;
    assert_ne!(other, first, "the first reviewer still runs: another one reads the new work");
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Reviewing);

    // Now the first reviewer's turn ends, with its approval of the old work on record.
    late.notify_one();
    wait_for("the first reviewer to exit", 40, async || {
        let exited = |e: &EventRow| e.agent_id.as_deref() == Some(first.as_str()) && matches!(e.kind, EventKind::Exited { .. });
        events(&core).await.iter().any(exited).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Reviewing, "the second review is still under way");
    assert!(verdicts(&events(&core).await).is_empty(), "the late verdict was not read");

    second.notify_one();
    wait_plan_done(&core).await;
    let all = events(&core).await;
    let read: Vec<&EventRow> = all.iter().filter(|e| matches!(e.kind, EventKind::Reviewed { .. })).collect();
    assert_eq!(verdicts(&all), ["approve"]);
    assert_eq!(read[0].agent_id.as_deref(), Some(other.as_str()), "the verdict of the round's own reviewer");
    assert_eq!(reviewers(&turns).len(), 2, "no third review");
    assert_eq!(worker_prompts(&turns).len(), 2, "the brief, then your comments");
    core.shutdown().await;
}
