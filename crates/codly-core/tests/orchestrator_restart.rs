//! What a restart of Codly asks, says and keeps, with fake agents in place of the CLIs. Work
//! that was under way when Codly closed starts only after your Resume (the default, "ask"),
//! and so does work that was about to start, and a finished plan's integration checks; a
//! question you left open is asked again at the next start, for the same tasks; each resumed
//! task says so in its history; the fix rounds a task used are not handed out again; and a
//! task that waits for your answer keeps its files. The question counts what still waits: a
//! hold whose tasks you cancelled ends by itself, an office whose folder is not there holds
//! nothing, a task whose worktree is gone goes on through its worker, and a review that was
//! under way is done again by a new reviewer.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::blockers::{Blocker, BlockerSeverity, BlockerStatus};
use codly_core::model::*;
use codly_core::orchestrator::resume::{HELD, ResumeOffer};
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::{RunRow, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// Where the store keeps the tasks that wait for your Resume.
const HOLD_ROW: &str = "resume_hold";

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

/// What a fake worker's turn does.
enum Turn {
    /// It writes `<name of its worktree folder>.txt` (what the tasks here ask for) and completes.
    Work,
    /// It completes with nothing written: the task's check fails.
    Nothing,
    /// It stays open until Codly stops the run.
    Stay,
    /// It asks you a blocking question and completes.
    Ask,
}

/// Decides what the worker's `n`-th turn (over every run of the test) does.
type Script = Arc<dyn Fn(usize) -> Turn + Send + Sync>;

#[derive(Default)]
struct Seen {
    /// (agent id, session) of every run that started.
    runs: Vec<(String, SessionMode)>,
    /// Every prompt a worker got.
    prompts: Vec<String>,
    /// (run id, session) of every reviewer run that gave its verdict.
    reviews: Vec<(String, SessionMode)>,
}

struct Fake {
    seen: Arc<Mutex<Seen>>,
    script: Script,
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
        let (seen, script) = (self.seen.clone(), self.script.clone());
        Box::pin(async move {
            seen.lock().unwrap().runs.push((spec.agent_id.clone(), spec.session.clone()));
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
                        _ => continue,
                    };
                    send(EventKind::TurnStarted { input: Some(text.clone()) });
                    // A reviewer approves the work, whatever the script says about workers.
                    if spec.role == Role::Reviewer {
                        let reply = tool(&spec, "codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": [], "issues": [] })).await;
                        assert_eq!(reply["ok"], true, "{reply}");
                        seen.lock().unwrap().reviews.push((spec.run_id.clone(), spec.session.clone()));
                        send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        continue;
                    }
                    let n = {
                        let mut s = seen.lock().unwrap();
                        s.prompts.push(text);
                        s.prompts.len() - 1
                    };
                    match script(n) {
                        Turn::Work => {
                            let name = spec.cwd.file_name().unwrap().to_string_lossy().into_owned();
                            std::fs::write(spec.cwd.join(format!("{name}.txt")), "done\n").unwrap();
                        }
                        Turn::Nothing => {}
                        Turn::Stay => {
                            while let Some(cmd) = rx.recv().await {
                                if matches!(cmd, AgentCmd::Shutdown) {
                                    break;
                                }
                            }
                            return send(EventKind::Exited { code: Some(0) });
                        }
                        Turn::Ask => {
                            let reply = tool(&spec, "codly_ask_user", json!({ "title": "Which greeting?", "question": "Formal or casual?" })).await;
                            assert_eq!(reply["result"]["status"], "waiting_for_user", "{reply}");
                        }
                    }
                    send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                }
            });
            Ok(handle)
        })
    }
}

/// Every turn does the work.
fn works() -> Script {
    Arc::new(|_| Turn::Work)
}

