//! The repo owner's required checks (`.codly/checks`), end to end with fake agents and real
//! commands in a temp git repo:
//! - the gate runs them for a task after the task's own verify commands, with no OK asked
//!   (you turned that text on), and a failure goes back to the worker like any failed check;
//!   they run again on the finished plan;
//! - a checks file that is not the text you turned on runs nothing and passes nothing: the
//!   task waits until you read the new text;
//! - a required check that fails on the merged work fails the plan;
//! - "Check the work" runs them in the worktree of an agent you chat with: one run for an
//!   agent at a time, and no result when the agent worked while the commands ran;
//! - a plan whose task has only checks that prove nothing is marked, and a plan that runs at
//!   once says so in its chat line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::required_checks::{CheckResult, ChecksChoice, RequiredChecks};
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const CHECKS_FILE: &str = ".codly/checks";

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").arg("-C").arg(dir).args(args).output().unwrap()
}

/// A repo with one commit and this `.codly/checks` in the checkout.
fn temp_repo(root: &Path, checks: &str) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".codly")).unwrap();
    for args in [&["init", "-q", "-b", "main"][..], &["config", "user.name", "Test"], &["config", "user.email", "test@example.com"]] {
        assert!(git(&repo, args).status.success());
    }
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    assert!(git(&repo, &["add", "-A"]).status.success());
    assert!(git(&repo, &["commit", "-qm", "init"]).status.success());
    std::fs::write(repo.join(CHECKS_FILE), checks).unwrap();
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

    /// Write hello.txt and report the task done.
    async fn finish(&self, hello: &str) {
        std::fs::write(self.spec.cwd.join("hello.txt"), hello).unwrap();
        self.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote hello.txt." })).await;
    }
}

type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;
/// (role, prompt) of every turn.
type Turns = Arc<Mutex<Vec<(Role, String)>>>;

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
                            turns.lock().unwrap().push((spec2.role, text.clone()));
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

/// A Lead that plans one task for every request: write hello.txt, checked by `verify`.
fn lead_with(verify: &str) -> Behavior {
    let plan = json!({ "summary": "Add a hello file", "tasks": [{
        "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt", "consultant": { "title": "Backend" },
        "acceptance_criteria": ["hello.txt says hello"], "write_scope": ["hello.txt"], "depends_on": [], "verify": [verify], "size": "M",
    }] });
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

/// A worker that writes "hello" on every turn.
fn hello_worker() -> Behavior {
    Arc::new(|turn: FakeTurn| Box::pin(async move { turn.finish("hello\n").await }))
}

/// A core with the repo open, where a request runs its plan at once. No request is sent.
async fn start(root: &Path, repo: &Path, spawner: FakeSpawner) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
}

/// Turn the required checks on for the text the file has now.
async fn turn_on(core: &Core) -> RequiredChecks {
    let sha = core.required_checks(None).await.unwrap().sha;
    let on = core.required_checks(Some(ChecksChoice { allow: sha })).await.unwrap();
    assert!(on.on && !on.changed, "{on:?}");
    on
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

async fn wait_task(core: &Core, status: TaskStatus) -> Task {
    wait_for(&format!("t1 to be {status:?}"), 40, async || core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "t1" && t.status == status)).await
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

/// Codly's check runs, oldest first: (command, passed, for a task).
fn verified(events: &[EventRow]) -> Vec<(&str, bool, bool)> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { command, ok, .. } => Some((command.as_str(), *ok, e.task_id.is_some())),
            _ => None,
        })
        .collect()
}

/// The detail of every decision, oldest first.
fn decisions(events: &[EventRow]) -> Vec<&Value> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { detail, .. } => Some(detail),
            _ => None,
        })
        .collect()
}

