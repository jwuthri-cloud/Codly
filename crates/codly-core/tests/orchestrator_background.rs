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
//!   one question comes after it, when the worker's last turn still gave no report, and the
//!   record of the stop says which of the two comes next, the question or the checks;
//! - the wait is one budget for a round: a turn of the worker in the middle does not start it
//!   again, and a round that the gate starts (a failed check) or that you start ("Send back"
//!   from the diff) has the whole wait again.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    BackgroundTask { id: id.into(), kind: kind.into(), description: description.into(), started_at: None }
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
    /// Knob: an interrupt ends the helpers it lists, and it says its list without them, as
    /// Claude Code was recorded to do with no turn running (part 4 §1 and §2).
    interrupt_ends_helpers: bool,
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
                                let mut said = cli.on_interrupt.clone();
                                if cli.interrupt_ends_helpers {
                                    cli.open.retain(|t| t.kind != "helper");
                                    said.push(listed(cli.open.clone()));
                                }
                                said
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
    // A dev server that `stop_task` ends, and a helper that the interrupt does not end (no
    // recording has such a helper: this is the way out when one does not stop).
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

    // Nothing ends by itself. After the wait Codly asks the CLI to stop both (the helper with
    // an interrupt, the command with `stop_task`), reads from its list which one is gone, and
    // checks the work as it is.
    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["interrupt", "tid-1"]);
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
    // A helper that the interrupt does not end.
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
    assert_eq!(cli.lock().unwrap().stops, ["interrupt"], "asked to stop once");
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

    // The pass waits for the command. It does not ask for the report first, and it says that
    // the report is the next step. (Before, it said "Codly checks the work when that ends".)
    let held = wait_held(&core).await;
    let reason = "its worker left the background command `sleep 8; echo hello > hello.txt` running; \
                  Codly asks the worker for its report when that ends, and stops it after 5 min";
    assert_eq!(held.status_reason.as_deref(), Some(reason));
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
    // The record of the stop names the step that comes next: the question, not the checks.
    let stopped = at("t1: the background command `npm run dev` still ran after 1 s. Codly stopped it and asks the worker for its report");
    let (waits, asked) = (at("not checked yet"), at("without a report; Codly asked for it"));
    let committed = after.iter().position(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert!(waits < stopped && stopped < asked && asked < committed, "{waits} {stopped} {asked} {committed}");
    let prompts = cli.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2, "the task, and one question: {prompts:?}");
    assert!(prompts[1].starts_with(REPORT_QUESTION), "{}", prompts[1]);
    assert_eq!(cli.lock().unwrap().stops, ["tid-1"]);
    assert_eq!(verified(&after), [true]);
    core.shutdown().await;
}

/// A worker that was asked for its report before is not asked a second time. When Codly then
/// stops what the worker left running, the record says that the checks come next, and they do.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_after_the_question_is_followed_by_the_checks() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // The first turn does the work and gives no report. The turn that Codly's question starts
    // gives none either, and leaves a dev server running (`stop_task` ends it).
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
                turn.emit(listed(open));
            } else {
                std::fs::write(turn.spec.cwd.join("hello.txt"), "hello\n").unwrap();
            }
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(1))).await;

    wait_plan_done(&core).await;
    let after = events(&core).await;
    let at = |words: &str| {
        let found = after.iter().position(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains(words)));
        found.unwrap_or_else(|| panic!("no decision says {words:?}"))
    };
    let asked = at("without a report; Codly asked for it");
    let stopped = at("t1: the background command `npm run dev` still ran after 1 s. Codly stopped it and checks the work now");
    let committed = after.iter().position(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert!(asked < at("not checked yet") && at("not checked yet") < stopped && stopped < committed, "{asked} {stopped} {committed}");
    let prompts = cli.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2, "the task, and one question: {prompts:?}");
    assert_eq!(verified(&after), [true]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_wait_for_background_work_is_one_budget_inside_a_round() {
    const BUDGET: Duration = Duration::from_secs(8);
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
    // When the worker's turn was about to end: no wait of the gate began before that.
    let turn_over = Arc::new(Mutex::new(None));
    let at_end = turn_over.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let (open, at_end) = (open.clone(), at_end.clone());
        Box::pin(async move {
            turn.work("hello\n", listed(open)).await;
            *at_end.lock().unwrap() = Some(Instant::now());
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(BUDGET)).await;

    let held = wait_held(&core).await;
    assert!(held.status_reason.as_deref().is_some_and(|r| r.ends_with("and stops them after 8 s")), "{:?}", held.status_reason);
    // 2 of the 8 seconds later the tests end, and Claude Code starts a turn by itself. The
    // pass at the end of that turn waits for the server with the time that is left.
    tokio::time::sleep(Duration::from_secs(2)).await;
    cli.lock().unwrap().open = vec![server.clone()];
    emit(&cli, listed(vec![server]));
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("The tests pass.".into()), duration_ms: Some(5) });

    // Codly stops the server when the budget is used up: not before, and not a second budget
    // after the turn in the middle. (No tight limit: the build box may be slow.)
    let limit = BUDGET + Duration::from_secs(30);
    wait_for("Codly to stop the server", limit.as_secs(), async || (!cli.lock().unwrap().stops.is_empty()).then_some(())).await;
    let took = turn_over.lock().unwrap().expect("the worker's turn ended").elapsed();
    assert!(took >= BUDGET, "the server was stopped {took:?} after the worker's turn: before the budget was used up");
    assert!(took <= limit, "the server was stopped {took:?} after the worker's turn");

    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["tid-1"]);
    let after = events(&core).await;
    // The records in order: the first wait with the whole budget, the wait of the turn's pass
    // with what the first one left, and the stop.
    let said = background_decisions(&after);
    assert_eq!(said.len(), 3, "{said:?}");
    assert_eq!((&said[0].1["wait_secs"], said[0].0.ends_with("and stops them after 8 s")), (&json!(8), true), "{}", said[0].0);
    let left = said[1].1["wait_secs"].as_u64().unwrap();
    assert!((1..=6).contains(&left), "the first wait took 2 s or more of the 8 s, and the second has what is left: {}", said[1].0);
    assert!(said[1].0.ends_with(&format!("running; Codly checks the work when that ends, and stops it after {left} s")), "{}", said[1].0);
    assert_eq!(said[2].0, "t1: the background command `npm run dev` still ran after Codly waited 8 s in all. Codly stopped it and checks the work now");
    assert_eq!(verified(&after), [true]);
    assert_eq!(cli.lock().unwrap().prompts.len(), 1);
    core.shutdown().await;
}