/// A core on `dir`'s data folder, as a start of Codly finds it.
async fn start_core(dir: &Path, seen: &Arc<Mutex<Seen>>, script: Script) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let fake = Fake { seen: seen.clone(), script };
    let opts = StartOptions { spawner: Some(Arc::new(fake)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

fn store_of(dir: &Path) -> Store {
    Store::open(&dir.join("data").join("codly.sqlite")).unwrap()
}

/// The office of `repo`, as an earlier run of Codly left it in the store, with these settings.
fn seed_office(store: &Store, repo: &Path, settings: Settings) -> Workspace {
    let ws = Workspace {
        id: new_id(),
        repo_path: std::fs::canonicalize(repo).unwrap().to_string_lossy().into_owned(),
        name: "repo".into(),
        default_branch: "main".into(),
        current_branch: None,
        head: None,
        dirty: false,
        created_at: now_ms(),
    };
    store.upsert_workspace(&ws).unwrap();
    store.save_settings(&Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..settings }).unwrap();
    ws
}

/// An ad-hoc task that writes `<key>.txt`, checked with `test -f`.
fn adhoc_task(id: &str, key: &str, status: &str) -> Value {
    json!({
        "id": id, "plan_id": null, "key": key, "title": format!("Write {key}.txt"), "goal": format!("Create {key}.txt"),
        "acceptance": [], "write_scope": [format!("{key}.txt")], "verify": [format!("test -f {key}.txt")], "depends_on": [], "status": status,
        "assignee": null, "agent_hint": null, "branch": null, "worktree": null, "attempt": 0,
        "status_reason": null, "summary": null, "created_at": now_ms(), "updated_at": now_ms(),
    })
}

/// What the worker of a seeded task had done when Codly closed.
#[derive(Clone, Copy, PartialEq)]
enum Left {
    /// Its turn was over and the work is there: only Codly's checks were left.
    Checked,
    /// Its turn was over, and the work is not there: the check fails.
    Unfinished,
    /// It was in the middle of another turn.
    MidTurn,
}

/// What a quit leaves behind for one ad-hoc task that was in its checks: its worker (session
/// `sess-<key>`), the run that was live, its worktree and the worker's turns. Returns
/// (task id, agent id).
fn seed_task_in_checks(store: &Store, ws: &Workspace, data: &Path, key: &str, left: Left) -> (Id, Id) {
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
    let agent: Agent = serde_json::from_value(json!({
        "id": agent_id, "name": format!("Worker {key}"), "provider": "claude", "model": null, "role": "worker", "status": "working",
        "activity": null, "activity_label": null, "task_id": task_id, "run_id": run_id, "session_id": format!("sess-{key}"),
        "worktree": wt, "billing": "subscription", "account": null, "color": 0, "look_seed": agent_id, "thread": null,
        "usage": Usage::default(), "created_at": now_ms(), "updated_at": now_ms(),
    }))
    .unwrap();
    store.append_event(Some(&ws.id), &ev(EventKind::AgentChanged { agent })).unwrap();
    let mut task = adhoc_task(&task_id, key, "verifying");
    task["assignee"] = json!(agent_id);
    task["branch"] = json!(format!("codly/adhoc/{key}"));
    task["worktree"] = json!(wt);
    task["attempt"] = json!(1);
    task["status_reason"] = json!("Codly is checking the work");
    store.append_event(Some(&ws.id), &ev(EventKind::TaskChanged { task: serde_json::from_value(task).unwrap() })).unwrap();
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
    store.append_event(Some(&ws.id), &ev(EventKind::TurnStarted { input: None })).unwrap();
    store.append_event(Some(&ws.id), &ev(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) })).unwrap();
    match left {
        Left::Checked => std::fs::write(wt.join(format!("{key}.txt")), "done\n").unwrap(),
        Left::Unfinished => {}
        Left::MidTurn => {
            // A message of yours started another turn, which had written a part of the file.
            store.append_event(Some(&ws.id), &ev(EventKind::TurnStarted { input: None })).unwrap();
            std::fs::write(wt.join(format!("{key}.txt")), "half\n").unwrap();
        }
    }
    (task_id, agent_id)
}

/// The task had failed before Codly closed: its worker's run is on record, and nothing of it
/// was in work.
fn fail_task(store: &Store, ws: &Workspace, task_id: &str) {
    let mut task = store.task(task_id).unwrap().expect("the seeded task");
    task.status = TaskStatus::Failed;
    task.status_reason = Some("failed: its check did not pass".into());
    let kind = EventKind::TaskChanged { task };
    let ev = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: Some(task_id.to_string()), run_id: None, workspace_id: None, kind };
    store.append_event(Some(&ws.id), &ev).unwrap();
}

