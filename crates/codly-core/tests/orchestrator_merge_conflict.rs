//! The last merge, into the user's branch, end to end with fake agents in a temp git repo:
//! - a plan that conflicts with the user's branch says which files conflict and how to go on;
//!   "have an agent resolve it" brings the user's branch into the plan's own worktree, a task of
//!   the plan resolves the conflict there and passes the gate with the plan's verify commands,
//!   the plan finishes again, and the merge (and its undo) work; the user's checkout is never
//!   touched on the way;
//! - when the user's branch has commits the plan was never checked with, the plan's verify
//!   commands run on the merge result before the branch moves: a failure stops the merge until
//!   the user merges anyway, and a pass is said in the plan's thread;
//! - a solo agent resolves a conflict of its own branch in its own worktree, and Codly neither
//!   commits nor merges a worktree that still has conflict markers; its conversation says so,
//!   and the resolution can be stopped;
//! - a resolution that does not finish leaves nothing behind: one that failed its checks is
//!   taken off the plan's branch when its task is cancelled, and Cancel plan cancels the
//!   resolution, not the finished plan; a restart of Codly keeps the open merge, and a Retry
//!   starts it again after its rounds ran out;
//! - a plan that started from another branch is resolved only after the user confirms it; a
//!   merge whose checks could not run, or whose branches moved while they ran, is not made;
//! - with reviews on, the task that resolved a conflict is reviewed, and its reviewer reads the
//!   resolution only, not the user's commits that the merge brought in;
//! - checks that fail on the merge result with no conflict can go to an agent: one more task
//!   of the plan makes them pass, and one that does not finish takes the merge back;
//! - a resolution that does not finish takes its worker's new files out of the plan's worktree;
//! - before a merge writes to the user's branch, a ref keeps the branch as it was, until the
//!   next merge or the undo, for a plan's branch and for a solo agent's; the state says how far
//!   the Codly branch is behind the user's;
//! - a resolution (or a fix of the checks) that could not start takes its merge back: the
//!   user's branch is never in the plan's branch with no task on it and no check of the result;
//!   the same after Codly was closed in the middle of a start, and the error says the branch is
//!   back also when that could not be written down;
//! - a cancelled plan says in plain words what is left for you to do, a fix of its checks or a
//!   conflict, also when what its last merge check said cannot be read;
//! - a new file that was in the plan's worktree before a task on the plan's own branch started
//!   stays out of that task's commit, on record, and stays on disk;
//! - the task that works on the plan's own branch stays in its write scope with what it
//!   changed by hand, or declares the exception; what git merged by itself is not counted, and
//!   a file name in that scope is no pattern;
//! - with reviews on, a fix of the checks that changed no file goes on with no reviewer run.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::pull_request::{PrConfirm, PrSource};
use codly_core::orchestrator::worktree_setup::SetupChoice;
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

/// Commit one file on the checked-out branch, as the user would. Returns the new commit.
fn commit(repo: &Path, file: &str, content: &str) -> String {
    std::fs::write(repo.join(file), content).unwrap();
    sh(repo, &["add", "-A"]);
    sh(repo, &["commit", "-qm", &format!("change {file}")]);
    sh(repo, &["rev-parse", "HEAD"])
}

fn in_merge(wt: &Path) -> bool {
    git(wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).status.success()
}

/// The ref that keeps your branch as it was before the last merge Codly made into it.
const BEFORE_MERGE: &str = "refs/codly/before-merge";

/// The commit `BEFORE_MERGE` keeps, if the ref is there.
fn kept_before_merge(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "-q", "--verify", BEFORE_MERGE]);
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.name", "Test"]);
    sh(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    std::fs::write(repo.join("greeting.txt"), "hello\n").unwrap();
    std::fs::write(repo.join("mode.txt"), "good\n").unwrap();
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
/// (role, working directory, prompt) of every turn.
type Turns = Arc<Mutex<Vec<(Role, PathBuf, String)>>>;

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
                            turns.lock().unwrap().push((spec2.role, spec2.cwd.clone(), text.clone()));
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

/// A Lead that submits a one-task plan for every request: t1 writes `content` to `file`.
fn lead_with(file: &str, verify: &str, integration: &[&str]) -> Behavior {
    lead_scoped(file, file, verify, integration)
}

/// The same, where the write scope of t1 is `scope` and not the file's own name.
fn lead_scoped(file: &str, scope: &str, verify: &str, integration: &[&str]) -> Behavior {
    let plan = json!({
        "summary": "Change the greeting",
        "tasks": [{
            "id": "t1", "title": format!("Write {file}"), "goal": format!("Write {file}"), "consultant": { "title": "Backend" },
            "acceptance_criteria": [format!("{file} is right")], "write_scope": [scope], "depends_on": [], "verify": [verify], "size": "M",
        }],
        "integration_checks": integration,
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

/// A core with one open repo, where a request runs its plan at once. Returns the repo too.
async fn start(root: &Path, lead: Behavior, worker: Behavior, turns: Turns) -> (Arc<Core>, PathBuf) {
    let repo = temp_repo(root);
    (boot(root, &repo, lead, worker, turns).await, repo)
}

/// Codly starts (again) on the data under `root`, with `repo` open.
async fn boot(root: &Path, repo: &Path, lead: Behavior, worker: Behavior, turns: Turns) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let spawner = FakeSpawner { lead, worker, turns };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let settings = Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
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

async fn wait_plan(core: &Core, status: PlanStatus) -> Plan {
    wait_for(&format!("the plan to be {status:?}"), 40, async || {
        core.snapshot().await.unwrap().plans.into_iter().find(|p| p.status == status)
    })
    .await
}

/// What Codly said in a thread, oldest first.
async fn said(core: &Core, thread: &str) -> Vec<String> {
    let s = core.snapshot().await.unwrap();
    s.chats.iter().filter(|c| c.thread == thread && c.author == "codly").map(|c| c.text.clone()).collect()
}

/// Codly's verify runs, oldest first: (the task it ran for, command, passed).
async fn verified(core: &Core) -> Vec<(Option<Id>, String, bool)> {
    let mut all = core.events(None, None, 4000).await.unwrap();
    all.sort_by_key(|e| e.id);
    all.iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { command, ok, .. } => Some((e.task_id.clone(), command.clone(), *ok)),
            _ => None,
        })
        .collect()
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

/// Where a merge check would make its scratch worktree, per office.
fn scratch_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root.join("data").join("worktrees")).unwrap().map(|office| office.unwrap().path().join("merge-check")).collect()
}

/// The scratch worktree of a merge check is gone once the check is over.
fn no_scratch_left(root: &Path) {
    for scratch in scratch_dirs(root) {
        assert!(!scratch.exists(), "{} is still there", scratch.display());
    }
}

async fn task_with(core: &Core, key: &str, status: TaskStatus) -> Task {
    wait_for(&format!("task {key} to be {status:?}"), 40, async || {
        core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key && t.status == status)
    })
    .await
}

const RESOLVED: &str = "hello from main and the plan\n";

/// A plan's worker: t1 writes the plan's greeting; the task that resolves a conflict finds the
/// markers Codly left and resolves them.
fn plan_worker() -> Behavior {
    Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                assert!(turn.read("greeting.txt").contains(">>>>>>> refs/heads/main"), "{}", turn.read("greeting.txt"));
                turn.write("greeting.txt", RESOLVED);
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_resolves_a_plans_conflict_with_your_branch_and_the_merge_goes_through() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &["test -f greeting.txt"]);
    let (core, repo) = start(dir.path(), lead, plan_worker(), turns.clone()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());

    // Nothing in the way yet.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.target.as_str(), state.branch.as_str(), state.conflicts.len(), state.resolving, state.failed_checks.len()), ("main", branch.as_str(), 0, None, 0));
    assert_eq!(Path::new(&state.repo), std::fs::canonicalize(&repo).unwrap());
    assert_eq!(state.behind, 0, "the plan is built on main as it is");

    // You kept working on main: the same line, and a file the plan never touched.
    std::fs::write(repo.join("README.md"), "# demo\nmine\n").unwrap();
    let mine = commit(&repo, "greeting.txt", "hello from main\n");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.conflicts[..], state.behind), (&["greeting.txt".to_string()][..], 1), "behind main by your one commit, which conflicts");

    // Merge: refused, with the files and both ways out; your checkout is as it was.
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert!(err.contains("An agent (on the plan's card) can resolve it") && err.ends_with(&format!("git merge {branch}")), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (mine.clone(), String::new()));
    no_scratch_left(dir.path());
    // Without the checks first, git itself finds the conflict: no merge, and no ref of one.
    let err = core.merge_plan_unchecked(&plan.id, None).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (mine.clone(), None));

    // Have an agent resolve it: the plan runs again with one more task, in its own worktree.
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let s = core.snapshot().await.unwrap();
    let task = s.tasks.iter().find(|t| t.key == "resolve").expect("a task that resolves the conflict").clone();
    assert_eq!(state.resolving.as_deref(), Some(task.id.as_str()));
    assert_eq!((task.title.as_str(), task.branch.as_deref(), task.worktree.as_deref()), ("Resolve conflicts with main", Some(branch.as_str()), plan.integration_worktree.as_deref()));
    assert_eq!((&task.write_scope, &task.verify), (&vec!["greeting.txt".to_string()], &vec!["grep -q plan greeting.txt".to_string(), "test -f greeting.txt".to_string()]));
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Running);
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert!(err.contains("the plan is still running"), "a second click starts nothing: {err}");

    // The task goes through the gate, and the plan finishes again with its checks.
    let again = wait_plan(&core, PlanStatus::Done).await;
    let s = core.snapshot().await.unwrap();
    let task = s.tasks.iter().find(|t| t.key == "resolve").unwrap();
    assert_eq!((task.status, task.worktree.as_deref()), (TaskStatus::Done, plan.integration_worktree.as_deref()));
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), RESOLVED, "resolved in the plan's own worktree, which is kept");
    assert!(!in_merge(&integ_wt), "Codly committed the merge");
    assert_eq!(sh(&repo, &["merge-base", "--is-ancestor", &mine, &branch]), "", "your commits are in the plan's branch now");
    // Its worker ran in that worktree and was told what to resolve.
    {
        let turns = turns.lock().unwrap();
        let (_, cwd, brief) = turns.iter().find(|(role, _, prompt)| *role == Role::Worker && prompt.contains("Task resolve")).expect("the resolving task's brief");
        assert_eq!(std::fs::canonicalize(cwd).unwrap(), std::fs::canonicalize(&integ_wt).unwrap());
        assert!(brief.contains("Codly started merging main into this worktree") && brief.contains("conflict markers (<<<<<<< ======= >>>>>>>): greeting.txt"), "{brief}");
    }
    // The plan's verify commands ran on the merged result, for the task and then for the plan.
    let runs = verified(&core).await;
    let for_task: Vec<&str> = runs.iter().filter(|(t, _, ok)| t.as_deref() == Some(task.id.as_str()) && *ok).map(|(_, c, _)| c.as_str()).collect();
    assert_eq!(for_task, ["grep -q plan greeting.txt", "test -f greeting.txt"]);
    // The task ran the integration check on the plan's branch as it is: the plan, when it
    // finished again, did not run it a second time, and the record says why.
    assert_eq!(runs.iter().filter(|(t, c, ok)| t.is_none() && c == "test -f greeting.txt" && *ok).count(), 1, "for the plan, only when it first finished");
    let skipped = "`grep -q plan greeting.txt`, `test -f greeting.txt` passed in task resolve on the plan's branch as it is: the plan does not run them a second time";
    assert!(decisions(&core).await.iter().any(|d| d == skipped), "{:?}", decisions(&core).await);
    // Its write scope is the file with the conflict, and that is the one file it changed by
    // hand: README.md, which the merge brought in from main by itself, is not counted.
    let in_scope = "resolve: verify passed (2 commands), 1 file changed within scope";
    assert!(decisions(&core).await.iter().any(|d| d == in_scope), "{:?}", decisions(&core).await);
    // All along, your checkout was not touched.
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (mine.clone(), String::new()));
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from main\n");
    let lines = said(&core, "lead").await;
    let brought = "main and this plan both changed greeting.txt. Codly brought main into the plan's branch";
    assert!(lines.iter().any(|l| l.starts_with(brought) && l.contains("An agent resolves the conflicts there: that is task ")), "{lines:?}");

    // The plan builds on your branch as it is now: its diff is the plan's work, not yours.
    assert_eq!((again.base_commit.as_deref(), again.base_branch.as_deref()), (Some(mine.as_str()), Some("main")));
    let diff = core.plan_diff(&plan.id).await.unwrap();
    assert!(diff.contains("+hello from main and the plan") && !diff.contains("mine"), "{diff}");

    // Nothing conflicts any more: the merge goes through, and can still be undone.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.resolving, state.behind), (0, None, 0));
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "# demo\nmine\n");
    assert!(!integ_wt.exists(), "the integration worktree is cleaned up once merged");
    // main as it was before the merge is kept under a ref of its own, and Undo names it.
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(mine.as_str()));
    let last = core.repo_sync().await.unwrap().last_merge.unwrap();
    assert_eq!((last.sha.as_str(), last.plan_id.as_deref(), last.blocked, last.kept.as_deref()), (sha.as_str(), Some(plan.id.as_str()), None, Some(BEFORE_MERGE)));
    core.undo_merge(&sha).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from main\n");
    assert_eq!(kept_before_merge(&repo), None, "main is back there: nothing is left to keep");
    core.shutdown().await;
}

