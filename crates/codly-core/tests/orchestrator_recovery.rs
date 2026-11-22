//! The scheduler recovers by itself, with fake agents in place of the CLIs: a worker that does
//! not start or that exits is started again after a wait, and only a cause you must fix or a
//! third failure in a row blocks its task (a turn the worker completes ends the row); a session
//! out of context starts again as a new one;
//! a plan that cannot go on says so once and can finish with what is merged; a task and the
//! tasks that wait for it are cancelled together; after a restart of Codly a task that was in
//! its checks is checked again with no worker turn, and a worker that is resumed is told what
//! Codly sees in its worktree; a lock file another git process holds for a moment does not block
//! a task; the work under way is counted for the shell; and a plan you merged with git is noticed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::activity::Activity;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::{RunRow, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
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

/// How a fake worker's turn ends.
enum End {
    /// It writes `<name of its worktree folder>.txt` (what the tasks here ask for) and completes.
    Done,
    /// It completes with nothing written: the task's check fails, and Codly asks for a fix.
    Idle,
    /// The turn fails with this error as its result, as the Claude CLI reports one.
    Fail(&'static str),
    /// The CLI dies mid-turn: exit code 1.
    Crash,
    /// As `Done`, and when the turn ends another git process holds the index lock of the
    /// worktree, for a moment (an editor's git panel, a watcher).
    Locked,
    /// As `Done`, once the test lets the turn end.
    Hold(Arc<tokio::sync::Notify>),
}

/// Decides how the worker's `n`-th turn (over every run of the test) ends.
type Script = Arc<dyn Fn(usize) -> End + Send + Sync>;
/// The error of the `n`-th start of a worker (over every run), or None: it starts.
type Starts = Arc<dyn Fn(&SpawnSpec, usize) -> Option<&'static str> + Send + Sync>;

#[derive(Default)]
struct Seen {
    /// Every start of a worker that was tried.
    tries: usize,
    /// (agent id, session) of every worker run that started.
    runs: Vec<(String, SessionMode)>,
    /// Every prompt a worker got.
    prompts: Vec<String>,
}

struct Fake {
    seen: Arc<Mutex<Seen>>,
    starts: Starts,
    script: Script,
    /// What the Lead submits when you ask it for something.
    plan: Value,
}

/// Call a codly_* tool the way the stdio bridge does.
async fn tool(spec: &SpawnSpec, name: &str, args: Value) -> Value {
    let mcp = spec.mcp.as_ref().expect("Codly gives every agent its MCP server");
    let env = |k: &str| mcp.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
    let mut s = UnixStream::connect(env("CODLY_MCP_SOCK")).await.unwrap();
    let req = json!({ "token": env("CODLY_MCP_TOKEN"), "tool": name, "args": args });
    s.write_all(format!("{req}\n").as_bytes()).await.unwrap();
    let mut lines = BufReader::new(s).lines();
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (seen, starts, script, plan) = (self.seen.clone(), self.starts.clone(), self.script.clone(), self.plan.clone());
        Box::pin(async move {
            let worker = spec.role != Role::Lead;
            if worker {
                let n = {
                    let mut s = seen.lock().unwrap();
                    s.tries += 1;
                    s.tries - 1
                };
                if let Some(error) = starts(&spec, n) {
                    return Err(anyhow::anyhow!(error));
                }
                seen.lock().unwrap().runs.push((spec.agent_id.clone(), spec.session.clone()));
            }
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            let session = match &spec.session {
                SessionMode::New { session_id } => session_id.clone().unwrap_or_else(|| format!("fake-{}", spec.run_id)),
                SessionMode::Resume { session_id } => session_id.clone(),
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
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let text = match cmd {
                        AgentCmd::Prompt(text) => text,
                        AgentCmd::Shutdown => return send(EventKind::Exited { code: Some(0) }),
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => continue,
                    };
                    send(EventKind::TurnStarted { input: Some(text.clone()) });
                    let done = |status, result: Option<&str>| {
                        send(EventKind::TurnCompleted { status, result_text: result.map(String::from), duration_ms: Some(1) });
                    };
                    if !worker {
                        if !text.contains("[Codly]") {
                            let reply = tool(&spec, "codly_submit_plan", plan.clone()).await;
                            assert_eq!(reply["ok"], true, "{reply}");
                        }
                        done(TurnStatus::Completed, None);
                        continue;
                    }
                    let n = {
                        let mut s = seen.lock().unwrap();
                        s.prompts.push(text);
                        s.prompts.len() - 1
                    };
                    let work = || {
                        let name = spec.cwd.file_name().unwrap().to_string_lossy().into_owned();
                        std::fs::write(spec.cwd.join(format!("{name}.txt")), "done\n").unwrap();
                    };
                    match script(n) {
                        End::Done => {
                            work();
                            done(TurnStatus::Completed, None);
                        }
                        End::Locked => {
                            work();
                            let lock = Path::new(&git(&spec.cwd, &["rev-parse", "--absolute-git-dir"])).join("index.lock");
                            std::fs::write(&lock, "").unwrap();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(700)).await;
                                std::fs::remove_file(lock).unwrap();
                            });
                            done(TurnStatus::Completed, None);
                        }
                        End::Hold(go) => {
                            go.notified().await;
                            work();
                            done(TurnStatus::Completed, None);
                        }
                        End::Idle => done(TurnStatus::Completed, None),
                        End::Fail(error) => done(TurnStatus::Failed, Some(error)),
                        End::Crash => return send(EventKind::Exited { code: Some(1) }),
                    }
                }
            });
            Ok(handle)
        })
    }
}