/// What a quit leaves behind for one ad-hoc task whose review was under way: the task of
/// `seed_task_in_checks` with its worker's turn over and the work there, in its review, and
/// the reviewer's run that was live. Returns (task id, that run's id).
fn seed_task_in_review(store: &Store, ws: &Workspace, data: &Path, key: &str) -> (Id, Id) {
    let (task_id, _) = seed_task_in_checks(store, ws, data, key, Left::Checked);
    let (reviewer_id, run_id) = (new_id(), new_id());
    let ev = |agent_id: Option<&Id>, kind| EventRow { id: 0, ts: now_ms(), agent_id: agent_id.cloned(), task_id: Some(task_id.clone()), run_id: None, workspace_id: None, kind };
    let mut task = store.task(&task_id).unwrap().expect("the seeded task");
    task.status = TaskStatus::Reviewing;
    task.status_reason = Some("review by Rev (Claude Code)".into());
    let cwd = task.worktree.clone().expect("the seeded task has a worktree");
    store.append_event(Some(&ws.id), &ev(None, EventKind::TaskChanged { task })).unwrap();
    let reviewer: Agent = serde_json::from_value(json!({
        "id": reviewer_id, "name": "Rev", "provider": "claude", "model": null, "role": "reviewer", "status": "working",
        "activity": null, "activity_label": null, "task_id": task_id, "run_id": run_id, "session_id": "sess-review",
        "worktree": null, "billing": "subscription", "account": null, "color": 1, "look_seed": reviewer_id, "thread": null,
        "usage": Usage::default(), "created_at": now_ms(), "updated_at": now_ms(),
    }))
    .unwrap();
    store.append_event(Some(&ws.id), &ev(Some(&reviewer_id), EventKind::AgentChanged { agent: reviewer })).unwrap();
    store
        .insert_run(&RunRow {
            id: run_id.clone(),
            agent_id: reviewer_id,
            task_id: Some(task_id.clone()),
            role: Role::Reviewer,
            provider: Provider::Claude,
            model: None,
            cwd,
            session_id: Some("sess-review".into()),
            account: None,
            status: "running".into(),
            raw_log: "/dev/null".into(),
            report: None,
            started_at: now_ms(),
            ended_at: None,
        })
        .unwrap();
    (task_id, run_id)
}

/// The check of the seeded plan.
const PLAN_CHECK: &str = "test -f README.md";

/// What a quit leaves behind for a plan that was in its integration checks: its one task is
/// done, and its branch and worktree are there. Returns the plan's id.
fn seed_plan_in_checks(store: &Store, ws: &Workspace, data: &Path) -> Id {
    let repo = Path::new(&ws.repo_path);
    let root = format!("{}-{}", codly_core::orchestrator::names::slug(&ws.name, 24), &ws.id[ws.id.len() - 6..]);
    let wt = data.join("worktrees").join(root).join("p1").join("integration");
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(repo, &["worktree", "add", "-q", "-b", "codly/p1/integration", wt.to_str().unwrap()]);
    let plan_id = new_id();
    let plan: Plan = serde_json::from_value(json!({
        "id": plan_id, "workspace_id": ws.id, "thread": "lead", "request": "Add a greeting", "summary": "Add a greeting",
        "assumptions": [], "open_questions": [], "status": "integrating", "base_commit": git(repo, &["rev-parse", "HEAD"]),
        "base_branch": "main", "integration_branch": "codly/p1/integration", "integration_worktree": wt, "merged_sha": null, "kept": false,
        "created_at": now_ms(), "updated_at": now_ms(),
    }))
    .unwrap();
    let ev = |task_id: Option<&str>, kind| EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: task_id.map(String::from), run_id: None, workspace_id: None, kind };
    store.append_event(Some(&ws.id), &ev(None, EventKind::PlanChanged { plan })).unwrap();
    store.with(|db| codly_core::store::set_plan_json(db, &plan_id, &json!({ "integration_checks": [PLAN_CHECK] }))).unwrap();
    let task_id = new_id();
    let mut task = adhoc_task(&task_id, "t1", "done");
    task["plan_id"] = json!(plan_id);
    store.append_event(Some(&ws.id), &ev(Some(&task_id), EventKind::TaskChanged { task: serde_json::from_value(task).unwrap() })).unwrap();
    plan_id
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

/// The task, in whichever office it is.
async fn task_now(core: &Core, id: &str) -> Task {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .rev()
        .find_map(|e| match e.kind {
            EventKind::TaskChanged { task } if task.id == id => Some(task),
            _ => None,
        })
        .expect("the task is on the board")
}

async fn task_ended(core: &Core, id: &str) -> Task {
    wait_for("the task to end", async || {
        Some(task_now(core, id).await).filter(|t| matches!(t.status, TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Failed))
    })
    .await
}

/// The plan, as its last change left it.
async fn plan_now(core: &Core, id: &str) -> Plan {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .rev()
        .find_map(|e| match e.kind {
            EventKind::PlanChanged { plan } if plan.id == id => Some(plan),
            _ => None,
        })
        .expect("the plan is on record")
}

/// The commands Codly ran as checks, with their results.
async fn checks_run(core: &Core) -> Vec<(String, bool)> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Verified { command, ok, .. } => Some((command, ok)),
            _ => None,
        })
        .collect()
}

/// The verdicts on record, oldest first: (the reviewer's run, its verdict).
async fn verdicts(core: &Core) -> Vec<(Option<Id>, String)> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Reviewed { verdict, .. } => Some((e.run_id, verdict)),
            _ => None,
        })
        .collect()
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

