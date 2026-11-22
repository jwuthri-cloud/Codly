//! A worker turn that did not finish is never checked as finished work. With a fake agent in
//! place of the CLI, on an ad-hoc task (no verify commands: the gate would pass anything):
//! a turn that fails or hits its turn limit is continued in the same session, one that keeps
//! failing blocks the task, a crash restarts the worker, and a worker the circuit breaker
//! constrains waits for your Resume. Neither your Resume nor a turn that answers a message
//! sends the unfinished work to the gate. A breaker stop says so on the task.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::breaker::BreakerConfig;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::{Notify, mpsc};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
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
    /// With this status and result text.
    Turn(TurnStatus, Option<&'static str>),
    /// As `Turn`, once the test lets it end.
    Held(Arc<Notify>, TurnStatus, Option<&'static str>),
    /// The CLI dies mid-turn: a failed turn, then an exit with code 1.
    Crash,
    /// It keeps going until Codly interrupts it.
    Interrupted,
}

/// One prompt the fake worker got.
struct Turn<'a> {
    /// Its number, over every run of the test.
    n: usize,
    prompt: &'a str,
    cwd: &'a Path,
    emit: &'a (dyn Fn(EventKind) + Send + Sync),
}

type Script = Arc<dyn for<'a> Fn(&Turn<'a>) -> End + Send + Sync>;

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the worker, in order.
    prompts: Vec<String>,
    runs: usize,
    shutdowns: usize,
}

struct Fake {
    seen: Arc<Mutex<Seen>>,
    script: Script,
}

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (seen, script) = (self.seen.clone(), self.script.clone());
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            seen.lock().unwrap().runs += 1;
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("sess".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let cwd = spec.cwd.clone();
            tokio::spawn(async move {
                let exit = |code| {
                    seen.lock().unwrap().shutdowns += usize::from(code == 0);
                    send(EventKind::Exited { code: Some(code) });
                };
                // A prompt that arrives during a turn waits for the turn's end, as in the Claude adapter.
                let mut waiting = VecDeque::new();
                loop {
                    let cmd = match waiting.pop_front() {
                        Some(prompt) => AgentCmd::Prompt(prompt),
                        None => match rx.recv().await {
                            Some(cmd) => cmd,
                            None => return,
                        },
                    };
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            let n = {
                                let mut s = seen.lock().unwrap();
                                s.prompts.push(text.clone());
                                s.prompts.len() - 1
                            };
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            let end = script(&Turn { n, prompt: &text, cwd: &cwd, emit: &send });
                            let done = |status, result: Option<&str>| {
                                send(EventKind::TurnCompleted { status, result_text: result.map(String::from), duration_ms: Some(1) });
                            };
                            match end {
                                End::Turn(status, result) => done(status, result),
                                End::Held(go, status, result) => {
                                    go.notified().await;
                                    done(status, result)
                                }
                                End::Crash => {
                                    done(TurnStatus::Failed, Some("the CLI exited before the turn finished"));
                                    send(EventKind::Error { message: "the CLI exited unexpectedly".into(), fatal: true });
                                    return exit(1);
                                }
                                End::Interrupted => loop {
                                    match rx.recv().await {
                                        Some(AgentCmd::Interrupt) => break done(TurnStatus::Interrupted, None),
                                        Some(AgentCmd::Prompt(prompt)) => waiting.push_back(prompt),
                                        Some(AgentCmd::Shutdown) => return exit(0),
                                        Some(AgentCmd::Approval { .. }) => {}
                                        None => return,
                                    }
                                },
                            }
                        }
                        AgentCmd::Shutdown => return exit(0),
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

/// A started core with one ad-hoc task that is ready to run, and what its worker sees.
async fn start(dir: &Path, breaker: Option<BreakerConfig>, script: Script) -> (Arc<Core>, Arc<Mutex<Seen>>, Task) {
    let repo = temp_repo(dir);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake { seen: seen.clone(), script })),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { lead_followups: false, ..Default::default() }).await.unwrap();
    core.set_turn_retry_waits(vec![Duration::from_millis(200); 2]);
    if let Some(breaker) = breaker {
        core.set_breaker_config(breaker).await.unwrap();
    }
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let task = core.create_task("Write the greeting", "Create hello.txt containing hello").await.unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    (core, seen, task)
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

/// The hello.txt of each commit Codly made, oldest first.
async fn committed(core: &Core, task: &Task) -> Vec<String> {
    let wt = PathBuf::from(task.worktree.as_deref().expect("the task has a worktree"));
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Committed { sha, .. } => Some(git(&wt, &["show", &format!("{sha}:hello.txt")])),
            _ => None,
        })
        .collect()
}