/// Every worker starts, and every turn does the work.
fn works() -> (Starts, Script) {
    (Arc::new(|_: &SpawnSpec, _: usize| None::<&'static str>), Arc::new(|_: usize| End::Done))
}

/// A plan task that writes `<its key>.txt`, checked with `test -f`.
fn plan_task(id: &str, deps: &[&str]) -> Value {
    json!({ "id": id, "title": format!("Write {id}.txt"), "goal": format!("Create {id}.txt"), "consultant": { "title": "Backend" },
            "acceptance_criteria": [format!("{id}.txt exists")], "write_scope": [format!("{id}.txt")], "depends_on": deps,
            "verify": [format!("test -f {id}.txt")], "size": "M" })
}

/// A plan of three tasks, each writing `<its key>.txt`: t1 alone, t2 alone, t3 after t2.
fn three_task_plan() -> Value {
    json!({ "summary": "Write three files", "tasks": [plan_task("t1", &[]), plan_task("t2", &[]), plan_task("t3", &["t2"])] })
}

const SHORT: Duration = Duration::from_millis(100);

/// A started core on a new repo, with short retry waits.
async fn start(dir: &Path, starts: Starts, script: Script, plan: Value) -> (Arc<Core>, Arc<Mutex<Seen>>, PathBuf) {
    let repo = temp_repo(dir);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir, Fake { seen: seen.clone(), starts, script, plan }).await;
    core.update_settings(Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    (core, seen, repo)
}

async fn start_core(dir: &Path, fake: Fake) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(fake)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.set_worker_retry_waits(vec![SHORT; 2]);
    core
}

/// An ad-hoc task that is ready to run (no verify commands: the gate takes its work as it is).
async fn ready_task(core: &Core) -> Task {
    let task = core.create_task("Write the greeting", "Create a text file").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap()
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn task_now(core: &Core, id: &str) -> Task {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.id == id).expect("the task is on the board")
}

async fn task_ended(core: &Core, id: &str) -> Task {
    wait_for("the task to end", async || {
        Some(task_now(core, id).await).filter(|t| matches!(t.status, TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Failed))
    })
    .await
}

async fn task_by_key(core: &Core, key: &str) -> Task {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == key).expect("the task is on the board")
}

/// The summaries of the decisions of one type, oldest first.
async fn decisions(core: &Core, kind: &str) -> Vec<String> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type, summary, .. } if decision_type == kind => Some(summary),
            _ => None,
        })
        .collect()
}

/// Every status the task went through, oldest first.
async fn statuses(core: &Core, id: &str) -> Vec<TaskStatus> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    let mut out: Vec<TaskStatus> = vec![];
    for e in events {
        if let EventKind::TaskChanged { task } = e.kind
            && task.id == id
            && out.last() != Some(&task.status)
        {
            out.push(task.status);
        }
    }
    out
}

