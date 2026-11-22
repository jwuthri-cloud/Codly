//! A budget that is used up stops spend. With a fake agent in place of the CLI, on ad-hoc tasks:
//! past the token budget the scheduler starts no task, the waiting tasks say which budget and
//! the way out, you are told once, the count is the same after a restart, and Reset lets the
//! waiting tasks start. And a resumed OpenCode session is told what it cost before, so its cost
//! is not counted a second time.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
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
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
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
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(spawner), env: Some(std::env::vars().collect()), clis: Some(vec![cli(provider)]), index: false };
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
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
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
