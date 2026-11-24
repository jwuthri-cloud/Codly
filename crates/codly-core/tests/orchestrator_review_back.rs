//! Review comments on a finished plan, end to end with fake agents in a temp git repo:
//! - Send back on a finished plan's diff makes the comments one more task of the plan, for the
//!   agent that wrote the file they are on; it works on the plan's own branch in the plan's
//!   worktree, passes the gate with the plan's checks (its reviewer reads its own commits), the
//!   plan is done again, and it merges with the change;
//! - a task that does not finish leaves the plan as it was: its branch at its old tip, done;
//! - a plan that is running, merged, has an open pull request or no comment is refused in
//!   words the Changes room can show, and a done task of a finished plan points to its plan.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::pull_request::PrSource;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap()
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Commit one file on the checked-out branch, as the user would.
fn commit(repo: &Path, file: &str, content: &str) {
    std::fs::write(repo.join(file), content).unwrap();
    sh(repo, &["add", "-A"]);
    sh(repo, &["commit", "-qm", &format!("change {file}")]);
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.name", "Test"]);
    sh(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    std::fs::write(repo.join("b.txt"), "empty\n").unwrap();
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

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.spec.cwd.join(rel)).unwrap_or_default()
    }

    fn write(&self, rel: &str, content: &str) {
        std::fs::write(self.spec.cwd.join(rel), content).unwrap();
    }

    async fn done(&self) {
        self.tool("codly_submit_report", json!({ "status": "done", "summary": "Did it." })).await;
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
/// (role, agent, working directory, prompt) of every turn.
type Turns = Arc<Mutex<Vec<(Role, Id, PathBuf, String)>>>;

struct FakeSpawner {
    lead: Behavior,
    worker: Behavior,
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
            // Each turn ends by itself, and the run ends when Codly tells it to stop.
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            turns.lock().unwrap().push((spec2.role, spec2.agent_id.clone(), spec2.cwd.clone(), text.clone()));
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
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

/// b.txt's check: t2 must leave "two" in it, and so must the review.
const B_CHECK: &str = "grep -q two b.txt";

/// A Lead that plans two tasks for the two members of your team: Ana writes a.txt (t1), Bo
/// writes b.txt (t2).
fn lead() -> Behavior {
    let task = |id: &str, file: &str, verify: &str, member: &str| {
        json!({
            "id": id, "title": format!("Write {file}"), "goal": format!("Write {file}"), "assignee": member,
            "acceptance_criteria": [format!("{file} is right")], "write_scope": [file], "depends_on": [], "verify": [verify], "size": "S",
        })
    };
    let plan = json!({
        "summary": "Write the two files",
        "tasks": [task("t1", "a.txt", "test -f a.txt", "Ana"), task("t2", "b.txt", B_CHECK, "Bo")],
        "integration_checks": [],
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

const A: &str = "first file\n";
const B: &str = "two\n";
const REVIEWED: &str = "two, said twice\n";
const COMMENTS: &str = "Review comments (1):\n\n1. `b.txt` (the whole file)\n   Say it twice.";

/// The plan's workers, and a reviewer that approves what it is given. The task with your
/// review comments writes `review` into b.txt; a conflict with your branch is resolved with
/// the plan's line.
fn worker(review: &'static str) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.spec.role == Role::Reviewer {
                let reply = turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": [], "issues": [] })).await;
                assert_eq!(reply["ok"], true, "{reply}");
                return;
            }
            if turn.prompt.contains("Task t1") {
                turn.write("a.txt", A);
            } else if turn.prompt.contains("Task t2") {
                turn.write("b.txt", B);
            } else if turn.prompt.contains("Task review") {
                turn.write("b.txt", review);
            } else if turn.read("b.txt").contains("<<<<<<<") {
                turn.write("b.txt", B);
            }
            turn.done().await;
        })
    })
}

/// A core with one open repo, where a request runs its plan at once. Returns the repo too.
async fn start(root: &Path, worker: Behavior, turns: Turns) -> (Arc<Core>, PathBuf) {
    let repo = temp_repo(root);
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let spawner = FakeSpawner { lead: lead(), worker, turns };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let team = ["Ana", "Bo"].map(|name| TeamMember { name: name.into(), role: "Backend dev".into(), ..Default::default() }).to_vec();
    let settings = Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], team, ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    (core, repo)
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

async fn wait_plan(core: &Core, status: PlanStatus) -> Plan {
    wait_for(&format!("the plan to be {status:?}"), 60, async || core.snapshot().await.unwrap().plans.into_iter().find(|p| p.status == status)).await
}

async fn task_with(core: &Core, key: &str, status: TaskStatus) -> Task {
    wait_for(&format!("task {key} to be {status:?}"), 60, async || {
        core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key && t.status == status)
    })
    .await
}

