//! The gate and the work a worker left running, end to end with fake agents in a temp git repo:
//! - a task is not committed or checked while its worker's CLI lists a background command;
//!   the checks run after the turn the CLI starts by itself when the command ends, and the
//!   watchdog does not call the waiting worker idle;
//! - a Codex subagent that still works holds the gate too, and its end lets the pass go on
//!   with no turn of the worker;
//! - work that does not end is stopped when the wait runs out (Claude Code: `stop_task`, a
//!   Codex subagent: an interrupt), and the task's record says what Codly stopped and what it
//!   could not stop;
//! - a turn of the worker while Codly stops that work does not leave the task waiting for work
//!   that nobody waits for any more;
//! - a worker that waits for its own command is not asked for its report before the wait: the
//!   one question comes after it, when the worker's last turn still gave no report;
//! - the wait is one budget for a run: a turn of the worker in the middle does not start it
//!   again.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SessionRequest, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::watchdog::WatchdogSettings;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const VERIFY: &str = "grep -q hello hello.txt";
const HELPER: &str = "Ada (explorer): find the callers";

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

fn cli(provider: Provider) -> CliInfo {
    CliInfo {
        provider,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(provider),
        default_model: None,
    }
}

fn background(id: &str, kind: &str, description: &str) -> BackgroundTask {
    BackgroundTask { id: id.into(), kind: kind.into(), description: description.into() }
}

/// Claude Code's whole list of background tasks, as it says it at every change. (What ended
/// is for the agent sheet: the gate reads the list.)
fn listed(tasks: Vec<BackgroundTask>) -> EventKind {
    EventKind::BackgroundTasks { tasks, ended: vec![] }
}

fn helper(status: &str) -> EventKind {
    EventKind::SubAgent { call_id: "thread-1".into(), description: HELPER.into(), status: status.into() }
}

/// The worker's fake CLI, as the test sees it and steers it between turns.
#[derive(Default)]
struct WorkerCli {
    /// Its run, once it started: where it works, and where its events go.
    run: Option<(Arc<SpawnSpec>, mpsc::UnboundedSender<AdapterEvent>)>,
    /// The prompt of each of its turns.
    prompts: Vec<String>,
    /// What Codly asked it to stop, in order: a task id (`stop_task`), or "interrupt".
    stops: Vec<String>,
    /// The background tasks it lists, and the ones among them that a `stop_task` ends.
    open: Vec<BackgroundTask>,
    stoppable: Vec<String>,
    /// What it says when it is interrupted.
    on_interrupt: Vec<EventKind>,
}

type Cli = Arc<Mutex<WorkerCli>>;

/// An event of the worker's CLI outside a turn of Codly's.
fn emit(cli: &Cli, kind: EventKind) {
    let cli = cli.lock().unwrap();
    let (spec, events) = cli.run.as_ref().expect("the worker runs");
    let _ = events.send(AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind });
}

fn worktree(cli: &Cli) -> PathBuf {
    cli.lock().unwrap().run.as_ref().expect("the worker runs").0.cwd.clone()
}

/// A turn that the worker's CLI starts by itself, for the test to act in: it has the worker's
/// tools, and its start and its end are the test's to say (`emit`).
fn own_turn(cli: &Cli) -> FakeTurn {
    let cli = cli.lock().unwrap();
    let (spec, events) = cli.run.as_ref().expect("the worker runs");
    FakeTurn { spec: spec.clone(), prompt: String::new(), events: events.clone() }
}

const REPORT_QUESTION: &str = "[Codly] You ended your turn without a report.";

#[derive(Clone)]
struct FakeTurn {
    spec: Arc<SpawnSpec>,
    prompt: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
}