/// The agent could not resolve it and you cancel its task: the merge Codly had started in the
/// plan's worktree is undone, the plan is done as its tasks left it, and a second try gets a
/// task of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_resolution_leaves_the_plan_as_it_was_and_can_be_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let tries = Arc::new(AtomicUsize::new(0));
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let tries = tries.clone();
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                // The first agent gives up; the second one resolves it.
                if tries.fetch_add(1, Ordering::SeqCst) == 0 {
                    turn.tool("codly_submit_report", json!({ "status": "blocked", "summary": "I cannot tell which greeting you want." })).await;
                    return;
                }
                turn.write("greeting.txt", RESOLVED);
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, worker, Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = wait_for("the resolving task to stop", 40, async || {
        core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve" && t.status == TaskStatus::Blocked)
    })
    .await;
    assert!(in_merge(&integ_wt), "the merge stays open while the task can still go on");
    assert_eq!(core.merge_state(&source).await.unwrap().resolving.as_deref(), Some(task.id.as_str()));

    // You cancel the task: the plan finishes again, without the half-done merge.
    core.move_task(&task.id, TaskStatus::Cancelled).await.unwrap();
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!((&again.base_commit, &again.base_branch), (&plan.base_commit, &plan.base_branch), "the plan builds on what it built on before");
    assert!(!in_merge(&integ_wt));
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "");
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.conflicts, state.resolving), (&vec!["greeting.txt".to_string()], None));
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);

    // A second try is a new task, and this one goes through.
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let second = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve-2").expect("a second resolving task");
    assert_eq!(state.resolving.as_deref(), Some(second.id.as_str()));
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), RESOLVED);
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_plans_checks_run_on_the_merge_result_before_your_branch_moves() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("feature.txt", "on\n");
            turn.done().await;
        })
    });
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), worker, Default::default()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());

    // You changed a file the plan does not touch: no conflict, but its check fails with it.
    let bad = commit(&repo, "mode.txt", "bad\n");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.behind), (0, 1), "behind main by one commit that merges by itself");
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert_eq!(err, format!("with the plan merged into main, `{check}` failed, so nothing was merged and main is unchanged"));
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (bad.clone(), String::new()));
    assert!(!repo.join("feature.txt").exists());
    no_scratch_left(dir.path());
    assert_eq!(core.merge_state(&source).await.unwrap().failed_checks, [check]);
    assert_eq!(verified(&core).await.last().unwrap(), &(None, check.to_string(), false), "it ran on the merge result, for no task");
    let lines = said(&core, "lead").await;
    assert!(lines.last().unwrap().starts_with(&format!("Not merged: with \"Change the greeting\" merged into main, `{check}` failed. main was not changed.")), "{lines:?}");

    // Merge anyway: your choice, on record; and that merge can be undone like any other.
    let sha = core.merge_plan_unchecked(&plan.id, None).await.unwrap();
    assert!(repo.join("feature.txt").exists());
    let lines = said(&core, "lead").await;
    assert!(lines.last().unwrap().ends_with("You merged it without the plan's checks on the merge result."), "{lines:?}");
    assert!(core.merge_state(&source).await.unwrap().failed_checks.is_empty());
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(bad.as_str()), "main as it was is kept before the merge");
    core.undo_merge(&sha).await.unwrap();
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (bad.clone(), None));

    // You fix your side: the check passes on the merge result, the merge is made, and Codly says what ran.
    let fixed = commit(&repo, "mode.txt", "good again\n");
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(fixed.as_str()), "the ref is of the last merge");
    assert_eq!(std::fs::read_to_string(repo.join("feature.txt")).unwrap(), "on\n");
    no_scratch_left(dir.path());
    assert_eq!(verified(&core).await.last().unwrap(), &(None, check.to_string(), true));
    let lines = said(&core, "lead").await;
    let told = format!(" main had changed since the plan was checked. Codly ran the plan's checks on the merge result first: `{check}` passed.");
    assert!(lines.last().unwrap().starts_with("Merged \"Change the greeting\" into main (") && lines.last().unwrap().ends_with(&told), "{lines:?}");

    // Merged already: nothing is checked or merged twice, and the ref stays on that merge.
    let runs = verified(&core).await.len();
    assert_eq!(core.merge_plan(&plan.id).await.unwrap(), sha);
    assert_eq!(verified(&core).await.len(), runs);
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(fixed.as_str()));

    // You commit on top of the merge: Undo refuses, main is not moved, and the ref still
    // says where main was before the merge.
    let yours = commit(&repo, "README.md", "# demo\nafter the merge\n");
    let last = core.repo_sync().await.unwrap().last_merge.unwrap();
    assert!(last.blocked.as_deref().is_some_and(|why| why.starts_with("main has changed since this merge")), "{:?}", last.blocked);
    assert_eq!(last.kept.as_deref(), Some(BEFORE_MERGE));
    let err = core.undo_merge(&sha).await.unwrap_err().to_string();
    assert!(err.starts_with("the merge cannot be undone: main has changed since this merge"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (yours, Some(fixed)));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_solo_agent_resolves_the_conflict_of_its_own_branch() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    // Asked to resolve, it first ends its turn with the markers still there; asked again, it resolves them.
    let asked = Arc::new(AtomicUsize::new(0));
    let agent: Behavior = Arc::new(move |turn: FakeTurn| {
        let asked = asked.clone();
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") && asked.fetch_add(1, Ordering::SeqCst) > 0 {
                turn.write("greeting.txt", "hello from main and the agent\n");
            }
            turn.say("ok");
        })
    });
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let (core, repo) = start(dir.path(), idle, agent, turns.clone()).await;
    let remote = dir.path().join("remote.git");
    sh(dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
    sh(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let source = PrSource::Agent(solo.id.clone());
    let thread = format!("agent:{}", solo.id);
    let wt = PathBuf::from(solo.worktree.clone().unwrap());
    std::fs::write(wt.join("greeting.txt"), "hello from the agent\n").unwrap();
    let mine = commit(&repo, "greeting.txt", "hello from main\n");
    // Its branch has no commit of its own yet: there is nothing to merge, so it is not "behind".
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.behind), (0, 0));

    // Merge: refused, with the file and both ways out.
    let err = core.merge_agent(&solo.id).await.unwrap_err().to_string();
    let branch = sh(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(err.contains("merging into main conflicts in greeting.txt") && err.ends_with(&format!("git merge {branch}")), "{err}");
    assert!(err.contains(&format!("{} (from its Git tab) can resolve it", solo.name)), "{err}");
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.target.as_str(), state.branch.as_str(), &state.conflicts), ("main", branch.as_str(), &vec!["greeting.txt".to_string()]));
    assert_eq!(state.behind, 1, "its work is committed now, and main has your one commit that it was not built on");

    // Have the agent resolve it: the merge is started in its own worktree and it is asked in its conversation.
    core.resolve_merge_conflict(&source).await.unwrap();
    assert!(in_merge(&wt));
    let ask = wait_for("the agent to be asked", 20, async || {
        turns.lock().unwrap().iter().find(|(role, _, _)| *role == Role::Solo).map(|(_, cwd, prompt)| (cwd.clone(), prompt.clone()))
    })
    .await;
    assert_eq!(std::fs::canonicalize(&ask.0).unwrap(), std::fs::canonicalize(&wt).unwrap());
    assert!(ask.1.contains("Please resolve the merge conflicts with main") && ask.1.contains("greeting.txt"), "{}", ask.1);

    // Its turn ended with the markers still there: Codly commits nothing and merges nothing.
    let err = wait_for("the unresolved merge to be refused", 20, async || {
        let err = core.merge_agent(&solo.id).await.expect_err("nothing merges while markers are left").to_string();
        (!err.contains("is still working")).then_some(err)
    })
    .await;
    assert!(err.contains("has not finished resolving a merge: greeting.txt still has conflict markers"), "{err}");
    assert!(in_merge(&wt), "the merge stays open for its next turn");
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);
    // Its conversation says that the turn's work is not committed, and the Git tab knows it is on it.
    let told = format!("{}'s turn ended, but greeting.txt still has conflict markers, so Codly did not commit this turn's work.", solo.name);
    wait_for("its conversation to say so", 20, async || said(&core, &thread).await.iter().any(|l| l.starts_with(&told)).then_some(())).await;
    assert_eq!(core.merge_state(&source).await.unwrap().resolving.as_deref(), Some(solo.id.as_str()));
    // Nor is the half-resolved merge committed for a push: nothing goes to the remote.
    let confirm = PrConfirm { remote: "origin".into(), branch: branch.clone(), base: "main".into() };
    let err = core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
    assert!(err.contains("has not finished resolving a merge: greeting.txt still has conflict markers"), "{err}");
    assert_eq!(sh(&repo, &["ls-remote", "origin"]), "");
    assert!(in_merge(&wt));

    // Asked again, it resolves them; Codly commits the merge when that turn ends, and the merge goes through.
    core.send_chat(&thread, "Please finish resolving it").await.unwrap();
    wait_for("Codly to commit the resolved merge", 20, async || (!in_merge(&wt)).then_some(())).await;
    assert_eq!(sh(&wt, &["status", "--porcelain"]), "");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.resolving), (0, None));
    let sha = wait_for("the merge to go through", 20, async || match core.merge_agent(&solo.id).await {
        Ok(sha) => Some(sha),
        Err(e) if e.to_string().contains("is still working") => None,
        Err(e) => panic!("{e}"),
    })
    .await;
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from main and the agent\n");
    // Merged: main has the merge commit, which the agent's branch does not have, but there is
    // nothing left to merge, so its Git tab does not say "behind".
    assert_ne!(sh(&repo, &["rev-parse", &branch]), sha);
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.resolving, state.behind), (0, None, 0));
    core.shutdown().await;
}