async fn task(core: &Core, key: &str) -> Task {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key).unwrap_or_else(|| panic!("no task {key}"))
}

/// What `author` said in a thread, oldest first.
async fn said(core: &Core, thread: &str, author: &str) -> Vec<String> {
    let s = core.snapshot().await.unwrap();
    s.chats.iter().filter(|c| c.thread == thread && c.author == author).map(|c| c.text.clone()).collect()
}

/// What Codly put on record, oldest first.
async fn decisions(core: &Core) -> Vec<String> {
    let mut all = core.events(None, None, 4000).await.unwrap();
    all.sort_by_key(|e| e.id);
    all.iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .collect()
}

/// A plan of two tasks by two agents, finished: (core, repo, plan, the agent that did t2).
async fn finished_plan(root: &Path, review: &'static str, turns: Turns) -> (Arc<Core>, PathBuf, Plan, Agent) {
    let (core, repo) = start(root, worker(review), turns).await;
    core.send_chat("lead", "Please write the two files").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let (t1, t2) = (task(&core, "t1").await, task(&core, "t2").await);
    assert_eq!((t1.status, t2.status), (TaskStatus::Done, TaskStatus::Done));
    let agents = core.snapshot().await.unwrap().agents;
    let of = |t: &Task| agents.iter().find(|a| t.assignee.as_deref() == Some(a.id.as_str())).map(|a| a.name.clone());
    assert_eq!((of(&t1).as_deref(), of(&t2).as_deref()), (Some("Ana"), Some("Bo")));
    let bo = agents.iter().find(|a| a.name == "Bo").unwrap().clone();
    (core, repo, plan, bo)
}

fn files() -> Vec<String> {
    vec!["b.txt".to_string()]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_comments_on_a_finished_plan_go_to_the_agent_that_wrote_the_file_and_the_plan_finishes_again() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let (core, repo, plan, bo) = finished_plan(dir.path(), REVIEWED, turns.clone()).await;
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    // A done task points to its plan: its comments go there.
    let t2 = task(&core, "t2").await;
    let err = core.send_back_task(&t2.id, COMMENTS).await.unwrap_err().to_string();
    assert_eq!(err, "t2 is done; send your comments on the plan and they become one more task of it");
    core.update_settings(Settings { review_gate: true, ..core.settings() }).await.unwrap();

    // Send back: one more task of the plan, for Bo, who wrote b.txt; the plan runs again.
    let sent = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap();
    assert_eq!((sent.key.as_str(), sent.title.as_str(), sent.assignee.as_deref()), ("review", "Changes you asked for in review", Some(bo.id.as_str())));
    assert_eq!((sent.branch.as_deref(), sent.worktree.as_deref(), sent.fresh_session), (Some(branch.as_str()), plan.integration_worktree.as_deref(), true));
    assert_eq!((&sent.write_scope, &sent.verify), (&vec!["a.txt".to_string(), "b.txt".to_string()], &vec!["test -f a.txt".to_string(), B_CHECK.to_string()]));
    assert!(sent.goal.ends_with(COMMENTS), "{}", sent.goal);
    let s = core.snapshot().await.unwrap();
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Running);
    let err = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap_err().to_string();
    assert_eq!(err, "the plan is still running; send your comments again when it is done", "a second click makes no second task");

    // The task goes through the gate, and the plan is done again.
    task_with(&core, "review", TaskStatus::Done).await;
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!((&again.base_commit, &again.base_branch), (&plan.base_commit, &plan.base_branch), "it builds on what it built on before");
    assert_eq!(std::fs::read_to_string(integ_wt.join("b.txt")).unwrap(), REVIEWED);
    // Its work is commits of its own on the plan's branch: no merge was made.
    assert_eq!(sh(&repo, &["rev-list", "--count", &format!("{tip}..{branch}")]), "1");
    assert_eq!(sh(&repo, &["rev-list", "--merges", &format!("{tip}..{branch}")]), "");
    // Bo did it, in the plan's worktree.
    {
        let turns = turns.lock().unwrap();
        let (_, agent, cwd, _) = turns.iter().find(|(role, _, _, prompt)| *role == Role::Worker && prompt.contains("Task review")).expect("the review task's brief");
        assert_eq!(agent, &bo.id);
        assert_eq!(std::fs::canonicalize(cwd).unwrap(), std::fs::canonicalize(&integ_wt).unwrap());
        // Its reviewer read the task's own commit, not the plan's whole work, and was told so.
        let review = turns.iter().filter(|(role, _, _, _)| *role == Role::Reviewer).map(|(_, _, _, p)| p.clone()).last().expect("a review");
        assert!(review.contains("[Codly] Review task review: Changes you asked for in review"), "{review}");
        assert!(review.contains("+two, said twice") && !review.contains("+first file"), "{review}");
        assert!(review.contains("About the diff: this task worked on the plan's finished branch; the diff is what it changed there"), "{review}");
    }
    // On record: your choice, your comments in the plan's thread, and where they went.
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| *d == format!("you sent the plan back with review comments: task review for {}", bo.name)), "{record:?}");
    assert!(record.iter().any(|d| d == "review: review verdict approve"), "{record:?}");
    assert_eq!(said(&core, "lead", "user").await.last().map(String::as_str), Some(COMMENTS));
    let told = format!("Your review comments are task review of this plan, for {} (who wrote t2): it works in the plan's worktree", bo.name);
    assert!(said(&core, "lead", "codly").await.iter().any(|l| l.starts_with(&told)), "{:?}", said(&core, "lead", "codly").await);

    // It merges with the change; then the comments are a new request.
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!((std::fs::read_to_string(repo.join("a.txt")).unwrap(), std::fs::read_to_string(repo.join("b.txt")).unwrap()), (A.to_string(), REVIEWED.to_string()));
    let err = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap_err().to_string();
    assert_eq!(err, "this plan is merged into your branch; ask for the changes in a new request");
    let err = core.send_back_task(&t2.id, COMMENTS).await.unwrap_err().to_string();
    assert_eq!(err, "t2 is done and its work is merged; ask for the changes in a new request");
    core.shutdown().await;
}