/// The lines that say a task was resumed, sorted.
async fn resumed(core: &Core) -> Vec<String> {
    let mut lines: Vec<String> = decisions(core, "retry").await.into_iter().filter(|d| d.starts_with("Resumed after restart")).collect();
    lines.sort();
    lines
}

/// What the gate decided about a task's fix rounds, oldest first.
async fn fix_rounds(core: &Core) -> Vec<String> {
    decisions(core, "gate").await.into_iter().filter(|d| d.contains(" fix round")).collect()
}

/// The blocking question that waits for your answer.
async fn open_question(core: &Core) -> Option<Blocker> {
    core.list_blockers().await.unwrap().into_iter().find(|b| b.status == BlockerStatus::Open && b.severity == BlockerSeverity::Blocking)
}

/// Scheduling passes go by.
async fn passes(core: &Core) {
    for _ in 0..3 {
        core.update_settings(core.settings()).await.unwrap(); // starts a pass
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Codly closed while it checked two tasks: one worker's turn was over, the other was in a
/// turn. At the next start nothing goes on: no check, no worker. Your Resume lets both go on,
/// each as it would have without the question, and each says that it was resumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_that_was_under_way_goes_on_only_after_your_resume() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (over, mid) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        (seed_task_in_checks(&store, &ws, &data, "a1", Left::Checked), seed_task_in_checks(&store, &ws, &data, "b1", Left::MidTurn))
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }), "the question: two tasks were in work");

    passes(&core).await;
    for id in [&over.0, &mid.0] {
        let task = task_now(&core, id).await;
        assert_eq!((task.status, task.status_reason.as_deref()), (TaskStatus::Ready, Some(HELD)), "{}", task.key);
    }
    assert!(seen.lock().unwrap().runs.is_empty(), "no worker started before your answer");
    assert!(decisions(&core, "gate").await.is_empty() && decisions(&core, "merge").await.is_empty(), "no work was checked or merged");
    assert!(resumed(&core).await.is_empty());
    assert!(!core.activity().unwrap().busy(), "work that waits for you does not keep the Mac awake");

    core.resume_work().await.unwrap();
    assert_eq!(core.resume_offer(), None);
    for (task_id, _) in [&over, &mid] {
        let task = task_ended(&core, task_id).await;
        assert_eq!(task.status, TaskStatus::Done, "{}: {:?}", task.key, task.status_reason);
    }
    {
        let seen = seen.lock().unwrap();
        // The first task had only its checks left: it got no worker turn.
        assert!(!seen.runs.iter().any(|r| r.0 == over.1), "{:?}", seen.runs);
        // The second one's worker resumed its session, and knows why it had stopped.
        assert_eq!(seen.runs.len(), 1);
        assert!(seen.runs[0].0 == mid.1 && matches!(&seen.runs[0].1, SessionMode::Resume { session_id } if session_id == "sess-b1"), "{:?}", seen.runs);
        assert!(seen.prompts[0].contains("resuming after Codly restarted"), "{}", seen.prompts[0]);
    }
    let lines = ["Resumed after restart: Codly checks its work again", "Resumed after restart: its worker goes on where it stopped"];
    assert_eq!(resumed(&core).await, lines);

    // A second Resume has nothing to do.
    core.resume_work().await.unwrap();
    assert_eq!(resumed(&core).await.len(), 2);
    core.shutdown().await;
}

/// With "never" there is no question, and the rule is the same: nothing starts before Resume
/// work. A task that was only about to start waits too, and is not said to be resumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_that_was_about_to_start_waits_for_your_resume_too() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let task_id = new_id();
    {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings { resume_on_launch: ResumeOnLaunch::Never, ..Default::default() });
        let task: Task = serde_json::from_value(adhoc_task(&task_id, "a1", "ready")).unwrap();
        let kind = EventKind::TaskChanged { task };
        let ev = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: Some(task_id.clone()), run_id: None, workspace_id: None, kind };
        store.append_event(Some(&ws.id), &ev).unwrap();
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 0 }), "no task was in work, one was about to start");

    passes(&core).await;
    let task = task_now(&core, &task_id).await;
    assert_eq!((task.status, task.status_reason.as_deref()), (TaskStatus::Ready, Some(HELD)));
    assert!(seen.lock().unwrap().runs.is_empty(), "no worker started before your Resume");

    core.resume_work().await.unwrap();
    let task = task_ended(&core, &task_id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert!(resumed(&core).await.is_empty(), "it started for the first time: nothing was resumed");
    core.shutdown().await;
}