/// A worker for a plan whose resolution takes two tries: asked to resolve, the first agent
/// reports that it cannot; the next one resolves the conflict.
fn gives_up_once(tries: Arc<AtomicUsize>) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        let tries = tries.clone();
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                if tries.fetch_add(1, Ordering::SeqCst) == 0 {
                    turn.tool("codly_submit_report", json!({ "status": "blocked", "summary": "I cannot tell which greeting you want." })).await;
                    return;
                }
                turn.write("greeting.txt", RESOLVED);
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    })
}

/// The agent removed the markers, Codly committed the merge on the plan's branch, and then the
/// plan's check failed on it through every fix round. You cancel the task: the plan's branch
/// is where it was before the merge, so the resolution that failed its check cannot be merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resolution_that_failed_its_checks_is_taken_off_the_plans_branch() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            let greeting = turn.read("greeting.txt");
            if greeting.contains("<<<<<<<") {
                // No marker is left, but the plan's check (`grep -q plan`) fails on this.
                turn.write("greeting.txt", "hello from nobody\n");
            } else if greeting == "hello\n" {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let check = "grep -q plan greeting.txt";
    let (core, repo) = start(dir.path(), lead_with("greeting.txt", check, &[]), worker, Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "resolve", TaskStatus::Failed).await;
    assert!(task.status_reason.as_deref().unwrap().contains(&format!("`{check}` failed")), "{:?}", task.status_reason);
    // The merge is committed on the plan's branch, with a greeting that failed the check.
    assert!(git(&repo, &["merge-base", "--is-ancestor", &mine, &branch]).status.success());
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from nobody\n");
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("the plan is still running"), "{err}");

    // You cancel the task: the plan is done again, on the branch as its tasks left it.
    core.move_task(&task.id, TaskStatus::Cancelled).await.unwrap();
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip, "the plan's branch is back where it was");
    assert_eq!((&again.base_commit, &again.base_branch), (&plan.base_commit, &plan.base_branch));
    assert!(!in_merge(&integ_wt));
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "");
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n");
    let lines = said(&core, "lead").await;
    assert!(lines.iter().any(|l| l.starts_with("The conflict with your branch was not resolved. Codly took the unfinished merge back")), "{lines:?}");

    // The conflict is still there, and nothing of the failed resolution can reach your branch.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.conflicts, state.resolving), (&vec!["greeting.txt".to_string()], None));
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (mine, String::new()));
    core.shutdown().await;
}

/// Cancel plan while an agent resolves the conflict of a finished plan: the resolution is
/// cancelled, not the plan. The plan is done again, as it was, and can still be merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_plan_during_a_resolution_cancels_the_resolution_only() {
    let dir = tempfile::tempdir().unwrap();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, gives_up_once(Default::default()), Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "resolve", TaskStatus::Blocked).await;
    assert!(in_merge(&integ_wt));
    assert_eq!(core.snapshot().await.unwrap().plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Running);

    core.cancel_plan(&plan.id).await.unwrap();
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!((&again.id, &again.base_commit, &again.base_branch), (&plan.id, &plan.base_commit, &plan.base_branch));
    let s = core.snapshot().await.unwrap();
    let cancelled = s.tasks.iter().find(|t| t.id == task.id).unwrap();
    assert_eq!((cancelled.status, cancelled.status_reason.as_deref()), (TaskStatus::Cancelled, Some("conflict resolution cancelled by you")));
    assert_eq!(s.tasks.iter().find(|t| t.key == "t1").unwrap().status, TaskStatus::Done);
    assert!(!in_merge(&integ_wt), "no merge is left open in the plan's worktree");
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.conflicts, state.resolving), (&vec!["greeting.txt".to_string()], None));

    // The finished work is not lost: another try resolves it, and the plan is merged.
    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// The plan started from main and your checkout is on another branch now: like the merge
/// itself, the resolution brings that branch into the plan only after you confirm it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_from_another_branch_is_resolved_only_after_you_confirm_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                assert!(turn.read("greeting.txt").contains(">>>>>>> refs/heads/feature-x"), "{}", turn.read("greeting.txt"));
                turn.write("greeting.txt", RESOLVED);
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, worker, Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    sh(&repo, &["checkout", "-q", "-b", "feature-x"]);
    let mine = commit(&repo, "greeting.txt", "hello from feature-x\n");
    assert_eq!(core.merge_state(&source).await.unwrap().target, "feature-x");

    // Not confirmed: nothing starts, and nothing changed.
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert_eq!(err, "This plan started from main, and your checkout is on feature-x now. Nothing was changed: have it resolved again and confirm feature-x to go on.");
    let err = core.resolve_merge_conflict_into(&source, "main").await.unwrap_err().to_string();
    assert_eq!(err, "your checkout is on feature-x now, not main; nothing was changed");
    let s = core.snapshot().await.unwrap();
    assert!(!s.tasks.iter().any(|t| t.key == "resolve"));
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Done);
    assert_eq!((sh(&repo, &["rev-parse", &branch]), in_merge(&integ_wt)), (tip, false));

    // Confirmed: feature-x comes into the plan, which builds on it from then on and merges into it.
    let state = core.resolve_merge_conflict_into(&source, "feature-x").await.unwrap();
    assert!(state.resolving.is_some());
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!((again.base_commit.as_deref(), again.base_branch.as_deref()), (Some(mine.as_str()), Some("feature-x")));
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!((sh(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]), sh(&repo, &["rev-parse", "HEAD"])), ("feature-x".to_string(), sha));
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// The worktree for the checks on the merge result cannot be set up (the repo's setup script
/// fails there): nothing is merged, the plan's card says why, and you can still merge anyway.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_whose_checks_could_not_run_is_not_made_until_you_merge_anyway() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("feature.txt", "on\n");
            turn.done().await;
        })
    });
    let (core, repo) = start(dir.path(), lead_with("feature.txt", "test -f feature.txt", &[]), worker, Default::default()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());

    // You turn on a setup script for new worktrees, and it fails with what main has now.
    std::fs::create_dir_all(repo.join(".codly")).unwrap();
    std::fs::write(repo.join(".codly/setup.sh"), "#!/bin/sh\necho no network >&2\ntest ! -f offline\n").unwrap();
    let setup = core.worktree_setup(None).await.unwrap();
    assert!(setup.sha.is_some(), "{setup:?}");
    assert!(core.worktree_setup(Some(SetupChoice { allow: setup.sha })).await.unwrap().on);
    let moved = commit(&repo, "offline", "1\n");

    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.starts_with("the plan's checks could not run on the merge result, so nothing was merged and main is unchanged: .codly/setup.sh exited with code 1") && err.contains("no network"), "{err}");
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), moved);
    assert!(!repo.join("feature.txt").exists());
    no_scratch_left(dir.path());
    let state = core.merge_state(&source).await.unwrap();
    assert!(state.conflicts.is_empty() && state.failed_checks.is_empty(), "{state:?}");
    assert!(state.checks_not_run.as_deref().is_some_and(|why| why.starts_with(".codly/setup.sh exited with code 1")), "{state:?}");
    let lines = said(&core, "lead").await;
    assert!(lines.last().unwrap().starts_with("Not merged: Codly could not run the plan's checks on \"Change the greeting\" merged into main, because the worktree for them was not set up: .codly/setup.sh exited with code 1"), "{lines:?}");
    assert!(verified(&core).await.iter().all(|(task, _, _)| task.is_some()), "no check ran on the merge result");

    // Merge anyway: your choice, and the reason is history.
    let sha = core.merge_plan_unchecked(&plan.id, None).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert!(repo.join("feature.txt").exists());
    assert_eq!(core.merge_state(&source).await.unwrap().checks_not_run, None);
    core.shutdown().await;
}