impl FakeTurn {
    fn emit(&self, kind: EventKind) {
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

    /// The worker's whole turn: it writes hello.txt, says what it leaves running, and reports.
    async fn work(&self, content: &str, left_running: EventKind) {
        std::fs::write(self.spec.cwd.join("hello.txt"), content).unwrap();
        self.emit(left_running);
        self.tool("codly_submit_report", json!({ "status": "done", "summary": "Did it." })).await;
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

struct FakeSpawner {
    worker: Behavior,
    cli: Cli,
}

/// A Lead that plans one task for every request: create hello.txt, checked by `VERIFY`.
async fn lead(turn: FakeTurn) {
    if turn.prompt.contains("[Codly]") {
        return;
    }
    let task = json!({
        "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt", "consultant": { "title": "Backend" },
        "acceptance_criteria": ["hello.txt is right"], "write_scope": ["hello.txt"], "depends_on": [], "verify": [VERIFY], "size": "M",
    });
    let reply = turn.tool("codly_submit_plan", json!({ "summary": "Add a hello file", "tasks": [task] })).await;
    assert_eq!(reply["ok"], true, "{reply}");
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (worker, cli) = (self.worker.clone(), self.cli.clone());
        Box::pin(async move {
            let session = match &spec.session {
                SessionMode::New { session_id } => session_id.clone().unwrap_or_else(|| format!("fake-{}", spec.run_id)),
                SessionMode::Resume { session_id } => session_id.clone(),
            };
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let spec = Arc::new(spec);
            let is_worker = spec.role == Role::Worker;
            if is_worker {
                cli.lock().unwrap().run = Some((spec.clone(), events.clone()));
            }
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
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            let turn = FakeTurn { spec: spec2.clone(), prompt: text.clone(), events: events.clone() };
                            if is_worker {
                                cli.lock().unwrap().prompts.push(text);
                                worker(turn).await;
                            } else {
                                lead(turn).await;
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(5) });
                            send(EventKind::StatusChanged { status: AgentStatus::Idle });
                        }
                        // As recorded for Claude Code: the task's last lines, then the answer,
                        // which is the same whether anything was stopped or not.
                        AgentCmd::Session { request: SessionRequest::StopTask(id), reply } => {
                            let left = {
                                let mut cli = cli.lock().unwrap();
                                cli.stops.push(id.clone());
                                let ends = cli.stoppable.contains(&id);
                                cli.open.retain(|t| !(ends && t.id == id));
                                ends.then(|| cli.open.clone())
                            };
                            if let Some(tasks) = left {
                                send(listed(tasks));
                            }
                            let _ = reply.send(Ok(json!({})));
                        }
                        AgentCmd::Interrupt => {
                            let said = {
                                let mut cli = cli.lock().unwrap();
                                cli.stops.push("interrupt".into());
                                cli.on_interrupt.clone()
                            };
                            said.into_iter().for_each(&send);
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

/// A core with one open repo, where a request runs its plan at once and `worker` is the CLI
/// of its task's worker. `wait`: how long the gate waits for background work in this test.
async fn start(root: &Path, worker: Provider, behavior: Behavior, cli_state: &Cli, wait: Option<Duration>) -> Arc<Core> {
    let repo = temp_repo(root);
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let clis = if worker == Provider::Claude { vec![cli(Provider::Claude)] } else { vec![cli(Provider::Claude), cli(worker)] };
    let spawner = FakeSpawner { worker: behavior, cli: cli_state.clone() };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(clis), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let settings = Settings {
        auto_run_plans: true,
        lead_followups: false,
        worker_providers: vec![worker],
        // One second between turns is "idle" here, so a beat of the watchdog can show it.
        watchdog: WatchdogSettings { idle_secs: 1, ..Default::default() },
        ..Default::default()
    };
    core.update_settings(settings).await.unwrap();
    if let Some(wait) = wait {
        core.set_background_wait(wait);
    }
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

/// The task once the gate waits for what its worker left running.
async fn wait_held(core: &Core) -> Task {
    let held = |t: &Task| t.status_reason.as_deref().is_some_and(|r| r.starts_with("its worker left "));
    wait_for("the gate to wait for the worker's background work", 40, async || the_task(core).await.filter(held)).await
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

/// (summary, detail) of the gate's decisions about background work, oldest first.
fn background_decisions(events: &[EventRow]) -> Vec<(String, Value)> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { summary, detail, .. } if summary.contains("not checked yet") || summary.contains("still ran after") => {
                Some((summary.clone(), detail.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Did Codly commit or check anything yet?
fn checked(events: &[EventRow]) -> bool {
    events.iter().any(|e| matches!(e.kind, EventKind::Committed { .. } | EventKind::Verified { .. }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_is_not_checked_while_its_worker_has_a_background_command() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // The turn ends with a draft on disk and a command still at work: the command writes the
    // real file later. (As recorded: the list comes when the command starts.)
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            let command = background("tid-1", "command", "sleep 8; echo hello > hello.txt");
            turn.work("draft\n", listed(vec![command])).await;
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, None).await;

    let held = wait_held(&core).await;
    assert_eq!(held.status, TaskStatus::Running);
    let reason = "its worker left the background command `sleep 8; echo hello > hello.txt` running; \
                  Codly checks the work when that ends, and stops it after 5 min";
    assert_eq!(held.status_reason.as_deref(), Some(reason));
    // A moment later nothing is committed or checked, and the watchdog, which calls a worker
    // idle after one second here, leaves this one alone: Codly waits for it.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    core.watchdog_beat();
    let before = events(&core).await;
    assert!(!checked(&before), "nothing is committed or checked under a command that still runs");
    let said = background_decisions(&before);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].0, format!("t1: not checked yet, {reason}"));
    assert_eq!(said[0].1["background"], json!([{ "id": "tid-1", "kind": "command", "description": "sleep 8; echo hello > hello.txt" }]));
    assert_eq!(the_task(&core).await.unwrap().status_reason.as_deref(), Some(reason), "not flagged as idle");

    // The command ends: its file is there, the list is empty, and Claude Code starts a turn
    // by itself. The end of that turn is what runs the gate.
    std::fs::write(worktree(&cli).join("hello.txt"), "hello\n").unwrap();
    emit(&cli, listed(vec![]));
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("The command finished.".into()), duration_ms: Some(5) });
    wait_plan_done(&core).await;
    assert_eq!(the_task(&core).await.unwrap().status, TaskStatus::Done);
    let after = events(&core).await;
    assert_eq!(verified(&after), [true], "checked once, on the file the command wrote; the draft would have failed");
    assert_eq!(cli.lock().unwrap().prompts.len(), 1, "no fix round: the draft was never checked");
    assert!(cli.lock().unwrap().stops.is_empty(), "nothing was stopped");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_codex_subagent_that_still_works_holds_the_gate_until_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.work("hello\n", helper("running")).await }));
    let core = start(dir.path(), Provider::Codex, worker, &cli, None).await;

    let held = wait_held(&core).await;
    let reason = format!("its worker left the helper `{HELPER}` running; Codly checks the work when that ends, and stops it after 5 min");
    assert_eq!((held.status, held.status_reason.as_deref()), (TaskStatus::Running, Some(reason.as_str())));
    assert!(!checked(&events(&core).await), "nothing is committed or checked under a helper that still works");

    // The subagent's turn ends on its own thread: the agent itself starts no turn, and the
    // pass goes on.
    emit(&cli, helper("completed"));
    wait_plan_done(&core).await;
    let after = events(&core).await;
    assert_eq!(verified(&after), [true]);
    let ended = after.iter().position(|e| matches!(&e.kind, EventKind::SubAgent { status, .. } if status == "completed")).unwrap();
    let committed = after.iter().position(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert!(ended < committed, "Codly committed after the helper was done");
    let cli = cli.lock().unwrap();
    assert_eq!((cli.prompts.len(), cli.stops.len()), (1, 0), "no other turn of the worker, and nothing was stopped");
    drop(cli);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_work_that_does_not_end_is_stopped_when_the_wait_runs_out() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // A dev server that `stop_task` ends, and a helper that it does not end.
    let open = vec![background("tid-1", "command", "npm run dev"), background("tid-2", "helper", "Run the tests")];
    {
        let mut cli = cli.lock().unwrap();
        cli.open = open.clone();
        cli.stoppable = vec!["tid-1".into()];
    }
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let open = open.clone();
        Box::pin(async move { turn.work("hello\n", listed(open)).await })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(1))).await;

    let held = wait_held(&core).await;
    let both = "the background command `npm run dev`, the helper `Run the tests`";
    assert_eq!(held.status_reason, Some(format!("its worker left {both} running; Codly checks the work when that ends, and stops them after 1 s")));

    // Nothing ends by itself. After the wait Codly asks the CLI to stop both, reads from its
    // list which one is gone, and checks the work as it is.
    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["tid-1", "tid-2"]);
    let after = events(&core).await;
    let said = background_decisions(&after);
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(
        said[1].0,
        format!(
            "t1: {both} still ran after 1 s. Codly stopped the background command `npm run dev`. \
             It could not stop the helper `Run the tests`, and checks the work as it is"
        )
    );
    assert_eq!(said[1].1["stopped"], json!([{ "id": "tid-1", "kind": "command", "description": "npm run dev" }]));
    assert_eq!(said[1].1["not_stopped"], json!([{ "id": "tid-2", "kind": "helper", "description": "Run the tests" }]));
    // The helper is still listed: the gate did not wait for it a second time.
    assert_eq!(verified(&after), [true]);
    let stopped = after.iter().position(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("still ran after"))).unwrap();
    let committed = after.iter().position(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert!(stopped < committed, "Codly committed after it stopped the command");
    assert_eq!(cli.lock().unwrap().prompts.len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_while_codly_stops_the_work_does_not_leave_the_task_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // A helper that `stop_task` does not end.
    let open = vec![background("tid-2", "helper", "Run the tests")];
    cli.lock().unwrap().open = open.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let open = open.clone();
        Box::pin(async move { turn.work("hello\n", listed(open)).await })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(6))).await;

    wait_held(&core).await;
    // The wait runs out, and Codly asks the CLI to stop the helper. While Codly looks at what
    // is left, the CLI starts a short turn by itself. The pass at its end finds the helper
    // still listed and nothing left of the run's wait: the stop that is under way is its stop.
    wait_for("Codly to ask the CLI to stop the helper", 20, async || (!cli.lock().unwrap().stops.is_empty()).then_some(())).await;
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("Still at it.".into()), duration_ms: Some(5) });
    let again = |t: &Task| t.status_reason.as_deref() == Some("its worker left the helper `Run the tests` running; Codly stops it now and checks the work");
    wait_for("the turn's pass to wait for the stop", 20, async || the_task(&core).await.filter(again)).await;

    // The helper is still listed, and nobody waits for it a second time: the task is checked
    // as it is, at once, and does not stay in a wait that nothing ends.
    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["tid-2"], "asked to stop once");
    let after = events(&core).await;
    let said: Vec<String> = background_decisions(&after).into_iter().map(|(summary, _)| summary).collect();
    let waits = "t1: not checked yet, its worker left the helper `Run the tests` running; Codly checks the work when that ends, and stops it after 6 s";
    let stopped = "t1: the helper `Run the tests` still ran after 6 s. Codly could not stop it, and checks the work as it is";
    assert_eq!(said, [waits, stopped], "the first pass waits its 6 s, and the stop ends that wait and the wait of the turn's pass");
    assert_eq!(verified(&after), [true]);
    assert_eq!(cli.lock().unwrap().prompts.len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_that_waits_for_its_own_command_is_not_asked_for_a_report_first() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // The turn ends with a draft, a command still at work and no report: the worker means to
    // report when its command is done.
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            std::fs::write(turn.spec.cwd.join("hello.txt"), "draft\n").unwrap();
            turn.emit(listed(vec![background("tid-1", "command", "sleep 8; echo hello > hello.txt")]));
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, None).await;

    // The pass waits for the command. It does not ask for the report first.
    wait_held(&core).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(cli.lock().unwrap().prompts.len(), 1, "no question while the worker's command runs");

    // The command ends, and the turn Claude Code starts by itself gives the report.
    std::fs::write(worktree(&cli).join("hello.txt"), "hello\n").unwrap();
    emit(&cli, listed(vec![]));
    emit(&cli, EventKind::TurnStarted { input: None });
    let reply = own_turn(&cli).tool("codly_submit_report", json!({ "status": "done", "summary": "Did it." })).await;
    assert_eq!(reply["ok"], true, "{reply}");
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("The command finished.".into()), duration_ms: Some(5) });
    wait_plan_done(&core).await;
    let after = events(&core).await;
    assert_eq!(verified(&after), [true]);
    assert_eq!(cli.lock().unwrap().prompts.len(), 1, "the worker was never asked for its report");
    assert!(!after.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("without a report"))));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_question_about_a_missing_report_comes_once_after_the_wait() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // A dev server that `stop_task` ends. The worker leaves it running and gives no report,
    // until Codly asks for one.
    let open = vec![background("tid-1", "command", "npm run dev")];
    {
        let mut cli = cli.lock().unwrap();
        cli.open = open.clone();
        cli.stoppable = vec!["tid-1".into()];
    }
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let open = open.clone();
        Box::pin(async move {
            if turn.prompt.starts_with(REPORT_QUESTION) {
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Did it." })).await;
            } else {
                std::fs::write(turn.spec.cwd.join("hello.txt"), "hello\n").unwrap();
                turn.emit(listed(open));
            }
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(1))).await;

    wait_plan_done(&core).await;
    let after = events(&core).await;
    // First the wait and the stop of the server, then the one question, then the checks.
    let at = |words: &str| {
        let found = after.iter().position(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains(words)));
        found.unwrap_or_else(|| panic!("no decision says {words:?}"))
    };
    let (waits, stopped, asked) = (at("not checked yet"), at("still ran after 1 s. Codly stopped it"), at("without a report; Codly asked for it"));
    let committed = after.iter().position(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert!(waits < stopped && stopped < asked && asked < committed, "{waits} {stopped} {asked} {committed}");
    let prompts = cli.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2, "the task, and one question: {prompts:?}");
    assert!(prompts[1].starts_with(REPORT_QUESTION), "{}", prompts[1]);
    assert_eq!(cli.lock().unwrap().stops, ["tid-1"]);
    assert_eq!(verified(&after), [true]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_wait_for_background_work_is_one_budget_for_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // A dev server that never ends by itself (`stop_task` ends it), and a test run that ends
    // by itself and wakes the CLI.
    let (server, tests) = (background("tid-1", "command", "npm run dev"), background("tid-2", "command", "npm test"));
    let open = vec![server.clone(), tests];
    {
        let mut cli = cli.lock().unwrap();
        cli.open = open.clone();
        cli.stoppable = vec!["tid-1".into()];
    }
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let open = open.clone();
        Box::pin(async move { turn.work("hello\n", listed(open)).await })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(6))).await;

    let held = wait_held(&core).await;
    assert!(held.status_reason.as_deref().is_some_and(|r| r.ends_with("and stops them after 6 s")), "{:?}", held.status_reason);
    // 4 of the 6 seconds later the tests end, and Claude Code starts a turn by itself. The
    // pass at the end of that turn waits for the server with the time that is left.
    tokio::time::sleep(Duration::from_secs(4)).await;
    cli.lock().unwrap().open = vec![server.clone()];
    emit(&cli, listed(vec![server]));
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("The tests pass.".into()), duration_ms: Some(5) });

    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["tid-1"]);
    let after = events(&core).await;
    let said = background_decisions(&after);
    assert_eq!(said.len(), 3, "{said:?}");
    assert_eq!((&said[0].1["wait_secs"], said[0].0.ends_with("and stops them after 6 s")), (&json!(6), true), "{}", said[0].0);
    let left = said[1].1["wait_secs"].as_u64().unwrap();
    assert!((1..=2).contains(&left), "the second wait has what the first left of the 6 s: {}", said[1].0);
    assert!(said[1].0.ends_with(&format!("running; Codly checks the work when that ends, and stops it after {left} s")), "{}", said[1].0);
    assert_eq!(said[2].0, "t1: the background command `npm run dev` still ran after Codly waited 6 s in all. Codly stopped it and checks the work now");
    // The server was stopped about 6 s after the first wait began: not 4 s and 6 more.
    let at = |summary: &str| after.iter().find(|e| matches!(&e.kind, EventKind::Decision { summary: s, .. } if s == summary)).unwrap().ts;
    let took = at(&said[2].0) - at(&said[0].0);
    assert!((5_500..8_500).contains(&took), "stopped {took} ms after the first wait began");
    assert_eq!(verified(&after), [true]);
    assert_eq!(cli.lock().unwrap().prompts.len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_codex_subagent_that_does_not_end_is_interrupted_when_the_wait_runs_out() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    cli.lock().unwrap().on_interrupt = vec![helper("stopped")];
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.work("hello\n", helper("running")).await }));
    let core = start(dir.path(), Provider::Codex, worker, &cli, Some(Duration::from_secs(1))).await;

    wait_held(&core).await;
    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["interrupt"]);
    let after = events(&core).await;
    let said = background_decisions(&after);
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(said[1].0, format!("t1: the helper `{HELPER}` still ran after 1 s. Codly stopped it and checks the work now"));
    assert_eq!(said[1].1["not_stopped"], json!([]));
    assert_eq!(verified(&after), [true]);
    core.shutdown().await;
}