fn worker_prompts(turns: &Turns) -> Vec<String> {
    turns.lock().unwrap().iter().filter(|t| t.0 == Role::Worker).map(|t| t.1.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gate_runs_your_required_checks_for_each_task_and_for_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    // `rm` is a risk for the approval rules: as a verify command of the Lead it would wait
    // for your OK. As a required check it is yours, and you turned it on.
    let required = "rm -f stale.txt && grep -q hello hello.txt";
    let repo = temp_repo(dir.path(), &format!("# the owner's checks\n\n{required}\n"));
    let turns = Turns::default();
    // The Lead's own check proves little; the worker's first try passes it and fails yours.
    let tries = Arc::new(AtomicUsize::new(0));
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let first = tries.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move { turn.finish(if first { "hi\n" } else { "hello\n" }).await })
    });
    let core = start(dir.path(), &repo, FakeSpawner { lead: lead_with("test -f hello.txt"), worker, turns: turns.clone() }).await;
    let shown = turn_on(&core).await;
    assert_eq!(shown.commands.len(), 1);
    assert!(shown.commands[0].risks.contains(&"deletes files".to_string()), "you saw the risk before you turned it on: {shown:?}");

    core.send_chat("lead", "Please do it").await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    let all = events(&core).await;
    let own = "test -f hello.txt";
    assert_eq!(
        verified(&all),
        [(own, true, true), (required, false, true), (own, true, true), (required, true, true), (required, true, false)],
        "after the task's own check, for the task; then once more for the finished plan"
    );
    let prompts = worker_prompts(&turns);
    assert_eq!(prompts.len(), 2, "the brief and one fix round");
    assert!(prompts[0].contains("The owner of this repo requires these checks") && prompts[0].contains(&format!("  - {required}\n")), "{}", prompts[0]);
    assert!(prompts[1].contains(&format!("Verification failed (fix round 1/2): `{required}` exited 1")), "{}", prompts[1]);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_that_runs_at_once_says_which_task_has_no_real_check() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path(), "# none yet\n");
    // A check that only looks for the file: it passes whatever the worker wrote in it.
    let own = "test -f hello.txt";
    let core = start(dir.path(), &repo, FakeSpawner { lead: lead_with(own), worker: hello_worker(), turns: Turns::default() }).await;

    // An Auto request: nobody clicks Run plan, so the plan never waits with its card open.
    core.send_chat("lead", "Please do it").await.unwrap();
    wait_plan(&core, PlanStatus::Done).await;
    let snap = core.snapshot().await.unwrap();
    let ready = snap.chats.iter().find(|c| c.author == "codly" && c.text.starts_with("Plan ready:")).expect("the plan's line");
    assert_eq!(ready.text, "Plan ready: Add a hello file (1 task). t1 has no check that can fail on the work: the plan card says why. Running it now.");
    // The mark is the plan's: its card shows it while the plan runs and when it is done.
    let all = events(&core).await;
    let weak = decisions(&all).into_iter().find(|d| d["rule"] == "weak_verify").expect("the weak check is marked");
    assert_eq!((&weak["task"], &weak["plan_id"]), (&json!("t1"), &json!(snap.plans[0].id)));
    assert_eq!(weak["checks"], json!([{ "command": own, "why": "only checks that a file is there" }]));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_checks_file_that_changed_stops_the_task_until_you_read_it() {
    let dir = tempfile::tempdir().unwrap();
    let own = "grep -q hello hello.txt";
    let repo = temp_repo(dir.path(), &format!("{own}\n"));
    let turns = Turns::default();
    let core = start(dir.path(), &repo, FakeSpawner { lead: lead_with(own), worker: hello_worker(), turns: turns.clone() }).await;
    turn_on(&core).await;
    // The file changes after you turned it on (a pull, a merge): you have not read this text.
    std::fs::write(repo.join(CHECKS_FILE), format!("{own}\ntest -f hello.txt\n")).unwrap();

    core.send_chat("lead", "Please do it").await.unwrap();
    let t = wait_task(&core, TaskStatus::Blocked).await;
    let reason = t.status_reason.clone().unwrap();
    assert!(reason.contains("Codly did not run your required checks: .codly/checks changed since you turned them on"), "{reason}");
    assert!(reason.contains("Settings → Safety → Required checks") && reason.contains("merge anyway"), "{reason}");
    let before = events(&core).await;
    assert!(verified(&before).is_empty(), "no check runs before all of them may: {:?}", verified(&before));
    assert!(decisions(&before).iter().any(|d| d["required"] == ".codly/checks changed since you turned them on"), "the task's card offers Merge anyway");
    assert_eq!(worker_prompts(&turns).len(), 1, "not the worker's to fix");
    let shown = core.required_checks(None).await.unwrap();
    assert!(!shown.on && shown.changed, "{shown:?}");

    // You read the new text, turn it on and re-run the checks: no worker turn, and the task lands.
    turn_on(&core).await;
    assert!(core.recheck_task(&t.id, vec![]).await.unwrap().is_empty());
    wait_plan(&core, PlanStatus::Done).await;
    let after = events(&core).await;
    // A required check the task has as its own verify command runs once for the task.
    let for_task: Vec<&str> = verified(&after).into_iter().filter(|v| v.2).map(|v| v.0).collect();
    assert_eq!(for_task, [own, "test -f hello.txt"]);
    assert!(verified(&after).iter().all(|v| v.1), "{:?}", verified(&after));
    assert_eq!(worker_prompts(&turns).len(), 1);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_required_check_that_fails_on_the_merged_work_fails_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    // Passes in the task's worktree, fails in the plan's integration worktree.
    let required = "test \"$(basename \"$PWD\")\" != integration";
    let repo = temp_repo(dir.path(), &format!("{required}\n"));
    let spawner = FakeSpawner { lead: lead_with("grep -q hello hello.txt"), worker: hello_worker(), turns: Turns::default() };
    let core = start(dir.path(), &repo, spawner).await;
    turn_on(&core).await;

    core.send_chat("lead", "Please do it").await.unwrap();
    wait_plan(&core, PlanStatus::Failed).await;
    let all = events(&core).await;
    assert_eq!(verified(&all), [("grep -q hello hello.txt", true, true), (required, true, true), (required, false, false)]);
    let snap = core.snapshot().await.unwrap();
    let said = snap.chats.iter().find(|c| c.author == "codly" && c.text.contains("finished")).map(|c| c.text.clone()).expect("Codly reports the plan");
    assert!(said.contains(&format!("integration checks failed: `{required}`")) && said.contains("merge it anyway"), "{said}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_the_work_runs_your_required_checks_in_a_solo_agents_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path(), "grep -q hello hello.txt\ntest -f hello.txt\n");
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), &repo, FakeSpawner { lead: idle.clone(), worker: idle, turns: Turns::default() }).await;
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = PathBuf::from(solo.worktree.clone().unwrap());
    let thread = format!("agent:{}", solo.id);

    // Off: nothing runs, and the error says where to turn them on.
    let e = core.check_agent(&solo.id).await.unwrap_err().to_string();
    assert!(e.contains("no required checks turned on") && e.contains(".codly/checks") && e.contains("Settings → Safety → Required checks"), "{e}");
    turn_on(&core).await;

    // All of them run, also after a failure: you see every result, with its output.
    let first = core.check_agent(&solo.id).await.unwrap();
    assert_eq!(first.iter().map(|r| (r.command.as_str(), r.ok, r.exit_code)).collect::<Vec<_>>(), [("grep -q hello hello.txt", false, Some(2)), ("test -f hello.txt", false, Some(1))]);
    assert!(first[0].tail.contains("hello.txt") && !first[0].timed_out, "{first:?}");
    std::fs::write(wt.join("hello.txt"), "hello\n").unwrap();
    let results = core.check_agent(&solo.id).await.unwrap();
    assert!(results.len() == 2 && results.iter().all(|r| r.ok && r.exit_code == Some(0)), "{results:?}");

    let said: Vec<String> = core.snapshot().await.unwrap().chats.into_iter().filter(|c| c.thread == thread && c.author == "codly").map(|c| c.text).collect();
    let failed = format!("Checked {}'s work: 2 of 2 required checks failed: `grep -q hello hello.txt` (exit 2), `test -f hello.txt` (exit 1).", solo.name);
    assert!(said.contains(&failed), "{said:?}");
    assert!(said.contains(&format!("Checked {}'s work: 2 required checks passed.", solo.name)), "{said:?}");
    let all = events(&core).await;
    let runs = all.iter().filter(|e| matches!(e.kind, EventKind::Verified { .. }) && e.agent_id.as_deref() == Some(solo.id.as_str())).count();
    assert_eq!(runs, 4, "each run is in the agent's record");
    // Each result is on record in full, output too: the agent's Git tab reads the last one from there.
    let checked: Vec<&Value> = decisions(&all).into_iter().filter(|d| d["agent_id"] == json!(solo.id)).collect();
    assert_eq!(checked.len(), 2);
    assert_eq!((&checked[0]["checks"], &checked[1]["checks"]), (&serde_json::to_value(&first).unwrap(), &serde_json::to_value(&results).unwrap()));

    // A text you did not read does not run here either.
    std::fs::write(repo.join(CHECKS_FILE), "true\n").unwrap();
    let e = core.check_agent(&solo.id).await.unwrap_err().to_string();
    assert!(e.starts_with("Codly did not run your required checks: .codly/checks changed since you turned them on."), "{e}");
    core.shutdown().await;
}