/// The office was paused when Codly closed. The task that was in work still waits for your
/// Resume, and after it for the office: its line says it was resumed, and no worker starts
/// before you take the pause off.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_of_a_paused_office_is_resumed_and_waits_for_the_office() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (task_id, _) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings { paused: true, ..Default::default() });
        seed_task_in_checks(&store, &ws, &data, "a1", Left::MidTurn)
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }));
    // The pause has its own reason for a task that waits.
    passes(&core).await;
    wait_for("the pause to show on the task", async || {
        Some(task_now(&core, &task_id).await).filter(|t| t.status_reason.as_deref().is_some_and(|r| r.starts_with("paused: no new tasks start")))
    })
    .await;

    core.resume_work().await.unwrap();
    assert_eq!(resumed(&core).await, ["Resumed after restart: its worker goes on where it stopped"]);
    passes(&core).await;
    assert_eq!(task_now(&core, &task_id).await.status, TaskStatus::Ready);
    assert!(seen.lock().unwrap().runs.is_empty(), "the office is still paused");

    core.update_settings(Settings { paused: false, ..core.settings() }).await.unwrap();
    let task = task_ended(&core, &task_id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    core.shutdown().await;
}

/// You close Codly again without an answer: the next start asks again, for the same tasks.
/// The task that only had its checks left still gets no worker turn, and a task you retried
/// while Codly waited is not one that was in work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_you_left_open_is_asked_again_at_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (over, mid, retried) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        let over = seed_task_in_checks(&store, &ws, &data, "a1", Left::Checked);
        let mid = seed_task_in_checks(&store, &ws, &data, "b1", Left::MidTurn);
        let retried = seed_task_in_checks(&store, &ws, &data, "c1", Left::MidTurn);
        fail_task(&store, &ws, &retried.0);
        (over, mid, retried)
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }), "the failed task was not in work");
    // Your retry of the failed task waits with the rest, and shows the same reason.
    core.move_task(&retried.0, TaskStatus::Ready).await.unwrap();
    wait_for("the retried task to wait for your Resume", async || {
        Some(task_now(&core, &retried.0).await).filter(|t| t.status_reason.as_deref() == Some(HELD))
    })
    .await;
    core.shutdown().await;
    drop(core);
    assert!(store_of(dir.path()).kv_get(HOLD_ROW).unwrap().is_some(), "the tasks that wait are in the store");

    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }), "the same two tasks, not the one you retried");
    passes(&core).await;
    assert!(seen.lock().unwrap().runs.is_empty(), "no worker started in either run");
    assert!(decisions(&core, "gate").await.is_empty(), "no work was checked in either run");

    core.resume_work().await.unwrap();
    for (task_id, _) in [&over, &mid, &retried] {
        let task = task_ended(&core, task_id).await;
        assert_eq!(task.status, TaskStatus::Done, "{}: {:?}", task.key, task.status_reason);
    }
    {
        let seen = seen.lock().unwrap();
        // Only its checks were left, two starts ago: still no worker turn.
        assert!(!seen.runs.iter().any(|r| r.0 == over.1), "{:?}", seen.runs);
        // The other two got one worker run each, and the one that was in a turn resumed its session.
        assert_eq!(seen.runs.len(), 2, "{:?}", seen.runs);
        assert!(seen.runs.iter().any(|r| r.0 == mid.1 && matches!(&r.1, SessionMode::Resume { session_id } if session_id == "sess-b1")), "{:?}", seen.runs);
        assert!(seen.prompts.iter().any(|p| p.contains("resuming after Codly restarted")), "{:?}", seen.prompts);
        assert!(!seen.prompts.iter().any(|p| p.contains(HELD)), "{:?}", seen.prompts);
    }
    let lines = ["Resumed after restart: Codly checks its work again", "Resumed after restart: its worker goes on where it stopped"];
    assert_eq!(resumed(&core).await, lines, "the retried task started over: it was not resumed");
    core.shutdown().await;
    drop(core);
    assert_eq!(store_of(dir.path()).kv_get(HOLD_ROW).unwrap(), None, "your Resume ended the hold");
}

/// The office was paused, and you closed Codly without an answer. The pause had written its
/// own reason on the task that was in work: the next start still asks about it, and it is
/// resumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_left_open_in_a_paused_office_is_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (task_id, _) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings { paused: true, ..Default::default() });
        seed_task_in_checks(&store, &ws, &data, "a1", Left::MidTurn)
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }));
    passes(&core).await;
    wait_for("the pause to show on the task", async || {
        Some(task_now(&core, &task_id).await).filter(|t| t.status_reason.as_deref().is_some_and(|r| r.starts_with("paused: no new tasks start")))
    })
    .await;
    core.shutdown().await;
    drop(core);

    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }), "the task that was in work still waits for your Resume");
    core.resume_work().await.unwrap();
    assert_eq!(resumed(&core).await, ["Resumed after restart: its worker goes on where it stopped"]);
    passes(&core).await;
    assert!(seen.lock().unwrap().runs.is_empty(), "the office is still paused");

    core.update_settings(Settings { paused: false, ..core.settings() }).await.unwrap();
    let task = task_ended(&core, &task_id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(resumed(&core).await.len(), 1);
    core.shutdown().await;
}

