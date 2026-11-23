//! A budget that is used up stops spend. With a fake agent in place of the CLI, on ad-hoc tasks:
//! past the token budget the scheduler starts no task, the waiting tasks say which budget and
//! the way out, you are told once, the count is the same after a restart, and Reset lets the
//! waiting tasks start. And a resumed OpenCode session is told what it cost before, so its cost
//! is not counted a second time.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec, claude};
use codly_core::model::*;
use codly_core::orchestrator::breaker::BreakerConfig;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::mpsc;

/// What one turn of the fake worker uses.
const TURN_TOKENS: i64 = 5000;
const BUDGET: i64 = 1000;
const HOLD: &str = "paused: the token budget of 1000 work tokens is used up; raise it or press Reset counters in Settings → Autonomy & Budgets";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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

/// A worker whose every prompt is one turn: it writes a file, reports `TURN_TOKENS` more work
/// tokens and is done.
struct Fake {
    runs: Arc<AtomicUsize>,
}

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let runs = self.runs.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            let n = runs.fetch_add(1, Ordering::SeqCst);
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(format!("sess-{n}")),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let cwd = spec.cwd.clone();
            tokio::spawn(async move {
                let mut used = 0;
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            send(EventKind::TurnStarted { input: Some(text) });
                            std::fs::write(cwd.join(format!("work-{n}.txt")), "done\n").unwrap();
                            used += TURN_TOKENS;
                            send(EventKind::UsageUpdated { usage: Usage { input_tokens: used, ..Default::default() } });
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => return send(EventKind::Exited { code: Some(0) }),
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A core on `dir`'s data (a second call is the app started again), with the repo open.
async fn start(dir: &Path, repo: &Path, runs: &Arc<AtomicUsize>) -> Arc<Core> {
    start_with(dir, repo, Arc::new(Fake { runs: runs.clone() }), Provider::Claude).await
}

/// The same, with `spawner` in place of the one installed CLI, `provider`.
async fn start_with(dir: &Path, repo: &Path, spawner: Arc<dyn Spawner>, provider: Provider) -> Arc<Core> {
    start_on(dir, repo, spawner, &[provider]).await
}

/// The same, with these CLIs installed.
async fn start_on(dir: &Path, repo: &Path, spawner: Arc<dyn Spawner>, providers: &[Provider]) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let clis = providers.iter().map(|p| cli(*p)).collect();
    let opts = StartOptions { spawner: Some(spawner), env: Some(std::env::vars().collect()), clis: Some(clis), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
}

async fn ready_task(core: &Core, title: &str) -> Task {
    let task = core.create_task(title, "Write one file").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap()
}

async fn task_now(core: &Core, id: &str) -> Task {
    core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.id == id).expect("the task is on the board")
}

/// The task, once it waits for the budget: still ready, with nobody on it.
async fn held(core: &Core, id: &str) -> Task {
    let task = wait_for("the task to wait for the budget", async || Some(task_now(core, id).await).filter(|t| t.status_reason.as_deref() == Some(HOLD))).await;
    assert_eq!((task.status, task.assignee.as_deref()), (TaskStatus::Ready, None));
    task
}

/// What Codly said in the conversation that starts with `start`.
async fn said(core: &Core, start: &str) -> Vec<String> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.author == "codly" && message.text.starts_with(start) => Some(message.text),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_the_budget_no_task_starts_also_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let runs = Arc::new(AtomicUsize::new(0));
    let core = start(dir.path(), &repo, &runs).await;
    core.set_breaker_config(BreakerConfig { token_budget: BUDGET, ..Default::default() }).await.unwrap();

    // Nothing is used yet: the first task runs. Its one turn takes the team over the budget.
    let first = ready_task(&core, "Write the greeting").await;
    wait_for("the first task to end", async || Some(task_now(&core, &first.id).await).filter(|t| t.status == TaskStatus::Done)).await;
    let before = core.breaker_status().await.unwrap();
    assert!(before.used.tokens >= TURN_TOKENS, "{before:?}");
    assert_eq!(before.today, before.used);
    assert!(before.hold.as_deref().is_some_and(|why| why.ends_with("work tokens, over the budget of 1000")), "{before:?}");

    // The next task does not start, and says which budget and the way out.
    let second = ready_task(&core, "Write the farewell").await;
    held(&core, &second.id).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);

    // You are told once, however many beats pass.
    core.breaker_beat().await;
    core.breaker_beat().await;
    let told = said(&core, "Budget reached").await;
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("work tokens, over the budget of 1000. No new task starts.") && told[0].contains("Reset counters"), "{}", told[0]);

    // Quit and start again: the count is the same, so the budget still holds.
    core.shutdown().await;
    drop(core);
    let core = start(dir.path(), &repo, &runs).await;
    assert_eq!(core.breaker_status().await.unwrap().used, before.used, "a restart does not give the budget back");
    let third = ready_task(&core, "Write the thanks").await;
    held(&core, &third.id).await;
    held(&core, &second.id).await;
    core.breaker_beat().await;
    assert_eq!(said(&core, "Budget reached").await.len(), 1, "you were told before the restart");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "no task started past the budget");

    // Reset: the count starts over, and the waiting tasks start without another word from you.
    core.reset_breaker().await.unwrap();
    wait_for("a waiting task to start", async || (runs.load(Ordering::SeqCst) > 1).then_some(())).await;
    assert_eq!(said(&core, "The budget has room again").await.len(), 1);
    assert!(core.breaker_status().await.unwrap().today.tokens >= before.today.tokens, "today's total is not a budget: Reset keeps it");
    core.shutdown().await;
}

