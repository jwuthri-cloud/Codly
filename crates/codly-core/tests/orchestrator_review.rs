//! Send back, from a task's diff, end to end with fake agents in a temp git repo:
//! - a worker that still runs the task reads your review comments after its current turn, and
//!   the gate checks the work only once it answered them;
//! - a task the gate stopped starts again with the comments, with new fix rounds;
//! - a done task takes none, and what you asked stays in the task's record and in the
//!   worker's conversation.

use std::path::{Path, PathBuf};
use std::process::Command;
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
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let behavior = if spec.role == Role::Lead { lead() } else { self.worker.clone() };
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
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            turns.lock().unwrap().push((spec2.role, spec2.agent_id.clone(), text.clone()));
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            send(EventKind::StatusChanged { status: AgentStatus::Thinking });
                            behavior(FakeTurn { spec: spec2.clone(), prompt: text, events: events.clone() }).await;
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(5) });
                            send(EventKind::StatusChanged { status: AgentStatus::Idle });
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
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
    let repo = temp_repo(root);
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let spawner = FakeSpawner { worker, turns };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
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

    // Its work is merged now: the changes are a new request, and nothing is recorded.
    let e = core.send_back_task(&t.id, COMMENTS).await.unwrap_err().to_string();
    assert!(e.contains("t1 is done") && e.contains("new request"), "{e}");
    assert_eq!(asked_in_chat(&core, t.assignee.as_deref().unwrap()).await.len(), 1);
    core.shutdown().await;
}