async fn decisions(core: &Core, kind: &str) -> Vec<String> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type, summary, .. } if decision_type == kind => Some(summary),
            _ => None,
        })
        .collect()
}

fn write_hello(turn: &Turn, text: &str) {
    std::fs::write(turn.cwd.join("hello.txt"), text).unwrap();
}

/// The worker's first turn ends short with half the work; the test expects the same session
/// to be asked to continue, and only the finished work to be checked.
async fn a_turn_that_ends_short_is_continued(status: TurnStatus, result: Option<&'static str>, said: &str) {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(move |turn: &Turn| {
        if turn.n == 0 {
            write_hello(turn, "hel");
            return End::Turn(status, result);
        }
        write_hello(turn, "hello\n");
        End::Turn(TurnStatus::Completed, None)
    });
    let (core, seen, task) = start(dir.path(), None, script).await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    {
        let seen = seen.lock().unwrap();
        assert_eq!((seen.runs, seen.prompts.len()), (1, 2), "the same session goes on: {:?}", seen.prompts);
        let again = &seen.prompts[1];
        assert!(again.contains("Your last turn on task") && again.contains(said), "{again}");
    }
    assert_eq!(committed(&core, &task).await, vec!["hello\n"], "the half-done work was never committed or checked");
    let retries = decisions(&core, "retry").await;
    assert!(retries.iter().any(|s| s.contains(said) && s.contains("Codly asks it to continue (try 1 of 2)")), "{retries:?}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_turn_is_not_gated_and_the_worker_continues() {
    a_turn_that_ends_short_is_continued(TurnStatus::Failed, Some("API Error: 529 Overloaded"), "API Error: 529 Overloaded").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_at_its_turn_limit_is_not_gated_and_the_worker_continues() {
    a_turn_that_ends_short_is_continued(TurnStatus::MaxTurns, None, "it reached its turn limit").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_keeps_failing_blocks_the_task_until_you_retry() {
    let dir = tempfile::tempdir().unwrap();
    let signed_in = Arc::new(AtomicBool::new(false));
    let works = signed_in.clone();
    let script: Script = Arc::new(move |turn: &Turn| {
        if !works.load(Ordering::SeqCst) {
            return End::Turn(TurnStatus::Failed, Some("Invalid API key · Please run /login"));
        }
        write_hello(turn, "hello\n");
        End::Turn(TurnStatus::Completed, None)
    });
    let (core, seen, task) = start(dir.path(), None, script).await;
    let blocked = task_ended(&core, &task.id).await;
    assert_eq!(blocked.status, TaskStatus::Blocked, "nothing was built, so nothing is done");
    let reason = blocked.status_reason.clone().unwrap_or_default();
    assert!(reason.contains("failed 3 times in a row") && reason.contains("Invalid API key"), "{reason}");
    assert_eq!(seen.lock().unwrap().prompts.len(), 3, "the brief and two retries");
    wait_for("the worker to be released", async || (seen.lock().unwrap().shutdowns == 1).then_some(())).await;
    assert!(committed(&core, &blocked).await.is_empty(), "nothing was committed or checked");

    // You sign in again and retry: the task runs to its end.
    signed_in.store(true, Ordering::SeqCst);
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(committed(&core, &task).await, vec!["hello\n"]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_mid_turn_restarts_the_worker_and_is_not_gated() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|turn: &Turn| {
        if turn.n == 0 {
            write_hello(turn, "hel");
            return End::Crash;
        }
        write_hello(turn, "hello\n");
        End::Turn(TurnStatus::Completed, None)
    });
    let (core, seen, task) = start(dir.path(), None, script).await;
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(committed(&core, &task).await, vec!["hello\n"], "the work of the crashed turn was never committed or checked");
    let retries = decisions(&core, "retry").await;
    assert!(retries.iter().any(|s| s.contains("exited unexpectedly")), "{retries:?}");
    // Past the retry wait: the restart owned the crash, so nobody asked the new run to continue again.
    tokio::time::sleep(Duration::from_millis(400)).await;
    {
        let seen = seen.lock().unwrap();
        assert_eq!((seen.runs, seen.prompts.len()), (2, 2), "{:?}", seen.prompts);
        assert!(seen.prompts[1].contains("exited unexpectedly") && !seen.prompts[1].contains("Your last turn on task"), "{}", seen.prompts[1]);
    }
    core.shutdown().await;
}

/// What agents said themselves in their own threads, oldest first.
async fn agents_said(core: &Core) -> Vec<String> {
    let mut events = core.events(None, None, 2000).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.thread == format!("agent:{}", message.author) => Some(message.text),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_turn_does_not_repeat_the_error_it_reported_as_the_agents_words() {
    const LIMIT: &str = "Claude's usage limit is reached: wait for it to reset, then try again";
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|turn: &Turn| {
        let (error, result) = match turn.n {
            // The turn reported why it failed, and its result is that same line.
            0 => (Some(LIMIT), LIMIT),
            // Another error came first: the result is news.
            1 => (Some("tool call failed"), "the CLI exited before the turn finished"),
            // No error at all: the result is all the thread gets.
            _ => (None, "API Error: 529 Overloaded"),
        };
        if let Some(message) = error {
            (turn.emit)(EventKind::Error { message: message.into(), fatal: false });
        }
        End::Turn(TurnStatus::Failed, Some(result))
    });
    let (core, seen, task) = start(dir.path(), None, script).await;
    let blocked = task_ended(&core, &task.id).await;
    assert_eq!(blocked.status, TaskStatus::Blocked, "{:?}", blocked.status_reason);
    assert_eq!(seen.lock().unwrap().prompts.len(), 3, "the brief and two retries");
    assert_eq!(agents_said(&core).await, ["the CLI exited before the turn finished", "API Error: 529 Overloaded"]);
    core.shutdown().await;
}

/// A retry wait no test reaches: only something else can ask the worker to continue.
const LONG_WAIT: Duration = Duration::from_secs(600);

/// A worker whose first turn fails with half the work once the test lets it end, and that
/// finishes the work only when Codly asks it to continue.
fn fails_then_continues(go: Arc<Notify>) -> Script {
    Arc::new(move |turn: &Turn| {
        if turn.n == 0 {
            write_hello(turn, "hel");
            return End::Held(go.clone(), TurnStatus::Failed, Some("API Error: 529 Overloaded"));
        }
        if turn.prompt.contains("Your last turn on task") {
            write_hello(turn, "hello\n");
        }
        End::Turn(TurnStatus::Completed, None)
    })
}

async fn failed_turn_seen(core: &Core) {
    wait_for("Codly to see the failed turn", async || (!decisions(core, "retry").await.is_empty()).then_some(())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn your_resume_after_a_failed_turn_continues_the_work_and_does_not_check_it() {
    let dir = tempfile::tempdir().unwrap();
    let go = Arc::new(Notify::new());
    let (core, seen, task) = start(dir.path(), None, fails_then_continues(go.clone())).await;
    core.set_turn_retry_waits(vec![LONG_WAIT; 2]);
    let worker = first_turn_worker(&core, &seen, &task).await;
    // "Stop after this step", and the step fails.
    assert!(core.stop_agent_after_step(&worker).await.unwrap().stop_requested);
    go.notify_one();
    wait_for("the pause", async || core.agent_control(&worker).await.unwrap().paused.then_some(())).await;
    failed_turn_seen(&core).await;

    // You resume it before Codly asked it to continue: it goes on, and nothing is checked yet.
    core.resume_agent(&worker).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.prompts.len(), 2, "{:?}", seen.prompts);
        assert!(seen.prompts[1].contains("Your last turn on task"), "{}", seen.prompts[1]);
    }
    assert_eq!(committed(&core, &task).await, vec!["hello\n"], "the resume did not check the half-done work");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_answers_you_after_a_failed_turn_is_not_checked_as_the_work() {
    let dir = tempfile::tempdir().unwrap();
    let go = Arc::new(Notify::new());
    let (core, seen, task) = start(dir.path(), None, fails_then_continues(go.clone())).await;
    core.set_turn_retry_waits(vec![LONG_WAIT; 2]);
    let worker = first_turn_worker(&core, &seen, &task).await;
    go.notify_one();
    failed_turn_seen(&core).await;

    // A note reaches the worker during the wait. Its answer is not the work: it is asked to continue.
    core.steer_agent(&worker, "use plain words").await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.prompts.len(), 3, "{:?}", seen.prompts);
        assert!(seen.prompts[1].contains("use plain words"), "{}", seen.prompts[1]);
        assert!(seen.prompts[2].contains("Your last turn on task"), "{}", seen.prompts[2]);
    }
    assert_eq!(committed(&core, &task).await, vec!["hello\n"], "the answer to the note did not send the half-done work to the gate");
    core.shutdown().await;
}

/// Two errors in a row trip the breaker; nothing else does.
fn error_storm(hard_stop: bool) -> BreakerConfig {
    BreakerConfig { enabled: true, token_budget: 0, velocity_per_min: 0, repeated_tool_limit: 0, error_storm_limit: 2, hard_stop }
}

fn stuck(turn: &Turn) -> End {
    for _ in 0..2 {
        (turn.emit)(EventKind::Error { message: "tool call failed".into(), fatal: false });
    }
    End::Interrupted
}

/// The task's worker, once it is in its first turn.
async fn first_turn_worker(core: &Core, seen: &Mutex<Seen>, task: &Task) -> Id {
    wait_for("the worker's first turn", async || (seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    task_now(core, &task.id).await.assignee.expect("the task has a worker")
}

async fn chats(core: &Core) -> Vec<String> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Chat { message } if message.author == "codly" => Some(message.text),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_the_breaker_constrains_waits_for_your_resume() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|turn: &Turn| {
        if turn.n == 0 {
            write_hello(turn, "hel");
            return stuck(turn);
        }
        if turn.prompt.contains("The user resumed you") {
            write_hello(turn, "hello\n");
        }
        End::Turn(TurnStatus::Completed, None) // the breaker's notes: it says where it stands
    });
    let (core, seen, task) = start(dir.path(), Some(error_storm(false)), script).await;
    let worker = first_turn_worker(&core, &seen, &task).await;
    // One level a beat: steered, then constrained (which holds it).
    wait_for("the breaker to constrain it", async || {
        core.breaker_beat().await;
        let control = core.agent_control(&worker).await.unwrap();
        (control.stop_requested || control.paused).then_some(())
    })
    .await;

    // It answers both notes, and that is all: paused, with the way back to its task held.
    wait_for("the worker to pause", async || {
        let control = core.agent_control(&worker).await.unwrap();
        (seen.lock().unwrap().prompts.len() == 3 && control.paused && control.held == 1).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await; // a gate would have started by now
    let paused = task_now(&core, &task.id).await;
    assert_eq!(paused.status, TaskStatus::Running, "{:?}", paused.status_reason);
    let reason = paused.status_reason.clone().unwrap_or_default();
    assert!(reason.starts_with("paused by the circuit breaker: it hit 2 errors in a row"), "{reason}");
    assert!(committed(&core, &paused).await.is_empty(), "its summary is not finished work");
    let said = chats(&core).await;
    assert!(said.iter().any(|t| t.starts_with("The circuit breaker paused") && t.contains("Resume")), "{said:?}");

    core.resume_agent(&worker).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    assert_eq!(committed(&core, &task).await, vec!["hello\n"]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_you_hold_that_the_breaker_constrains_gets_no_note_to_answer() {
    let dir = tempfile::tempdir().unwrap();
    let script: Script = Arc::new(|turn: &Turn| {
        if turn.n == 0 {
            write_hello(turn, "hel");
            return stuck(turn);
        }
        if turn.prompt.contains("The user resumed you") {
            write_hello(turn, "hello\n");
        }
        End::Turn(TurnStatus::Completed, None)
    });
    let (core, seen, task) = start(dir.path(), Some(error_storm(false)), script).await;
    let worker = first_turn_worker(&core, &seen, &task).await;
    assert!(core.stop_agent_after_step(&worker).await.unwrap().stop_requested);
    // One level a beat: the steer note is held, then the constrain interrupts the turn and
    // holds the way back to the task.
    wait_for("the breaker to interrupt it", async || {
        core.breaker_beat().await;
        let control = core.agent_control(&worker).await.unwrap();
        (control.paused && control.held == 2).then_some(())
    })
    .await;
    let paused = task_now(&core, &task.id).await;
    assert_eq!(paused.status, TaskStatus::Running, "{:?}", paused.status_reason);
    let reason = paused.status_reason.clone().unwrap_or_default();
    assert!(reason.starts_with("paused by the circuit breaker: it hit 2 errors in a row"), "{reason}");
    assert_eq!(seen.lock().unwrap().prompts.len(), 1, "nothing reached it while you hold it");

    // Your Resume: it reads why it was stopped and goes back to its task; only that is checked.
    core.resume_agent(&worker).await.unwrap();
    let task = task_ended(&core, &task.id).await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    {
        let seen = seen.lock().unwrap();
        assert!(!seen.prompts.iter().any(|p| p.contains("Stop active work now")), "no summary turn to gate: {:?}", seen.prompts);
        let back = seen.prompts.last().unwrap();
        assert!(back.contains("The user resumed you. The breaker stopped your work: it hit 2 errors in a row"), "{back}");
    }
    assert_eq!(committed(&core, &task).await, vec!["hello\n"], "the half-done work was never committed or checked");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_breaker_stop_is_not_reported_as_yours() {
    let dir = tempfile::tempdir().unwrap();
    let (core, seen, task) = start(dir.path(), Some(error_storm(true)), Arc::new(stuck)).await;
    let worker = first_turn_worker(&core, &seen, &task).await;
    // One level a beat: steered, constrained, then stopped.
    let task = wait_for("the breaker to stop it", async || {
        core.breaker_beat().await;
        Some(task_now(&core, &task.id).await).filter(|t| t.status != TaskStatus::Running)
    })
    .await;
    assert_eq!(task.status, TaskStatus::Blocked, "{:?}", task.status_reason);
    let reason = task.status_reason.clone().unwrap_or_default();
    assert!(reason.starts_with("stopped by the circuit breaker: it hit ") && reason.contains("errors in a row (limit 2)"), "{reason}");
    assert!(reason.contains("move it to Ready to resume with") && !reason.contains("by you"), "{reason}");
    wait_for("the worker to exit", async || (seen.lock().unwrap().shutdowns == 1).then_some(())).await;
    assert!(!core.snapshot().await.unwrap().agents.iter().any(|a| a.id == worker && a.task_id.is_some()), "the worker is free");
    let said = chats(&core).await;
    assert!(said.iter().any(|t| t.starts_with("The circuit breaker stopped") && t.contains("blocked until you retry it")), "{said:?}");
    core.shutdown().await;
}