/// The first round uses the whole wait on a dev server, and its check fails. The gate sends
/// the work back, and the worker leaves a test run open after its fix: that round waits again,
/// with the whole budget. (With one budget for the run, Codly would stop the test run at once
/// and the worker would never see its result.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_that_the_gate_starts_waits_again_for_background_work() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // `stop_task` ends both.
    cli.lock().unwrap().stoppable = vec!["tid-1".into(), "tid-2".into()];
    let state = cli.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let state = state.clone();
        Box::pin(async move {
            // The first turn: a draft that fails the check, beside a dev server. The turn
            // after the failed check: the fix, beside a test run.
            let first = state.lock().unwrap().prompts.len() == 1;
            let (content, left) = match first {
                true => ("draft\n", background("tid-1", "command", "npm run dev")),
                false => ("hello\n", background("tid-2", "command", "npm test")),
            };
            state.lock().unwrap().open = vec![left.clone()];
            turn.work(content, listed(vec![left])).await;
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(2))).await;

    wait_plan_done(&core).await;
    let after = events(&core).await;
    let said: Vec<String> = background_decisions(&after).into_iter().map(|(summary, _)| summary).collect();
    let waits = |what: &str| format!("t1: not checked yet, its worker left the background command `{what}` running; Codly checks the work when that ends, and stops it after 2 s");
    let stopped = |what: &str| format!("t1: the background command `{what}` still ran after 2 s. Codly stopped it and checks the work now");
    assert_eq!(said, [waits("npm run dev"), stopped("npm run dev"), waits("npm test"), stopped("npm test")], "each round has the whole wait");
    assert_eq!(cli.lock().unwrap().stops, ["tid-1", "tid-2"]);
    assert_eq!(verified(&after), [false, true], "the draft failed its check, and the fix passed");
    assert_eq!(cli.lock().unwrap().prompts.len(), 2, "the task, and one fix round");
    core.shutdown().await;
}

/// Your own "Send back" from the diff starts a round too. The first round has used 2 seconds
/// or more of its wait on a dev server when your comments arrive. The worker does what you
/// asked and leaves a test run open: that round waits with the whole budget, not with what the
/// first one left. (With the first round's wait all used up, Codly would stop the test run at
/// once and the worker would never see its result.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_that_your_send_back_starts_waits_again_for_background_work() {
    // Long, so that it cannot run out on a slow build box: the test ends the open work itself.
    const BUDGET: Duration = Duration::from_secs(30);
    const COMMENTS: &str = "1. `hello.txt:1`\n   Say hello twice.";
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    let both = vec![background("tid-1", "command", "npm run dev"), background("tid-2", "command", "npm test")];
    let state = cli.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let (state, both) = (state.clone(), both.clone());
        Box::pin(async move {
            // The first turn: the work, beside a dev server. The turn for your comments: the
            // change, beside the server and a test run.
            let first = state.lock().unwrap().prompts.len() == 1;
            let (content, open) = match first {
                true => ("hello\n", both[..1].to_vec()),
                false => ("hello\nhello\n", both),
            };
            state.lock().unwrap().open = open.clone();
            turn.work(content, listed(open)).await;
        })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(BUDGET)).await;

    let held = wait_held(&core).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let sent = core.send_back_task(&held.id, COMMENTS).await.unwrap();
    assert_eq!(sent.status, TaskStatus::ChangesRequested);

    // The pass at the end of the worker's next turn waits for both, 30 seconds again: with
    // one budget for both rounds it would be 28 seconds or less.
    let said = wait_for("the wait of the second round", 40, async || {
        let said = background_decisions(&events(&core).await);
        (said.len() == 2).then_some(said)
    })
    .await;
    let waits = |what: &str, it: &str| format!("t1: not checked yet, its worker left {what} running; Codly checks the work when that ends, and stops {it} after 30 s");
    let server = "the background command `npm run dev`";
    assert_eq!(said[0].0, waits(server, "it"));
    assert_eq!(said[1].0, waits(&format!("{server}, the background command `npm test`"), "them"), "your comments began a round with the whole wait");
    assert_eq!(said[1].1["wait_secs"], json!(30));

    // Both end, and Claude Code starts a turn by itself: its end runs the gate.
    cli.lock().unwrap().open = vec![];
    emit(&cli, listed(vec![]));
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("The tests pass.".into()), duration_ms: Some(5) });
    wait_plan_done(&core).await;
    let after = events(&core).await;
    assert_eq!(background_decisions(&after).len(), 2, "no other wait, and no stop");
    assert!(cli.lock().unwrap().stops.is_empty(), "nothing was stopped");
    assert_eq!(verified(&after), [true], "checked once, on the work with your comments in it");
    let prompts = cli.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2, "the task, and your comments");
    assert!(prompts[1].contains(COMMENTS), "{}", prompts[1]);
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

