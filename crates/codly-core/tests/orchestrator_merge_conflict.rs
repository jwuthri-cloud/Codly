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
//!   merge whose checks could not run, or whose branches moved while they ran, is not made.

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
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

/// A Lead that submits a one-task plan for every request: t1 writes `content` to `file`.
fn lead_with(file: &str, verify: &str, integration: &[&str]) -> Behavior {
    let plan = json!({
        "summary": "Change the greeting",
        "tasks": [{
            "id": "t1", "title": format!("Write {file}"), "goal": format!("Write {file}"), "consultant": { "title": "Backend" },
            "acceptance_criteria": [format!("{file} is right")], "write_scope": [file], "depends_on": [], "verify": [verify], "size": "M",
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

    // You kept working on main: the same line, and a file the plan never touched.
    std::fs::write(repo.join("README.md"), "# demo\nmine\n").unwrap();
    let mine = commit(&repo, "greeting.txt", "hello from main\n");
    assert_eq!(core.merge_state(&source).await.unwrap().conflicts, ["greeting.txt"]);

    // Merge: refused, with the files and both ways out; your checkout is as it was.
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("merging into main conflicts in greeting.txt"), "{err}");
    assert!(err.contains("An agent (on the plan's card) can resolve it") && err.ends_with(&format!("git merge {branch}")), "{err}");
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (mine.clone(), String::new()));
    no_scratch_left(dir.path());

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
    assert_eq!(runs.iter().filter(|(t, c, ok)| t.is_none() && c == "test -f greeting.txt" && *ok).count(), 2, "the integration check ran when the plan finished, both times");
    // All along, your checkout was not touched.
    assert_eq!((sh(&repo, &["rev-parse", "HEAD"]), sh(&repo, &["status", "--porcelain"])), (mine.clone(), String::new()));
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from main\n");
    let lines = said(&core, "lead").await;
    assert!(lines.iter().any(|l| l.starts_with("main and this plan both changed greeting.txt. Codly brought main into the plan's branch")), "{lines:?}");

    // The plan builds on your branch as it is now: its diff is the plan's work, not yours.
    assert_eq!((again.base_commit.as_deref(), again.base_branch.as_deref()), (Some(mine.as_str()), Some("main")));
    let diff = core.plan_diff(&plan.id).await.unwrap();
    assert!(diff.contains("+hello from main and the plan") && !diff.contains("mine"), "{diff}");

    // Nothing conflicts any more: the merge goes through, and can still be undone.
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.conflicts.len(), state.resolving), (0, None));
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), RESOLVED);
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "# demo\nmine\n");
    assert!(!integ_wt.exists(), "the integration worktree is cleaned up once merged");
    let last = core.repo_sync().await.unwrap().last_merge.unwrap();
    assert_eq!((last.sha.as_str(), last.plan_id.as_deref(), last.blocked), (sha.as_str(), Some(plan.id.as_str()), None));
    core.undo_merge(&sha).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);
    assert_eq!(std::fs::read_to_string(repo.join("greeting.txt")).unwrap(), "hello from main\n");
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
    assert!(core.merge_state(&source).await.unwrap().conflicts.is_empty());
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
    core.undo_merge(&sha).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), bad);

    // You fix your side: the check passes on the merge result, the merge is made, and Codly says what ran.
    commit(&repo, "mode.txt", "good again\n");
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(std::fs::read_to_string(repo.join("feature.txt")).unwrap(), "on\n");
    no_scratch_left(dir.path());
    assert_eq!(verified(&core).await.last().unwrap(), &(None, check.to_string(), true));
    let lines = said(&core, "lead").await;
    let told = format!(" main had changed since the plan was checked. Codly ran the plan's checks on the merge result first: `{check}` passed.");
    assert!(lines.last().unwrap().starts_with("Merged \"Change the greeting\" into main (") && lines.last().unwrap().ends_with(&told), "{lines:?}");

    // Merged already: nothing is checked or merged twice.
    let runs = verified(&core).await.len();
    assert_eq!(core.merge_plan(&plan.id).await.unwrap(), sha);
    assert_eq!(verified(&core).await.len(), runs);
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

    // Merge: refused, with the file and both ways out.
    let err = core.merge_agent(&solo.id).await.unwrap_err().to_string();
    let branch = sh(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(err.contains("merging into main conflicts in greeting.txt") && err.ends_with(&format!("git merge {branch}")), "{err}");
    assert!(err.contains(&format!("{} (from its Git tab) can resolve it", solo.name)), "{err}");
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), mine);
    let state = core.merge_state(&source).await.unwrap();
    assert_eq!((state.target.as_str(), state.branch.as_str(), &state.conflicts), ("main", branch.as_str(), &vec!["greeting.txt".to_string()]));

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