/// Codly asked, you chose "always" in Settings and closed it without an answer. At the next
/// start the tasks that waited go on at once, as resumed tasks: each has its line, the one
/// that only had its checks left gets no worker turn, and the other's worker is told why it
/// is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_you_left_waiting_goes_on_at_the_next_start_once_you_chose_always() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (over, mid) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        (seed_task_in_checks(&store, &ws, &data, "a1", Left::Checked), seed_task_in_checks(&store, &ws, &data, "b1", Left::MidTurn))
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }));
    core.update_settings(Settings { resume_on_launch: ResumeOnLaunch::Always, ..core.settings() }).await.unwrap();
    passes(&core).await;
    assert!(seen.lock().unwrap().runs.is_empty(), "the choice is for the next start: this one still waits for your Resume");
    core.shutdown().await;
    drop(core);

    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), None, "always: nothing waits for a Resume");
    for (task_id, _) in [&over, &mid] {
        let task = task_ended(&core, task_id).await;
        assert_eq!(task.status, TaskStatus::Done, "{}: {:?}", task.key, task.status_reason);
    }
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.runs.len(), 1, "{:?}", seen.runs);
        assert!(seen.runs[0].0 == mid.1 && matches!(&seen.runs[0].1, SessionMode::Resume { session_id } if session_id == "sess-b1"), "{:?}", seen.runs);
        assert!(seen.prompts[0].contains("resuming after Codly restarted"), "{}", seen.prompts[0]);
    }
    let lines = ["Resumed after restart: Codly checks its work again", "Resumed after restart: its worker goes on where it stopped"];
    assert_eq!(resumed(&core).await, lines);
    core.shutdown().await;
    drop(core);
    assert_eq!(store_of(dir.path()).kv_get(HOLD_ROW).unwrap(), None, "nothing waits any more");
}

/// Codly closed while a plan was in its integration checks: every task was done. The checks
/// are work too: none runs before your Resume, and then the plan finishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_plan_runs_its_checks_only_after_your_resume() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let plan_id = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        seed_plan_in_checks(&store, &ws, &data)
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 0 }), "no task was in work: the plan's checks were");

    passes(&core).await;
    assert_eq!(plan_now(&core, &plan_id).await.status, PlanStatus::Running, "the plan waits, with its checks still to run");
    assert!(checks_run(&core).await.is_empty(), "no check ran before your answer");
    assert!(decisions(&core, "merge").await.is_empty(), "the plan was not finished before your answer");
    assert!(!core.activity().unwrap().busy(), "a plan that waits for you does not keep the Mac awake");

    core.resume_work().await.unwrap();
    assert_eq!(core.resume_offer(), None);
    wait_for("the plan to finish", async || (plan_now(&core, &plan_id).await.status == PlanStatus::Done).then_some(())).await;
    assert_eq!(checks_run(&core).await, [(PLAN_CHECK.to_string(), true)]);
    assert!(seen.lock().unwrap().runs.is_empty(), "no worker was needed");
    core.shutdown().await;
}

/// A task used its first fix round, and Codly restarts while its worker is on the fix. The
/// round stays used: the next failed check is round 2 of 2, and the one after fails the task.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fix_rounds_used_before_a_restart_stay_used() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (task_id, _) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings { resume_on_launch: ResumeOnLaunch::Always, ..Default::default() });
        seed_task_in_checks(&store, &ws, &data, "a1", Left::Unfinished)
    };
    let rounds_key = format!("task_rounds:{task_id}");
    let seen = Arc::new(Mutex::new(Seen::default()));
    // The worker never writes the file. Its first turn (on the first fix) is still open when
    // Codly closes.
    let script: Script = Arc::new(|n| if n == 0 { Turn::Stay } else { Turn::Nothing });
    let core = start_core(dir.path(), &seen, script.clone()).await;
    wait_for("the worker's turn on the first fix", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    let before = fix_rounds(&core).await;
    assert!(before.len() == 1 && before[0].starts_with("a1: fix round 1/2"), "{before:?}");
    core.shutdown().await;
    drop(core);
    let kept: Value = serde_json::from_str(&store_of(dir.path()).kv_get(&rounds_key).unwrap().expect("the count is in the store")).unwrap();
    assert_eq!(kept["fix"], 1, "{kept}");

    let core = start_core(dir.path(), &seen, script).await;
    let task = task_ended(&core, &task_id).await;
    assert_eq!(task.status, TaskStatus::Failed, "{:?}", task.status_reason);
    assert!(task.status_reason.as_deref().is_some_and(|r| r.ends_with("after 2 fix rounds")), "{:?}", task.status_reason);
    let all = fix_rounds(&core).await;
    assert_eq!(all.len(), 3, "{all:?}");
    assert!(all[1].starts_with("a1: fix round 2/2"), "the round used before the restart was not handed out again: {all:?}");
    assert!(all[2].contains("after 2 fix rounds"), "{all:?}");
    core.shutdown().await;
    drop(core);
    assert_eq!(store_of(dir.path()).kv_get(&rounds_key).unwrap(), None, "a task that is over keeps no count");
}