/// A Claude worker leaves a helper and a command running. When the wait runs out, the helper
/// gets one interrupt (recorded with no turn running: it ends every helper and starts no turn,
/// part 4 §1 and §2) and the command gets `stop_task`. Before, the helper got `stop_task` too,
/// after which Claude Code starts a turn by itself, whose end ran the gate a second time. Now
/// the record's "checks the work now" is what happens: one check, and no other turn. (A
/// command alone gets no interrupt: `the_question_about_a_missing_report_comes_once_after_the_wait`.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_claude_workers_helper_is_stopped_with_an_interrupt_and_its_command_with_stop_task() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    let open = vec![background("tid-1", "command", "npm run dev"), background("tid-2", "helper", "Run the tests")];
    {
        let mut cli = cli.lock().unwrap();
        cli.open = open.clone();
        cli.stoppable = vec!["tid-1".into()];
        cli.interrupt_ends_helpers = true;
    }
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let open = open.clone();
        Box::pin(async move { turn.work("hello\n", listed(open)).await })
    });
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(1))).await;

    wait_plan_done(&core).await;
    assert_eq!(cli.lock().unwrap().stops, ["interrupt", "tid-1"], "one interrupt for the helper, `stop_task` for the command");
    let after = events(&core).await;
    let said = background_decisions(&after);
    assert_eq!(said.len(), 2, "{said:?}");
    let both = "the background command `npm run dev`, the helper `Run the tests`";
    assert_eq!(said[1].0, format!("t1: {both} still ran after 1 s. Codly stopped them and checks the work now"));
    assert_eq!(said[1].1["not_stopped"], json!([]));
    assert_eq!(verified(&after), [true], "checked once");
    assert_eq!(cli.lock().unwrap().prompts.len(), 1);
    core.shutdown().await;
}

/// A wait with nothing left of its budget, for a worker that gave no report: the task says
/// that Codly stops the work now and asks the worker for its report, which is what the pass
/// does next. (Before, it said "and checks the work".)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wait_with_nothing_left_names_the_question_about_the_report_as_the_next_step() {
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::default();
    // A helper that the interrupt does not end. The worker reports only when Codly asks.
    let open = vec![background("tid-2", "helper", "Run the tests")];
    cli.lock().unwrap().open = open.clone();
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
    let core = start(dir.path(), Provider::Claude, worker, &cli, Some(Duration::from_secs(6))).await;

    wait_held(&core).await;
    // The wait runs out and Codly asks the CLI to stop the helper. A short turn of the CLI's
    // own meanwhile: the pass at its end has nothing left of the budget.
    wait_for("Codly to ask the CLI to stop the helper", 20, async || (!cli.lock().unwrap().stops.is_empty()).then_some(())).await;
    emit(&cli, EventKind::TurnStarted { input: None });
    emit(&cli, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("Still at it.".into()), duration_ms: Some(5) });
    let now = "its worker left the helper `Run the tests` running; Codly stops it now and asks the worker for its report";
    wait_for("the turn's pass to say what comes next", 20, async || the_task(&core).await.filter(|t| t.status_reason.as_deref() == Some(now))).await;

    wait_plan_done(&core).await;
    let after = events(&core).await;
    let stopped = "t1: the helper `Run the tests` still ran after 6 s. Codly could not stop it, and asks the worker for its report";
    assert!(background_decisions(&after).iter().any(|(summary, _)| summary == stopped), "{:?}", background_decisions(&after));
    let prompts = cli.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2, "the task, and the question: {prompts:?}");
    assert!(prompts[1].starts_with(REPORT_QUESTION), "{}", prompts[1]);
    assert_eq!(verified(&after), [true]);
    core.shutdown().await;
}