/// The worker leaves b.txt so that the plan's check fails through every fix round. You cancel
/// the task: the plan's branch is at its old tip, and the plan is done as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_review_task_that_does_not_finish_leaves_the_plan_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let (core, repo, plan, _) = finished_plan(dir.path(), "nothing\n", Default::default()).await;
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);

    core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap();
    let failed = task_with(&core, "review", TaskStatus::Failed).await;
    assert!(failed.status_reason.as_deref().unwrap().contains(&format!("`{B_CHECK}` failed")), "{:?}", failed.status_reason);
    core.move_task(&failed.id, TaskStatus::Cancelled).await.unwrap();

    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip, "the plan's branch is back where it was");
    assert_eq!((&again.base_commit, &again.base_branch, &again.merged_sha), (&plan.base_commit, &plan.base_branch, &None));
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "");
    assert_eq!(std::fs::read_to_string(integ_wt.join("b.txt")).unwrap(), B);
    let lines = said(&core, "lead", "codly").await;
    assert!(lines.iter().any(|l| l.starts_with("The changes you asked for in review were not finished. Codly took them back")), "{lines:?}");
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| *d == format!("the changes you asked for in review did not finish: {branch} is back where it was before them")), "{record:?}");
    // The plan can be sent back again: a new task of its own.
    let second = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap();
    assert_eq!(second.key, "review-2");
    core.shutdown().await;
}

/// Store a pull request for the plan as Codly keeps the one it opened, as last read.
fn pull_request_on_record(root: &Path, plan_id: &str, state: &str) {
    let db = rusqlite::Connection::open(root.join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let pr = json!({ "url": "https://github.com/acme/web/pull/7", "remote": "origin", "branch": "b", "base": "main", "created_at": 0, "state": state, "checks": null });
    let sql = "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value";
    db.execute(sql, rusqlite::params![format!("pull_request:plan:{plan_id}"), pr.to_string()]).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_that_cannot_take_review_comments_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let (core, repo, plan, _) = finished_plan(dir.path(), REVIEWED, Default::default()).await;
    let err = core.send_back_plan(&plan.id, "  \n", &files()).await.unwrap_err().to_string();
    assert_eq!(err, "there is no comment to send");

    // An open pull request: the changes go to it through a new request, not on its branch here.
    pull_request_on_record(dir.path(), &plan.id, "open");
    let err = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap_err().to_string();
    assert_eq!(err, "this plan has an open pull request: ask for the changes in a new request");
    // Closed without a merge: the plan takes them again.
    pull_request_on_record(dir.path(), &plan.id, "closed");

    // A task of the plan is at work on its branch (it resolves a conflict with your branch).
    commit(&repo, "b.txt", "yours\n");
    core.resolve_merge_conflict(&PrSource::Plan(plan.id.clone())).await.unwrap();
    let err = core.send_back_plan(&plan.id, COMMENTS, &files()).await.unwrap_err().to_string();
    assert_eq!(err, "the plan is still running; send your comments again when it is done");
    assert!(!core.snapshot().await.unwrap().tasks.iter().any(|t| t.key.starts_with("review")), "no task was made");
    task_with(&core, "resolve", TaskStatus::Done).await;
    wait_plan(&core, PlanStatus::Done).await;
    core.shutdown().await;
}