/// The agent left the markers in through all its rounds: the task fails and the merge is
/// undone. Retry starts the merge again, and the next agent resolves it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_starts_the_merge_again_after_the_rounds_ran_out() {
    let dir = tempfile::tempdir().unwrap();
    let left = Arc::new(AtomicUsize::new(0));
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let left = left.clone();
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                // Its first turn and both fix rounds end with the markers still there.
                if left.fetch_add(1, Ordering::SeqCst) >= 3 {
                    turn.write("greeting.txt", RESOLVED);
                }
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, worker, Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "resolve", TaskStatus::Failed).await;
    assert!(task.status_reason.as_deref().unwrap().contains("after 2 fix rounds"), "{:?}", task.status_reason);
    assert!(!in_merge(&integ_wt), "the merge nobody resolved is undone");
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n");
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip);
    assert_eq!(core.merge_state(&source).await.unwrap().resolving.as_deref(), Some(task.id.as_str()), "the task can still be retried");

    // Retry: Codly starts the merge again in the plan's worktree, and this agent resolves it.
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(core.snapshot().await.unwrap().tasks.iter().find(|t| t.id == task.id).unwrap().status, TaskStatus::Done);
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), RESOLVED);
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// Codly is closed while a resolution waits for you: the merge in the plan's worktree is not
/// one a crash left half-done, so the next start keeps it, and the task goes on from there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_of_codly_keeps_the_merge_of_an_open_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let tries = Arc::new(AtomicUsize::new(0));
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead.clone(), gives_up_once(tries.clone()), Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    commit(&repo, "greeting.txt", "hello from main\n");
    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "resolve", TaskStatus::Blocked).await;
    assert!(in_merge(&integ_wt));
    core.shutdown().await;
    drop(core);

    let core = boot(dir.path(), &repo, lead, gives_up_once(tries), Default::default()).await;
    assert!(in_merge(&integ_wt), "the open resolution's merge is still there after the restart");
    assert!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap().contains("<<<<<<<"));
    let s = core.snapshot().await.unwrap();
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Running);
    assert_eq!(core.merge_state(&source).await.unwrap().resolving.as_deref(), Some(task.id.as_str()));

    // Retry: the next agent finds the markers and resolves them.
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), RESOLVED);
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// Your branch moves while the plan's checks run on the merge result: what they checked is
/// not what would be merged now, so nothing is merged, and the next try checks again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_is_not_made_when_your_branch_moved_while_its_checks_ran() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("feature.txt", "on\n");
            turn.done().await;
        })
    });
    // The check waits for as long as this file is there.
    let hold = dir.path().join("hold");
    let check = format!("while [ -f '{}' ]; do sleep 0.05; done", hold.display());
    let (core, repo) = start(dir.path(), lead_with("feature.txt", &check, &[]), worker, Default::default()).await;
    // Codly runs the check by itself, whatever the approval rules see in its path.
    core.update_settings(Settings { approval_mode: ApprovalMode::Full, ..core.settings() }).await.unwrap();
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;

    commit(&repo, "mode.txt", "changed\n");
    std::fs::write(&hold, "").unwrap();
    let merging = tokio::spawn({
        let (core, id) = (core.clone(), plan.id.clone());
        async move { core.merge_plan(&id).await }
    });
    // The merge result is in the scratch worktree and its check runs: you commit once more.
    wait_for("the check on the merge result to start", 40, async || scratch_dirs(dir.path()).iter().any(|s| s.join("feature.txt").exists()).then_some(())).await;
    let moved = commit(&repo, "README.md", "# demo\nmore\n");
    std::fs::remove_file(&hold).unwrap();
    let err = merging.await.unwrap().unwrap_err().to_string();
    assert_eq!(err, "main or the plan's branch changed while the checks ran, so they say nothing about this merge; nothing was merged. Merge again.");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (moved, String::new()));
    assert!(!repo.join("feature.txt").exists());
    assert_eq!(core.snapshot().await.unwrap().plans.iter().find(|p| p.id == plan.id).unwrap().merged_sha, None);
    no_scratch_left(dir.path());

    // Again: the check runs on the merge with main as it is now, and the merge is made.
    let runs = verified(&core).await.len();
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(verified(&core).await.len(), runs + 1);
    assert!(repo.join("feature.txt").exists());
    core.shutdown().await;
}

/// The agent cannot resolve it: you stop the resolution, and its worktree is no longer in a
/// merge. Its branch is as it was, the conflict is still there, and its turns are committed again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn you_can_stop_a_solo_agents_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let (core, repo) = start(dir.path(), idle.clone(), idle, Default::default()).await;
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let source = PrSource::Agent(solo.id.clone());
    let thread = format!("agent:{}", solo.id);
    let wt = PathBuf::from(solo.worktree.clone().unwrap());
    std::fs::write(wt.join("greeting.txt"), "hello from the agent\n").unwrap();
    commit(&repo, "greeting.txt", "hello from main\n");
    assert_eq!(core.merge_state(&source).await.unwrap().resolving, None);
    let err = core.stop_resolving_merge(&solo.id).await.unwrap_err().to_string();
    assert_eq!(err, format!("{} is not resolving a merge", solo.name));

    // Asked to resolve, its turn ends with the markers still there.
    core.resolve_merge_conflict(&source).await.unwrap();
    wait_for("its turn to end without a commit", 20, async || {
        said(&core, &thread).await.iter().any(|l| l.contains("so Codly did not commit this turn's work")).then_some(())
    })
    .await;
    let tip = sh(&wt, &["rev-parse", "HEAD"]);
    assert!(in_merge(&wt));

    // Stop resolving: the merge is taken out of its worktree; nothing of it is in its branch.
    let state = wait_for("the resolution to stop", 20, async || match core.stop_resolving_merge(&solo.id).await {
        Ok(state) => Some(state),
        Err(e) if e.to_string().contains("is still working") => None,
        Err(e) => panic!("{e}"),
    })
    .await;
    assert_eq!((&state.conflicts, state.resolving), (&vec!["greeting.txt".to_string()], None));
    assert!(!in_merge(&wt));
    assert_eq!((sh(&wt, &["rev-parse", "HEAD"]), sh(&wt, &["status", "--porcelain"])), (tip, String::new()));
    assert_eq!(std::fs::read_to_string(wt.join("greeting.txt")).unwrap(), "hello from the agent\n");
    let lines = said(&core, &thread).await;
    assert!(lines.last().unwrap().starts_with("You stopped the conflict resolution. Codly took the merge out of"), "{lines:?}");
    core.shutdown().await;
}

/// `worker` for every task, and a reviewer that approves what it is given.
fn with_reviewer(worker: Behavior) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        let worker = worker.clone();
        Box::pin(async move {
            if turn.spec.role != Role::Reviewer {
                return worker(turn).await;
            }
            let reply = turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": [], "issues": [] })).await;
            assert_eq!(reply["ok"], true, "{reply}");
        })
    })
}

/// With reviews on, the task that resolved a conflict is reviewed like every task. Its
/// reviewer reads the resolution only: the conflict as git left it against what the worker
/// made of it, and none of your own commits that the merge brought in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_reviewer_of_a_resolution_reads_the_resolution_only() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, with_reviewer(plan_worker()), turns.clone()).await;
    core.update_settings(Settings { review_gate: true, ..core.settings() }).await.unwrap();
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    // You kept working on main: the same line, and a file the plan never touched.
    std::fs::write(repo.join("README.md"), "# demo\nmine\n").unwrap();
    commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_with(&core, "resolve", TaskStatus::Done).await.summary.as_deref(), Some("Did it."));
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| d == "resolve: review verdict approve"), "the resolution was reviewed: {record:?}");

    let briefs: Vec<String> = turns.lock().unwrap().iter().filter(|(role, _, _)| *role == Role::Reviewer).map(|(_, _, prompt)| prompt.clone()).collect();
    assert_eq!(briefs.len(), 2, "t1 and the resolution each got one review");
    // t1: its whole change, as for every task.
    assert!(briefs[0].contains("[Codly] Review task t1:") && briefs[0].contains("+hello from the plan") && !briefs[0].contains("About the diff:"), "{}", briefs[0]);
    // The resolution: both sides of the conflict, what the worker made of it, and a word on what the diff is.
    let brief = &briefs[1];
    assert!(brief.contains("[Codly] Review task resolve: Resolve conflicts with main"), "{brief}");
    assert!(brief.contains("\n-<<<<<<<") && brief.contains("\n-hello from the plan\n") && brief.contains("\n-hello from main\n") && brief.contains("\n+hello from main and the plan\n"), "{brief}");
    assert!(!brief.contains("mine") && !brief.contains("README.md"), "your own commit merged by itself: it is not for review: {brief}");
    assert!(brief.contains("About the diff: this task worked on a merge of the plan's branch with the user's branch.") && brief.contains("Check that each resolution keeps what both sides changed."), "{brief}");
    core.shutdown().await;
}

/// mode.txt as the task that fixes the checks leaves it: written from the plan's feature.txt
/// and main's mode.txt, which it finds merged in its worktree.
const FIXED: &str = "good with the feature on\nand mode bad\n";

/// A plan's worker for a check that fails once main moved: t1 adds the feature, and the task
/// that fixes the checks makes mode.txt good again. mode.txt is not a file the plan changed,
/// so it is outside that task's write scope, and the report says so. `fixes` = false: that
/// task gives up.
fn feature_worker(fixes: bool) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if !turn.prompt.contains("Task fix-checks") {
                turn.write("feature.txt", "on\n");
            } else if fixes {
                // What it finds there: the plan's work and main's change, merged.
                turn.write("mode.txt", &format!("good with the feature {}and mode {}", turn.read("feature.txt"), turn.read("mode.txt")));
                let report = json!({ "status": "done", "summary": "Did it.", "scope_exceptions": ["mode.txt: the check reads it"] });
                turn.tool("codly_submit_report", report).await;
                return;
            } else {
                turn.write("attempt.txt", "half an idea\n");
                turn.tool("codly_submit_report", json!({ "status": "blocked", "summary": "I cannot tell which mode you want." })).await;
                return;
            }
            turn.done().await;
        })
    })
}