/// What Codly said in the Lead's thread.
async fn said(core: &Core) -> Vec<String> {
    core.snapshot().await.unwrap().chats.into_iter().filter(|c| c.thread == "lead" && c.author == "codly").map(|c| c.text).collect()
}

/// Ask the Lead for the three-task plan and run it.
async fn run_three_task_plan(core: &Core) -> Plan {
    core.send_chat("lead", "Write three files").await.unwrap();
    let plan = wait_for("the plan", async || core.snapshot().await.unwrap().plans.first().cloned()).await;
    core.run_plan(&plan.id).await.unwrap();
    plan
}

const HANDSHAKE: &str = "the agent did not finish the handshake";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_that_fails_is_tried_again_after_a_wait_and_the_task_finishes() {
    let dir = tempfile::tempdir().unwrap();
    // The first start fails, and so does the second (a resume, then a new session).
    let starts: Starts = Arc::new(|_: &SpawnSpec, n: usize| (n < 3).then_some(HANDSHAKE));
    let (core, seen, _) = start(dir.path(), starts, works().1, json!({})).await;
    let task = ready_task(&core).await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    let retries = decisions(&core, "retry").await;
    assert_eq!(
        retries,
        [
            format!("could not start the worker: {HANDSHAKE}; Codly starts it again in a moment (retry 1 of 2)"),
            format!("could not start the worker: {HANDSHAKE}; Codly starts it again in a moment (retry 2 of 2)"),
        ]
    );
    assert!(!statuses(&core, &task.id).await.contains(&TaskStatus::Blocked), "a start that fails for a moment never blocks the task");
    assert_eq!(seen.lock().unwrap().runs.len(), 1, "one worker run did the work");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_third_failure_in_a_row_blocks_the_task_and_your_retry_starts_it_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let broken = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let starts: Starts = {
        let broken = broken.clone();
        Arc::new(move |_: &SpawnSpec, _: usize| broken.load(std::sync::atomic::Ordering::SeqCst).then_some(HANDSHAKE))
    };
    let (core, _, _) = start(dir.path(), starts, works().1, json!({})).await;
    let task = ready_task(&core).await;
    let blocked = task_ended(&core, &task.id).await;
    assert_eq!(blocked.status, TaskStatus::Blocked);
    assert_eq!(
        blocked.status_reason.as_deref(),
        Some(format!("blocked: the worker failed 3 times in a row (last: could not start the worker: {HANDSHAKE}); fix the cause, then Retry").as_str())
    );
    assert_eq!(decisions(&core, "retry").await.len(), 3, "two retries, then the block");

    // The cause is fixed and you retry: no wait is left over from the failures.
    broken.store(false, std::sync::atomic::Ordering::SeqCst);
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_that_only_you_can_fix_blocks_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let starts: Starts = Arc::new(|_: &SpawnSpec, _: usize| Some("Claude Code is not signed in"));
    let (core, seen, _) = start(dir.path(), starts, works().1, json!({})).await;
    let task = ready_task(&core).await;
    let blocked = task_ended(&core, &task.id).await;
    assert_eq!(blocked.status, TaskStatus::Blocked);
    assert_eq!(blocked.status_reason.as_deref(), Some("blocked: could not start the worker: Claude Code is not signed in"));
    assert_eq!(seen.lock().unwrap().tries, 1, "waiting does not sign you in: nothing was tried again");
    core.shutdown().await;
}