/// What one turn of the fake OpenCode agent costs.
const TURN_COST: f64 = 0.25;

/// An agent that keeps one provider session: every prompt is one turn that costs `TURN_COST`.
/// As the adapters do, each run reports its own cost, from zero.
struct Talker {
    /// What each run was told its session cost before it.
    told: Arc<Mutex<Vec<f64>>>,
    /// The turns of all runs together.
    turns: Arc<AtomicUsize>,
}

impl Spawner for Talker {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        self.told.lock().unwrap().push(spec.session_cost_usd);
        let turns = self.turns.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            let session_id = match &spec.session {
                SessionMode::Resume { session_id } => session_id.clone(),
                SessionMode::New { .. } => "ses-1".to_string(),
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(session_id),
                billing: Billing::ApiKey,
                tools: vec![],
            });
            tokio::spawn(async move {
                let mut cost = 0.0;
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            send(EventKind::TurnStarted { input: Some(text) });
                            send(EventKind::Message { item_id: new_id(), text: "ok".into(), phase: MessagePhase::Final });
                            cost += TURN_COST;
                            turns.fetch_add(1, Ordering::SeqCst);
                            send(EventKind::UsageUpdated { usage: Usage { cost_usd: Some(cost), ..Default::default() } });
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("ok".into()), duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => return send(EventKind::Exited { code: Some(0) }),
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

/// An OpenCode agent reports its session's cost as a running total, so the adapter must know
/// what the session cost before a run. Here the Lead runs on OpenCode and resumes its session
/// after each restart: every run is told what the runs before it cost together.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resumed_opencode_session_is_told_what_it_cost_before() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let told = Arc::new(Mutex::new(Vec::new()));
    let turns = Arc::new(AtomicUsize::new(0));
    let mut before = vec![];
    for say in ["hi", "back", "again"] {
        let spawner = Arc::new(Talker { told: told.clone(), turns: turns.clone() });
        let core = start_with(dir.path(), &repo, spawner, Provider::Opencode).await;
        // The Lead is the only agent: what the team used so far is what its session cost.
        let cost = core.breaker_status().await.unwrap().used.cost_usd;
        before.push(cost);
        core.send_chat("lead", say).await.unwrap();
        wait_for("the Lead's turn to be counted", async || {
            let used = core.breaker_status().await.unwrap().used.cost_usd;
            (used > cost && used == turns.load(Ordering::SeqCst) as f64 * TURN_COST).then_some(())
        })
        .await;
        core.shutdown().await;
    }
    assert_eq!(*told.lock().unwrap(), before, "each run is told what the session cost before it");
    assert_eq!(before[0], 0.0, "a new session cost nothing before");
    assert!(before[1] > 0.0 && before[2] > before[1], "the cost of every earlier run, not only of the last: {before:?}");
}

const COST_HOLD: &str = "paused: the cost budget of $1.00 is used up; raise it or press Reset counters in Settings → Autonomy & Budgets";

/// One turn of the fake agent below: what it costs, and whether it reaches the limit its run
/// started with (`SpawnSpec::max_budget_usd`), as Claude Code then fails it.
#[derive(Clone, Copy)]
struct Turn {
    cost: f64,
    reaches: bool,
}

/// An agent whose turns follow `script`, or `by_role` first for an agent of that role (a turn
/// neither has an entry for costs nothing and completes; a worker's turn that completes writes
/// a file). It records the limit each run
/// started with, the prompts of each role, and how many runs it was told to stop.
#[derive(Clone, Default)]
struct Limited {
    script: Arc<Mutex<VecDeque<Turn>>>,
    by_role: Arc<Mutex<Vec<(Role, Turn)>>>,
    limits: Arc<Mutex<Vec<(Provider, Option<f64>)>>>,
    prompts: Arc<Mutex<Vec<Role>>>,
    stops: Arc<AtomicUsize>,
}

impl Limited {
    fn then(&self, turns: &[Turn]) {
        self.script.lock().unwrap().extend(turns.iter().copied());
    }

    fn then_for(&self, role: Role, turn: Turn) {
        self.by_role.lock().unwrap().push((role, turn));
    }

    fn limits(&self) -> Vec<(Provider, Option<f64>)> {
        self.limits.lock().unwrap().clone()
    }

    fn stops(&self) -> usize {
        self.stops.load(Ordering::SeqCst)
    }

    /// How many prompts the agents of `role` got.
    fn prompts_of(&self, role: Role) -> usize {
        self.prompts.lock().unwrap().iter().filter(|r| **r == role).count()
    }
}

impl Spawner for Limited {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        self.limits.lock().unwrap().push((spec.provider, spec.max_budget_usd));
        let (script, by_role, prompts, stops) = (self.script.clone(), self.by_role.clone(), self.prompts.clone(), self.stops.clone());
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(format!("sess-{}", spec.agent_id)),
                billing: Billing::ApiKey,
                tools: vec![],
            });
            let (name, limit, role, cwd) = (spec.agent_name.clone(), spec.max_budget_usd, spec.role, spec.cwd.clone());
            tokio::spawn(async move {
                let mut cost = 0.0;
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            prompts.lock().unwrap().push(role);
                            send(EventKind::TurnStarted { input: Some(text) });
                            let turn = {
                                let mut by_role = by_role.lock().unwrap();
                                match by_role.iter().position(|(r, _)| *r == role) {
                                    Some(i) => by_role.remove(i).1,
                                    None => script.lock().unwrap().pop_front().unwrap_or(Turn { cost: 0.0, reaches: false }),
                                }
                            };
                            cost += turn.cost;
                            send(EventKind::UsageUpdated { usage: Usage { cost_usd: Some(cost), ..Default::default() } });
                            if turn.reaches {
                                // What the Claude adapter says for the result `error_max_budget_usd`.
                                let why = claude::run_limit_reached(&name, limit.expect("a run with a limit of its own"));
                                send(EventKind::Error { message: why.clone(), fatal: false });
                                send(EventKind::TurnCompleted { status: TurnStatus::Failed, result_text: Some(why), duration_ms: Some(1) });
                            } else {
                                if role == Role::Worker {
                                    std::fs::write(cwd.join(format!("work-{}.txt", new_id())), "done\n").unwrap();
                                }
                                send(EventKind::Message { item_id: new_id(), text: "ok".into(), phase: MessagePhase::Final });
                                send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some("ok".into()), duration_ms: Some(1) });
                            }
                        }
                        AgentCmd::Shutdown => {
                            stops.fetch_add(1, Ordering::SeqCst);
                            return send(EventKind::Exited { code: Some(0) });
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

/// Hire an agent on `provider` and say hi: the limit its run started with.
async fn hire_and_talk(core: &Core, fake: &Limited, provider: Provider) -> Option<f64> {
    let runs = fake.limits().len();
    let agent = core.hire_agent(provider, None, None).await.unwrap();
    core.send_chat(&format!("agent:{}", agent.id), "hi").await.unwrap();
    let limits = wait_for("its run to start", async || Some(fake.limits()).filter(|l| l.len() > runs)).await;
    let (started_on, limit) = limits[runs];
    assert_eq!(started_on, provider);
    limit
}

/// The errors the agent's turns reported, oldest first.
async fn errors_of(core: &Core, agent: &str) -> Vec<String> {
    let mut events = core.events(Some(agent), None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Error { message, .. } => Some(message),
            _ => None,
        })
        .collect()
}