/// Your branch and the plan merge without a conflict, but a check of the plan fails on the
/// result. "Have an agent fix it": Codly brings your branch into the plan's branch in its own
/// worktree, one more task of the plan makes the check pass there, and the merge goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_fixes_checks_that_fail_on_the_merge_result() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), feature_worker(true), turns.clone()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let bad = commit(&repo, "mode.txt", "bad\n");

    // Nothing conflicts, and no check is known to fail yet: an agent has nothing to do, and
    // main is not brought into the plan's branch behind the checks of the merge.
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert_eq!(err, format!("{branch} and main merge without a conflict now, so there is nothing to resolve; merge the plan (Codly runs its checks on the result first)"));
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip);
    assert_eq!(core.snapshot().await.unwrap().plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Done);

    core.merge_plan(&plan.id).await.unwrap_err();
    assert_eq!(core.merge_state(&source).await.unwrap().failed_checks, [check]);
    let lines = said(&core, "lead").await;
    assert!(lines.last().unwrap().ends_with("have an agent fix it (on the plan's card), or merge anyway if you know why it fails."), "{lines:?}");

    // Have an agent fix it: main comes into the plan's branch, and a task works on the result.
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let s = core.snapshot().await.unwrap();
    let task = s.tasks.iter().find(|t| t.key == "fix-checks").expect("a task that fixes the checks").clone();
    assert_eq!((state.conflicts.len(), state.resolving.as_deref(), state.failed_checks.len(), state.behind), (0, Some(task.id.as_str()), 0, 0));
    assert_eq!((task.title.as_str(), task.branch.as_deref(), task.worktree.as_deref()), ("Fix the checks with main merged in", Some(branch.as_str()), plan.integration_worktree.as_deref()));
    assert_eq!((&task.write_scope, &task.verify), (&vec!["feature.txt".to_string()], &vec![check.to_string()]));
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Running);
    let lines = said(&core, "lead").await;
    assert!(lines.iter().any(|l| l.starts_with(&format!("main and this plan merge without a conflict, but `{check}` failed on the result. Codly brought main into the plan's branch"))), "{lines:?}");

    // The task goes through the gate with the plan's check, and the plan finishes again.
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(task_with(&core, "fix-checks", TaskStatus::Done).await.id, task.id);
    assert_eq!(std::fs::read_to_string(integ_wt.join("mode.txt")).unwrap(), FIXED, "it worked on the merge of the plan with main");
    assert_eq!((again.base_commit.as_deref(), again.base_branch.as_deref()), (Some(bad.as_str()), Some("main")));
    {
        let turns = turns.lock().unwrap();
        let (_, cwd, brief) = turns.iter().find(|(role, _, prompt)| *role == Role::Worker && prompt.contains("Task fix-checks")).expect("the fixing task's brief");
        assert_eq!(std::fs::canonicalize(cwd).unwrap(), std::fs::canonicalize(&integ_wt).unwrap());
        assert!(brief.contains(&format!("these checks fail: `{check}`")) && brief.contains("Codly merged main into this worktree"), "{brief}");
    }
    let runs = verified(&core).await;
    assert_eq!(runs.last().unwrap(), &(Some(task.id.clone()), check.to_string(), true), "the check passed in the task's gate, on the merge");
    // mode.txt is outside its write scope (the files the plan changed) and was declared.
    let in_scope = "fix-checks: verify passed (1 command), 1 file changed within scope plus 1 declared exception(s)";
    assert!(decisions(&core).await.iter().any(|d| d == in_scope), "{:?}", decisions(&core).await);
    // All along, your checkout was not touched.
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (bad.clone(), String::new()));

    // main is in the plan's branch now: the merge is made, with no check to run again.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.resolving, state.failed_checks.len(), state.behind), (0, None, 0, 0));
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(verified(&core).await.len(), runs.len());
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (sha, Some(bad)));
    assert_eq!(std::fs::read_to_string(repo.join("mode.txt")).unwrap(), FIXED);
    assert_eq!(std::fs::read_to_string(repo.join("feature.txt")).unwrap(), "on\n");
    core.shutdown().await;
}

/// The agent cannot make the check pass and you cancel its task: the merge of your branch is
/// taken off the plan's branch, the file the agent left in the plan's worktree goes, and the
/// plan is done as it was, with the same choice as before. A new file that was in the plan's
/// worktree before (a check's output) is not in the merge Codly commits, and it stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fix_of_the_checks_that_does_not_finish_takes_the_merge_back() {
    let dir = tempfile::tempdir().unwrap();
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), feature_worker(false), Default::default()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    // In the plan's worktree before the fix: what a check left there.
    std::fs::write(integ_wt.join("junit.xml"), "<testsuite/>\n").unwrap();
    let bad = commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "fix-checks", TaskStatus::Blocked).await;
    assert!(git(&repo, &["merge-base", "--is-ancestor", &bad, &branch]).status.success(), "the merge of main is on the plan's branch");
    assert!(integ_wt.join("attempt.txt").exists());
    // The merge is committed as git made it: the plan's tip and main, and no file of the worktree.
    assert_eq!(sh(&repo, &["rev-parse", &format!("{branch}^1"), &format!("{branch}^2")]), format!("{tip}\n{bad}"));
    assert_eq!(sh(&repo, &["ls-tree", "--name-only", &branch, "junit.xml"]), "", "the check's output is not in the merge commit");
    assert_eq!(sh(&integ_wt, &["status", "--porcelain", "--", "junit.xml"]), "?? junit.xml");
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("the plan is still running"), "{err}");

    core.move_task(&task.id, TaskStatus::Cancelled).await.unwrap();
    let again = wait_plan(&core, PlanStatus::Done).await;
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip, "the plan's branch is back where it was");
    assert_eq!((&again.base_commit, &again.base_branch), (&plan.base_commit, &plan.base_branch));
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "?? junit.xml", "the file the agent left is gone with the merge, the one from before is still there");
    assert_eq!(std::fs::read_to_string(integ_wt.join("junit.xml")).unwrap(), "<testsuite/>\n");
    assert_eq!(std::fs::read_to_string(integ_wt.join("mode.txt")).unwrap(), "good\n");
    let lines = said(&core, "lead").await;
    assert!(lines.iter().any(|l| l.starts_with("The plan's checks still fail with your branch merged in. Codly took that merge back")), "{lines:?}");

    // Where you were: the check still fails on this merge, and both ways on are there again.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.failed_checks, state.resolving, state.behind), (&vec![check.to_string()], None, 1));
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (bad, String::new()));
    core.shutdown().await;
}

/// A resolution that does not finish takes the new files of its worker out of the plan's
/// worktree: they are on no branch, and the plan's checks must not see them. What was there
/// before the task started (worktree setup, a check's output) stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_new_files_of_a_resolution_that_did_not_finish_go() {
    let dir = tempfile::tempdir().unwrap();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.read("greeting.txt").contains("<<<<<<<") {
                // Notes and a scratch folder, and then it gives up.
                std::fs::create_dir_all(turn.spec.cwd.join("scratch/deep")).unwrap();
                turn.write("scratch/deep/try.txt", "one try\n");
                turn.write("notes.txt", "what I found\n");
                turn.tool("codly_submit_report", json!({ "status": "blocked", "summary": "I cannot tell which greeting you want." })).await;
                return;
            }
            turn.write("greeting.txt", "hello from the plan\n");
            turn.done().await;
        })
    });
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, worker, Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    // In the plan's worktree before any resolution: what setup or a check left there.
    std::fs::create_dir_all(integ_wt.join("cache")).unwrap();
    std::fs::write(integ_wt.join("cache/built.txt"), "from a check\n").unwrap();
    std::fs::write(integ_wt.join("local.env"), "PORT=1\n").unwrap();
    commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    let task = task_with(&core, "resolve", TaskStatus::Blocked).await;
    assert!(integ_wt.join("notes.txt").exists() && integ_wt.join("scratch/deep/try.txt").exists());

    core.move_task(&task.id, TaskStatus::Cancelled).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    assert!(!integ_wt.join("notes.txt").exists() && !integ_wt.join("scratch").exists(), "the worker's new files are gone");
    assert_eq!(std::fs::read_to_string(integ_wt.join("cache/built.txt")).unwrap(), "from a check\n");
    assert_eq!(std::fs::read_to_string(integ_wt.join("local.env")).unwrap(), "PORT=1\n");
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "?? cache/\n?? local.env".to_string());
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n");
    core.shutdown().await;
}

/// Makes the store refuse one write of a resolution's start (`write`: the trigger's event, in
/// SQL), or lets every write through again (None).
fn store_refuses(root: &Path, write: Option<&str>) {
    store_trigger(root, "refused", write);
}

/// The same under a name of its own, for a second write that the store refuses at the same
/// time.
fn store_trigger(root: &Path, name: &str, write: Option<&str>) {
    let db = rusqlite::Connection::open(root.join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let sql = match write {
        Some(write) => format!("CREATE TRIGGER {name} BEFORE {write} BEGIN SELECT RAISE(ABORT, 'the disk is full'); END"),
        None => format!("DROP TRIGGER {name}"),
    };
    db.execute_batch(&sql).unwrap();
}

/// The record a resolution keeps of where the plan stood: its first write once the merge is in
/// the plan's worktree, before the task exists.
const THE_RECORD: &str = "INSERT ON kv WHEN NEW.key LIKE 'resolution:%'";
/// The commit a new task's work is measured from: written after the task exists.
const THE_TASKS_BASE: &str = "UPDATE OF base_commit ON task";

/// Codly merged your branch into the plan's branch and committed the merge, and then the task
/// that fixes the checks could not be made. The merge is taken back at once: left there, the
/// next Merge would find your branch in the plan's already and check nothing. So the next
/// Merge runs the plan's check on the result again, and refuses again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fix_of_the_checks_that_cannot_start_leaves_no_unchecked_merge_behind() {
    let dir = tempfile::tempdir().unwrap();
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), feature_worker(true), Default::default()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let bad = commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    store_refuses(dir.path(), Some(THE_RECORD));
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert!(err.contains("the disk is full") && err.ends_with(&format!(". Codly took the merge of main back: {branch} is as it was")), "{err}");
    assert_eq!(sh(&repo, &["rev-parse", &branch]), tip, "the plan's branch is where it was before the call");
    assert_eq!((in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (false, String::new()));
    let s = core.snapshot().await.unwrap();
    assert_eq!(s.plans.iter().find(|p| p.id == plan.id).unwrap().status, PlanStatus::Done);
    assert!(!s.tasks.iter().any(|t| t.key.starts_with("fix-checks")), "no task was made");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.failed_checks, state.resolving, state.behind), (&vec![check.to_string()], None, 1));
    let record = decisions(&core).await;
    let taken_back = format!("; {branch} is back where it was before it");
    assert!(record.iter().any(|d| d.starts_with(&format!("bringing main into {branch} did not start (")) && d.ends_with(&taken_back)), "{record:?}");

    // The next Merge finds the plan as it was: the check runs on the result, and fails.
    let runs = verified(&core).await.len();
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert_eq!(err, format!("with the plan merged into main, `{check}` failed, so nothing was merged and main is unchanged"));
    assert_eq!(verified(&core).await.len(), runs + 1, "the check ran on the merge result again");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["rev-parse", &branch])), (bad, tip));

    // The store takes writes again: the fix starts as if nothing had happened.
    store_refuses(dir.path(), None);
    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    task_with(&core, "fix-checks", TaskStatus::Done).await;
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("mode.txt")).unwrap(), FIXED);
    core.shutdown().await;
}