/// A worker asked you something, and Codly restarts with the question open. Its process is
/// gone, but its task still has its files: a task that may change the same files waits until
/// you answered and the first task is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_that_waits_for_your_answer_keeps_its_files_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    // The first turn asks; every later one does the work.
    let script: Script = Arc::new(|n| if n == 0 { Turn::Ask } else { Turn::Work });
    let core = start_core(dir.path(), &seen, script.clone()).await;
    core.update_settings(Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let asks = core.create_task("Write the greeting", "Create a text file").await.unwrap();
    core.move_task(&asks.id, TaskStatus::Ready).await.unwrap();
    let card = wait_for("the question", async || open_question(&core).await).await;
    wait_for("the task to wait for you", async || (task_now(&core, &asks.id).await.status == TaskStatus::Blocked).then_some(())).await;
    core.shutdown().await;
    drop(core);

    let core = start_core(dir.path(), &seen, script).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    assert_eq!(core.resume_offer(), None, "nothing was in work: the task waits for your answer, not for a Resume");
    // A task with no write scope may change every file: it overlaps the one that waits.
    let other = core.create_task("Write the farewell", "Create another text file").await.unwrap();
    core.move_task(&other.id, TaskStatus::Ready).await.unwrap();
    let waits = wait_for("the second task to wait for the first one's files", async || {
        Some(task_now(&core, &other.id).await).filter(|t| t.status_reason.as_deref().is_some_and(|r| r.ends_with("which waits for your answer")))
    })
    .await;
    assert_eq!(waits.status, TaskStatus::Ready, "{:?}", waits.status_reason);
    passes(&core).await;
    assert_eq!(seen.lock().unwrap().runs.len(), 1, "no worker started while the question is open");

    // Your answer: the first task goes on, then the second one starts.
    core.answer_blocker(&card.id, "casual please").await.unwrap();
    for id in [&asks.id, &other.id] {
        let task = task_ended(&core, id).await;
        assert_eq!(task.status, TaskStatus::Done, "{}: {:?}", task.key, task.status_reason);
    }
    let answered = seen.lock().unwrap().prompts[1].clone();
    assert!(answered.starts_with("[User] Answer to your question \"Which greeting?\": casual please"), "{answered}");
    core.shutdown().await;
}

/// You cancel the tasks that were in work instead of resuming them. The question counts only
/// the ones that still wait. When none is left and nothing else would start, the hold ends
/// by itself: the store forgets it, and work you start then runs with no Resume.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hold_ends_by_itself_once_you_cancelled_the_tasks_it_held() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (first, second) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        (seed_task_in_checks(&store, &ws, &data, "a1", Left::Checked), seed_task_in_checks(&store, &ws, &data, "b1", Left::MidTurn))
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }));

    core.move_task(&first.0, TaskStatus::Cancelled).await.unwrap();
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }), "one task still waits");
    core.move_task(&second.0, TaskStatus::Cancelled).await.unwrap();
    assert_eq!(core.resume_offer(), None, "nothing waits any more");

    // Work you start now runs: nobody has to press Resume for it.
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let task = core.create_task("Write the greeting", "Create a text file").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(seen.lock().unwrap().runs.len(), 1, "only the new task's worker ran");
    assert!(resumed(&core).await.is_empty(), "nothing was resumed");
    core.shutdown().await;
    drop(core);
    assert_eq!(store_of(dir.path()).kv_get(HOLD_ROW).unwrap(), None, "the hold is over in the store too");
}