/// Three crashes in a row block the task. After your Retry the count starts again: the next
/// crash restarts the worker, whatever the number of runs the task had before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crashes_are_counted_in_a_row_not_over_the_life_of_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|n: usize| if n < 4 { End::Crash } else { End::Done });
    let (core, seen, _) = start(dir.path(), works().0, script, json!({})).await;
    let task = ready_task(&core).await;
    let blocked = task_ended(&core, &task.id).await;
    assert_eq!(blocked.status, TaskStatus::Blocked);
    assert_eq!(
        blocked.status_reason.as_deref(),
        Some("blocked: the worker failed 3 times in a row (last: the worker exited unexpectedly (exit 1)); fix the cause, then Retry")
    );
    assert_eq!(blocked.attempt, 3);

    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "the fourth crash restarted the worker: {:?}", task.status_reason);
    let retries = decisions(&core, "retry").await;
    let first = "the worker exited unexpectedly (exit 1); Codly starts it again in a moment (retry 1 of 2)";
    assert_eq!(retries.iter().filter(|r| r.as_str() == first).count(), 2, "{retries:?}");
    // The restarted worker resumes its session and is told why it is back.
    {
        let seen = seen.lock().unwrap();
        assert!(matches!(&seen.runs[1].1, SessionMode::Resume { .. }), "{:?}", seen.runs);
        assert!(seen.prompts[1].contains("the worker exited unexpectedly (exit 1)"), "{}", seen.prompts[1]);
    }
    core.shutdown().await;
}