/// The same with a conflict: the merge is open in the plan's worktree, with its markers, the
/// task that resolves it exists, and then its start fails. The merge is taken out, the task
/// is cancelled, and the plan is done as it was. A second try gets a task of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resolution_that_cannot_start_leaves_no_open_merge_behind() {
    let dir = tempfile::tempdir().unwrap();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, plan_worker(), Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    store_refuses(dir.path(), Some(THE_TASKS_BASE));
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert!(err.contains("the disk is full") && err.ends_with(&format!(". Codly took the merge of main back: {branch} is as it was")), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", &branch]), in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (tip, false, String::new()));
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n", "no marker is left");
    let s = core.snapshot().await.unwrap();
    let again = s.plans.iter().find(|p| p.id == plan.id).unwrap();
    assert_eq!((again.status, &again.base_commit, &again.base_branch), (PlanStatus::Done, &plan.base_commit, &plan.base_branch));
    let task = s.tasks.iter().find(|t| t.key == "resolve").expect("the task had reached the store");
    assert_eq!((task.status, task.status_reason.as_deref()), (TaskStatus::Cancelled, Some("Codly could not start it")));
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.conflicts, state.resolving), (&vec!["greeting.txt".to_string()], None));
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);

    store_refuses(dir.path(), None);
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let second = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve-2").expect("a second resolving task");
    assert_eq!(state.resolving.as_deref(), Some(second.id.as_str()));
    wait_plan(&core, PlanStatus::Done).await;
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// The rows of Codly's store whose key starts with `prefix`, read through a connection of the
/// test's own.
fn kv_rows(root: &Path, prefix: &str) -> Vec<(String, String)> {
    let db = rusqlite::Connection::open(root.join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let mut st = db.prepare("SELECT key, value FROM kv WHERE key LIKE ?1 ORDER BY key").unwrap();
    let rows = st.query_map([format!("{prefix}%")], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    rows
}

/// What a start keeps on record while it is under way, and what a resolution keeps until it
/// is settled.
fn start_records(root: &Path) -> Vec<String> {
    let rows = kv_rows(root, "merge_start:").into_iter().chain(kv_rows(root, "resolution:"));
    rows.map(|(key, _)| key).collect()
}

/// Codly was closed in the middle of a start: it had written the start down, merged your
/// branch into the plan's branch and committed the merge, and no task was made yet. Nothing
/// else is on record. The next Merge takes that merge back first: left there, it would find
/// your branch in the plan's already and check nothing. So the plan's check runs on the
/// result again, and the record says what happened. A start that completes leaves no record
/// of itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_that_codly_was_closed_in_the_middle_of_is_taken_back_before_the_merge() {
    let dir = tempfile::tempdir().unwrap();
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), feature_worker(true), Default::default()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let bad = commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    // That state, made by hand: the start on record (from another run of Codly), and the
    // merge of main committed on the plan's branch.
    let start = json!({ "tip": tip, "target": "main", "head": bad, "run": "an earlier run" });
    let db = rusqlite::Connection::open(dir.path().join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    db.execute("INSERT INTO kv (key, value) VALUES (?1, ?2)", [format!("merge_start:{}", plan.id), start.to_string()]).unwrap();
    drop(db);
    sh(&integ_wt, &["merge", "-q", "--no-ff", "-m", &format!("codly: merge main into {branch}"), "main"]);
    assert!(git(&repo, &["merge-base", "--is-ancestor", &bad, &branch]).status.success(), "main is in the plan's branch");
    assert!(!core.snapshot().await.unwrap().tasks.iter().any(|t| t.key.starts_with("fix-checks")), "no task is on it");

    // The next Merge finds the plan as it was before the start: the check runs on the result.
    let runs = verified(&core).await.len();
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert_eq!(err, format!("with the plan merged into main, `{check}` failed, so nothing was merged and main is unchanged"));
    assert_eq!(verified(&core).await.len(), runs + 1, "the check ran on the merge result again");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["rev-parse", &branch])), (bad.clone(), tip.clone()));
    assert_eq!((in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (false, String::new()));
    let record = decisions(&core).await;
    let taken_back = format!("bringing main into {branch} did not finish when Codly closed; {branch} is back where it was before it");
    assert_eq!(record.iter().filter(|d| **d == taken_back).count(), 1, "{record:?}");
    let lines = said(&core, "lead").await;
    assert!(lines.iter().any(|l| l.starts_with("Bringing main into the plan's branch did not finish when Codly closed. Codly took that merge back")), "{lines:?}");
    assert_eq!(start_records(dir.path()), Vec::<String>::new(), "the start is not on record any more");
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((&state.failed_checks, state.resolving, state.behind), (&vec![check.to_string()], None, 1));

    // The fix starts as if nothing had happened, and a start that completes leaves no record.
    core.resolve_merge_conflict(&source).await.unwrap();
    assert_eq!(kv_rows(dir.path(), "merge_start:"), vec![], "{:?}", start_records(dir.path()));
    wait_plan(&core, PlanStatus::Done).await;
    task_with(&core, "fix-checks", TaskStatus::Done).await;
    assert_eq!(start_records(dir.path()), Vec::<String>::new());
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("mode.txt")).unwrap(), FIXED);
    core.shutdown().await;
}

/// Every record of a merge that Codly keeps in the store, the first one a start writes after
/// the merge and the one a take-back writes.
const A_MERGE_RECORD: &str = "INSERT ON decision WHEN NEW.type = 'merge'";

/// A start fails after the merge, Codly takes the merge back, and then it cannot write down
/// that it did: the store refuses that too. The error says what is true: the plan's branch
/// is back where it was. It does not say that the branch may have your branch merged in, and
/// it gives no git command to run. What could not be written down then is written down
/// before the next Merge, and that Merge checks the plan as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_take_back_that_cannot_be_written_down_still_says_the_branch_is_back() {
    let dir = tempfile::tempdir().unwrap();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, plan_worker(), Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    store_refuses(dir.path(), Some(A_MERGE_RECORD));
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    let back = format!(". Codly took the merge of main back: {branch} is as it was. It could not write that down: ");
    let (why, not_written) = err.split_once(&back).unwrap_or_else(|| panic!("{err}"));
    assert!(why.contains("the disk is full") && not_written.contains("the disk is full"), "{err}");
    assert!(!err.contains("could not take the merge") && !err.contains("may have main merged in") && !err.contains("reset --hard"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", &branch]), in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (tip.clone(), false, String::new()));
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n", "no marker is left");
    let s = core.snapshot().await.unwrap();
    let again = s.plans.iter().find(|p| p.id == plan.id).unwrap();
    assert_eq!((again.status, &again.base_commit, &again.base_branch), (PlanStatus::Done, &plan.base_commit, &plan.base_branch));
    let task = s.tasks.iter().find(|t| t.key == "resolve").expect("the task had reached the store");
    assert_eq!((task.status, task.status_reason.as_deref()), (TaskStatus::Cancelled, Some("Codly could not start it")));
    assert_eq!(core.merge_state(&source).await.unwrap().resolving, None);
    assert_eq!(start_records(dir.path()).len(), 2, "the start is still on record: {:?}", start_records(dir.path()));

    // The store takes writes again. The next Merge writes down what happened, in words that
    // fit this run of Codly (it was not closed), and finds the plan as it was: the conflict.
    store_refuses(dir.path(), None);
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    let record = decisions(&core).await;
    let taken_back = format!("bringing main into {branch} did not finish; {branch} is back where it was before it");
    assert_eq!(record.iter().filter(|d| **d == taken_back).count(), 1, "{record:?}");
    assert_eq!(start_records(dir.path()), Vec::<String>::new());
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["rev-parse", &branch])), (mine, tip));

    // A second try gets a task of its own, and the merge goes through.
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let second = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve-2").expect("a second resolving task");
    assert_eq!(state.resolving.as_deref(), Some(second.id.as_str()));
    wait_plan(&core, PlanStatus::Done).await;
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// The write that cancels a task.
const A_TASK_CANCELLED: &str = "UPDATE OF status ON task WHEN NEW.status = 'cancelled'";