/// With a cost budget of 1.00 of which 0.40 is used, a Claude run starts with 0.60 as its own
/// limit. A Codex run gets none (only Claude Code takes one), and with the breaker off no run does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_claude_run_starts_with_what_is_left_of_the_cost_budget() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude, Provider::Codex]).await;
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();

    fake.then(&[Turn { cost: 0.4, reaches: false }]);
    assert_eq!(hire_and_talk(&core, &fake, Provider::Claude).await, Some(1.0), "nothing used yet: the whole budget");
    wait_for("the turn to be counted", async || (core.breaker_status().await.unwrap().used.cost_usd == 0.4).then_some(())).await;

    assert_eq!(hire_and_talk(&core, &fake, Provider::Claude).await, Some(0.6));
    assert_eq!(hire_and_talk(&core, &fake, Provider::Codex).await, None);
    core.set_breaker_config(BreakerConfig { enabled: false, cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();
    assert_eq!(hire_and_talk(&core, &fake, Provider::Claude).await, None);
    core.shutdown().await;
}

/// A turn that reaches its run's own limit says why, and Codly ends that run: the process
/// would fail every later turn at once. Your next message starts a new run with what is left then.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_reaches_its_runs_own_limit_ends_that_run() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude]).await;
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let thread = format!("agent:{}", agent.id);
    let ended = async |n: usize| {
        wait_for("the run to end", async || {
            let a = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == agent.id)?;
            (fake.stops() == n && a.run_id.is_none()).then_some(())
        })
        .await
    };

    // The turn takes the team past its budget: it says so, with the way out.
    fake.then(&[Turn { cost: 1.05, reaches: true }]);
    core.send_chat(&thread, "Run three commands").await.unwrap();
    ended(1).await;
    let budget = "Budget reached: the team has used about $1.05, over the cost budget of $1.00. To go on, raise the budget or press Reset counters in Settings → Autonomy & Budgets.";
    assert_eq!(errors_of(&core, &agent.id).await, [budget]);

    // You raise the budget. The next message starts a new run with what is left now. Its turn
    // reaches that limit while the budget still has room: it says what to do.
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 5.0, ..Default::default() }).await.unwrap();
    fake.then(&[Turn { cost: 0.1, reaches: true }]);
    core.send_chat(&thread, "Again").await.unwrap();
    ended(2).await;
    let reached = claude::run_limit_reached(&agent.name, 3.95);
    assert!(reached.contains("3.95 USD of your budget were left then") && reached.ends_with("Send your message again: the next run starts with what is left now."));
    assert_eq!(errors_of(&core, &agent.id).await, [budget.to_string(), reached]);

    core.send_chat(&thread, "Hi").await.unwrap();
    wait_for("a third run", async || (fake.limits().len() == 3).then_some(())).await;
    assert_eq!(fake.limits(), [(Provider::Claude, Some(1.0)), (Provider::Claude, Some(3.95)), (Provider::Claude, Some(3.85))]);
    core.shutdown().await;
}