/// An office whose repo folder is not there (a disk that is not mounted) gets no work from
/// the scheduler. Its waiting task is no reason to ask, and holds nothing: work in another
/// office starts with no Resume.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_office_whose_folder_is_not_there_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let waiting = new_id();
    {
        let store = store_of(dir.path());
        let here = seed_office(&store, &repo, Settings::default());
        let folder = dir.path().join("unmounted").join("repo").to_string_lossy().into_owned();
        let away = Workspace { id: new_id(), repo_path: folder, name: "away".into(), ..here };
        store.upsert_workspace(&away).unwrap();
        let task: Task = serde_json::from_value(adhoc_task(&waiting, "a1", "ready")).unwrap();
        let kind = EventKind::TaskChanged { task };
        let ev = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: Some(waiting.clone()), run_id: None, workspace_id: None, kind };
        store.append_event(Some(&away.id), &ev).unwrap();
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), None, "nothing would start: there is nothing to ask");

    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let task = core.create_task("Write the greeting", "Create a text file").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(seen.lock().unwrap().runs.len(), 1, "only the new task's worker ran");
    core.shutdown().await;
    drop(core);
    let left = store_of(dir.path()).task(&waiting).unwrap().expect("the other office's task");
    assert_eq!((left.status, left.status_reason), (TaskStatus::Ready, None), "it waits for its folder, and shows no hold");
}

/// Codly asked, and you closed it without an answer. Before the next start, the worktree of
/// the task that only had its checks left is removed. Your Resume does not send that task to
/// checks that have nothing to run on: its worker goes on, in a new worktree on the task's
/// branch, and the task is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_whose_worktree_was_removed_goes_on_through_its_worker() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let (task_id, worker) = {
        let store = store_of(dir.path());
        let ws = seed_office(&store, &repo, Settings::default());
        seed_task_in_checks(&store, &ws, &data, "a1", Left::Checked)
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }));
    let worktree = PathBuf::from(task_now(&core, &task_id).await.worktree.expect("the task has a worktree"));
    core.shutdown().await;
    drop(core);
    std::fs::remove_dir_all(&worktree).unwrap();

    let core = start_core(dir.path(), &seen, works()).await;
    assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }), "the task still waits for your Resume");
    core.resume_work().await.unwrap();
    let task = task_ended(&core, &task_id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.runs.len(), 1, "{:?}", seen.runs);
        assert!(seen.runs[0].0 == worker && matches!(&seen.runs[0].1, SessionMode::Resume { session_id } if session_id == "sess-a1"), "{:?}", seen.runs);
        assert!(seen.prompts[0].contains("resuming after Codly restarted"), "{}", seen.prompts[0]);
    }
    assert_eq!(resumed(&core).await, ["Resumed after restart: its worker goes on where it stopped"]);
    core.shutdown().await;
}

/// Codly closed while a task was in its review. No run outlives Codly: the reviewer's is
/// gone, and Codly keeps no note of it. When the task goes on (after your Resume, or at once
/// with "always") it is checked again and reviewed by a new reviewer run, and that run's
/// verdict is the one Codly reads: the task is done, with one review on record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_review_that_was_under_way_is_done_again_by_a_new_reviewer() {
    for choice in [ResumeOnLaunch::Ask, ResumeOnLaunch::Always] {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path());
        let data = dir.path().join("data");
        let (task_id, lost_run) = {
            let store = store_of(dir.path());
            let ws = seed_office(&store, &repo, Settings { review_gate: true, resume_on_launch: choice, ..Default::default() });
            seed_task_in_review(&store, &ws, &data, "a1")
        };
        let seen = Arc::new(Mutex::new(Seen::default()));
        let core = start_core(dir.path(), &seen, works()).await;
        if choice == ResumeOnLaunch::Ask {
            assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }));
            passes(&core).await;
            assert!(seen.lock().unwrap().runs.is_empty(), "no reviewer started before your answer");
            assert!(verdicts(&core).await.is_empty(), "no review before your answer");
            core.resume_work().await.unwrap();
        }
        let task = task_ended(&core, &task_id).await;
        assert_eq!(task.status, TaskStatus::Done, "{choice:?}: {:?}", task.status_reason);
        let checks = checks_run(&core).await;
        assert!(!checks.is_empty() && checks.iter().all(|check| check == &("test -f a1.txt".to_string(), true)), "{choice:?}: it was checked again: {checks:?}");
        let (runs, given) = {
            let seen = seen.lock().unwrap();
            assert!(seen.prompts.is_empty(), "{choice:?}: the worker got no turn: {:?}", seen.prompts);
            (seen.runs.len(), seen.reviews.clone())
        };
        assert_eq!((runs, given.len()), (1, 1), "{choice:?}: one run, the reviewer's: {given:?}");
        let (run, session) = &given[0];
        assert!(run != &lost_run && matches!(session, SessionMode::New { .. }), "{choice:?}: a new run, not the one that was lost: {given:?}");
        assert_eq!(verdicts(&core).await, [(Some(run.clone()), "approve".to_string())], "{choice:?}: its verdict is the one on record");
        assert_eq!(resumed(&core).await, ["Resumed after restart: Codly checks its work again"], "{choice:?}");
        core.shutdown().await;
    }
}
