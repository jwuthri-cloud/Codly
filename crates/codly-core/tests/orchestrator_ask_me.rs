//! "Ask me" end to end with fake agents in a temp git repo: a worker raises a blocking card
//! with codly_ask_user, its task parks as Blocked ("waiting for you") and is not gated or
//! merged; your answer resumes the same worker with a [User] prompt, and the plan finishes.
//! A heads-up card does not stop the worker, and dismissing a blocking card unblocks it too.

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

fn sh(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
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
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
type Turns = Arc<Mutex<Vec<(String, String)>>>;

struct FakeSpawner {
    lead: Behavior,
    worker: Behavior,
    /// (agent id, prompt) of every turn.
    turns: Turns,
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
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            turns.lock().unwrap().push((spec2.agent_id.clone(), text.clone()));
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

/// A Lead that turns a request into a one-task plan writing hello.txt.
fn planning_lead() -> Behavior {
    let plan = json!({
        "summary": "Add a hello file",
        "tasks": [{
            "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing the greeting",
            "consultant": { "title": "Backend" }, "acceptance_criteria": ["hello.txt exists"], "write_scope": ["hello.txt"],
            "depends_on": [], "verify": ["test -f hello.txt"], "size": "M",
        }],
    });
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
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answering_a_blocking_card_resumes_the_worker_and_the_plan_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let (core, turns, card) = run_until_card(dir.path(), asking_worker("Formal or casual hello?")).await;

    // One heads-up and one blocking card (the repeat was folded), both attributed to the worker.
    let cards = core.list_blockers().await.unwrap();
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert!(cards.iter().any(|b| b.severity == BlockerSeverity::HeadsUp && b.title == "Tests are slow"));
    assert_eq!(card.body, "Formal or casual hello?");
    let task = task_of(&core).await;
    assert_eq!(card.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(task.status, TaskStatus::Blocked);
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
    core.shutdown().await;
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
    core.shutdown().await;
}