/// Codly was closed in the middle of a start, later than in the test of the start with no
/// task: the task that resolves the conflict was made, and the plan did not run again yet.
/// On record are the start, where the plan stood, and a task that waits; the merge is open in
/// the plan's worktree. The next Merge takes the merge out and cancels that task: left
/// waiting, it would start beside the task of the next resolution, in the same worktree. The
/// next resolution has one task, and the old one never runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_cut_short_after_its_task_was_made_cancels_that_task() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, plan_worker(), turns.clone()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    // That state, made with Codly's own writes as far as they go: a start that fails once its
    // task is made, and whose take-back cannot cancel the task. The start, where the plan
    // stood and the task stay on record.
    store_refuses(dir.path(), Some(A_MERGE_RECORD));
    store_trigger(dir.path(), "kept", Some(A_TASK_CANCELLED));
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert!(err.contains(&format!("Codly took the merge of main back: {branch} is as it was. It could not write that down: ")), "{err}");
    store_refuses(dir.path(), None);
    store_trigger(dir.path(), "kept", None);
    // The rest by hand: the start is one of another run of Codly, and the merge it made is
    // still open in the plan's worktree (that run was closed before it could take it back).
    let (start_key, start) = kv_rows(dir.path(), "merge_start:").pop().expect("the start is on record");
    let mut start: Value = serde_json::from_str(&start).unwrap();
    start["run"] = json!("an earlier run");
    let db = rusqlite::Connection::open(dir.path().join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(db.execute("UPDATE kv SET value = ?2 WHERE key = ?1", [start_key, start.to_string()]).unwrap(), 1);
    drop(db);
    assert!(!git(&integ_wt, &["merge", "main"]).status.success(), "the merge of main conflicts");
    assert!(in_merge(&integ_wt) && std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap().contains("<<<<<<<"));
    let s = core.snapshot().await.unwrap();
    let task = s.tasks.iter().find(|t| t.key == "resolve").expect("the task had reached the store").clone();
    let stood = s.plans.iter().find(|p| p.id == plan.id).unwrap();
    assert_eq!((task.status, stood.status, &stood.base_commit), (TaskStatus::Pending, PlanStatus::Done, &plan.base_commit));
    assert_eq!(start_records(dir.path()).len(), 2, "{:?}", start_records(dir.path()));

    // The next Merge takes the start back first: the merge is out, and the task is cancelled.
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    let s = core.snapshot().await.unwrap();
    let old = s.tasks.iter().find(|t| t.id == task.id).unwrap();
    assert_eq!((old.status, old.status_reason.as_deref()), (TaskStatus::Cancelled, Some("Codly could not start it")));
    assert_eq!(core.merge_state(&source).await.unwrap().resolving, None);
    assert_eq!((sh(&repo, &["rev-parse", &branch]), in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (tip.clone(), false, String::new()));
    assert_eq!(std::fs::read_to_string(integ_wt.join("greeting.txt")).unwrap(), "hello from the plan\n", "no marker is left");
    let record = decisions(&core).await;
    let taken_back = format!("bringing main into {branch} did not finish when Codly closed; {branch} is back where it was before it");
    assert_eq!(record.iter().filter(|d| **d == taken_back).count(), 1, "{record:?}");
    assert_eq!(start_records(dir.path()), Vec::<String>::new());
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["rev-parse", &branch])), (mine, tip));

    // The next resolution has a task of its own, the only one that runs, and the merge goes
    // through.
    let state = core.resolve_merge_conflict(&source).await.unwrap();
    let second = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve-2").expect("a second resolving task");
    assert_eq!(state.resolving.as_deref(), Some(second.id.as_str()));
    wait_plan(&core, PlanStatus::Done).await;
    let briefed: Vec<String> = turns
        .lock()
        .unwrap()
        .iter()
        .filter(|(role, _, _)| *role == Role::Worker)
        .filter_map(|(_, _, prompt)| prompt.split("[Codly] Task resolve").nth(1).and_then(|rest| rest.split(':').next()).map(|n| format!("resolve{n}")))
        .collect();
    assert_eq!(briefed, ["resolve-2"], "the task of the start that was cut short never ran");
    assert_eq!(core.snapshot().await.unwrap().tasks.iter().find(|t| t.id == task.id).unwrap().status, TaskStatus::Cancelled);
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// No agent works on a cancelled plan any more, and the refusal says what is left for you to
/// do: resolve a conflict, or, when the plan's checks failed on this very merge, fix the
/// checks. When that cannot be read, the refusal is the same plain words, not the error of
/// the read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_plan_says_what_is_left_for_you_to_do() {
    const CANCELLED: &str = "this plan was cancelled, so no agent works on it any more";
    let dir = tempfile::tempdir().unwrap();
    // The plan's last check passes only once this file is there: the plan fails, and you
    // cancel it. Its branch has the work of its one task.
    let released = dir.path().join("released");
    let last = format!("test -f '{}'", released.display());
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[last.as_str()]), feature_worker(true), Default::default()).await;
    // Codly runs the check by itself, whatever the approval rules see in its path.
    core.update_settings(Settings { approval_mode: ApprovalMode::Full, ..core.settings() }).await.unwrap();
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Failed).await;
    core.cancel_plan(&plan.id).await.unwrap();
    wait_plan(&core, PlanStatus::Cancelled).await;
    let source = PrSource::Plan(plan.id.clone());
    assert!(plan.integration_branch.is_some());
    std::fs::write(&released, "").unwrap();
    commit(&repo, "mode.txt", "bad\n");

    // No check is known to fail on this merge: what is left is called a conflict.
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert_eq!(err, format!("{CANCELLED}; resolve the conflict yourself"));

    // You try the merge: the plan's check fails on the result, and that is on record.
    core.merge_plan(&plan.id).await.unwrap_err();
    assert_eq!(core.merge_state(&source).await.unwrap().failed_checks, [check]);
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert_eq!(err, format!("{CANCELLED}; fix the checks yourself"));

    // What that merge check said cannot be read any more (the row, damaged by hand): the same
    // plain words, in the conflict form, and not the error of the read.
    let db = rusqlite::Connection::open(dir.path().join("data/codly.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(db.execute("UPDATE kv SET value = 'not a record' WHERE key = ?1", [format!("merge_check:{}", plan.id)]).unwrap(), 1);
    drop(db);
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert_eq!(err, format!("{CANCELLED}; resolve the conflict yourself"));
    core.shutdown().await;
}

/// A new file that was in the plan's worktree before a fix of the checks started (a check's
/// output, a note) is on no branch and is not the task's work. Codly's commit of the task
/// leaves it out, with what is under a folder that was there, and the task's record says so.
/// What the worker makes is committed, a new file too. The files from before stay on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_that_was_in_the_plans_worktree_before_a_fix_stays_out_of_its_commit() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("Task fix-checks") {
                // A tracked file, a new file, and one more file in a folder that was there.
                turn.write("mode.txt", FIXED);
                turn.write("new.txt", "made by the task\n");
                turn.write("cache/more.txt", "later\n");
                let exceptions = json!(["mode.txt: the check reads it", "new.txt: the fix needs it"]);
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Did it.", "scope_exceptions": exceptions })).await;
                return;
            }
            turn.write("feature.txt", "on\n");
            turn.done().await;
        })
    });
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), worker, turns.clone()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    // In the plan's worktree before the fix: what a check or worktree setup left there.
    std::fs::create_dir_all(integ_wt.join("cache")).unwrap();
    std::fs::write(integ_wt.join("cache/built.txt"), "from a check\n").unwrap();
    std::fs::write(integ_wt.join("notes.tmp"), "from before\n").unwrap();
    commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    task_with(&core, "fix-checks", TaskStatus::Done).await;
    // The task's commit has its work, a new file too, and nothing of what was there before.
    assert_eq!(sh(&repo, &["show", "--name-only", "--format=", &branch]), "mode.txt\nnew.txt");
    assert_eq!(sh(&repo, &["ls-tree", "-r", "--name-only", &branch, "--", "notes.tmp", "cache"]), "", "on no branch, as before");
    assert_eq!(sh(&integ_wt, &["status", "--porcelain"]), "?? cache/\n?? notes.tmp");
    assert_eq!(std::fs::read_to_string(integ_wt.join("notes.tmp")).unwrap(), "from before\n");
    assert_eq!(std::fs::read_to_string(integ_wt.join("cache/more.txt")).unwrap(), "later\n");
    // Its record names them, once, and its brief said so beforehand.
    let record = decisions(&core).await;
    let left = "fix-checks: Codly did not commit cache/, notes.tmp (was there before the task). What was in the plan's worktree before \
                the task started and is on no branch is not the task's work. It is still in the worktree.";
    assert_eq!(record.iter().filter(|d| *d == left).count(), 1, "{record:?}");
    assert!(record.iter().any(|d| d == "fix-checks: verify passed (1 command), 2 files changed within scope plus 2 declared exception(s)"), "{record:?}");
    {
        let turns = turns.lock().unwrap();
        let (_, _, brief) = turns.iter().find(|(role, _, prompt)| *role == Role::Worker && prompt.contains("Task fix-checks")).expect("the fixing task's brief");
        assert!(brief.contains("Files that were in this worktree before you started and are on no branch are not committed."), "{brief}");
    }

    // The merge brings the task's files to your branch, and not the others.
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "made by the task\n");
    assert!(!repo.join("notes.tmp").exists() && !repo.join("cache").exists());
    core.shutdown().await;
}

/// What the workers were told in their scope rounds.
fn scope_rounds(turns: &Turns) -> Vec<String> {
    let turns = turns.lock().unwrap();
    turns.iter().filter(|(role, _, prompt)| *role == Role::Worker && prompt.contains("Scope check failed")).map(|(_, _, prompt)| prompt.clone()).collect()
}

/// The task that fixes the checks has a write scope, the files the plan changed, and the gate
/// holds it to that scope: a file outside it costs a scope round, until the worker declares
/// it. What the merge brought in from your branch is yours: it is not counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fix_of_the_checks_outside_the_plans_files_needs_a_declared_exception() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("Scope check failed") {
                // Asked about mode.txt: the fix needs it, and now the report says so.
                let report = json!({ "status": "done", "summary": "Did it.", "scope_exceptions": ["mode.txt: the check reads it"] });
                turn.tool("codly_submit_report", report).await;
                return;
            }
            if turn.prompt.contains("Task fix-checks") {
                turn.write("mode.txt", FIXED);
            } else {
                turn.write("feature.txt", "on\n");
            }
            turn.done().await;
        })
    });
    let check = "grep -q good mode.txt";
    let (core, repo) = start(dir.path(), lead_with("feature.txt", check, &[]), worker, turns.clone()).await;
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    // On main: the change that fails the check, and a file of your own.
    std::fs::write(repo.join("README.md"), "# demo\nmine\n").unwrap();
    commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    let task = task_with(&core, "fix-checks", TaskStatus::Done).await;
    assert_eq!(task.write_scope, ["feature.txt"]);
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| d == "fix-checks: fix round 1/2: 1 file(s) outside the write scope"), "the undeclared file cost one round: {record:?}");
    assert!(record.iter().any(|d| d == "fix-checks: verify passed (1 command), 1 file changed within scope plus 1 declared exception(s)"), "{record:?}");
    let told = scope_rounds(&turns);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("these files are outside your write scope (feature.txt):\n  - mode.txt\n"), "{}", told[0]);
    assert!(!told[0].contains("README.md"), "your own file came with the merge: it is not the task's change: {}", told[0]);
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("mode.txt")).unwrap(), FIXED);
    core.shutdown().await;
}