/// A required check that waits until the file `go` is in the worktree; `started` says it runs.
const WAITS: &str = "touch started; while [ ! -f go ]; do sleep 0.1; done";

/// A core with one agent you chat with, whose every turn says "ok", and `WAITS` turned on.
async fn solo_with_a_check_that_waits(root: &Path) -> (Arc<Core>, Agent, PathBuf) {
    let repo = temp_repo(root, &format!("{WAITS}\n"));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(root, &repo, FakeSpawner { lead: idle.clone(), worker: idle, turns: Turns::default() }).await;
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = PathBuf::from(solo.worktree.clone().unwrap());
    turn_on(&core).await;
    (core, solo, wt)
}

fn check_in_background(core: &Arc<Core>, agent: &Agent) -> tokio::task::JoinHandle<anyhow::Result<Vec<CheckResult>>> {
    let (core, id) = (core.clone(), agent.id.clone());
    tokio::spawn(async move { core.check_agent(&id).await })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_the_work_is_one_run_for_an_agent_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let (core, solo, wt) = solo_with_a_check_that_waits(dir.path()).await;

    // Two clicks (the Git tab was closed and opened again while the checks ran).
    let (first, second) = (check_in_background(&core, &solo), check_in_background(&core, &solo));
    wait_for("one of the two to be refused", 20, async || (first.is_finished() || second.is_finished()).then_some(())).await;
    std::fs::write(wt.join("go"), "").unwrap();
    let (ran, refused): (Vec<_>, Vec<_>) = [first.await.unwrap(), second.await.unwrap()].into_iter().partition(|r| r.is_ok());
    assert_eq!((ran.len(), refused.len()), (1, 1), "{ran:?} {refused:?}");
    assert_eq!(refused[0].as_ref().unwrap_err().to_string(), format!("Codly is still checking {}'s work; the result comes when its checks end", solo.name));
    assert!(ran[0].as_ref().unwrap().iter().all(|r| r.ok), "{ran:?}");
    let all = events(&core).await;
    assert_eq!(verified(&all).len(), 1, "the commands ran once: {:?}", verified(&all));

    // The run is over: the next click runs them again.
    assert!(core.check_agent(&solo.id).await.unwrap().iter().all(|r| r.ok));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_the_work_keeps_no_result_when_the_agent_worked_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let (core, solo, wt) = solo_with_a_check_that_waits(dir.path()).await;
    let thread = format!("agent:{}", solo.id);

    let check = check_in_background(&core, &solo);
    wait_for("the check to run", 20, async || wt.join("started").exists().then_some(())).await;
    // You send the agent a message while the commands run: its turn can change the files under them.
    core.send_chat(&thread, "one more thing").await.unwrap();
    wait_for("the agent's turn", 20, async || {
        events(&core).await.iter().any(|e| e.agent_id.as_deref() == Some(solo.id.as_str()) && matches!(e.kind, EventKind::TurnCompleted { .. })).then_some(())
    })
    .await;
    std::fs::write(wt.join("go"), "").unwrap();
    let e = check.await.unwrap().unwrap_err().to_string();
    assert_eq!(e, format!("{} worked while the checks ran, so their result is not about its files as they are now. Check its work again when its turn ends", solo.name));
    let said = |snap: &Snapshot| snap.chats.iter().filter(|c| c.thread == thread && c.author == "codly" && c.text.starts_with("Checked")).count();
    assert_eq!(said(&core.snapshot().await.unwrap()), 0, "no result is said");
    assert!(!decisions(&events(&core).await).iter().any(|d| d["agent_id"] == json!(solo.id)), "and none is on record");

    // Its turn is over: the next run is about the files as they are, and it counts.
    let results = wait_for("the agent to rest", 20, async || core.check_agent(&solo.id).await.ok()).await;
    assert!(results.iter().all(|r| r.ok), "{results:?}");
    assert_eq!(said(&core.snapshot().await.unwrap()), 1);
    core.shutdown().await;
}