/// Two crashes, a turn that completes with no report (Codly asks for it, and the worker has
/// nothing to add), a check that fails (a fix round), then a crash in the fix round. That
/// crash is the first of a new row, not the third of the old one: the worker ran in between,
/// so the task is not blocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_the_worker_completes_ends_the_row_of_crashes() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|n: usize| match n {
        0 | 1 | 4 => End::Crash,
        2 | 3 => End::Idle,
        _ => End::Done,
    });
    let plan = json!({ "summary": "Write one file", "tasks": [plan_task("t1", &[])] });
    let (core, seen, _) = start(dir.path(), works().0, script, plan).await;
    core.send_chat("lead", "Write one file").await.unwrap();
    let plan = wait_for("the plan", async || core.snapshot().await.unwrap().plans.first().cloned()).await;
    core.run_plan(&plan.id).await.unwrap();
    let task = task_by_key(&core, "t1").await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    let crash = |n: u32| format!("the worker exited unexpectedly (exit 1); Codly starts it again in a moment (retry {n} of 2)");
    assert_eq!(decisions(&core, "retry").await, [crash(1), crash(2), crash(1)]);
    assert!(!statuses(&core, &task.id).await.contains(&TaskStatus::Blocked));
    // The crash in the fix round came after the check that failed.
    let fix = seen.lock().unwrap().prompts[4].clone();
    assert!(fix.contains("test -f t1.txt"), "{fix}");
    // The worker that comes back after that crash is told what Codly last found, and where it is.
    let back = seen.lock().unwrap().prompts[5].clone();
    assert!(back.contains("What Codly sees in your worktree now:\n- Branch: codly/"), "{back}");
    assert!(back.contains("\n- Codly's last check: t1: fix round 1/2: `test -f t1.txt` failed (exit 1)\n"), "{back}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_out_of_context_starts_again_as_a_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|n: usize| if n == 0 { End::Fail("Prompt is too long") } else { End::Done });
    let (core, seen, _) = start(dir.path(), works().0, script, json!({})).await;
    let task = ready_task(&core).await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(decisions(&core, "retry").await, ["the worker's session ran out of context; Codly starts a new session in a moment (retry 1 of 2)"]);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.runs.len(), 2);
        let (SessionMode::New { session_id: first }, SessionMode::New { session_id: second }) = (&seen.runs[0].1, &seen.runs[1].1) else {
            panic!("the full session is not resumed: {:?}", seen.runs)
        };
        assert_ne!(first, second);
        // The new session gets the whole brief, and is told that the work so far is in the worktree.
        assert!(seen.prompts[1].contains("Write the greeting") && seen.prompts[1].contains("ran out of context"), "{}", seen.prompts[1]);
    }
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stuck_plan_says_so_once_and_finishes_with_what_is_merged() {
    let dir = tempfile::tempdir().unwrap();
    // The worker of t2 cannot start, for a cause only you can fix.
    let starts: Starts = Arc::new(|spec: &SpawnSpec, _: usize| spec.cwd.ends_with("t2").then_some("Claude Code is not signed in"));
    let (core, _, _) = start(dir.path(), starts, works().1, three_task_plan()).await;
    let plan = run_three_task_plan(&core).await;
    let stuck = |texts: Vec<String>| texts.into_iter().filter(|t| t.contains("cannot go on by itself")).collect::<Vec<_>>();
    let notice = wait_for("the notice", async || stuck(said(&core).await).pop()).await;
    let branch = core.snapshot().await.unwrap().plans[0].integration_branch.clone().unwrap();
    assert_eq!(
        notice,
        format!(
            "Plan \"Write three files\" cannot go on by itself. t2 \"Write t2.txt\" is blocked: could not start the worker: Claude Code is \
             not signed in. t3 waits for it. Merged into {branch}: 1 of 3 tasks. What you can do: start a stopped task again from its \
             card (Retry, or Reopen if you cancelled it); cancel it together with the tasks that wait for it; or choose Keep what is \
             merged on the plan card: the plan then finishes with the merged tasks, and the rest is cancelled."
        )
    );
    // More scheduling passes say nothing more.
    for _ in 0..3 {
        core.update_settings(core.settings()).await.unwrap(); // starts a pass
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert_eq!(stuck(said(&core).await).len(), 1);
    assert_eq!(core.snapshot().await.unwrap().plans[0].status, PlanStatus::Running);

    // Keep what is merged: the two tasks that cannot run are cancelled and the plan is done.
    core.finish_plan(&plan.id).await.unwrap();
    wait_for("the plan to finish", async || (core.snapshot().await.unwrap().plans[0].status == PlanStatus::Done).then_some(())).await;
    for key in ["t2", "t3"] {
        let t = task_by_key(&core, key).await;
        assert_eq!((t.status, t.status_reason.as_deref()), (TaskStatus::Cancelled, Some("cancelled: you kept what is merged")), "{key}");
    }
    assert_eq!(task_by_key(&core, "t1").await.status, TaskStatus::Done);
    assert!(said(&core).await.iter().any(|t| t.starts_with("Plan done:") && t.contains("1 task merged")), "{:?}", said(&core).await);
    // Only a running plan can finish early.
    let e = core.finish_plan(&plan.id).await.unwrap_err().to_string();
    assert!(e.contains("only a running plan"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_is_cancelled_together_with_the_tasks_that_wait_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let starts: Starts = Arc::new(|spec: &SpawnSpec, _: usize| spec.cwd.ends_with("t2").then_some("Claude Code is not signed in"));
    let (core, _, _) = start(dir.path(), starts, works().1, three_task_plan()).await;
    run_three_task_plan(&core).await;
    wait_for("t1 to be merged and t2 to be blocked", async || {
        let (t1, t2) = (task_by_key(&core, "t1").await, task_by_key(&core, "t2").await);
        (t1.status == TaskStatus::Done && t2.status == TaskStatus::Blocked).then_some(())
    })
    .await;
    let t2 = task_by_key(&core, "t2").await;
    let cancelled = core.cancel_task_tree(&t2.id).await.unwrap();
    assert_eq!(cancelled.iter().map(|t| (t.key.as_str(), t.status)).collect::<Vec<_>>(), [("t2", TaskStatus::Cancelled), ("t3", TaskStatus::Cancelled)]);
    // Nothing is left to wait for: the plan finishes with the task that was merged.
    wait_for("the plan to finish", async || (core.snapshot().await.unwrap().plans[0].status == PlanStatus::Done).then_some(())).await;
    core.shutdown().await;
}

/// What a quit leaves behind for one ad-hoc task that was in its checks: its worker (session
/// `sess-<key>`), the run that was live, the worktree where Codly would make it, and the
/// worker's turns. Returns (task id, agent id).
fn seed_task_in_checks(store: &Store, ws: &Workspace, data: &Path, key: &str, turn_over: bool) -> (Id, Id) {
    let repo = Path::new(&ws.repo_path);
    let root = format!("{}-{}", codly_core::orchestrator::names::slug(&ws.name, 24), &ws.id[ws.id.len() - 6..]);
    let wt = data.join("worktrees").join(root).join("adhoc").join(key);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(repo, &["worktree", "add", "-q", "-b", &format!("codly/adhoc/{key}"), wt.to_str().unwrap()]);
    let (task_id, agent_id, run_id) = (new_id(), new_id(), new_id());
    let ev = |kind| EventRow {
        id: 0,
        ts: now_ms(),
        agent_id: Some(agent_id.clone()),
        task_id: Some(task_id.clone()),
        run_id: Some(run_id.clone()),
        workspace_id: None,
        kind,
    };
    let agent = Agent {
        id: agent_id.clone(),
        name: format!("Worker {key}"),
        provider: Provider::Claude,
        model: None,
        role: Role::Worker,
        status: AgentStatus::Idle,
        activity: None,
        activity_label: None,
        task_id: Some(task_id.clone()),
        run_id: Some(run_id.clone()),
        session_id: Some(format!("sess-{key}")),
        worktree: Some(wt.to_string_lossy().into_owned()),
        billing: Billing::Subscription,
        account: None,
        color: 0,
        look_seed: agent_id.clone(),
        thread: None,
        member: None,
        title: None,
        consultant: false,
        kept: false,
        archived: false,
        usage: Usage::default(),
        career: Default::default(),
        away_until: None,
        away_since: None,
        tired_since: None,
        created_at: now_ms(),
        updated_at: now_ms(),
    };
    store.append_event(Some(&ws.id), &ev(EventKind::AgentChanged { agent })).unwrap();
    let task: Task = serde_json::from_value(json!({
        "id": task_id, "plan_id": null, "key": key, "title": format!("Write {key}.txt"), "goal": format!("Create {key}.txt"),
        "acceptance": [], "write_scope": [format!("{key}.txt")], "verify": [format!("test -f {key}.txt")], "depends_on": [], "status": "verifying",
        "assignee": agent_id, "agent_hint": null, "branch": format!("codly/adhoc/{key}"), "worktree": wt, "attempt": 1,
        "status_reason": "Codly is checking the work", "summary": null, "created_at": now_ms(), "updated_at": now_ms(),
    }))
    .unwrap();
    store.append_event(Some(&ws.id), &ev(EventKind::TaskChanged { task })).unwrap();
    store.set_task_commits(&task_id, Some(&git(repo, &["rev-parse", "HEAD"])), None).unwrap();
    store
        .insert_run(&RunRow {
            id: run_id.clone(),
            agent_id: agent_id.clone(),
            task_id: Some(task_id.clone()),
            role: Role::Worker,
            provider: Provider::Claude,
            model: None,
            cwd: wt.to_string_lossy().into_owned(),
            session_id: Some(format!("sess-{key}")),
            account: None,
            status: "running".into(),
            raw_log: "/dev/null".into(),
            report: None,
            started_at: now_ms(),
            ended_at: None,
        })
        .unwrap();
    // Its first turn did the work and ended. A message of yours started another turn after it,
    // unless `turn_over`.
    store.append_event(Some(&ws.id), &ev(EventKind::TurnStarted { input: None })).unwrap();
    store.append_event(Some(&ws.id), &ev(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) })).unwrap();
    if turn_over {
        std::fs::write(wt.join(format!("{key}.txt")), "done\n").unwrap();
    } else {
        store.append_event(Some(&ws.id), &ev(EventKind::TurnStarted { input: None })).unwrap();
        // That turn had written a part of the file when Codly quit.
        std::fs::write(wt.join(format!("{key}.txt")), "half\n").unwrap();
    }
    (task_id, agent_id)
}

/// Codly quit while it checked two tasks. One worker's turn was over: its work is checked
/// again and merged with no worker turn. The other worker was in a turn: it is resumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_restart_a_task_in_its_checks_is_checked_again_without_a_worker_turn() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (over, mid) = {
        let store = Store::open(&data.join("codly.sqlite")).unwrap();
        let ws = Workspace {
            id: new_id(),
            repo_path: std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&ws).unwrap();
        (seed_task_in_checks(&store, &ws, &data, "a1", true), seed_task_in_checks(&store, &ws, &data, "b1", false))
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (starts, script) = works();
    let core = start_core(dir.path(), Fake { seen: seen.clone(), starts, script, plan: json!({}) }).await;
    for (task_id, _) in [&over, &mid] {
        let task = task_ended(&core, task_id).await;
        assert_eq!(task.status, TaskStatus::Done, "{}: {:?}", task.key, task.status_reason);
    }
    // The first task never went back to a worker: Running (its gate) and on to Done.
    let went = statuses(&core, &over.0).await;
    assert!(!went.contains(&TaskStatus::Ready), "{went:?}");
    {
        let seen = seen.lock().unwrap();
        assert!(!seen.runs.iter().any(|r| r.0 == over.1), "no run for the worker whose turn was over: {:?}", seen.runs);
        // The second one's turn had not ended: its worker resumed its session and finished.
        assert_eq!(seen.runs.len(), 1);
        assert!(seen.runs[0].0 == mid.1 && matches!(&seen.runs[0].1, SessionMode::Resume { session_id } if session_id == "sess-b1"), "{:?}", seen.runs);
        assert!(seen.prompts[0].contains("resuming after Codly restarted"), "{}", seen.prompts[0]);
        // It is told what Codly sees in its worktree, so it neither starts over nor counts on
        // work that is not there.
        let state = "What Codly sees in your worktree now:\n\
                     - Branch: codly/adhoc/b1\n\
                     - Uncommitted changes in 1 file: b1.txt\n\
                     - Codly has not committed any of your work yet.\n\
                     - Codly has not checked your work yet.\n";
        assert!(seen.prompts[0].contains(state), "{}", seen.prompts[0]);
    }
    core.shutdown().await;
}