/// The task that resolves a conflict is held to its write scope too, the files with the
/// conflict: other files that it changes and does not declare cost a scope round. It takes
/// them back (a new file goes, a file of yours is as the merge brought it in again), and the
/// next pass finds nothing outside the scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resolution_that_touches_another_file_gets_a_scope_round() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("Scope check failed") {
                std::fs::remove_file(turn.spec.cwd.join("notes.txt")).unwrap();
                turn.write("README.md", "# demo\nmine\n");
            } else if turn.read("greeting.txt").contains("<<<<<<<") {
                turn.write("greeting.txt", RESOLVED);
                turn.write("notes.txt", "what I found\n");
                turn.write("README.md", "# demo\nmine\nand a line of the agent\n");
            } else if turn.prompt.contains("Task t1") {
                turn.write("greeting.txt", "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, worker, turns.clone()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    std::fs::write(repo.join("README.md"), "# demo\nmine\n").unwrap();
    commit(&repo, "greeting.txt", "hello from main\n");

    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    task_with(&core, "resolve", TaskStatus::Done).await;
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| d == "resolve: fix round 1/2: 2 file(s) outside the write scope"), "{record:?}");
    assert!(record.iter().any(|d| d == "resolve: verify passed (1 command), 1 file changed within scope"), "taken back, the two files are no change any more: {record:?}");
    let told = scope_rounds(&turns);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("these files are outside your write scope (greeting.txt):\n  - README.md\n  - notes.txt\n"), "{}", told[0]);
    assert_eq!(sh(&repo, &["ls-tree", "--name-only", &branch, "notes.txt"]), "", "the file is not in the plan's branch");
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "# demo\nmine\n");
    core.shutdown().await;
}

/// A file's name can hold what a write scope reads as a pattern: a route folder like `[id]`.
/// The write scope of the task that resolves a conflict in such a file is that name as it is,
/// so the file is in its scope and the task uses no fix round.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resolution_in_a_file_whose_name_reads_as_a_pattern_is_in_its_scope() {
    const PAGE: &str = "routes/[id]/page.txt";
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.read(PAGE).contains("<<<<<<<") {
                turn.write(PAGE, RESOLVED);
            } else if turn.prompt.contains("Task t1") {
                turn.write(PAGE, "hello from the plan\n");
            }
            turn.done().await;
        })
    });
    let repo = temp_repo(dir.path());
    std::fs::create_dir_all(repo.join("routes/[id]")).unwrap();
    commit(&repo, PAGE, "hello\n");
    // The plan's own task has the folder as its scope: there, `[id]` would be a pattern.
    let lead = lead_scoped(PAGE, "routes/", "grep -q plan 'routes/[id]/page.txt'", &[]);
    let core = boot(dir.path(), &repo, lead, worker, turns.clone()).await;
    core.send_chat("lead", "Please change the page").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    commit(&repo, PAGE, "hello from main\n");
    assert_eq!(core.merge_state(&source).await.unwrap().conflicts, [PAGE]);

    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    let task = task_with(&core, "resolve", TaskStatus::Done).await;
    assert_eq!(task.write_scope, [PAGE]);
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| d == "resolve: verify passed (1 command), 1 file changed within scope"), "{record:?}");
    assert!(!record.iter().any(|d| d.contains("outside the write scope")), "the file with the conflict is in the scope: {record:?}");
    assert_eq!(scope_rounds(&turns), Vec::<String>::new());
    core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join(PAGE)).unwrap(), RESOLVED);
    core.shutdown().await;
}

/// With reviews on, a fix of the checks that changed no file by hand (the check passes on the
/// merge as it is by now) goes on with no reviewer run: there is nothing to read, and the
/// task's record says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fix_of_the_checks_that_changed_nothing_is_not_reviewed() {
    let dir = tempfile::tempdir().unwrap();
    let turns: Turns = Default::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if !turn.prompt.contains("Task fix-checks") {
                turn.write("feature.txt", "on\n");
            }
            turn.done().await;
        })
    });
    // The check passes with a good mode.txt, or once this file is there.
    let fixed_elsewhere = dir.path().join("fixed-elsewhere");
    let check = format!("grep -q good mode.txt || test -f '{}'", fixed_elsewhere.display());
    let (core, repo) = start(dir.path(), lead_with("feature.txt", &check, &[]), with_reviewer(worker), turns.clone()).await;
    // Codly runs the check by itself, whatever the approval rules see in its path.
    core.update_settings(Settings { review_gate: true, approval_mode: ApprovalMode::Full, ..core.settings() }).await.unwrap();
    core.send_chat("lead", "Please add the feature").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    commit(&repo, "mode.txt", "bad\n");
    core.merge_plan(&plan.id).await.unwrap_err();

    // What made the check fail is put right outside the repository, and the agent finds
    // nothing to change.
    std::fs::write(&fixed_elsewhere, "").unwrap();
    core.resolve_merge_conflict(&source).await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    task_with(&core, "fix-checks", TaskStatus::Done).await;
    let record = decisions(&core).await;
    assert!(record.iter().any(|d| d == "fix-checks: no review, nothing was changed by hand in the merge with your branch"), "{record:?}");
    assert!(record.iter().any(|d| d == "fix-checks: verify passed (1 command), 0 files changed within scope"), "{record:?}");
    assert!(!record.iter().any(|d| d.starts_with("fix-checks: reviewer ") || d.starts_with("fix-checks: review verdict")), "{record:?}");
    let reviews = turns.lock().unwrap().iter().filter(|(role, _, _)| *role == Role::Reviewer).count();
    assert_eq!(reviews, 1, "t1 was reviewed, the fix of the checks was not");
    core.merge_plan(&plan.id).await.unwrap();
    assert!(repo.join("feature.txt").exists());
    core.shutdown().await;
}

/// A merge of a solo agent's branch keeps your branch as it was under the same ref as a merge
/// of a plan: written before the merge, taken away when the merge is not made or is undone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_of_a_solo_agents_branch_keeps_your_branch_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let (core, repo) = start(dir.path(), idle.clone(), idle, Default::default()).await;
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = PathBuf::from(solo.worktree.clone().unwrap());
    std::fs::write(wt.join("greeting.txt"), "hello from the agent\n").unwrap();
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    // A merge that conflicts is not made: no ref of it is left.
    let err = core.merge_agent(&solo.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (mine.clone(), None));

    // You take your change back: the merge is made, and main as it was is kept under the ref.
    let yours = commit(&repo, "greeting.txt", "hello\n");
    let sha = core.merge_agent(&solo.id).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from the agent\n");
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(yours.as_str()));
    let last = core.repo_sync().await.unwrap().last_merge.unwrap();
    assert_eq!((last.sha.as_str(), last.agent_id.as_deref(), last.blocked, last.kept.as_deref()), (sha.as_str(), Some(solo.id.as_str()), None, Some(BEFORE_MERGE)));

    // Undo: main is back there, and nothing is left to keep.
    core.undo_merge(&sha).await.unwrap();
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (yours.clone(), None));

    // Merged again, the ref is of this merge; a second click merges nothing and leaves it.
    let again = core.merge_agent(&solo.id).await.unwrap();
    assert_eq!(kept_before_merge(&repo).as_deref(), Some(yours.as_str()));
    let err = core.merge_agent(&solo.id).await.unwrap_err().to_string();
    assert!(err.starts_with("main already has everything on "), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), kept_before_merge(&repo)), (again, Some(yours)));
    core.shutdown().await;
}

/// The last write of a take-back: the records of the start go.
const THE_START_FORGOTTEN: &str = "DELETE ON kv WHEN OLD.key LIKE 'merge_start:%'";

/// A start fails after the merge, and Codly takes the merge back and writes that down. Only
/// the last write fails: the records of the start stay. The error does not say that it could
/// not be written down (the line is on record), and the next Merge, which removes the records,
/// finds that line and does not write a second one for the same start.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_take_back_whose_records_stay_says_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let lead = lead_with("greeting.txt", "grep -q plan greeting.txt", &[]);
    let (core, repo) = start(dir.path(), lead, plan_worker(), Default::default()).await;
    core.send_chat("lead", "Please change the greeting").await.unwrap();
    let plan = wait_plan(&core, PlanStatus::Done).await;
    let source = PrSource::Plan(plan.id.clone());
    let branch = plan.integration_branch.clone().unwrap();
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    let tip = sh(&repo, &["rev-parse", &branch]);
    let mine = commit(&repo, "greeting.txt", "hello from main\n");

    store_refuses(dir.path(), Some(THE_TASKS_BASE));
    store_trigger(dir.path(), "kept", Some(THE_START_FORGOTTEN));
    let err = core.resolve_merge_conflict(&source).await.unwrap_err().to_string();
    assert!(err.contains("the disk is full") && err.ends_with(&format!(". Codly took the merge of main back: {branch} is as it was")), "{err}");
    assert!(!err.contains("could not write that down"), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", &branch]), in_merge(&integ_wt), sh(&integ_wt, &["status", "--porcelain"])), (tip.clone(), false, String::new()));
    let task = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "resolve").expect("the task had reached the store");
    assert_eq!((task.status, task.status_reason.as_deref()), (TaskStatus::Cancelled, Some("Codly could not start it")));
    assert_eq!(start_records(dir.path()).len(), 2, "the start is still on record: {:?}", start_records(dir.path()));
    let did_not_start = |record: &[String]| record.iter().filter(|d| d.starts_with(&format!("bringing main into {branch} did not start ("))).count();
    assert_eq!(did_not_start(&decisions(&core).await), 1);

    // The store takes writes again. The next Merge removes the records, finds the plan as it
    // was (the conflict), and says nothing more about the start.
    store_refuses(dir.path(), None);
    store_trigger(dir.path(), "kept", None);
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert_eq!(start_records(dir.path()), Vec::<String>::new());
    let record = decisions(&core).await;
    assert_eq!(did_not_start(&record), 1, "{record:?}");
    assert!(!record.iter().any(|d| d.contains("did not finish")), "no second line for the same start: {record:?}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["rev-parse", &branch])), (mine, tip));
    core.shutdown().await;
}