/// A task's worker whose run reaches its limit fails as a worker does, never to the gate: its
/// run ends, and its task waits for the budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_whose_run_reaches_its_limit_waits_for_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude]).await;
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();

    fake.then(&[Turn { cost: 1.05, reaches: true }]);
    let task = ready_task(&core, "Write the greeting").await;
    let held = wait_for("the task to wait for the budget", async || {
        Some(task_now(&core, &task.id).await).filter(|t| t.status_reason.as_deref() == Some(COST_HOLD))
    })
    .await;
    // Still its worker's (a restart resumes that worker's session), but not running.
    assert_eq!(held.status, TaskStatus::Ready);
    let worker = held.assignee.clone().expect("the task keeps its worker for the restart");
    wait_for("the worker's run to end", async || {
        core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == worker && a.run_id.is_none()).map(|_| ())
    })
    .await;
    assert_eq!((fake.stops(), fake.limits()), (1, vec![(Provider::Claude, Some(1.0))]), "its run ended, and no other started");
    core.shutdown().await;
}

/// The decisions of `decision_type` so far, oldest first.
async fn decisions(core: &Core, decision_type: &str) -> Vec<String> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type: ty, summary, .. } if ty == decision_type => Some(summary),
            _ => None,
        })
        .collect()
}