/// Another git process holds the index lock of the worktree for a moment when the worker's
/// turn ends. The gate's commit fails on it, changes nothing, and is made once more: the task
/// is done, and never blocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lock_file_held_for_a_moment_does_not_block_the_task() {
    let dir = tempfile::tempdir().unwrap();
    // The first turn ends with no report and Codly asks for it; the gate commits after the second.
    let script: Script = Arc::new(|n: usize| if n == 1 { End::Locked } else { End::Done });
    let (core, _, _) = start(dir.path(), works().0, script, json!({})).await;
    let task = ready_task(&core).await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert!(!statuses(&core, &task.id).await.contains(&TaskStatus::Blocked));
    let gate = decisions(&core, "gate").await;
    let again: Vec<&String> = gate.iter().filter(|d| d.contains("Codly tries it once more")).collect();
    assert_eq!(again.len(), 1, "{gate:?}");
    assert!(again[0].contains("index.lock': File exists"), "{}", again[0]);
    core.shutdown().await;
}

/// What the shell asks before it lets the Mac sleep or Codly quit: an agent in a turn is work
/// under way, an office where nothing runs is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_work_under_way_is_counted_while_an_agent_is_in_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let go = Arc::new(tokio::sync::Notify::new());
    let script: Script = {
        let go = go.clone();
        Arc::new(move |n: usize| if n == 0 { End::Hold(go.clone()) } else { End::Done })
    };
    let (core, _, _) = start(dir.path(), works().0, script, json!({})).await;
    assert_eq!(core.activity().unwrap(), Activity::default(), "nothing runs in a new office");
    let task = ready_task(&core).await;
    let working = wait_for("the worker's turn", async || Some(core.activity().unwrap()).filter(|a| a.agents == 1)).await;
    assert!(working.busy(), "{working:?}");
    go.notify_one();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    wait_for("the work to end", async || (!core.activity().unwrap().busy()).then_some(())).await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_you_merged_with_git_is_noticed_when_your_branch_moves() {
    let dir = tempfile::tempdir().unwrap();
    let (starts, script) = works();
    let (core, _, repo) = start(dir.path(), starts, script, three_task_plan()).await;
    run_three_task_plan(&core).await;
    let plan = wait_for("the plan to finish", async || core.snapshot().await.unwrap().plans.into_iter().find(|p| p.status == PlanStatus::Done)).await;
    // Passes go by with your branch where it was: the plan stays unmerged.
    for _ in 0..3 {
        core.update_settings(core.settings()).await.unwrap(); // starts a pass
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert_eq!(core.snapshot().await.unwrap().plans[0].merged_sha, None);

    git(&repo, &["merge", "-q", "--no-edit", plan.integration_branch.as_deref().unwrap()]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    core.update_settings(core.settings()).await.unwrap(); // starts a pass
    let merged = wait_for("the merge to be noticed", async || core.snapshot().await.unwrap().plans[0].merged_sha.clone()).await;
    assert_eq!(merged, head);
    assert!(decisions(&core, "merge").await.iter().any(|d| d.contains("merged outside Codly")));
    core.shutdown().await;
}