/// A stop at the team's budget is no failure of the worker's: you raise the budget twice, its
/// run stops at the budget each time, and the task waits for the budget each time, never
/// blocked as "failed 3 times in a row", and no restart in a few seconds is promised.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_stopped_at_the_budget_again_after_a_raise_still_waits_for_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude]).await;
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();
    let hold = |budget: &str| format!("paused: the cost budget of ${budget} is used up; raise it or press Reset counters in Settings → Autonomy & Budgets");
    let waits = async |task: &Task, budget: &str, runs: usize| {
        let reason = hold(budget);
        wait_for("the task to wait for the budget", async || {
            let t = Some(task_now(&core, &task.id).await).filter(|t| t.status_reason.as_deref() == Some(reason.as_str()))?;
            let worker = t.assignee.clone()?;
            let free = core.snapshot().await.unwrap().agents.into_iter().any(|a| a.id == worker && a.run_id.is_none());
            (free && fake.limits().len() == runs && fake.stops() == runs).then_some(t)
        })
        .await
    };

    fake.then(&[Turn { cost: 1.05, reaches: true }, Turn { cost: 1.0, reaches: true }, Turn { cost: 1.0, reaches: true }]);
    let task = ready_task(&core, "Write the greeting").await;
    assert_eq!(waits(&task, "1.00", 1).await.status, TaskStatus::Ready);
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 2.0, ..Default::default() }).await.unwrap();
    waits(&task, "2.00", 2).await;
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 3.0, ..Default::default() }).await.unwrap();
    let t = waits(&task, "3.00", 3).await;
    assert_eq!(t.status, TaskStatus::Ready, "still waiting for the budget, not blocked");
    let limits: Vec<Option<f64>> = fake.limits().into_iter().map(|(_, l)| l).collect();
    assert_eq!(limits, [Some(1.0), Some(0.95), Some(0.95)], "each run starts with what is left then");
    assert_eq!(decisions(&core, "retry").await, Vec::<String>::new(), "no failure counted, no restart promised");
    core.shutdown().await;
}

/// A worker's run can reach its limit while the team's budget has room (you raised it while the
/// turn ran): the worker fails the way a worker does, counted, and starts again after its wait
/// with what is left now. Its thread does not tell you to send a message.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_whose_run_reaches_its_limit_with_budget_left_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude]).await;
    core.set_worker_retry_waits(vec![Duration::from_millis(100); 2]);
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();

    fake.then(&[Turn { cost: 0.2, reaches: true }]);
    let task = ready_task(&core, "Write the greeting").await;
    wait_for("a second run", async || (fake.limits().len() >= 2).then_some(())).await;
    assert_eq!(fake.limits()[..2], [(Provider::Claude, Some(1.0)), (Provider::Claude, Some(0.8))]);
    assert_eq!(
        decisions(&core, "retry").await,
        ["the worker's run reached the limit it started with (1.00 USD of your budget); Codly starts it again in a moment (retry 1 of 2)"]
    );
    let worker = task_now(&core, &task.id).await.assignee.expect("the task keeps its worker");
    let name = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == worker).unwrap().name;
    assert_eq!(
        errors_of(&core, &worker).await,
        [format!("This run of {name} reached the limit it started with (1.00 USD of your budget were left then). Codly ends this run; the task's card says what comes next.")]
    );
    core.shutdown().await;
}

/// A task's reviewer whose run reaches its limit before its verdict: the review stops, and the
/// task waits for you with the budget's reason. It is not merged without its review, the
/// reviewer is not nudged into a run that ends, and the gate does not fail on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reviewer_whose_run_reaches_its_limit_stops_the_review() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let fake = Limited::default();
    let core = start_on(dir.path(), &repo, Arc::new(fake.clone()), &[Provider::Claude]).await;
    core.update_settings(Settings { lead_followups: false, review_gate: true, ..Default::default() }).await.unwrap();
    core.set_breaker_config(BreakerConfig { cost_budget_usd: 1.0, ..Default::default() }).await.unwrap();

    // The worker's turn costs 0.30 and completes; the reviewer's run starts with 0.70 and its
    // turn takes the team past the budget.
    fake.then(&[Turn { cost: 0.3, reaches: false }]);
    fake.then_for(Role::Reviewer, Turn { cost: 0.75, reaches: true });
    let task = ready_task(&core, "Write the greeting").await;
    let reason = "blocked: the review stopped: the team has used about $1.05, over the cost budget of $1.00; raise the budget, then re-run the checks";
    let blocked = wait_for("the review to stop", async || Some(task_now(&core, &task.id).await).filter(|t| t.status == TaskStatus::Blocked)).await;
    assert_eq!(blocked.status_reason.as_deref(), Some(reason));
    wait_for("every run to end", async || core.snapshot().await.unwrap().agents.iter().all(|a| a.run_id.is_none()).then_some(())).await;
    assert_eq!(fake.limits(), [(Provider::Claude, Some(1.0)), (Provider::Claude, Some(0.7))]);
    assert_eq!(fake.prompts_of(Role::Reviewer), 1, "no nudge for a verdict");
    assert_eq!(task_now(&core, &task.id).await.status_reason.as_deref(), Some(reason), "the gate did not fail on it afterwards");
    let review = decisions(&core, "review").await;
    assert!(review.iter().any(|d| d.ends_with(reason)), "{review:?}");
    assert!(!review.iter().any(|d| d.contains("no verdict")), "not merged without its review: {review:?}");
    core.shutdown().await;
}
