//! End-to-end orchestration with fake agents standing in for the CLIs, in a temp git repo:
//! Lead → codly_submit_plan (over the real MCP socket) → integration branch → worker in its
//! worktree → codly_submit_report → TurnCompleted → Codly commits → verify → scope → merge →
//! done → merge_plan. Plus a verify failure fixed in one round (with an auto-approved
//! command), crash recovery resuming a lost run by session id, and parallel request
//! threads (one Lead each, at most 3 live, plans running side by side).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::{RunRow, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.name", "Test"]);
    sh(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "node_modules/\n.env\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    // Installed deps and a local env file: ignored, so a fresh worktree would lack them.
    std::fs::create_dir_all(repo.join("node_modules/dep")).unwrap();
    std::fs::write(repo.join("node_modules/dep/index.js"), "module.exports = 1;\n").unwrap();
    std::fs::write(repo.join(".env"), "API_URL=http://localhost\n").unwrap();
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

/// What a fake agent does with a prompt.
type Behavior = Arc<dyn Fn(FakeTurn) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Clone)]
struct FakeTurn {
    spec: Arc<SpawnSpec>,
    prompt: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
    cmds: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<AgentCmd>>>,
}

impl FakeTurn {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.spec.agent_id.clone(), run_id: self.spec.run_id.clone(), kind });
    }

    fn say(&self, text: &str) {
        self.emit(EventKind::Message { item_id: codly_core::model::new_id(), text: text.into(), phase: MessagePhase::Final });
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

    fn write(&self, rel: &str, content: &str) {
        std::fs::write(self.spec.cwd.join(rel), content).unwrap();
        self.emit(EventKind::ToolStarted {
            call_id: rel.into(),
            name: "Write".into(),
            activity: ActivityKind::Create,
            target: Target { path: Some(rel.into()), ..Default::default() },
            input: json!({ "file_path": rel }),
            confidence: "observed".into(),
        });
        self.emit(EventKind::ToolCompleted {
            call_id: rel.into(),
            ok: true,
            output: None,
            exit_code: None,
            duration_ms: Some(1),
            tests_passed: None,
            tests_failed: None,
        });
        self.emit(EventKind::FilesChanged { call_id: Some(rel.into()), paths: vec![rel.into()], source: "tool_input".into() });
    }

    /// Ask for a shell command the way a CLI would, and wait for Codly's answer.
    async fn ask_approval(&self, command: &str) -> Option<ApprovalChoice> {
        let approval = Approval {
            id: codly_core::model::new_id(),
            agent_id: self.spec.agent_id.clone(),
            run_id: self.spec.run_id.clone(),
            task_id: None,
            call_id: Some("bash-1".into()),
            subject: ApprovalSubject::Command { command: command.into(), cwd: Some(self.spec.cwd.to_string_lossy().into_owned()) },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        self.emit(EventKind::ApprovalRequested { approval });
        let mut rx = self.cmds.lock().await;
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(AgentCmd::Approval { decision, .. })) => Some(decision.choice),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Log {
    spawns: Vec<(Role, String, SessionMode)>,
    prompts: Vec<(Role, String)>,
    /// (agent id, session) of every run, in spawn order.
    runs: Vec<(String, SessionMode)>,
    /// (agent id, prompt) of every turn.
    turns: Vec<(String, String)>,
    /// Agents asked to shut down.
    shutdowns: Vec<String>,
    /// (role, agent id, CLAUDE_CONFIG_DIR) of every run, in spawn order.
    envs: Vec<(Role, String, Option<String>)>,
    /// (agent id, model) of every run, in spawn order.
    models: Vec<(String, Option<String>)>,
}

struct FakeSpawner {
    log: Arc<Mutex<Log>>,
    lead: Behavior,
    worker: Behavior,
}

/// Reviewers share the worker behavior unless a test routes on `spec.role`.
fn by_role(worker: Behavior, reviewer: Behavior) -> Behavior {
    Arc::new(move |turn: FakeTurn| if turn.spec.role == Role::Reviewer { reviewer(turn) } else { worker(turn) })
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let log = self.log.clone();
        let behavior = if spec.role == Role::Lead { self.lead.clone() } else { self.worker.clone() };
        Box::pin(async move {
            let session = match &spec.session {
                SessionMode::New { session_id } => session_id.clone().unwrap_or_else(|| format!("fake-{}", spec.run_id)),
                SessionMode::Resume { session_id } => session_id.clone(),
            };
            {
                let mut l = log.lock().unwrap();
                l.spawns.push((spec.role, spec.cwd.to_string_lossy().into_owned(), spec.session.clone()));
                l.runs.push((spec.agent_id.clone(), spec.session.clone()));
                l.envs.push((spec.role, spec.agent_id.clone(), spec.env.get("CLAUDE_CONFIG_DIR").cloned()));
                l.models.push((spec.agent_id.clone(), spec.model.clone()));
            }
            let (tx, rx) = mpsc::unbounded_channel::<AgentCmd>();
            let spec = Arc::new(spec);
            let ev = |kind| AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind };
            let _ = events.send(ev(EventKind::SessionStarted {
                provider: spec.provider,
                model: Some("fake-model".into()),
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(session),
                billing: Billing::Subscription,
                tools: vec![],
            }));
            let cmds = Arc::new(tokio::sync::Mutex::new(rx));
            let (spec2, events2, cmds2) = (spec.clone(), events.clone(), cmds.clone());
            tokio::spawn(async move {
                loop {
                    let cmd = { cmds2.lock().await.recv().await };
                    let Some(cmd) = cmd else { break };
                    let send = |kind| {
                        let _ = events2.send(AdapterEvent { agent_id: spec2.agent_id.clone(), run_id: spec2.run_id.clone(), kind });
                    };
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            {
                                let mut l = log.lock().unwrap();
                                l.prompts.push((spec2.role, text.clone()));
                                l.turns.push((spec2.agent_id.clone(), text.clone()));
                            }
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            send(EventKind::StatusChanged { status: AgentStatus::Thinking });
                            let turn = FakeTurn { spec: spec2.clone(), prompt: text, events: events2.clone(), cmds: cmds2.clone() };
                            behavior(turn).await;
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(5) });
                            send(EventKind::StatusChanged { status: AgentStatus::Idle });
                        }
                        AgentCmd::Shutdown => {
                            log.lock().unwrap().shutdowns.push(spec2.agent_id.clone());
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

/// Runs of one role hand back their agent only when the test says so (a slow handshake).
struct Slow(FakeSpawner, Role, Arc<tokio::sync::Notify>);

impl Spawner for Slow {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let up = (spec.role == self.1).then(|| self.2.clone());
        let spawn = self.0.spawn(spec, events);
        Box::pin(async move {
            let handle = spawn.await;
            if let Some(up) = up {
                up.notified().await;
            }
            handle
        })
    }
}

/// Runs take a while to shut down and take no more prompts meanwhile, like the real adapters.
struct SlowStop(FakeSpawner, Duration);

impl Spawner for SlowStop {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (spawn, delay) = (self.0.spawn(spec, events), self.1);
        Box::pin(async move {
            let agent = spawn.await?;
            let (tx, mut rx) = mpsc::unbounded_channel();
            let handle = AgentHandle { agent_id: agent.agent_id.clone(), run_id: agent.run_id.clone(), tx };
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let last = matches!(cmd, AgentCmd::Shutdown);
                    if last {
                        tokio::time::sleep(delay).await;
                    }
                    agent.send(cmd);
                    if last {
                        break; // what was sent meanwhile is dropped
                    }
                }
            });
            Ok(handle)
        })
    }
}

/// The first reviewer start dies during its handshake, and fails when the test says so.
struct ReviewerDiesOnce {
    spawner: FakeSpawner,
    died: Arc<tokio::sync::Notify>,
    fail: Arc<tokio::sync::Notify>,
    done: AtomicBool,
}

impl Spawner for ReviewerDiesOnce {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        if spec.role != Role::Reviewer || self.done.swap(true, Ordering::SeqCst) {
            return self.spawner.spawn(spec, events);
        }
        let (died, fail) = (self.died.clone(), self.fail.clone());
        Box::pin(async move {
            let _ = events.send(AdapterEvent { agent_id: spec.agent_id, run_id: spec.run_id, kind: EventKind::Exited { code: Some(1) } });
            died.notify_one();
            fail.notified().await;
            Err(anyhow::anyhow!("the reviewer's CLI died"))
        })
    }
}

/// Every start gives up at once on a CLI that still comes up a moment later, then exits (a
/// handshake that outlasts its timeout).
struct LateStart;

impl Spawner for LateStart {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        Box::pin(async move {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let ev = |kind| AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind };
                let _ = events.send(ev(EventKind::SessionStarted {
                    provider: spec.provider,
                    model: Some("fake-model".into()),
                    cwd: spec.cwd.to_string_lossy().into_owned(),
                    cli_version: None,
                    session_id: Some("stale".into()),
                    billing: Billing::Subscription,
                    tools: vec![],
                }));
                let _ = events.send(ev(EventKind::Exited { code: Some(0) }));
            });
            Err(anyhow::anyhow!("the agent did not finish the handshake"))
        })
    }
}

/// A Lead that turns every user request into a one-task plan writing hello.txt.
fn planning_lead() -> Behavior {
    planning_lead_with(&["hello.txt"], "grep -q hello hello.txt")
}

fn planning_lead_with(scope: &[&str], verify: &str) -> Behavior {
    let plan = json!({
        "summary": "Add a hello file",
        "tasks": [{
            "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing hello",
            "consultant": { "title": "Backend" }, "acceptance_criteria": ["hello.txt contains hello"], "write_scope": scope,
            "depends_on": [], "verify": [verify], "size": "M",
        }],
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
            turn.say("I submitted a one-task plan.");
        })
    })
}

async fn task_status(core: &Core) -> Option<(Task, Plan)> {
    let s = core.snapshot().await.unwrap();
    Some((s.tasks.first()?.clone(), s.plans.first()?.clone()))
}

async fn start(root: &Path, spawner: impl Spawner) -> Arc<Core> {
    start_with_clis(root, spawner, vec![claude()]).await
}

async fn start_with_clis(root: &Path, spawner: impl Spawner, clis: Vec<CliInfo>) -> Arc<Core> {
    start_full(root, spawner, clis, false).await
}

async fn start_full(root: &Path, spawner: impl Spawner, clis: Vec<CliInfo>, index: bool) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(std::env::vars().collect()), clis: Some(clis), index };
    Core::start_with(cfg, opts).await.unwrap()
}

async fn wait_for<T>(what: &str, secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn kinds(events: &[EventRow]) -> Vec<&'static str> {
    events.iter().map(|e| e.kind.type_name()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_task_plan_runs_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            let task = turn.tool("codly_get_task", json!({})).await;
            assert_eq!(task["result"]["key"], "t1", "{task}");
            assert!(turn.spec.cwd.join("node_modules/dep/index.js").exists(), "deps seeded into the worktree");
            assert!(turn.spec.cwd.join(".env").exists(), "local env seeded into the worktree");
            turn.write("hello.txt", "hello\n");
            let note = turn.tool("codly_post_note", json!({ "text": "hello.txt exists now" })).await;
            assert_eq!(note["ok"], true, "{note}");
            let r = turn
                .tool(
                    "codly_submit_report",
                    json!({ "status": "done", "summary": "Created hello.txt with a greeting.", "files_changed": ["hello.txt"] }),
                )
                .await;
            assert_eq!(r["ok"], true, "{r}");
            turn.say("Done: hello.txt created.");
        })
    });
    // The real code index: the worker's brief gets a pointer pack, logged as a Retrieval.
    let core = start_full(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }, vec![claude()], true).await;
    let mut settings = Settings { auto_run_plans: true, lead_followups: false, ..Default::default() };
    settings.worker_providers = vec![Provider::Claude];
    core.update_settings(settings).await.unwrap();
    let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    assert_eq!(ws.current_branch.as_deref(), Some("main"));

    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let snap = wait_for("the plan to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        let done = s.plans.first().is_some_and(|p| p.status == PlanStatus::Done);
        done.then_some(s)
    })
    .await;

    let plan = &snap.plans[0];
    assert_eq!(plan.request, "Please add a hello file");
    let integ = plan.integration_branch.clone().unwrap();
    assert!(integ.starts_with("codly/add-a-hello-file-") && integ.ends_with("/integration"), "{integ}");
    let task = &snap.tasks[0];
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.attempt, 1);
    assert_eq!(task.summary.as_deref(), Some("Created hello.txt with a greeting."));
    assert!(task.branch.as_deref().unwrap().ends_with("/t1"));
    let integ_wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
    assert_eq!(std::fs::read_to_string(integ_wt.join("hello.txt")).unwrap(), "hello\n");
    assert!(!Path::new(task.worktree.as_deref().unwrap()).exists(), "a merged, clean task worktree is removed");
    assert!(sh(&repo, &["branch", "--list", "codly/*"]).contains("/t1"), "branches are kept");

    // The event log tells the whole story, in order.
    let events = core.events(None, None, 2000).await.unwrap();
    let ks = kinds(&events);
    for k in [
        "chat",
        "plan_changed",
        "task_changed",
        "decision",
        "agent_changed",
        "session_started",
        "turn_started",
        "tool_started",
        "files_changed",
        "committed",
        "verified",
        "merged",
        "memory_added",
        "turn_completed",
        "retrieval",
    ] {
        assert!(ks.contains(&k), "missing {k} in {ks:?}");
    }
    let pos = |k: &str| events.iter().rposition(|e| e.kind.type_name() == k).unwrap();
    assert!(pos("committed") > pos("verified") && pos("verified") > pos("merged"), "newest first: merged after verified after committed");
    let verified = events.iter().find_map(|e| match &e.kind {
        EventKind::Verified { command, ok, .. } => Some((command.clone(), *ok)),
        _ => None,
    });
    assert_eq!(verified, Some(("grep -q hello hello.txt".into(), true)));
    let committed_by = events.iter().find(|e| matches!(e.kind, EventKind::Committed { .. })).unwrap();
    assert_eq!(committed_by.task_id.as_deref(), Some(task.id.as_str()));
    let EventKind::Committed { files, .. } = &committed_by.kind else { unreachable!() };
    assert_eq!(files, &vec!["hello.txt".to_string()], "seeded deps and env files are never committed");
    let decisions: Vec<(String, String)> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { decision_type, summary, .. } => Some((decision_type.clone(), summary.clone())),
            _ => None,
        })
        .collect();
    for t in ["plan_validate", "route", "gate", "merge", "context"] {
        assert!(decisions.iter().any(|d| d.0 == t), "no {t} decision in {decisions:?}");
    }
    assert!(decisions.iter().any(|d| d.0 == "route" && d.1.starts_with("Claude Code · new consultant Ada")), "{decisions:?}");

    // Chat: the user's words, the Lead's reply, the plan card and the summary.
    let lead_thread: Vec<(String, String)> =
        snap.chats.iter().filter(|c| c.thread == "lead").map(|c| (c.author.clone(), c.text.clone())).collect();
    assert_eq!(lead_thread[0], ("user".into(), "Please add a hello file".into()));
    assert!(lead_thread.iter().any(|(a, t)| a != "user" && a != "codly" && t == "I submitted a one-task plan."), "{lead_thread:?}");
    assert!(
        snap.chats
            .iter()
            .any(|c| c.text.starts_with("Plan ready") && c.attachment.as_ref().is_some_and(|a| a["plan_id"] == plan.id.as_str()))
    );
    assert!(lead_thread.iter().any(|(_, t)| t.starts_with("Plan done")), "{lead_thread:?}");

    // The Lead got the live board header, read-only, at the repo root.
    {
        let log = log.lock().unwrap();
        let lead_prompt = &log.prompts.iter().find(|p| p.0 == Role::Lead).unwrap().1;
        assert!(lead_prompt.starts_with("<codly-board>") && lead_prompt.ends_with("Please add a hello file"), "{lead_prompt}");
        let worker_prompt = &log.prompts.iter().find(|p| p.0 == Role::Worker).unwrap().1;
        assert!(
            worker_prompt.contains("Task t1: Create hello.txt") && worker_prompt.contains("grep -q hello hello.txt"),
            "{worker_prompt}"
        );
        assert!(worker_prompt.contains("Do not commit"));
        let (_, cwd, _) = log.spawns.iter().find(|s| s.0 == Role::Lead).unwrap();
        assert_eq!(std::fs::canonicalize(cwd).unwrap(), std::fs::canonicalize(&repo).unwrap());
    }

    // Agents: the Lead and one worker, back to idle with no task.
    let worker = &agents_ever(&snap).into_iter().find(|a| a.role == Role::Worker).unwrap();
    assert_eq!(worker.name, "Ada");
    assert_eq!(worker.task_id, None);
    assert_eq!(worker.session_id.as_deref().map(|s| s.len()), Some(36), "a pre-assigned Claude session id");
    let summary_memory = events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::MemoryAdded { memory } if memory.kind == MemoryKind::Summary && memory.author == worker.id));
    assert!(summary_memory, "a Summary memory authored by the worker");

    // The user merges the integration branch into main.
    let sha = core.merge_plan(&plan.id).await.unwrap();
    assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), sha);
    assert!(!integ_wt.exists(), "the integration worktree is cleaned up once merged");
    assert!(sh(&repo, &["branch", "--list", "codly/*"]).contains("/integration"), "its branch is kept");
    assert_eq!(std::fs::read_to_string(repo.join("hello.txt")).unwrap(), "hello\n");
    assert!(!core.plan_diff(&plan.id).await.unwrap().is_empty());
    assert!(core.task_diff(&task.id).await.unwrap().contains("+hello"));

    // Uncommitted work the plan doesn't touch doesn't block a merge...
    std::fs::write(repo.join("README.md"), "changed\n").unwrap();
    assert_eq!(core.merge_plan(&plan.id).await.unwrap(), sha);
    // ...but a file the plan also changed is refused, and only that one is named.
    std::fs::write(repo.join("hello.txt"), "mine\n").unwrap();
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("uncommitted changes in hello.txt, which this plan also changes"), "{err}");
    assert!(!err.contains("README.md"), "{err}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verify_failure_gets_one_fix_round_and_a_lesson() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let approvals = Arc::new(Mutex::new(vec![]));
    let seen = approvals.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let seen = seen.clone();
        Box::pin(async move {
            if turn.prompt.contains("Verification failed") {
                // The verify command is auto-approved by the balanced policy.
                let choice = turn.ask_approval("grep -q hello hello.txt").await;
                seen.lock().unwrap().push(choice);
                turn.write("hello.txt", "hello\n");
            } else {
                turn.write("hello.txt", "hi\n");
            }
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote the greeting." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let snap = wait_for("the plan to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        let over = s.plans.first().is_some_and(|p| matches!(p.status, PlanStatus::Done | PlanStatus::Failed));
        over.then_some(s)
    })
    .await;
    assert_eq!(snap.plans[0].status, PlanStatus::Done);
    assert_eq!(approvals.lock().unwrap().as_slice(), &[Some(ApprovalChoice::AllowOnce)]);

    let events = core.events(None, None, 2000).await.unwrap();
    let verifies: Vec<bool> = events
        .iter()
        .rev()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { ok, .. } => Some(*ok),
            _ => None,
        })
        .collect();
    assert_eq!(verifies, vec![false, true]);
    let commits = events.iter().filter(|e| matches!(e.kind, EventKind::Committed { .. })).count();
    assert_eq!(commits, 2, "Codly commits after each worker turn");
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::ApprovalResolved { status: ApprovalStatus::Approved, by, .. } if by == "policy"))
    );
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
        if decision_type == "approval_policy" && summary.starts_with("auto-allowed verify command"))));
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("fix round 1/2"))));
    let lesson = events.iter().find_map(|e| match &e.kind {
        EventKind::MemoryAdded { memory } if memory.kind == MemoryKind::Lesson => Some(memory.text.clone()),
        _ => None,
    });
    let lesson = lesson.expect("a Lesson memory");
    assert!(
        lesson.starts_with("t1: `grep -q hello hello.txt` failed first") && lesson.contains("fixed by: Wrote the greeting."),
        "{lesson}"
    );

    let prompts = log.lock().unwrap().prompts.iter().filter(|p| p.0 == Role::Worker).map(|p| p.1.clone()).collect::<Vec<_>>();
    assert_eq!(prompts.len(), 2, "one brief, one fix round on the same session");
    assert!(prompts[1].starts_with("[Codly] Verification failed (fix round 1/2): `grep -q hello hello.txt` exited 1"), "{}", prompts[1]);
    let spawns = log.lock().unwrap().spawns.iter().filter(|s| s.0 == Role::Worker).count();
    assert_eq!(spawns, 1, "the fix round reuses the live worker");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_recovery_marks_lost_runs_and_resumes_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let repo_s = std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned();

    // The state a crash leaves behind: a running run with a pending approval, its task running.
    let (task_id, agent_id, run_id, approval_id) = {
        let store = Store::open(&data.join("codly.sqlite")).unwrap();
        let ws = Workspace {
            id: new_id(),
            repo_path: repo_s.clone(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&ws).unwrap();
        let ev = |kind| EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind };
        let agent_id = new_id();
        let run_id = new_id();
        let agent = Agent {
            id: agent_id.clone(),
            name: "Ada".into(),
            provider: Provider::Claude,
            model: None,
            role: Role::Worker,
            status: AgentStatus::Working,
            activity: Some(ActivityKind::Edit),
            activity_label: Some("x.txt".into()),
            task_id: None,
            run_id: Some(run_id.clone()),
            session_id: Some("sess-1".into()),
            worktree: None,
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
        store.append_event(Some(&ws.id), &ev(EventKind::AgentChanged { agent: agent.clone() })).unwrap();
        let task_id = new_id();
        let task = Task {
            id: task_id.clone(),
            plan_id: None,
            key: "a1".into(),
            title: "Write x".into(),
            goal: "Create x.txt".into(),
            acceptance: vec![],
            write_scope: vec![],
            verify: vec!["test -f x.txt".into()],
            depends_on: vec![],
            status: TaskStatus::Running,
            assignee: Some(agent_id.clone()),
            agent_hint: None,
            account_hint: None,
            consultant: None,
            fresh_session: false,
            branch: None,
            worktree: None,
            attempt: 1,
            status_reason: None,
            summary: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        store.append_event(Some(&ws.id), &ev(EventKind::TaskChanged { task })).unwrap();
        store
            .insert_run(&RunRow {
                id: run_id.clone(),
                agent_id: agent_id.clone(),
                task_id: Some(task_id.clone()),
                role: Role::Worker,
                provider: Provider::Claude,
                model: None,
                cwd: repo_s.clone(),
                session_id: Some("sess-1".into()),
                account: None,
                status: "running".into(),
                raw_log: "/dev/null".into(),
                report: None,
                started_at: now_ms(),
                ended_at: None,
            })
            .unwrap();
        let approval = Approval {
            id: new_id(),
            agent_id: agent_id.clone(),
            run_id: run_id.clone(),
            task_id: Some(task_id.clone()),
            call_id: None,
            subject: ApprovalSubject::Command { command: "rm -rf build".into(), cwd: None },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        store.append_event(Some(&ws.id), &ev(EventKind::ApprovalRequested { approval: approval.clone() })).unwrap();
        (task_id, agent_id, run_id, approval.id)
    };

    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("x.txt", "x\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created x.txt after the restart." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;

    let snap = wait_for("the resumed task to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.tasks.iter().find(|t| t.id == task_id && t.status == TaskStatus::Done).map(|_| s.clone())
    })
    .await;
    // The approval of the lost run expired; nothing is pending.
    assert!(snap.approvals.is_empty());
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(events.iter().any(
        |e| matches!(&e.kind, EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Expired, .. } if *id == approval_id)
    ));
    assert!(events.iter().any(
        |e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. } if decision_type == "retry" && summary.contains("lost"))
    ));
    let reason = events.iter().find_map(|e| match &e.kind {
        EventKind::TaskChanged { task } if task.id == task_id && task.status == TaskStatus::Ready => task.status_reason.clone(),
        _ => None,
    });
    assert_eq!(reason.as_deref(), Some("resuming after Codly restarted"));

    // The same agent resumed its session, with a continuation prompt, in a fresh worktree.
    {
        let log = log.lock().unwrap();
        let (role, cwd, session) = &log.spawns[0];
        assert_eq!(*role, Role::Worker);
        assert!(matches!(session, SessionMode::Resume { session_id } if session_id == "sess-1"), "{session:?}");
        assert!(cwd.ends_with("/adhoc/a1"), "{cwd}");
        let first = &log.prompts[0].1;
        assert!(first.starts_with("[Codly] You are back on task a1 \"Write x\" (resuming after Codly restarted)"), "{first}");
    }
    let task = snap.tasks.iter().find(|t| t.id == task_id).unwrap();
    assert_eq!(task.assignee.as_deref(), Some(agent_id.as_str()));
    assert_eq!(task.attempt, 2);
    assert_eq!(task.branch.as_deref(), Some("codly/adhoc/a1"), "ad-hoc work stays on its branch");
    // Its task done, the worker is let go (a ghost now).
    let ghosts = core.ghosts().unwrap();
    let agent = snap.agents.iter().chain(&ghosts).find(|a| a.id == agent_id).unwrap();
    assert_eq!(agent.task_id, None);

    let store = Store::open(&data.join("codly.sqlite")).unwrap();
    assert_eq!(store.run(&run_id).unwrap().unwrap().status, "lost");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recover_expires_pending_approvals_nobody_owns() {
    let dir = tempfile::tempdir().unwrap();
    // What a hard stop can leave behind: a request still pending, of a run that is not `running`
    // any more (written off at shutdown), so no lost run brings it along.
    let approval_id = {
        let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
        let approval = Approval {
            id: new_id(),
            agent_id: "gone".into(),
            run_id: "gone".into(),
            task_id: None,
            call_id: None,
            subject: ApprovalSubject::Command { command: "rm -rf build".into(), cwd: None },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        let kind = EventKind::ApprovalRequested { approval: approval.clone() };
        let ev = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind };
        store.append_event(None, &ev).unwrap();
        approval.id
    };
    let log = Arc::new(Mutex::new(Log::default()));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle.clone(), worker: idle }).await;
    assert!(core.snapshot().await.unwrap().approvals.is_empty(), "a card nobody can answer does not wait for you");
    let events = core.events(None, None, 100).await.unwrap();
    assert!(events.iter().any(
        |e| matches!(&e.kind, EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Expired, .. } if *id == approval_id)
    ));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn board_moves_follow_the_rules() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle.clone(), worker: idle }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let t = core.create_task("Write docs", "").await.unwrap();
    assert_eq!(t.status, TaskStatus::Pending);
    assert_eq!(t.goal, "Write docs");
    assert!(core.move_task(&t.id, TaskStatus::Done).await.is_err(), "only Codly marks tasks done");
    let c = core.move_task(&t.id, TaskStatus::Cancelled).await.unwrap();
    assert_eq!(c.status, TaskStatus::Cancelled);
    assert!(core.move_task(&t.id, TaskStatus::Running).await.is_err());
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    assert_eq!(solo.role, Role::Solo);
    assert!(solo.worktree.as_deref().unwrap().contains("/solo/"));
    assert!(core.hire_agent(Provider::Codex, None, None).await.is_err(), "Codex is not installed here");
    let e = core.send_chat("nowhere", "hi").await.unwrap_err().to_string();
    assert!(e.contains("unknown thread"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn solo_model_switches_and_ignores_no_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle.clone(), worker: idle }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let opus = core.set_agent_model(&solo.id, Some("opus".into())).await.unwrap();
    assert_eq!(opus.model.as_deref(), Some("opus"));
    let same = core.set_agent_model(&solo.id, Some("opus".into())).await.unwrap();
    assert_eq!(same.model.as_deref(), Some("opus"));
    assert!(core.set_agent_model("nobody", None).await.is_err());
    // Its look in the office: saved as given, junk refused.
    let looked = core.set_agent_look(&solo.id, "moss#style=3&hair=2").await.unwrap();
    assert_eq!(looked.look_seed, "moss#style=3&hair=2");
    assert!(core.set_agent_look(&solo.id, "  ").await.is_err());
    assert!(core.set_agent_look(&solo.id, "a\nb").await.is_err());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn solo_chat_uses_its_own_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let choices = Arc::new(Mutex::new(vec![]));
    let seen = choices.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let seen = seen.clone();
        Box::pin(async move {
            // Not a read or a test run: balanced mode asks, autonomous allows it in the worktree.
            let choice = turn.ask_approval("node scripts/gen.js").await;
            seen.lock().unwrap().push(choice);
            turn.say("done");
        })
    });
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle, worker }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    assert_eq!(core.snapshot().await.unwrap().settings.approval_mode, ApprovalMode::Balanced);
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let thread = format!("agent:{}", solo.id);

    let bad = RequestPrefs { mode: Some(LeadMode::Auto), approval_mode: Some(ApprovalMode::Autonomous), ..Default::default() };
    assert!(core.set_request_prefs(&thread, bad).await.is_err(), "a 1:1 chat has no Lead mode");
    let auto = RequestPrefs { approval_mode: Some(ApprovalMode::Autonomous), ..Default::default() };
    assert_eq!(core.set_request_prefs(&thread, auto).await.unwrap().approval_mode, Some(ApprovalMode::Autonomous));
    core.send_chat(&thread, "generate the file").await.unwrap();
    wait_for("the approval", 20, async || (!choices.lock().unwrap().is_empty()).then_some(())).await;
    assert_eq!(choices.lock().unwrap().as_slice(), &[Some(ApprovalChoice::AllowOnce)]);
    assert_eq!(core.snapshot().await.unwrap().request_prefs[&thread].approval_mode, Some(ApprovalMode::Autonomous));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_to_one_prefs_are_saved_in_the_agents_office() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (temp_repo(&dir.path().join("a")), temp_repo(&dir.path().join("b")));
    let log = Arc::new(Mutex::new(Log::default()));
    let choices = Arc::new(Mutex::new(vec![]));
    let seen = choices.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let seen = seen.clone();
        Box::pin(async move {
            let choice = turn.ask_approval("node scripts/gen.js").await;
            seen.lock().unwrap().push(choice);
            turn.say("done");
        })
    });
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle, worker }).await;
    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let thread = format!("agent:{}", solo.id);

    // Saved while another office is the open one (a second window, or a save that crossed a switch).
    core.open_workspace(b.to_str().unwrap()).await.unwrap();
    let auto = RequestPrefs { approval_mode: Some(ApprovalMode::Autonomous), ..Default::default() };
    core.set_request_prefs(&thread, auto).await.unwrap();

    // The broker reads them in the agent's office: there they are.
    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    core.send_chat(&thread, "generate the file").await.unwrap();
    wait_for("the approval", 20, async || (!choices.lock().unwrap().is_empty()).then_some(())).await;
    assert_eq!(choices.lock().unwrap().as_slice(), &[Some(ApprovalChoice::AllowOnce)]);
    assert_eq!(core.snapshot().await.unwrap().request_prefs.get(&thread).and_then(|p| p.approval_mode), Some(ApprovalMode::Autonomous));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_mid_gate_stays_cancelled_and_merges_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote it." })).await;
        })
    });
    let lead = planning_lead_with(&["hello.txt"], "sleep 1 && grep -q hello hello.txt");
    let core = start(dir.path(), FakeSpawner { log, lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (_, plan) =
        wait_for("verification to start", 20, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Verifying)).await;
    core.cancel_plan(&plan.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await; // the verify command finishes meanwhile
    let (task, plan) = task_status(&core).await.unwrap();
    assert_eq!(plan.status, PlanStatus::Cancelled);
    assert_eq!(task.status, TaskStatus::Cancelled, "an in-flight gate never resurrects a cancelled task");
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(e.kind, EventKind::Merged { .. })), "nothing merged");
    let integ = PathBuf::from(plan.integration_worktree.unwrap());
    assert!(!integ.join("hello.txt").exists());
    assert!(sh(&repo, &["branch", "--list", "codly/*"]).contains("/t1"), "the task branch is kept");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_mid_gate_stays_blocked_and_the_plan_still_cancels() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote it." })).await;
        })
    });
    let lead = planning_lead_with(&["hello.txt"], "sleep 1 && false");
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, plan) =
        wait_for("verification to start", 20, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Verifying)).await;
    let worker_id = task.assignee.clone().unwrap();
    core.stop_agent(&worker_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await; // the verify command fails meanwhile
    let (task, _) = task_status(&core).await.unwrap();
    assert_eq!(task.status, TaskStatus::Blocked, "an in-flight gate never restarts a task you stopped");
    assert!(task.status_reason.as_deref().is_some_and(|r| r.starts_with("stopped by you")), "{:?}", task.status_reason);
    assert_eq!(log.lock().unwrap().runs.iter().filter(|r| r.0 == worker_id).count(), 1, "the stopped worker is not started again");
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(e.kind, EventKind::Merged { .. })), "nothing merged");
    // Cancel plan is your move too: it takes the task you stopped with it.
    core.cancel_plan(&plan.id).await.unwrap();
    let (task, plan) = task_status(&core).await.unwrap();
    assert_eq!(plan.status, PlanStatus::Cancelled);
    assert_eq!(task.status, TaskStatus::Cancelled);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_cancelled_while_integrating_stays_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote it." })).await;
        })
    });
    let lead: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                return turn.say("Noted.");
            }
            let task = json!({ "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing hello",
                "consultant": { "title": "Backend" }, "acceptance_criteria": ["hello.txt contains hello"], "write_scope": ["hello.txt"],
                "depends_on": [], "verify": ["grep -q hello hello.txt"], "size": "M" });
            let plan = json!({ "summary": "Add a hello file", "tasks": [task], "integration_checks": ["sleep 2"] });
            let reply = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(reply["ok"], true, "{reply}");
        })
    });
    let core = start(dir.path(), FakeSpawner { log, lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (_, plan) =
        wait_for("the integration checks to start", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Integrating)).await;
    core.cancel_plan(&plan.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(3000)).await; // the integration check finishes meanwhile
    let snap = core.snapshot().await.unwrap();
    assert_eq!(snap.plans[0].status, PlanStatus::Cancelled, "a finished check never brings a cancelled plan back");
    assert!(!snap.chats.iter().any(|c| c.text.starts_with("Plan done")), "no \"Plan done\" for a cancelled plan");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_while_the_worker_starts_sends_no_brief() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("on it") }));
    let up = Arc::new(tokio::sync::Notify::new());
    let spawner = Slow(FakeSpawner { log: log.clone(), lead: planning_lead(), worker }, Role::Worker, up.clone());
    let core = start(dir.path(), spawner).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let worker_id = wait_for("the worker's process to start", 20, async || {
        let log = log.lock().unwrap();
        let at = log.spawns.iter().position(|s| s.0 == Role::Worker)?;
        Some(log.runs[at].0.clone())
    })
    .await;
    let (_, plan) = task_status(&core).await.unwrap();
    core.cancel_plan(&plan.id).await.unwrap();
    up.notify_one(); // its handshake finishes only now, after the cancel
    tokio::time::sleep(Duration::from_millis(1500)).await;
    {
        let log = log.lock().unwrap();
        assert!(!log.prompts.iter().any(|p| p.0 == Role::Worker), "no brief for a cancelled task: {:?}", log.prompts);
        assert!(log.shutdowns.contains(&worker_id), "the worker that came up is shut down");
    }
    let snap = core.snapshot().await.unwrap();
    assert_eq!(snap.tasks[0].status, TaskStatus::Cancelled);
    let ghosts = core.ghosts().unwrap();
    let agent = snap.agents.iter().chain(&ghosts).find(|a| a.id == worker_id).unwrap();
    assert_eq!((agent.run_id.as_deref(), agent.task_id.as_deref()), (None, None), "nothing holds the agent");
    let events = core.events(None, None, 2000).await.unwrap();
    let failed_start = |e: &EventRow| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("could not start"));
    assert!(!events.iter().any(failed_start), "a cancel is not a failed start");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quitting_while_a_worker_starts_leaves_its_task_to_resume() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("on it") }));
    let up = Arc::new(tokio::sync::Notify::new());
    let spawner = Slow(FakeSpawner { log: log.clone(), lead: planning_lead(), worker }, Role::Worker, up.clone());
    let core = start(dir.path(), spawner).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    wait_for("the worker's process to start", 20, async || log.lock().unwrap().spawns.iter().any(|s| s.0 == Role::Worker).then_some(())).await;
    // Quit; its handshake finishes right after, into a run that takes no brief any more.
    tokio::join!(core.shutdown(), async { up.notify_one() });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (task, _) = task_status(&core).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running, "the next start resumes it: {:?}", task.status_reason);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_running_task_can_be_stopped_from_the_board() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    // Each turn ends when the test lets it: the first one only after the stop.
    let go = Arc::new(tokio::sync::Notify::new());
    let wait = go.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move {
            wait.notified().await;
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote it." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the worker's turn", 20, async || {
        let working = log.lock().unwrap().prompts.iter().any(|p| p.0 == Role::Worker);
        task_status(&core).await.filter(|(t, _)| working && t.status == TaskStatus::Running)
    })
    .await;
    let worker_id = task.assignee.clone().unwrap();
    let stopped = core.move_task(&task.id, TaskStatus::Blocked).await.unwrap();
    assert_eq!(stopped.status_reason.as_deref(), Some("stopped by you; Retry to resume"));
    go.notify_one();
    wait_for("the worker to exit", 10, async || log.lock().unwrap().shutdowns.contains(&worker_id).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(500)).await; // what its turn end set off has run its course
    let (task, _) = task_status(&core).await.unwrap();
    assert_eq!(task.status, TaskStatus::Blocked, "the turn that ended after the stop is not gated");
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(e.kind, EventKind::Verified { .. })), "nothing was checked");
    // Retry: nobody holds the task or its worker, so it runs to the end.
    go.notify_one();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)).await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gate_failure_frees_the_worker_so_retry_works() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    // The first time, the worktree breaks under the gate: its `.git` points nowhere.
    let broken = Arc::new(Mutex::new(None::<(PathBuf, String)>));
    let saved = broken.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let saved = saved.clone();
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            {
                let mut saved = saved.lock().unwrap();
                if saved.is_none() {
                    let dot_git = turn.spec.cwd.join(".git");
                    *saved = Some((dot_git.clone(), std::fs::read_to_string(&dot_git).unwrap()));
                    std::fs::write(&dot_git, "gitdir: /nonexistent\n").unwrap();
                }
            }
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote it." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the gate to fail", 30, async || {
        task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Blocked && t.status_reason.as_deref().is_some_and(|r| r.contains("the gate failed")))
    })
    .await;
    let worker_id = task.assignee.clone().unwrap();
    wait_for("the worker to be released", 5, async || log.lock().unwrap().shutdowns.contains(&worker_id).then_some(())).await;
    // You repair the worktree and retry: the same worker picks its session up again.
    let (dot_git, content) = broken.lock().unwrap().clone().unwrap();
    std::fs::write(dot_git, content).unwrap();
    core.move_task(&task.id, TaskStatus::Ready).await.unwrap();
    let (task, _) = wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)).await;
    assert_eq!(task.status, TaskStatus::Done);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_stopped_while_its_worktree_is_added_leaves_the_agents_next_chat_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    // A post-checkout hook (git-lfs, husky) makes every new worktree take a second to add.
    let hooks = dir.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 1\n").unwrap();
    std::fs::set_permissions(hooks.join("post-checkout"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    sh(&repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("here") }));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let worker_id = wait_for("the task to be claimed", 20, async || {
        let snap = core.snapshot().await.unwrap();
        let worker = snap.tasks.first().filter(|t| t.status == TaskStatus::Running)?.assignee.clone()?;
        snap.agents.iter().any(|a| a.id == worker && a.task_id.is_some()).then_some(worker)
    })
    .await;
    // Its worktree is still being added: stop it, then talk to it.
    core.stop_agent(&worker_id).await.unwrap();
    core.send_chat(&format!("agent:{worker_id}"), "are you there?").await.unwrap();
    tokio::time::sleep(Duration::from_millis(2000)).await; // the stopped start runs its course meanwhile
    {
        let log = log.lock().unwrap();
        assert!(log.turns.iter().any(|t| t.0 == worker_id && t.1.ends_with("are you there?")), "{:?}", log.turns);
        assert_eq!(log.runs.iter().filter(|r| r.0 == worker_id).count(), 1, "the stopped task starts no worker; the chat is its only run");
        assert!(!log.shutdowns.contains(&worker_id), "the chat is not the stopped start's to shut down");
    }
    let snap = core.snapshot().await.unwrap();
    assert_eq!(snap.tasks[0].status, TaskStatus::Blocked);
    assert!(snap.agents.iter().any(|a| a.id == worker_id && a.run_id.is_some()), "the chat is still running");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_plan_while_a_worktree_is_added_leaves_the_agents_chat_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    // A post-checkout hook (git-lfs, husky) makes every new worktree take two seconds to add.
    let hooks = dir.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 2\n").unwrap();
    std::fs::set_permissions(hooks.join("post-checkout"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    sh(&repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("here") }));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let worker_id = wait_for("the task to be claimed", 20, async || {
        let snap = core.snapshot().await.unwrap();
        let worker = snap.tasks.first().filter(|t| t.status == TaskStatus::Running)?.assignee.clone()?;
        snap.agents.iter().any(|a| a.id == worker && a.task_id.is_some()).then_some(worker)
    })
    .await;
    // Its worktree is still being added: you talk to it, then cancel the plan.
    core.send_chat(&format!("agent:{worker_id}"), "are you there?").await.unwrap();
    let (_, plan) = task_status(&core).await.unwrap();
    core.cancel_plan(&plan.id).await.unwrap();
    // The cancelled start runs its course: it notes the worktree once it is added, then stops short.
    wait_for("the worktree to be added", 20, async || task_status(&core).await.filter(|(t, _)| t.worktree.is_some())).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    {
        let log = log.lock().unwrap();
        assert!(log.turns.iter().any(|t| t.0 == worker_id && t.1.ends_with("are you there?")), "{:?}", log.turns);
        assert_eq!(log.runs.iter().filter(|r| r.0 == worker_id).count(), 1, "the cancelled task starts no worker; the chat is its only run");
        assert!(!log.shutdowns.contains(&worker_id), "the chat is not the cancelled task's to shut down");
    }
    let snap = core.snapshot().await.unwrap();
    assert_eq!(snap.tasks[0].status, TaskStatus::Cancelled);
    assert!(snap.agents.iter().any(|a| a.id == worker_id && a.run_id.is_some()), "the chat is still running");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_for_a_worker_that_is_shutting_down_restarts_its_task_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if !turn.prompt.contains("Use Stripe") {
                turn.tool("codly_ask_user", json!({ "title": "Which provider?", "question": "Stripe or Adyen?" })).await;
            }
        })
    });
    let spawner = SlowStop(FakeSpawner { log, lead: planning_lead(), worker }, Duration::from_millis(500));
    let core = start(dir.path(), spawner).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the worker's question", 20, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Blocked)).await;
    let card = core.list_blockers().await.unwrap().into_iter().next().unwrap();
    core.stop_agent(task.assignee.as_deref().unwrap()).await.unwrap();
    // Its process is still shutting down and takes no more prompts; a card is answered only once.
    core.answer_blocker(&card.id, "Use Stripe").await.unwrap();
    let (task, _) = task_status(&core).await.unwrap();
    assert!(task.goal.contains("Use Stripe"), "the answer goes into the task for its next run: {}", task.goal);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_events_of_a_failed_start_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let core = start(dir.path(), LateStart).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let e = core.send_chat(&format!("agent:{}", solo.id), "hi").await.unwrap_err().to_string();
    assert!(e.contains("did not finish the handshake"), "{e}");
    // The CLI Codly gave up on comes up after all, then exits: on record, and nothing else.
    let run_id = wait_for("the late exit to be recorded", 10, async || {
        let events = core.events(Some(&solo.id), None, 100).await.unwrap();
        events.into_iter().find(|e| matches!(e.kind, EventKind::Exited { .. }))?.run_id
    })
    .await;
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    assert_eq!(store.run(&run_id).unwrap().unwrap().status, "failed", "a late exit does not turn a failed start into a completed run");
    let agent = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == solo.id).unwrap();
    assert_ne!(agent.session_id.as_deref(), Some("stale"), "the session of a start that failed is not the agent's");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejected_event_is_not_broadcast() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    // The CLI closes a request Codly has no record of: the store refuses that row.
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.emit(EventKind::ApprovalResolved { approval_id: "nope".into(), status: ApprovalStatus::Cancelled, by: "provider".into() });
            turn.say("done");
        })
    });
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle, worker }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let solo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let mut bus = core.subscribe();
    core.send_chat(&format!("agent:{}", solo.id), "hi").await.unwrap();
    // Everything the UI is told until that turn ends.
    let mut shown = vec![];
    loop {
        let e = tokio::time::timeout(Duration::from_secs(20), bus.recv()).await.expect("the turn ends").unwrap();
        match e.kind {
            EventKind::ApprovalResolved { .. } => shown.push(e.id),
            EventKind::TurnCompleted { .. } => break,
            _ => {}
        }
    }
    assert!(shown.is_empty(), "a row that is not on record was shown, with id {shown:?}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_steer_during_the_gate_is_committed_before_merging() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.starts_with("[User]") {
                turn.write("bye.txt", "bye\n");
                turn.say("Added bye.txt too.");
            } else {
                turn.write("hello.txt", "hello\n");
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote hello.txt." })).await;
            }
        })
    });
    let lead = planning_lead_with(&["*.txt"], "sleep 1 && test -f hello.txt");
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) =
        wait_for("verification to start", 20, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Verifying)).await;
    let worker_id = task.assignee.clone().unwrap();
    core.send_chat(&format!("agent:{worker_id}"), "also add bye.txt").await.unwrap();
    let (task, plan) =
        wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)).await;
    assert_eq!(task.status, TaskStatus::Done);
    let integ = PathBuf::from(plan.integration_worktree.unwrap());
    assert!(integ.join("hello.txt").exists());
    assert!(integ.join("bye.txt").exists(), "the steered turn's work was committed and merged");
    let snap = core.snapshot().await.unwrap();
    assert!(snap.chats.iter().any(|c| c.thread == format!("agent:{worker_id}") && c.author == "user" && c.text == "also add bye.txt"));
    assert!(snap.chats.iter().any(|c| c.thread == format!("agent:{worker_id}") && c.text == "Added bye.txt too."));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_vendor_review_requests_changes_then_approves() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            let text = if turn.prompt.contains("The reviewer requested changes") { "hello world\n" } else { "hello\n" };
            turn.write("hello.txt", text);
            turn.tool(
                "codly_submit_report",
                json!({ "status": "done", "summary": "Greeting written.", "notes_for_reviewer": "check the wording" }),
            )
            .await;
        })
    });
    let reviews = Arc::new(Mutex::new(0));
    let count = reviews.clone();
    let reviewer: Behavior = Arc::new(move |turn: FakeTurn| {
        let count = count.clone();
        Box::pin(async move {
            assert!(turn.prompt.contains("[Codly] Review task t1") && turn.prompt.contains("+hello"), "{}", turn.prompt);
            assert!(turn.spec.read_only, "reviewers are read-only");
            let n = {
                let mut c = count.lock().unwrap();
                *c += 1;
                *c
            };
            let review = if n == 1 {
                // "approve" with a major issue is downgraded by Codly to request_changes.
                json!({ "verdict": "approve", "summary": "Say hello world.", "criteria": [{ "criterion": "hello.txt contains hello", "met": true }],
                        "issues": [{ "severity": "major", "file": "hello.txt", "line": 1, "description": "should greet the world" }] })
            } else {
                json!({ "verdict": "approve", "summary": "Looks good.", "criteria": [{ "criterion": "hello.txt contains hello", "met": true }], "issues": [] })
            };
            let r = turn.tool("codly_submit_review", review).await;
            assert_eq!(r["ok"], true, "{r}");
        })
    });
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let spawner = FakeSpawner { log: log.clone(), lead: planning_lead(), worker: by_role(worker, reviewer) };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings = Settings {
        auto_run_plans: true,
        lead_followups: false,
        review_gate: true,
        worker_providers: vec![Provider::Claude],
        ..Default::default()
    };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, plan) = wait_for("the plan to finish", 30, async || {
        task_status(&core).await.filter(|(_, p)| matches!(p.status, PlanStatus::Done | PlanStatus::Failed))
    })
    .await;
    assert_eq!(plan.status, PlanStatus::Done);
    assert_eq!(task.status, TaskStatus::Done);
    let integ = PathBuf::from(plan.integration_worktree.unwrap());
    assert_eq!(std::fs::read_to_string(integ.join("hello.txt")).unwrap(), "hello world\n");

    let events = core.events(None, None, 3000).await.unwrap();
    let verdicts: Vec<String> = events
        .iter()
        .rev()
        .filter_map(|e| match &e.kind {
            EventKind::Reviewed { verdict, .. } => Some(verdict.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts, vec!["request_changes", "approve"]);
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("downgraded from approve"))));
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::TaskChanged { task } if task.status == TaskStatus::ChangesRequested)));
    let snap = core.snapshot().await.unwrap();
    let reviewers: Vec<&Agent> = snap.agents.iter().filter(|a| a.role == Role::Reviewer).collect();
    assert_eq!(reviewers.len(), 1, "the idle reviewer is reused for the second review");
    assert_eq!(reviewers[0].provider, Provider::Codex, "a different vendor than the Claude worker");
    let fix = log.lock().unwrap().prompts.iter().find(|p| p.0 == Role::Worker && p.1.contains("requested changes")).unwrap().1.clone();
    assert!(fix.contains("[major] hello.txt:1 should greet the world"), "{fix}");
    assert!(fix.contains("<untrusted from=\"reviewer\">"), "reviewer text is quoted as data");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reviewer_that_got_busy_after_it_was_picked_is_replaced_not_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Greeting written." })).await;
        })
    });
    let reviewer: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if !turn.prompt.contains("[Codly] Review task") {
                return turn.say("hi");
            }
            let criteria = json!([{ "criterion": "hello.txt contains hello", "met": true }]);
            turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] })).await;
        })
    });
    let (died, fail) = (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
    let spawner = FakeSpawner { log, lead: planning_lead(), worker: by_role(worker, reviewer) };
    let spawner = ReviewerDiesOnce { spawner, died: died.clone(), fail: fail.clone(), done: AtomicBool::new(false) };
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: false, review_gate: true, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    // The picked reviewer's start is failing. Meanwhile you talk to that reviewer: it is running
    // again, for you, when the review's start gives up on it.
    tokio::time::timeout(Duration::from_secs(20), died.notified()).await.expect("the review starts");
    let picked = wait_for("the reviewer's dead start to be over", 10, async || {
        core.snapshot().await.unwrap().agents.into_iter().find(|a| a.role == Role::Reviewer && a.run_id.is_none())
    })
    .await;
    core.send_chat(&format!("agent:{}", picked.id), "hello").await.unwrap();
    fail.notify_one();
    let (task, _) =
        wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| matches!(p.status, PlanStatus::Done | PlanStatus::Failed))).await;
    assert_eq!(task.status, TaskStatus::Done);
    let events = core.events(None, None, 3000).await.unwrap();
    let skipped = |e: &EventRow| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("review skipped"));
    assert!(!events.iter().any(skipped), "a reviewer that is busy elsewhere is no reason to merge unreviewed");
    let reviewed = events.iter().find(|e| matches!(&e.kind, EventKind::Reviewed { verdict, .. } if verdict == "approve")).expect("someone reviewed it");
    assert_ne!(reviewed.agent_id.as_deref(), Some(picked.id.as_str()), "by another reviewer");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_reviewer_blocks_the_task_and_frees_its_worker() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Greeting written." })).await;
        })
    });
    // Mid-review when you stop it: its turn ends only then, without a verdict (a stopped CLI
    // still closes its turn with a TurnCompleted before it exits).
    let stopped = Arc::new(tokio::sync::Notify::new());
    let wait = stopped.clone();
    let reviewer: Behavior = Arc::new(move |_: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move { wait.notified().await })
    });
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let spawner = FakeSpawner { log: log.clone(), lead: planning_lead(), worker: by_role(worker, reviewer) };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: false, review_gate: true, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, reviewer_id) = wait_for("the review", 30, async || {
        let reading = log.lock().unwrap().prompts.iter().any(|p| p.0 == Role::Reviewer);
        let snap = core.snapshot().await.unwrap();
        let reviewer = snap.agents.iter().find(|a| reading && a.role == Role::Reviewer)?.id.clone();
        Some((snap.tasks.first().filter(|t| t.status == TaskStatus::Reviewing)?.clone(), reviewer))
    })
    .await;
    let worker_id = task.assignee.clone().unwrap();
    core.stop_agent(&reviewer_id).await.unwrap();
    stopped.notify_one();
    wait_for("the reviewer to exit", 10, async || log.lock().unwrap().shutdowns.contains(&reviewer_id).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(500)).await; // what its turn end set off has run its course
    let (task, _) = task_status(&core).await.unwrap();
    assert_eq!(task.status, TaskStatus::Blocked, "{:?}", task.status_reason);
    assert!(task.status_reason.as_deref().is_some_and(|r| r.starts_with("review stopped by you")), "{:?}", task.status_reason);
    assert!(log.lock().unwrap().shutdowns.contains(&worker_id), "its worker is released too: a Retry can start it");
    let events = core.events(None, None, 3000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(e.kind, EventKind::Merged { .. })), "a review you stopped merges nothing");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_review_that_ends_twice_merges_once() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Greeting written." })).await;
        })
    });
    // Its CLI exits by itself as its turn ends: the exit and the turn end each say the review is over.
    let reviewer: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            let criteria = json!([{ "criterion": "hello.txt contains hello", "met": true }]);
            turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] })).await;
            turn.emit(EventKind::Exited { code: Some(0) });
        })
    });
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let spawner = FakeSpawner { log, lead: planning_lead(), worker: by_role(worker, reviewer) };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: false, review_gate: true, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)).await;
    assert_eq!(task.status, TaskStatus::Done);
    tokio::time::sleep(Duration::from_millis(500)).await; // the review's second end has run its course
    let events = core.events(None, None, 3000).await.unwrap();
    let count = |f: fn(&EventKind) -> bool| events.iter().filter(|e| f(&e.kind)).count();
    assert_eq!(count(|k| matches!(k, EventKind::Reviewed { .. })), 1, "one review, read once");
    assert_eq!(count(|k| matches!(k, EventKind::Merged { .. })), 1, "merged once");
    let merged = |k: &EventKind| matches!(k, EventKind::Decision { summary, .. } if summary.contains("merged at"));
    assert_eq!(count(merged), 1, "and recorded once");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_steer_during_review_is_gated_before_merging() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.starts_with("[User]") {
                turn.write("bye.txt", "bye\n");
                turn.say("Added bye.txt too.");
            } else {
                turn.write("hello.txt", "hello\n");
                turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote hello.txt." })).await;
            }
        })
    });
    // The first review (a diff without bye.txt) ends only when the test says so: after your steer.
    let reviewed = Arc::new(tokio::sync::Notify::new());
    let wait = reviewed.clone();
    let reviewer: Behavior = Arc::new(move |turn: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move {
            if !turn.prompt.contains("bye.txt") {
                wait.notified().await;
            }
            let criteria = json!([{ "criterion": "hello.txt contains hello", "met": true }]);
            turn.tool("codly_submit_review", json!({ "verdict": "approve", "summary": "Looks good.", "criteria": criteria, "issues": [] })).await;
        })
    });
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let lead = planning_lead_with(&["*.txt"], "test -f hello.txt");
    let spawner = FakeSpawner { log: log.clone(), lead, worker: by_role(worker, reviewer) };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: false, review_gate: true, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the review", 30, async || {
        let reading = log.lock().unwrap().prompts.iter().any(|p| p.0 == Role::Reviewer);
        task_status(&core).await.filter(|(t, _)| reading && t.status == TaskStatus::Reviewing)
    })
    .await;
    // You steer the worker while its work is read; its turn is over before the review is.
    let worker_id = task.assignee.clone().unwrap();
    core.send_chat(&format!("agent:{worker_id}"), "also add bye.txt").await.unwrap();
    wait_chat(&core, &format!("agent:{worker_id}"), "Added bye.txt too.").await;
    reviewed.notify_one();
    let (task, plan) =
        wait_for("the plan to finish", 30, async || task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)).await;
    assert_eq!(task.status, TaskStatus::Done);
    let integ = PathBuf::from(plan.integration_worktree.unwrap());
    assert!(integ.join("hello.txt").exists());
    assert!(integ.join("bye.txt").exists(), "the steered turn's work was committed, reviewed and merged");
    let reviews = log.lock().unwrap().prompts.iter().filter(|p| p.0 == Role::Reviewer).count();
    assert_eq!(reviews, 2, "the work the first review never saw got its own review");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_at_its_usage_limit_hands_the_task_to_the_next_provider() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.spec.provider == Provider::Claude {
                // Half the work, then the usage limit (no RateLimit event, like Codex errors).
                turn.write("hello.txt", "hel");
                turn.emit(EventKind::StatusChanged { status: AgentStatus::RateLimited });
                return;
            }
            assert!(turn.prompt.contains("Claude Code hit its usage limit"), "{}", turn.prompt);
            assert_eq!(std::fs::read_to_string(turn.spec.cwd.join("hello.txt")).unwrap(), "hel", "same worktree");
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Finished the greeting." })).await;
        })
    });
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let core = start_with_clis(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }, vec![claude(), codex]).await;
    let settings = Settings {
        auto_run_plans: true,
        lead_followups: false,
        worker_providers: vec![Provider::Claude, Provider::Codex],
        ..Default::default()
    };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, plan) = wait_for("the plan to finish", 30, async || {
        task_status(&core).await.filter(|(_, p)| matches!(p.status, PlanStatus::Done | PlanStatus::Failed))
    })
    .await;
    assert_eq!(plan.status, PlanStatus::Done);
    assert_eq!(task.status, TaskStatus::Done);

    // Finished workers leave the roster, so read who ran from the events.
    let events = core.events(None, None, 3000).await.unwrap();
    let workers: Vec<Provider> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::AgentChanged { agent } if agent.role == Role::Worker => Some(agent.provider),
            _ => None,
        })
        .collect();
    assert!(workers.contains(&Provider::Claude) && workers.contains(&Provider::Codex), "{workers:?}");
    let verifies: Vec<bool> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Verified { ok, .. } => Some(*ok),
            _ => None,
        })
        .collect();
    assert_eq!(verifies, vec![true], "no gate on the limited turn");
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
        if decision_type == "retry" && summary.contains("Claude Code hit its usage limit"))));
    assert!(!events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("fix round"))));
    core.shutdown().await;
}

/// A team member's half-done work is in its own worktree, on the task's branch: nobody else
/// can take that branch over, so the task waits for the member's quota.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_at_its_limit_keeps_its_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.ends_with("the quota is back") {
                // A fresh reading for the account: under its limit again.
                return turn.emit(EventKind::RateLimit { window: "limit".into(), used_fraction: Some(0.1), resets_at: None, status: Some("allowed".into()) });
            }
            if turn.prompt.contains("[Codly]") {
                return;
            }
            let plan = json!({ "summary": "Add a hello file", "tasks": [{
                "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing hello", "assignee": "Ana",
                "acceptance_criteria": ["hello.txt contains hello"], "write_scope": ["hello.txt"], "depends_on": [], "verify": ["grep -q hello hello.txt"],
            }] });
            let r = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(r["ok"], true, "{r}");
        })
    });
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if !turn.prompt.contains("hit its usage limit") {
                // Half the work, then the usage limit (no RateLimit event, like Codex errors).
                turn.write("hello.txt", "hel");
                return turn.emit(EventKind::StatusChanged { status: AgentStatus::RateLimited });
            }
            assert_eq!(std::fs::read_to_string(turn.spec.cwd.join("hello.txt")).unwrap(), "hel", "same worktree");
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Finished the greeting." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    let ana = TeamMember { name: "Ana".into(), role: "Backend dev".into(), ..Default::default() };
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, team: vec![ana], ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the task to wait for quota", 30, async || {
        task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Ready && t.status_reason.as_deref().is_some_and(|r| r.contains("parked")))
    })
    .await;
    let ana = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.member.is_some()).expect("Ana is on the board");
    assert_eq!(task.assignee.as_deref(), Some(ana.id.as_str()), "the task stays with the member: {:?}", task.status_reason);
    core.send_chat("lead", "the quota is back").await.unwrap();
    let (task, _) = wait_for("the task to end", 30, async || {
        task_status(&core).await.filter(|(t, _)| matches!(t.status, TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Failed))
    })
    .await;
    assert_eq!(task.status, TaskStatus::Done, "{:?}", task.status_reason);
    let runs: Vec<SessionMode> = log.lock().unwrap().runs.iter().filter(|r| r.0 == ana.id).map(|r| r.1.clone()).collect();
    let SessionMode::New { session_id: Some(first) } = &runs[0] else { panic!("{runs:?}") };
    assert!(runs.len() == 2 && matches!(&runs[1], SessionMode::Resume { session_id } if session_id == first), "it continues its session: {runs:?}");
    let events = core.events(None, None, 3000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("could not start the worker"))));
    core.shutdown().await;
}

/// The Tauri shell awaits these inside `#[tauri::command] async fn`s, which need `Send`
/// futures: this only compiles if every Core future is `Send`.
#[allow(dead_code, unused_must_use, clippy::let_underscore_future)]
fn core_futures_are_send(core: Arc<Core>, d: ApprovalDecision, s: Settings) {
    fn send<T: Send>(_: T) {}
    send(Core::start(CoreConfig { data_dir: PathBuf::new(), app_exe: PathBuf::new() }));
    send(core.snapshot());
    send(core.redetect_clis());
    send(core.accounts(true));
    send(core.recent_workspaces());
    send(core.open_workspace("x"));
    send(core.update_settings(s));
    send(core.send_chat("lead", "x"));
    send(core.run_plan("p"));
    send(core.cancel_plan("p"));
    send(core.resolve_approval("a", d));
    send(core.interrupt_agent("a"));
    send(core.stop_agent("a"));
    send(core.hire_agent(Provider::Claude, None, None));
    send(core.set_workspace_team(WorkspaceTeam::default()));
    send(core.create_task("t", "g"));
    send(core.move_task("t", TaskStatus::Ready));
    send(core.assign_task("t", None));
    send(core.task_diff("t"));
    send(core.plan_diff("p"));
    send(core.merge_plan("p"));
    send(core.events(None, None, 10));
    send(core.search("q"));
    send(core.memories(None, None));
    send(core.add_memory("m", MemoryScope::Project, MemoryKind::Fact, MemoryMeta::default()));
    send(core.delete_memory("m"));
    send(core.triples(None));
    send(core.shutdown());
    send(core.subscribe());
    fn shared<T: Send + Sync + 'static>(_: &T) {}
    shared(&*core); // Tauri managed state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_worth_viewing_become_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("chart.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"4\" height=\"4\"/></svg>\n");
            turn.write("notes.txt", "not an artifact\n");
            turn.write("hello.txt", "hello\n"); // what the planned task's verify checks
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Drew a chart." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log, lead: planning_lead_with(&["**"], "grep -q hello hello.txt"), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() })
        .await
        .unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let snap = wait_for("the plan to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(s)
    })
    .await;

    let svgs: Vec<&Artifact> = snap.artifacts.iter().filter(|a| a.title == "chart.svg").collect();
    assert_eq!(svgs.len(), 1, "announced once (the commit of the same bytes is not a new artifact): {:?}", snap.artifacts);
    let a = svgs[0];
    assert_eq!((a.kind, a.path.as_str(), a.mime.as_str()), (ArtifactKind::Svg, "chart.svg", "image/svg+xml"));
    assert!(a.task_id.is_some() && a.agent_id.is_some());
    assert!(snap.artifacts.iter().all(|a| a.title != "notes.txt"));
    let (bytes, mime) = core.read_artifact(&a.id).await.unwrap();
    assert!(bytes.starts_with(b"<svg") && mime == "image/svg+xml");
    assert!(core.read_artifact("not-an-artifact").await.is_err(), "only recorded artifacts can be read");
}

// ───────────── parallel request threads ─────────────

/// A Lead that plans "Add a <file> file" for a request ending in "<file> file".
fn request_lead() -> Behavior {
    Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                turn.say("Noted.");
                return;
            }
            let file = if turn.prompt.ends_with("bye file") { "bye" } else { "hello" };
            let plan = json!({
                "summary": format!("Add a {file} file"),
                "tasks": [{
                    "id": "t1", "title": format!("Create {file}.txt"), "goal": format!("Create {file}.txt containing {file}"),
                    "consultant": { "title": "Backend" }, "acceptance_criteria": [format!("{file}.txt contains {file}")], "write_scope": [format!("{file}.txt")],
                    "depends_on": [], "verify": [format!("grep -q {file} {file}.txt")], "size": "M",
                }],
            });
            let reply = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(reply["ok"], true, "{reply}");
            turn.say(&format!("Planned {file}.txt."));
        })
    })
}

/// A Lead that answers "ok: <last line>"; a turn whose text starts with "hold" waits while
/// `hold` is set (≤ 20 s).
fn echo_lead(hold: Arc<AtomicBool>) -> Behavior {
    Arc::new(move |turn: FakeTurn| {
        let hold = hold.clone();
        Box::pin(async move {
            let text = turn.prompt.lines().last().unwrap_or_default().to_string();
            if text.starts_with("hold") {
                let until = Instant::now() + Duration::from_secs(20);
                while hold.load(Ordering::SeqCst) && Instant::now() < until {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            turn.say(&format!("ok: {text}"));
        })
    })
}

fn chats<'a>(s: &'a Snapshot, thread: &str) -> Vec<&'a ChatMessage> {
    s.chats.iter().filter(|c| c.thread == thread).collect()
}

/// Every agent the snapshot knows of, let-go consultants too (their last AgentChanged).
fn agents_ever(s: &Snapshot) -> Vec<Agent> {
    let mut out: Vec<Agent> = s.agents.clone();
    for e in s.events.iter().rev() {
        if let EventKind::AgentChanged { agent } = &e.kind
            && !out.iter().any(|a| a.id == agent.id)
        {
            out.push(agent.clone());
        }
    }
    out
}

fn lead_of<'a>(s: &'a Snapshot, thread: &str) -> Option<&'a Agent> {
    s.agents.iter().find(|a| a.role == Role::Lead && a.thread.as_deref() == Some(thread))
}

fn turns_of(log: &Mutex<Log>, agent: &str) -> Vec<String> {
    log.lock().unwrap().turns.iter().filter(|(a, _)| a == agent).map(|(_, t)| t.clone()).collect()
}

async fn wait_chat(core: &Core, thread: &str, text: &str) -> Snapshot {
    wait_for(&format!("\"{text}\" in {thread}"), 20, async || {
        let s = core.snapshot().await.unwrap();
        chats(&s, thread).iter().any(|c| c.text == text).then(|| s.clone())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_a_request_cancels_its_plan_and_frees_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let idle: Behavior = Arc::new(|_turn: FakeTurn| Box::pin(async {}));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: request_lead(), worker: idle }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead:b", "Add a bye file").await.unwrap();
    let s = wait_chat(&core, "lead:b", "Planned bye.txt.").await;
    let plan = s.plans.iter().find(|p| p.thread == "lead:b").unwrap().clone();
    let old = lead_of(&s, "lead:b").unwrap().clone();

    core.close_thread("lead:b").await.unwrap();
    let s = core.snapshot().await.unwrap();
    assert!(chats(&s, "lead:b").is_empty(), "its messages left the thread");
    assert!(s.chats.iter().all(|c| !c.thread.starts_with("closed:")), "closed threads stay out of the snapshot");
    let p = s.plans.iter().find(|p| p.id == plan.id).unwrap();
    assert_eq!(p.status, PlanStatus::Cancelled);
    assert!(p.thread.starts_with("closed:lead:b:"), "{}", p.thread);
    assert!(s.agents.iter().all(|a| a.id != old.id), "its Lead is archived");
    assert!(log.lock().unwrap().shutdowns.contains(&old.id), "and stopped");

    // The thread id works again, with a fresh Lead in a new session.
    core.send_chat("lead:b", "Add a bye file").await.unwrap();
    let s = wait_chat(&core, "lead:b", "Planned bye.txt.").await;
    let new = lead_of(&s, "lead:b").unwrap();
    assert_ne!(new.id, old.id);
    assert!(log.lock().unwrap().runs.iter().any(|(a, m)| a == &new.id && matches!(m, SessionMode::New { .. })));
    assert!(core.close_thread("agent:x").await.is_err(), "only requests close");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_lead_model_restarts_an_idle_lead_on_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let hold = Arc::new(AtomicBool::new(false));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: echo_lead(hold.clone()), worker: echo_lead(hold) }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "one").await.unwrap();
    wait_chat(&core, "lead", "ok: one").await;

    let settings = Settings { lead_model: Some("opus:high".into()), ..core.snapshot().await.unwrap().settings };
    core.update_settings(settings).await.unwrap();
    core.send_chat("lead", "two").await.unwrap();
    wait_chat(&core, "lead", "ok: two").await;
    core.send_chat("lead", "three").await.unwrap(); // unchanged: no restart
    let s = wait_chat(&core, "lead", "ok: three").await;

    let lead = lead_of(&s, "lead").unwrap();
    let l = log.lock().unwrap();
    let models: Vec<Option<String>> = l.models.iter().filter(|m| m.0 == lead.id).map(|m| m.1.clone()).collect();
    assert_eq!(models, [None, Some("opus:high".to_string())], "restarted once, on the new model");
    let runs: Vec<&SessionMode> = l.runs.iter().filter(|r| r.0 == lead.id).map(|r| &r.1).collect();
    assert!(matches!(runs[1], SessionMode::Resume { .. }), "same conversation: {runs:?}");
    drop(l);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_requests_plan_and_run_side_by_side() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let started = Arc::new(AtomicUsize::new(0));
    let overlapped = Arc::new(AtomicBool::new(false));
    let (s2, o2) = (started.clone(), overlapped.clone());
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let (started, overlapped) = (s2.clone(), o2.clone());
        Box::pin(async move {
            let task = turn.tool("codly_get_task", json!({})).await;
            let file = if task["result"]["title"].as_str().unwrap().contains("bye") { "bye" } else { "hello" };
            // Each plan's worker waits (≤ 10 s) until the other plan's worker has started too.
            started.fetch_add(1, Ordering::SeqCst);
            let until = Instant::now() + Duration::from_secs(10);
            while started.load(Ordering::SeqCst) < 2 && Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            if started.load(Ordering::SeqCst) >= 2 {
                overlapped.store(true, Ordering::SeqCst);
            }
            turn.write(&format!("{file}.txt"), &format!("{file}\n"));
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": format!("Created {file}.txt.") })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: request_lead(), worker }).await;
    let settings =
        Settings { auto_run_plans: true, lead_followups: true, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    core.send_chat("lead", "Please add a hello file").await.unwrap();
    wait_for("the first request's plan", 20, async || {
        core.snapshot().await.unwrap().plans.iter().any(|p| p.thread == "lead" && p.status == PlanStatus::Running).then_some(())
    })
    .await;
    // A second request while the first plan runs: its own Lead, its own plan.
    core.send_chat("lead:2", "Please add a bye file").await.unwrap();
    let snap = wait_for("both plans and their follow-ups", 30, async || {
        let s = core.snapshot().await.unwrap();
        let done = s.plans.len() == 2 && s.plans.iter().all(|p| p.status == PlanStatus::Done);
        let followed = s.agents.iter().filter(|a| a.role == Role::Lead).all(|l| turns_of(&log, &l.id).iter().any(|t| t.contains("[Codly] Plan done")));
        (done && followed).then_some(s)
    })
    .await;
    assert!(overlapped.load(Ordering::SeqCst), "the two plans' workers ran at the same time");

    let lead1 = lead_of(&snap, "lead").expect("the first request's Lead");
    let lead2 = lead_of(&snap, "lead:2").expect("the second request's Lead");
    assert_eq!((lead1.name.as_str(), lead2.name.as_str()), ("Lead", "Lead 2"));
    assert_eq!(snap.agents.iter().filter(|a| a.role == Role::Lead).count(), 2);
    let hello = snap.plans.iter().find(|p| p.summary == "Add a hello file").unwrap();
    let bye = snap.plans.iter().find(|p| p.summary == "Add a bye file").unwrap();
    assert_eq!((hello.thread.as_str(), bye.thread.as_str()), ("lead", "lead:2"));
    assert_eq!((hello.request.as_str(), bye.request.as_str()), ("Please add a hello file", "Please add a bye file"));
    let integ = |p: &Plan| PathBuf::from(p.integration_worktree.clone().unwrap());
    assert!(integ(hello).join("hello.txt").exists() && !integ(hello).join("bye.txt").exists());
    assert!(integ(bye).join("bye.txt").exists() && !integ(bye).join("hello.txt").exists());

    // Each thread has its own conversation: the user's words, its Lead's reply, its plan's cards.
    for (thread, lead, plan, file) in [("lead", lead1, hello, "hello"), ("lead:2", lead2, bye, "bye")] {
        let thread_chats = chats(&snap, thread);
        assert_eq!(thread_chats[0].text, format!("Please add a {file} file"));
        assert!(thread_chats.iter().any(|c| c.author == lead.id && c.text == format!("Planned {file}.txt.")), "{thread_chats:?}");
        for card in ["Plan ready", "Plan done"] {
            let found: Vec<_> = thread_chats.iter().filter(|c| c.text.starts_with(card)).collect();
            assert_eq!(found.len(), 1, "{card} in {thread}: {thread_chats:?}");
            assert_eq!(found[0].attachment.as_ref().unwrap()["plan_id"], plan.id.as_str());
        }
        let plan_cards = thread_chats.iter().filter_map(|c| c.attachment.as_ref()?.get("plan_id")?.as_str()).collect::<Vec<_>>();
        assert!(plan_cards.iter().all(|id| *id == plan.id), "only its own plan's cards in {thread}");
        // The plan-done follow-up went to the plan's own Lead only.
        let turns = turns_of(&log, &lead.id);
        assert!(turns.iter().any(|t| t.contains(&format!("Plan done: \"Add a {file} file\""))), "{turns:?}");
        let other = if file == "hello" { "bye" } else { "hello" };
        assert!(!turns.iter().any(|t| t.contains(&format!("Plan done: \"Add a {other} file\""))), "{turns:?}");
    }
    // The second Lead saw the first request's running plan as someone else's, with its files.
    let first = &turns_of(&log, &lead2.id)[0];
    assert!(
        first.contains("Plan <untrusted>Add a hello file</untrusted> (another request, Lead) — running")
            && first.contains("writes <untrusted>hello.txt</untrusted>"),
        "{first}"
    );
    assert!(first.ends_with("Please add a bye file"), "{first}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_workers_question_goes_to_its_plans_lead() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let answered = Arc::new(AtomicBool::new(false));
    let a2 = answered.clone();
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        let answered = a2.clone();
        let planner = request_lead();
        Box::pin(async move {
            if turn.prompt.contains("asks:") {
                turn.say("Use lowercase bye.");
                answered.store(true, Ordering::SeqCst);
            } else if turn.prompt.ends_with("hi") {
                turn.say("Hello.");
            } else {
                planner(turn).await;
            }
        })
    });
    let a3 = answered.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let answered = a3.clone();
        Box::pin(async move {
            if turn.prompt.contains("Answer from the lead") {
                turn.say("Thanks.");
                return;
            }
            let r = turn.tool("codly_ask_lead", json!({ "question": "Uppercase or lowercase?" })).await;
            assert_eq!(r["ok"], true, "{r}");
            let until = Instant::now() + Duration::from_secs(10);
            while !answered.load(Ordering::SeqCst) && Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await; // the Lead's turn ends meanwhile
            turn.write("bye.txt", "bye\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created bye.txt." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "hi").await.unwrap();
    wait_chat(&core, "lead", "Hello.").await;
    core.send_chat("lead:2", "Please add a bye file").await.unwrap();
    let snap = wait_for("the forwarded answer and the plan", 30, async || {
        let s = core.snapshot().await.unwrap();
        let worker = agents_ever(&s).into_iter().find(|a| a.role == Role::Worker)?;
        let forwarded = turns_of(&log, &worker.id).iter().any(|t| t.contains("Answer from the lead"));
        (forwarded && s.plans.first().is_some_and(|p| p.status == PlanStatus::Done)).then(|| s.clone())
    })
    .await;
    let worker = &agents_ever(&snap).into_iter().find(|a| a.role == Role::Worker).unwrap();
    let (lead1, lead2) = (lead_of(&snap, "lead").unwrap(), lead_of(&snap, "lead:2").unwrap());

    // The question and the answer live in the plan's thread; the answer links to the asker.
    let thread = chats(&snap, "lead:2");
    let q = thread.iter().find(|c| c.text.starts_with("Question from")).expect("the question in the plan's thread");
    assert_eq!((q.author.as_str(), q.text.as_str()), (worker.id.as_str(), format!("Question from {} (t1): Uppercase or lowercase?", worker.name).as_str()));
    let a = thread.iter().find(|c| c.text == "Use lowercase bye.").unwrap();
    assert_eq!(a.author, lead2.id);
    assert_eq!(a.attachment.as_ref().unwrap()["reply_to"], worker.id.as_str());
    assert!(!chats(&snap, "lead").iter().any(|c| c.text.starts_with("Question from") || c.text == "Use lowercase bye."));
    assert!(turns_of(&log, &lead2.id).iter().any(|t| t.contains("asks:") && t.contains("Uppercase or lowercase?")));
    assert!(!turns_of(&log, &lead1.id).iter().any(|t| t.contains("asks:")), "the other request's Lead is not asked");
    let fwd = turns_of(&log, &worker.id).into_iter().find(|t| t.contains("Answer from the lead")).unwrap();
    assert!(fwd.contains("Use lowercase bye."), "{fwd}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_most_three_leads_run_and_a_paused_one_resumes_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let hold = Arc::new(AtomicBool::new(false));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: echo_lead(hold.clone()), worker: idle }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let live_leads = |s: &Snapshot| s.agents.iter().filter(|a| a.role == Role::Lead && a.run_id.is_some()).count();

    for t in ["lead", "lead:2", "lead:3"] {
        core.send_chat(t, &format!("hi {t}")).await.unwrap();
    }
    wait_for("three idle Leads", 20, async || {
        let s = core.snapshot().await.unwrap();
        let idle = s.agents.iter().filter(|a| a.role == Role::Lead && a.run_id.is_some() && a.status == AgentStatus::Idle).count();
        (idle == 3 && ["lead", "lead:2", "lead:3"].iter().all(|t| chats(&s, t).iter().any(|c| c.text == format!("ok: hi {t}")))).then_some(())
    })
    .await;
    let snap = core.snapshot().await.unwrap();
    let ids: Vec<String> = ["lead", "lead:2", "lead:3"].iter().map(|t| lead_of(&snap, t).unwrap().id.clone()).collect();
    let session1 = lead_of(&snap, "lead").unwrap().session_id.clone().expect("a session id");

    // A 4th request: the least recently used idle Lead (the first) is paused to make room.
    core.send_chat("lead:4", "hi lead:4").await.unwrap();
    wait_chat(&core, "lead:4", "ok: hi lead:4").await;
    let snap = wait_for("the first Lead to exit", 10, async || {
        let s = core.snapshot().await.unwrap();
        lead_of(&s, "lead").is_some_and(|l| l.run_id.is_none()).then_some(s)
    })
    .await;
    assert_eq!(lead_of(&snap, "lead:4").unwrap().name, "Lead 4");
    assert_eq!(live_leads(&snap), 3);
    assert_eq!(log.lock().unwrap().shutdowns, vec![ids[0].clone()]);
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. }
        if summary.starts_with("paused Lead (idle) to start Lead 4"))), "an explained pause");

    // The first request again: Lead resumes its own session; now Lead 2 is the oldest idle one.
    core.send_chat("lead", "hi again").await.unwrap();
    let snap = wait_chat(&core, "lead", "ok: hi again").await;
    {
        let log = log.lock().unwrap();
        assert_eq!(log.shutdowns, vec![ids[0].clone(), ids[1].clone()]);
        let runs: Vec<&SessionMode> = log.runs.iter().filter(|(a, _)| *a == ids[0]).map(|(_, s)| s).collect();
        assert_eq!(runs.len(), 2);
        assert!(matches!(runs[1], SessionMode::Resume { session_id } if *session_id == session1), "{runs:?}");
    }
    assert_eq!(lead_of(&snap, "lead").unwrap().session_id.as_deref(), Some(session1.as_str()));
    assert!(chats(&snap, "lead").iter().any(|c| c.text == "ok: hi lead"), "the thread keeps its history");

    // Every live Lead mid-turn: a 4th request waits (and says so) instead of stopping one.
    hold.store(true, Ordering::SeqCst);
    for t in ["lead", "lead:3", "lead:4"] {
        core.send_chat(t, "hold on").await.unwrap();
    }
    wait_for("three Leads mid-turn", 10, async || {
        let held = log.lock().unwrap().turns.iter().filter(|(_, t)| t.ends_with("hold on")).count();
        (held == 3).then_some(())
    })
    .await;
    let t0 = Instant::now();
    core.send_chat("lead:2", "hi while busy").await.unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2), "send_chat does not wait for a Lead slot");
    let snap = core.snapshot().await.unwrap();
    let queued: Vec<_> = chats(&snap, "lead:2").into_iter().filter(|c| c.author == "codly" && c.text.starts_with("Queued:")).collect();
    assert_eq!(queued.len(), 1, "{:?}", chats(&snap, "lead:2"));
    assert!(lead_of(&snap, "lead:2").unwrap().run_id.is_none());
    assert_eq!(log.lock().unwrap().shutdowns.len(), 2, "no Lead is stopped mid-turn");
    assert_eq!(live_leads(&snap), 3);

    hold.store(false, Ordering::SeqCst);
    let snap = wait_chat(&core, "lead:2", "ok: hi while busy").await;
    {
        let log = log.lock().unwrap();
        let runs: Vec<&SessionMode> = log.runs.iter().filter(|(a, _)| *a == ids[1]).map(|(_, s)| s).collect();
        let session2 = lead_of(&snap, "lead:2").unwrap().session_id.clone().unwrap();
        assert!(matches!(runs.last().unwrap(), SessionMode::Resume { session_id } if *session_id == session2), "{runs:?}");
        assert_eq!(log.shutdowns.len(), 3, "one idle Lead made room");
        let paused = log.shutdowns[2].clone();
        assert!(paused != ids[1] && turns_of_locked(&log, &paused).last().unwrap().ends_with("hold on"), "a Lead whose turn had ended");
    }
    for t in ["lead", "lead:3", "lead:4"] {
        wait_chat(&core, t, "ok: hold on").await;
    }
    let snap = wait_for("the pause to settle", 10, async || {
        let s = core.snapshot().await.unwrap();
        (live_leads(&s) == 3).then_some(s)
    })
    .await;
    assert_eq!(snap.agents.iter().filter(|a| a.role == Role::Lead).count(), 4);
    core.shutdown().await;
}

fn turns_of_locked(log: &Log, agent: &str) -> Vec<String> {
    log.turns.iter().filter(|(a, _)| a == agent).map(|(_, t)| t.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_chat_returns_while_the_lead_is_still_working() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let hold = Arc::new(AtomicBool::new(true));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: echo_lead(hold.clone()), worker: idle }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let quick = Duration::from_secs(2);
    tokio::time::timeout(quick, core.send_chat("lead", "hold the first")).await.expect("send_chat returns at once").unwrap();
    tokio::time::timeout(quick, core.send_chat("lead", "then this")).await.expect("a second message too").unwrap();
    tokio::time::timeout(quick, core.send_chat("lead:2", "hold another request")).await.expect("another request too").unwrap();
    wait_for("both Leads mid-turn", 10, async || {
        let s = core.snapshot().await.unwrap();
        let busy = s.agents.iter().filter(|a| a.role == Role::Lead && a.status == AgentStatus::Thinking).count();
        (busy == 2).then_some(())
    })
    .await;
    let snap = core.snapshot().await.unwrap();
    assert!(!snap.chats.iter().any(|c| c.text.starts_with("ok:")), "no Lead turn has finished yet");
    assert_eq!(chats(&snap, "lead").iter().filter(|c| c.author == "user").count(), 2, "both messages are recorded at once");

    hold.store(false, Ordering::SeqCst);
    let snap = wait_chat(&core, "lead", "ok: then this").await;
    let replies: Vec<&str> = chats(&snap, "lead").iter().filter(|c| c.text.starts_with("ok:")).map(|c| c.text.as_str()).collect();
    assert_eq!(replies, vec!["ok: hold the first", "ok: then this"], "one thread's messages run in order");
    wait_chat(&core, "lead:2", "ok: hold another request").await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_requests_lead_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let hold = Arc::new(AtomicBool::new(false));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let log = Arc::new(Mutex::new(Log::default()));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: echo_lead(hold.clone()), worker: idle.clone() }).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead:7", "hi").await.unwrap();
    let snap = wait_chat(&core, "lead:7", "ok: hi").await;
    let lead = lead_of(&snap, "lead:7").unwrap().clone();
    assert_eq!(lead.name, "Lead", "the first Lead created is \"Lead\", whatever its thread");
    let session = lead.session_id.clone().unwrap();
    core.shutdown().await;
    drop(core);

    let log = Arc::new(Mutex::new(Log::default()));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: echo_lead(hold), worker: idle }).await;
    core.send_chat("lead:7", "back").await.unwrap();
    let snap = wait_chat(&core, "lead:7", "ok: back").await;
    let leads: Vec<&Agent> = snap.agents.iter().filter(|a| a.role == Role::Lead).collect();
    assert_eq!(leads.len(), 1);
    assert_eq!(leads[0].id, lead.id, "the thread's Lead is found again after the restart");
    let runs = log.lock().unwrap().runs.clone();
    assert!(matches!(&runs[0], (a, SessionMode::Resume { session_id }) if *a == lead.id && *session_id == session), "{runs:?}");
    core.shutdown().await;
}

// ───────────── accounts (several sign-ins of one CLI) ─────────────

/// Like `start`, with the login shell's CLAUDE_CONFIG_DIR unset (its sign-in is the default).
async fn start_signed_in(root: &Path, spawner: FakeSpawner) -> Arc<Core> {
    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    env.remove("CLAUDE_CONFIG_DIR");
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: Some(Arc::new(spawner)), env: Some(env), clis: Some(vec![claude()]), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

fn claude_home(root: &Path, name: &str) -> String {
    root.join("home").join(name).to_string_lossy().into_owned()
}

fn claude_account(home: &str) -> AccountConfig {
    AccountConfig { provider: Provider::Claude, home: Some(home.into()), label: None, enabled: true }
}

/// Usage windows as Codly persists them (`kv` "account_limits"), for one account.
fn seed_limits(data: &Path, account: &str, used: f64) {
    let store = Store::open(&data.join("codly.sqlite")).unwrap();
    let windows = json!({ account: [{
        "window": "five_hour", "used_fraction": used, "resets_at": now_ms() + 2 * 3_600_000, "status": "allowed", "seen_at": now_ms(),
    }] });
    store.kv_set("account_limits", &windows.to_string()).unwrap();
}

/// A Lead that plans two independent tasks: a.txt (t1) and b.txt (t2).
fn two_task_lead() -> Behavior {
    let task = |id: &str, file: &str| {
        json!({ "id": id, "title": format!("Create {file}.txt"), "goal": format!("Create {file}.txt"),
                "consultant": { "title": "Backend" }, "acceptance_criteria": [format!("{file}.txt exists")], "write_scope": [format!("{file}.txt")],
                "depends_on": [], "verify": [format!("test -f {file}.txt")], "size": "M" })
    };
    let plan = json!({ "summary": "Add two files", "tasks": [task("t1", "a"), task("t2", "b")] });
    Arc::new(move |turn: FakeTurn| {
        let plan = plan.clone();
        Box::pin(async move {
            if !turn.prompt.contains("[Codly]") {
                let reply = turn.tool("codly_submit_plan", plan).await;
                assert_eq!(reply["ok"], true, "{reply}");
            }
            turn.say("Planned.");
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_finishes_without_the_task_you_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("a.txt", "a\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created a.txt." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log, lead: two_task_lead(), worker }).await;
    let settings = Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Add two files").await.unwrap();
    let plan = wait_for("the plan", 30, async || core.snapshot().await.unwrap().plans.first().cloned()).await;
    let t2 = core.snapshot().await.unwrap().tasks.into_iter().find(|t| t.key == "t2").unwrap();
    core.move_task(&t2.id, TaskStatus::Cancelled).await.unwrap();
    core.run_plan(&plan.id).await.unwrap();
    wait_for("the plan to finish", 30, async || {
        core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(())
    })
    .await;
    let s = core.snapshot().await.unwrap();
    assert!(chats(&s, "lead").iter().any(|c| c.text.contains("1 task merged")), "the cancelled task isn't counted");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_with_every_task_cancelled_is_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let idle: Behavior = Arc::new(|_turn: FakeTurn| Box::pin(async {}));
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: two_task_lead(), worker: idle }).await;
    let settings = Settings { lead_followups: false, worker_providers: vec![Provider::Claude], ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Add two files").await.unwrap();
    let plan = wait_for("the plan", 30, async || core.snapshot().await.unwrap().plans.first().cloned()).await;
    for t in core.snapshot().await.unwrap().tasks {
        core.move_task(&t.id, TaskStatus::Cancelled).await.unwrap();
    }
    core.run_plan(&plan.id).await.unwrap();
    wait_for("the plan to be cancelled", 20, async || {
        core.snapshot().await.unwrap().plans.first().is_some_and(|p| p.status == PlanStatus::Cancelled).then_some(())
    })
    .await;
    assert!(!log.lock().unwrap().spawns.iter().any(|s| s.0 == Role::Worker), "nothing was started");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_tasks_spread_over_two_claude_accounts_and_skip_a_hot_one() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (work, perso, hot) = (claude_home(dir.path(), ".claude-work"), claude_home(dir.path(), ".claude-perso"), claude_home(dir.path(), ".claude-hot"));
    let id = |h: &str| format!("claude:{h}");
    seed_limits(&dir.path().join("data"), &id(&hot), 0.9); // hot is at 90% of its 5h window
    let log = Arc::new(Mutex::new(Log::default()));
    let work_dir = work.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let work_dir = work_dir.clone();
        Box::pin(async move {
            let task = turn.tool("codly_get_task", json!({})).await;
            let file = if task["result"]["key"] == "t1" { "a" } else { "b" };
            // Each worker reports its own account's usage, as the CLI would.
            let mine = turn.spec.env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_default();
            let used = if mine == work_dir { 0.42 } else { 0.17 };
            turn.emit(EventKind::RateLimit {
                window: "five_hour".into(),
                used_fraction: Some(used),
                resets_at: Some(now_ms() + 3_600_000),
                status: Some("allowed".into()),
            });
            turn.write(&format!("{file}.txt"), &format!("{file}\n"));
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": format!("Created {file}.txt.") })).await;
        })
    });
    let core = start_signed_in(dir.path(), FakeSpawner { log: log.clone(), lead: two_task_lead(), worker }).await;
    let accounts = vec![claude_account(&work), claude_account(&format!("{perso}/")), claude_account(&hot)];
    let settings = Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], accounts, ..Default::default() };
    let saved = core.update_settings(settings).await.unwrap();
    assert_eq!(saved.accounts[1].home.as_deref(), Some(perso.as_str()), "homes are normalized");

    // The registry: your three accounts in your order, then the login shell's (off: you listed others).
    let listed = core.accounts(false).await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, [id(&work).as_str(), id(&perso).as_str(), id(&hot).as_str(), "claude"]);
    assert_eq!(listed.iter().map(|a| a.enabled).collect::<Vec<_>>(), [true, true, true, false]);
    assert_eq!(listed[2].limits[0].used_fraction, 0.9, "usage persisted by an earlier run");

    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Add two files").await.unwrap();
    let snap = wait_for("the plan to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(s)
    })
    .await;

    // Two parallel workers, one per account, each spawned with its account's config dir.
    let workers: Vec<(String, Option<String>)> =
        log.lock().unwrap().envs.iter().filter(|e| e.0 == Role::Worker).map(|e| (e.1.clone(), e.2.clone())).collect();
    assert_eq!(workers.len(), 2, "{workers:?}");
    let dirs: std::collections::HashSet<Option<String>> = workers.iter().map(|w| w.1.clone()).collect();
    assert_eq!(dirs, [Some(work.clone()), Some(perso.clone())].into_iter().collect(), "never the account at 90%");
    for (agent, dir) in &workers {
        let a = agents_ever(&snap).into_iter().find(|a| a.id == *agent).unwrap();
        assert!(a.consultant && a.archived, "a consultant is let go after its task");
        assert_eq!(a.account, dir.as_deref().map(id), "the agent records its account");
        assert_eq!(a.billing, Billing::Subscription);
    }
    let lead_dir = log.lock().unwrap().envs.iter().find(|e| e.0 == Role::Lead).unwrap().2.clone();
    assert!(lead_dir == Some(work.clone()) || lead_dir == Some(perso.clone()), "the Lead runs on a usable account: {lead_dir:?}");

    // Each account's quota comes from its own agents' RateLimit events, also in the snapshot.
    let used = |list: &[AccountInfo], h: &str| list.iter().find(|a| a.id == id(h)).unwrap().limits.iter().map(|w| w.used_fraction).collect::<Vec<_>>();
    assert_eq!((used(&snap.accounts, &work), used(&snap.accounts, &perso), used(&snap.accounts, &hot)), (vec![0.42], vec![0.17], vec![0.9]));
    let events = core.events(None, None, 3000).await.unwrap();
    let routes: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Decision { decision_type, summary, detail, .. } if decision_type == "route" => {
                assert!(detail["account"].is_string(), "{detail}");
                Some(summary.clone())
            }
            _ => None,
        })
        .collect();
    assert!(routes.iter().any(|s| s.starts_with("Claude Code · work · new consultant")), "{routes:?}");
    assert!(routes.iter().any(|s| s.starts_with("Claude Code · perso · new consultant")), "{routes:?}");
    core.shutdown().await;
    drop(core);

    // A restart still knows them.
    let store = Store::open(&dir.path().join("data/codly.sqlite")).unwrap();
    let kept: Value = serde_json::from_str(&store.kv_get("account_limits").unwrap().unwrap()).unwrap();
    assert_eq!(kept[id(&work)][0]["used_fraction"], 0.42);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lead_pins_a_task_to_the_account_the_user_names() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (work, perso) = (claude_home(dir.path(), ".claude-work"), claude_home(dir.path(), ".claude-perso"));
    let perso_id = format!("claude:{perso}");
    seed_limits(&dir.path().join("data"), &perso_id, 0.6); // Spread alone would pick work
    let log = Arc::new(Mutex::new(Log::default()));
    let pin = perso_id.clone();
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        let pin = pin.clone();
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                turn.say("Noted.");
                return;
            }
            // The board lists both sign-ins with their ids and usage.
            assert!(turn.prompt.contains(&format!("{pin} = Claude Code perso (60% used)")), "{}", turn.prompt);
            let plan = |account: &str| {
                json!({ "summary": "Add a hello file", "tasks": [{
                    "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing hello",
                    "consultant": { "title": "Backend" }, "acceptance_criteria": ["hello.txt contains hello"], "write_scope": ["hello.txt"],
                    "depends_on": [], "verify": ["grep -q hello hello.txt"], "account": account }] })
            };
            let reply = turn.tool("codly_submit_plan", plan("claude:nope")).await;
            assert_eq!(reply["ok"], false, "{reply}");
            assert!(reply["error"].as_str().unwrap().contains("unknown account 'claude:nope'"), "{reply}");
            let reply = turn.tool("codly_submit_plan", plan(&pin)).await;
            assert_eq!(reply["ok"], true, "{reply}");
            turn.say("Planned on perso.");
        })
    });
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created hello.txt." })).await;
        })
    });
    let core = start_signed_in(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    let accounts = vec![claude_account(&work), claude_account(&perso)];
    let settings = Settings { auto_run_plans: true, lead_followups: false, worker_providers: vec![Provider::Claude], accounts, ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Add hello.txt, on my perso account").await.unwrap();
    let (task, _) = wait_for("the plan to finish", 30, async || {
        task_status(&core).await.filter(|(_, p)| p.status == PlanStatus::Done)
    })
    .await;

    assert_eq!(task.account_hint.as_deref(), Some(perso_id.as_str()));
    let worker_dir = log.lock().unwrap().envs.iter().find(|e| e.0 == Role::Worker).unwrap().2.clone();
    assert_eq!(worker_dir, Some(perso.clone()), "the pinned account, not the emptier one");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resumed_run_keeps_its_account() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let data = dir.path().join("data");
    let repo_s = std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned();
    let (work, perso) = (claude_home(dir.path(), ".claude-work"), claude_home(dir.path(), ".claude-perso"));
    let work_id = format!("claude:{work}");
    // Work is the busier account: a new session would go to perso.
    seed_limits(&data, &work_id, 0.7);

    // What a crash leaves behind: Ada's run on the work account, its task running.
    let (task_id, agent_id) = {
        let store = Store::open(&data.join("codly.sqlite")).unwrap();
        let settings = Settings { accounts: vec![claude_account(&perso), claude_account(&work)], lead_followups: false, ..Default::default() };
        store.save_settings(&settings).unwrap();
        let ws = Workspace {
            id: new_id(),
            repo_path: repo_s.clone(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&ws).unwrap();
        let ev = |kind| EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind };
        let (agent_id, run_id, task_id) = (new_id(), new_id(), new_id());
        let agent: Agent = serde_json::from_value(json!({
            "id": agent_id, "name": "Ada", "provider": "claude", "model": null, "role": "worker", "status": "working",
            "activity": null, "activity_label": null, "task_id": task_id, "run_id": run_id, "session_id": "sess-work",
            "worktree": null, "billing": "subscription", "account": work_id, "color": 0, "look_seed": agent_id, "thread": null,
            "usage": Usage::default(), "created_at": now_ms(), "updated_at": now_ms(),
        }))
        .unwrap();
        store.append_event(Some(&ws.id), &ev(EventKind::AgentChanged { agent })).unwrap();
        let task: Task = serde_json::from_value(json!({
            "id": task_id, "plan_id": null, "key": "a1", "title": "Write x", "goal": "Create x.txt", "acceptance": [],
            "write_scope": [], "verify": ["test -f x.txt"], "depends_on": [], "status": "running", "assignee": agent_id,
            "agent_hint": null, "account_hint": null, "branch": null, "worktree": null, "attempt": 1, "status_reason": null,
            "summary": null, "created_at": now_ms(), "updated_at": now_ms(),
        }))
        .unwrap();
        store.append_event(Some(&ws.id), &ev(EventKind::TaskChanged { task })).unwrap();
        store
            .insert_run(&RunRow {
                id: run_id,
                agent_id: agent_id.clone(),
                task_id: Some(task_id.clone()),
                role: Role::Worker,
                provider: Provider::Claude,
                model: None,
                cwd: repo_s.clone(),
                session_id: Some("sess-work".into()),
                account: Some(work_id.clone()),
                status: "running".into(),
                raw_log: "/dev/null".into(),
                report: None,
                started_at: now_ms(),
                ended_at: None,
            })
            .unwrap();
        (task_id, agent_id)
    };

    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("x.txt", "x\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created x.txt." })).await;
        })
    });
    let core = start_signed_in(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    let snap = wait_for("the resumed task to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.tasks.iter().find(|t| t.id == task_id && t.status == TaskStatus::Done).map(|_| s.clone())
    })
    .await;
    {
        let log = log.lock().unwrap();
        assert!(matches!(&log.runs[0], (a, SessionMode::Resume { session_id }) if *a == agent_id && session_id == "sess-work"), "{:?}", log.runs);
        assert_eq!(log.envs[0].2.as_deref(), Some(work.as_str()), "resumed where its session lives, not on the emptier account");
    }
    let ghosts = core.ghosts().unwrap(); // let go once its task is done
    let ada = snap.agents.iter().chain(&ghosts).find(|a| a.id == agent_id).unwrap();
    assert_eq!(ada.account.as_deref(), Some(work_id.as_str()));
    let store = Store::open(&data.join("codly.sqlite")).unwrap();
    let resumed = store.latest_task_run(&task_id, Some(Role::Worker)).unwrap().unwrap();
    assert_eq!(resumed.account.as_deref(), Some(work_id.as_str()), "the run records its account");
    core.shutdown().await;
}

/// A team member works task after task in its own worktree, continuing its own conversation;
/// the Lead can message it and gets its answer back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_team_member_keeps_its_session_and_worktree_across_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("answers your message") {
                turn.say("Noted.");
                return;
            }
            if turn.prompt.contains("[Codly]") {
                return;
            }
            if turn.prompt.ends_with("ask Ana") {
                let r = turn.tool("codly_message_member", json!({ "to": "ana", "text": "What did you change?" })).await;
                assert_eq!(r["ok"], true, "{r}");
                turn.say("Asked Ana.");
                return;
            }
            let file = if turn.prompt.ends_with("first") { "a.txt" } else { "b.txt" };
            let plan = json!({ "summary": format!("Add {file}"), "tasks": [{
                "id": "t1", "title": format!("Create {file}"), "goal": format!("Create {file}"), "assignee": "Ana",
                "acceptance_criteria": [format!("{file} exists")], "write_scope": [file], "depends_on": [], "verify": [format!("test -f {file}")],
            }] });
            let r = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(r["ok"], true, "{r}");
            turn.say("Planned.");
        })
    });
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.starts_with("[Lead]") {
                turn.say("I created a.txt and b.txt.");
                return;
            }
            let file = if turn.prompt.contains("Task t1: Create a.txt") { "a.txt" } else { "b.txt" };
            turn.write(file, "x\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": format!("Created {file}.") })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    let ana = TeamMember { name: "Ana".into(), role: "Backend dev: owns the text files".into(), ..Default::default() };
    let s = core.update_settings(Settings { auto_run_plans: true, lead_followups: false, team: vec![ana], ..Default::default() }).await.unwrap();
    assert!(s.team[0].id.starts_with("ana-"), "Codly gives a member an id: {:?}", s.team[0].id);
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let done = |n: usize| {
        let core = core.clone();
        async move {
            wait_for(&format!("{n} plan(s) done"), 30, async || {
                let s = core.snapshot().await.unwrap();
                (s.plans.iter().filter(|p| p.status == PlanStatus::Done).count() == n).then_some(s)
            })
            .await
        }
    };
    core.send_chat("lead", "first").await.unwrap();
    let snap = done(1).await;
    let ana = snap.agents.iter().find(|a| a.member.is_some()).expect("Ana is on the board").clone();
    assert_eq!((ana.name.as_str(), ana.title.as_deref(), ana.archived), ("Ana", Some("Backend dev"), false));
    core.send_chat("lead", "second").await.unwrap();
    let snap = done(2).await;
    assert!(snap.agents.iter().any(|a| a.id == ana.id), "a member stays after its tasks");

    let l = log.lock().unwrap();
    let runs: Vec<(String, SessionMode)> = l.runs.iter().filter(|r| r.0 == ana.id).cloned().collect();
    assert_eq!(runs.len(), 2, "{runs:?}");
    let SessionMode::New { session_id: Some(first) } = &runs[0].1 else { panic!("{runs:?}") };
    assert!(matches!(&runs[1].1, SessionMode::Resume { session_id } if session_id == first), "the second task continues the conversation: {runs:?}");
    let cwds: Vec<&String> = l.spawns.iter().filter(|s| s.0 == Role::Worker).map(|s| &s.1).collect();
    assert!(cwds.len() == 2 && cwds[0] == cwds[1] && cwds[0].contains("/team/ana-"), "one stable worktree: {cwds:?}");
    let second = l.turns.iter().filter(|t| t.0 == ana.id).nth(1).unwrap().1.clone();
    assert!(second.starts_with("[Codly] New task.") && second.contains("Your role: Backend dev"), "{second}");
    drop(l);
    assert!(Path::new(cwds_first(&log)).exists(), "the member's worktree is kept after the merge");

    core.send_chat("lead", "ask Ana").await.unwrap();
    wait_for("Ana's answer at the Lead", 20, async || {
        turns_of(&log, &lead_of(&core.snapshot().await.unwrap(), "lead")?.id)
            .iter()
            .any(|t| t.contains("Ana answers your message") && t.contains("I created a.txt and b.txt."))
            .then_some(())
    })
    .await;
    let l = log.lock().unwrap();
    let chat = l.spawns.iter().filter(|s| s.0 == Role::Worker).nth(2).expect("a chat run for the message");
    assert!(chat.1.contains("/team/ana-") && matches!(&chat.2, SessionMode::Resume { session_id } if session_id == first), "{chat:?}");
    drop(l);
    wait_for("the answer-only run to stop", 10, async || log.lock().unwrap().shutdowns.iter().any(|a| a == &ana.id).then_some(())).await;
    core.shutdown().await;
}

fn cwds_first(log: &Mutex<Log>) -> &'static str {
    let l = log.lock().unwrap();
    Box::leak(l.spawns.iter().find(|s| s.0 == Role::Worker).unwrap().1.clone().into_boxed_str())
}

/// A consultant hired with the Lead's harness and model is let go once its task is merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consultant_is_hired_with_the_leads_model_and_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                return;
            }
            // Without a job title the plan is refused, telling the Lead the titles.
            let untitled = json!({ "summary": "Add hello", "tasks": [{
                "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt",
                "acceptance_criteria": ["it exists"], "write_scope": ["hello.txt"], "depends_on": [], "verify": ["test -f hello.txt"],
            }] });
            let r = turn.tool("codly_submit_plan", untitled).await;
            assert!(r["ok"] == false && r["error"].as_str().unwrap().contains("needs consultant.title, one of: Backend"), "{r}");
            let plan = json!({ "summary": "Add hello", "tasks": [{
                "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt", "assignee": "consultant",
                "consultant": { "title": "Frontend", "provider": "claude", "model": "haiku" },
                "acceptance_criteria": ["it exists"], "write_scope": ["hello.txt"], "depends_on": [], "verify": ["test -f hello.txt"],
            }] });
            let r = turn.tool("codly_submit_plan", plan).await;
            assert_eq!(r["ok"], true, "{r}");
        })
    });
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created it." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "add hello").await.unwrap();
    let snap = wait_for("the consultant let go", 30, async || {
        let s = core.snapshot().await.unwrap();
        let done = s.plans.first().is_some_and(|p| p.status == PlanStatus::Done);
        (done && !s.agents.iter().any(|a| a.role == Role::Worker)).then_some(s)
    })
    .await;
    let c = agents_ever(&snap).into_iter().find(|a| a.role == Role::Worker).unwrap();
    assert!(c.consultant && c.archived);
    assert_eq!(c.title.as_deref(), Some("Frontend"), "a consultant wears the Lead's title");
    assert!(log.lock().unwrap().models.iter().any(|(a, m)| a == &c.id && m.as_deref() == Some("haiku")), "spawned with the Lead's model");
    core.shutdown().await;
}

/// A request's own mode and Lead model, picked in the composer: Ask refuses plans, Direct runs
/// a quick task at once (even with auto-run off), and the Lead runs on the request's model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_requests_mode_and_model_steer_its_lead() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                return;
            }
            let task = json!({ "title": "Create hello.txt", "goal": "Create hello.txt", "assignee": "consultant", "consultant": { "title": "Docs" },
                "acceptance_criteria": ["it exists"], "write_scope": ["hello.txt"], "verify": ["test -f hello.txt"] });
            if turn.prompt.contains("Mode: Ask") {
                let r = turn.tool("codly_submit_plan", json!({ "summary": "Add hello", "tasks": [{ "id": "t1", "depends_on": [], "title": "x",
                    "goal": "x", "acceptance_criteria": ["x"], "write_scope": ["x"], "verify": ["true"] }] })).await;
                turn.say(&format!("refused: {}", r["error"].as_str().unwrap_or("no")));
            } else if turn.prompt.contains("Mode: Direct") {
                let r = turn.tool("codly_quick_task", task).await;
                assert_eq!(r["ok"], true, "{r}");
                assert_eq!(r["result"]["status"], "running", "{r}");
                turn.say("On it.");
            }
        })
    });
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created it." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: false, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    core.set_request_prefs("lead", RequestPrefs { mode: Some(LeadMode::Ask), ..Default::default() }).await.unwrap();
    core.send_chat("lead", "add hello").await.unwrap();
    let s = wait_for("the refusal", 20, async || {
        let s = core.snapshot().await.unwrap();
        chats(&s, "lead").iter().any(|c| c.text.starts_with("refused: Not submitted: this request is in Ask mode")).then_some(s)
    })
    .await;
    assert!(s.plans.is_empty());

    let prefs = RequestPrefs { mode: Some(LeadMode::Direct), model: Some(" opus:high ".into()), ..Default::default() };
    assert_eq!(core.set_request_prefs("lead", prefs).await.unwrap().model.as_deref(), Some("opus:high"));
    assert!(core.set_request_prefs("agent:x", RequestPrefs::default()).await.is_err(), "only requests have modes");
    core.send_chat("lead", "add hello now").await.unwrap();
    let s = wait_for("the quick task done", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(s)
    })
    .await;
    assert_eq!(s.request_prefs["lead"].mode, Some(LeadMode::Direct));
    let lead = lead_of(&s, "lead").unwrap();
    let models: Vec<Option<String>> = log.lock().unwrap().models.iter().filter(|m| m.0 == lead.id).map(|m| m.1.clone()).collect();
    assert_eq!(models.last().unwrap().as_deref(), Some("opus:high"), "the Lead restarts on the request's model: {models:?}");

    core.close_thread("lead").await.unwrap();
    assert!(core.snapshot().await.unwrap().request_prefs.is_empty(), "a closed request forgets its choices");
    core.shutdown().await;
}

/// A request can pick its own Lead CLI: its Lead runs there on that CLI's default model, while
/// another request keeps the Settings CLI and model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_picks_its_own_lead_cli() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let codex = CliInfo { provider: Provider::Codex, ..claude() };
    let spawner = FakeSpawner { log: log.clone(), lead: echo_lead(Arc::new(AtomicBool::new(false))), worker: echo_lead(Arc::new(AtomicBool::new(false))) };
    let core = start_with_clis(dir.path(), spawner, vec![claude(), codex]).await;
    let settings = Settings { lead_provider: Some(Provider::Claude), lead_model: Some("opus".into()), lead_followups: false, ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    core.set_request_prefs("lead:2", RequestPrefs { provider: Some(Provider::Codex), ..Default::default() }).await.unwrap();
    core.send_chat("lead:2", "hi").await.unwrap();
    core.send_chat("lead", "hi").await.unwrap();
    let s = wait_for("both Leads", 20, async || {
        let s = core.snapshot().await.unwrap();
        (chats(&s, "lead:2").len() > 1 && chats(&s, "lead").len() > 1).then_some(s)
    })
    .await;
    let (own, usual) = (lead_of(&s, "lead:2").unwrap(), lead_of(&s, "lead").unwrap());
    let model = |id: &str| log.lock().unwrap().models.iter().rfind(|m| m.0 == id).and_then(|m| m.1.clone());
    assert_eq!((own.provider, model(&own.id)), (Provider::Codex, None), "Codex on its default model, not opus");
    assert_eq!((usual.provider, model(&usual.id).as_deref()), (Provider::Claude, Some("opus")));
    core.shutdown().await;
}

/// Your staff works before anyone is hired: the Lead sees a free agent you keep on its Team
/// line, and a consultant it asks for on that CLI goes to that agent instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_free_agent_on_staff_takes_the_task_before_a_hire() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let plan = json!({
        "summary": "Add a hello file",
        "tasks": [{
            "id": "t1", "title": "Create hello.txt", "goal": "Create hello.txt containing hello",
            "consultant": { "title": "Backend", "provider": "claude" }, "acceptance_criteria": ["hello.txt contains hello"],
            "write_scope": ["hello.txt"], "depends_on": [], "verify": ["grep -q hello hello.txt"], "size": "M",
        }],
    });
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
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
    });
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created it.", "files_changed": ["hello.txt"] })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let ivo = core.hire_agent(Provider::Claude, None, None).await.unwrap();

    core.send_chat("lead", "add hello").await.unwrap();
    let (task, _) = wait_for("the task done", 30, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Done)).await;
    assert_eq!(task.assignee.as_deref(), Some(ivo.id.as_str()), "the free agent on staff took it");
    let s = core.snapshot().await.unwrap();
    assert!(!agents_ever(&s).iter().any(|a| a.consultant), "nobody was hired");
    let prompts = log.lock().unwrap().prompts.clone();
    let board = prompts.iter().find(|(r, _)| *r == Role::Lead).map(|(_, p)| p.clone()).unwrap();
    assert!(board.contains(&format!("Team (assign tasks by name):\n  {}", ivo.name)), "{board}");
    core.shutdown().await;
}

/// A 1:1 agent on staff that took a task is that task's worker: stopping it stops the task.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_solo_on_a_task_blocks_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    // Mid-turn when you stop it: the turn ends only then, with its work reported as done.
    let stopped = Arc::new(tokio::sync::Notify::new());
    let wait = stopped.clone();
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let wait = wait.clone();
        Box::pin(async move {
            wait.notified().await;
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created it." })).await;
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let ivo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    assert_eq!(ivo.role, Role::Solo);
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the task's worker to be mid-turn", 30, async || {
        let working = log.lock().unwrap().prompts.iter().any(|p| p.0 == Role::Worker);
        task_status(&core).await.filter(|(t, _)| working && t.status == TaskStatus::Running)
    })
    .await;
    assert_eq!(task.assignee.as_deref(), Some(ivo.id.as_str()), "the free agent on staff took it");
    core.stop_agent(&ivo.id).await.unwrap();
    stopped.notify_one();
    wait_for("the agent to exit", 10, async || log.lock().unwrap().shutdowns.contains(&ivo.id).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(500)).await; // what its turn end set off has run its course
    let (task, _) = task_status(&core).await.unwrap();
    assert_eq!(task.status, TaskStatus::Blocked, "{:?}", task.status_reason);
    assert!(task.status_reason.as_deref().is_some_and(|r| r.starts_with("stopped by you")), "{:?}", task.status_reason);
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(!events.iter().any(|e| matches!(e.kind, EventKind::Merged { .. })), "nothing merged");
    core.shutdown().await;
}

/// A 1:1 agent on staff that did a task talks to you in its own conversation afterwards, not in
/// the task's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_solo_keeps_its_own_session_after_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.starts_with("[User]") {
                return turn.say("hi");
            }
            turn.write("hello.txt", "hello\n");
            turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Created it." })).await;
        })
    });
    let spawner = || FakeSpawner { log: log.clone(), lead: planning_lead(), worker: worker.clone() };
    let core = start(dir.path(), spawner()).await;
    core.update_settings(Settings { auto_run_plans: true, lead_followups: false, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let ivo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let thread = format!("agent:{}", ivo.id);
    // Its own conversation, from before a restart (while its chat runs, a 1:1 agent takes no task).
    core.send_chat(&thread, "hello").await.unwrap();
    wait_chat(&core, &thread, "hi").await;
    core.shutdown().await;
    drop(core);

    let core = start(dir.path(), spawner()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();
    let (task, _) = wait_for("the task done", 30, async || task_status(&core).await.filter(|(t, _)| t.status == TaskStatus::Done)).await;
    assert_eq!(task.assignee.as_deref(), Some(ivo.id.as_str()), "the free agent on staff took it");
    core.send_chat(&thread, "hello again").await.unwrap();
    let runs: Vec<SessionMode> = log.lock().unwrap().runs.iter().filter(|r| r.0 == ivo.id).map(|r| r.1.clone()).collect();
    let SessionMode::New { session_id: Some(own) } = &runs[0] else { panic!("{runs:?}") };
    assert!(matches!(&runs[1], SessionMode::New { .. }), "the task ran in a session of its own: {runs:?}");
    assert!(runs.len() == 3 && matches!(&runs[2], SessionMode::Resume { session_id } if session_id == own), "back in its own conversation: {runs:?}");
    core.shutdown().await;
}

/// A new consultant is hired through a job offer: it stays up until you pick an applicant
/// (or it closes on the Lead's pick); the one you pick walks in with its own name and traits,
/// and earns XP for the work it does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_offer_hires_the_applicant_you_pick() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.write("hello.txt", "hello\n");
            let r = turn.tool("codly_submit_report", json!({ "status": "done", "summary": "Wrote hello.", "files_changed": ["hello.txt"] })).await;
            assert_eq!(r["ok"], true, "{r}");
            turn.say("Done.");
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead: planning_lead(), worker }).await;
    core.set_hiring_window(Duration::from_secs(60));
    let mut settings = Settings { auto_run_plans: true, lead_followups: false, ..Default::default() };
    settings.worker_providers = vec![Provider::Claude];
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core.send_chat("lead", "Please add a hello file").await.unwrap();

    let offer = wait_for("the job offer", 15, async || {
        core.events(None, None, 500).await.unwrap().into_iter().find_map(|e| match e.kind {
            EventKind::JobOffer { offer } => Some(offer),
            _ => None,
        })
    })
    .await;
    assert_eq!((offer.applicants.len(), offer.title.as_deref(), offer.hired), (3, Some("Backend"), None));
    let task = core.snapshot().await.unwrap().tasks[0].clone();
    assert_eq!(task.status_reason.as_deref(), Some("waiting: hiring (the job offer is up in the office)"));
    assert!(core.snapshot().await.unwrap().agents.iter().all(|a| a.role != Role::Worker), "nobody is hired while the offer is up");

    let pick = offer.applicants[2].clone();
    core.pick_applicant(&offer.id, 2).unwrap();
    assert!(core.pick_applicant(&offer.id, 1).is_err(), "a closed offer can't be picked again");
    let snap = wait_for("the plan to finish", 30, async || {
        let s = core.snapshot().await.unwrap();
        s.plans.first().is_some_and(|p| p.status == PlanStatus::Done).then_some(s)
    })
    .await;
    // Newest first: the worker's last AgentChanged (it may have been let go since).
    let hired = core.events(None, None, 2000).await.unwrap().into_iter().find_map(|e| match e.kind {
        EventKind::AgentChanged { agent } if agent.role == Role::Worker => Some(agent),
        _ => None,
    });
    let hired = hired.unwrap();
    assert_eq!((hired.name.as_str(), hired.color, hired.look_seed.as_str()), (pick.name.as_str(), pick.color, pick.avatar.as_str()));
    assert_eq!(hired.career.traits, pick.traits);
    assert!(hired.career.xp >= 15 + 10 + 30, "commit, passing check and merge all count: {:?}", hired.career);
    assert_eq!(hired.career.stats.merged, 1);
    assert_eq!(snap.tasks[0].status, TaskStatus::Done);

    // The daily edition reports the same work, counted by the same rules.
    let day = core.digest(0).unwrap();
    let report = day.reports.iter().find(|r| r.agent_id == hired.id).expect("the hire has a report");
    assert_eq!((report.stats.commits, report.stats.merged), (hired.career.stats.commits, 1));
    assert!(report.xp >= 55, "{report:?}");
    assert_eq!(day.done, vec![snap.tasks[0].title.clone()]);
    assert!(day.milestones.iter().any(|(id, m)| id == &hired.id && m.text == "First merge"));
    assert!(core.digest(i64::MAX).unwrap().reports.is_empty(), "nothing since the future");

    // Let go after its task, it haunts the archive with its career, and can come back.
    let ghost = wait_for("the consultant to be let go", 30, async || core.ghosts().unwrap().into_iter().find(|g| g.id == hired.id)).await;
    assert_eq!(ghost.career.stats.merged, 1);
    assert_eq!(ghost.worktree, None, "its task's worktree went with the task");
    // You merge the plan with plain git, not from the app.
    let integ = snap.plans[0].integration_branch.clone().unwrap();
    let out = std::process::Command::new("git").args(["merge", "--no-ff", "-m", "by hand", &integ]).current_dir(&repo).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // …and delete its branch, after removing its worktree: only the tasks' commits remain.
    let wt = snap.plans[0].integration_worktree.clone().unwrap();
    for args in [vec!["worktree", "remove", "--force", wt.as_str()], vec!["branch", "-D", integ.as_str()]] {
        let out = std::process::Command::new("git").args(&args).current_dir(&repo).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let err = core.plan_diff(&snap.plans[0].id).await.unwrap_err().to_string();
    assert!(err.contains("was deleted"), "a plain reason, not git's: {err}");
    let back = core.bring_back(&ghost.id).unwrap();
    assert!(!back.archived && !back.consultant && back.kept && back.status == AgentStatus::Idle);
    // bring_back runs a scheduling pass: a worker you brought back is not let go again.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(core.ghosts().unwrap().is_empty(), "kept between tasks");
    let plan = wait_for("the outside merge to be noticed", 10, async || {
        core.snapshot().await.unwrap().plans.into_iter().find(|p| p.id == snap.plans[0].id && p.merged_sha.is_some())
    })
    .await;
    assert!(!Path::new(plan.integration_worktree.as_deref().unwrap()).exists());
    assert_eq!(back.career.milestones.last().map(|m| m.text.as_str()), Some("Came back"));
    assert!(core.ghosts().unwrap().is_empty());
    assert!(core.snapshot().await.unwrap().agents.iter().any(|a| a.id == ghost.id), "back on the team");
    assert!(core.bring_back(&ghost.id).is_err(), "only ghosts come back");

    // What it earned is coins, and coins buy furniture (Build/Buy); the price comes back when you sell.
    let shop = core.shop().unwrap();
    assert!(shop.wallet.earned >= 55 && shop.wallet.spent == 0 && shop.placed.is_empty(), "{shop:?}");
    let piece = |uid: &str, item: &str, x: f64| Placed { uid: uid.into(), item: item.into(), x, y: 0.0, z: 10.0, rot: 0.0 };
    core.place(piece("a", "kenney.chair", 3.0)).unwrap();
    let shop = core.place(piece("b", "kenney.chair", 4.0)).unwrap();
    assert_eq!((shop.wallet.spent, shop.placed.len()), (50, 2), "two of the same");
    let shop = core.place(piece("a", "kenney.chair", 5.0)).unwrap();
    assert_eq!((shop.wallet.spent, shop.placed[0].x), (50, 5.0), "moving is free");
    assert!(core.place(piece("a", "kenney.desk", 5.0)).is_err(), "a move keeps the piece");
    assert!(core.place(piece("c", "throne", 1.0)).is_err());
    let mut n = 0;
    let err = loop {
        n += 1;
        match core.place(piece(&format!("sofa{n}"), "kenney.loungeDesignSofa", 1.0)) {
            Ok(s) => assert!(s.wallet.spent <= s.wallet.earned && s.wallet.balance >= 0),
            Err(e) => break e,
        }
    };
    assert!(err.to_string().contains("not enough coins"), "{err}");
    let shop = core.sell("a").unwrap();
    assert_eq!(shop.wallet.spent, 25 + 340 * (n - 1) as i64);
    assert!(core.sell("a").is_err());
    core.shutdown().await;
}

/// An agent at home (the office's energy) gets no new work from the Lead: its plans, quick
/// tasks and messages to it are refused with when it is back, and the board says so. You can
/// still give it work; and when the whole team is home, the Lead may assign it anyway.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_at_home_gets_no_new_work_from_the_lead_but_from_you() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                return;
            }
            let plan = json!({ "summary": "Add a.txt", "tasks": [{
                "id": "t1", "title": "Create a.txt", "goal": "Create a.txt", "assignee": "Ana",
                "acceptance_criteria": ["a.txt exists"], "write_scope": ["a.txt"], "depends_on": [], "verify": ["test -f a.txt"],
            }] });
            let r = turn.tool("codly_submit_plan", plan).await;
            let plan = r["error"].as_str().map_or_else(|| "ok".to_string(), String::from);
            let r = turn.tool("codly_message_member", json!({ "to": "Ana", "text": "Hi" })).await;
            let msg = r["error"].as_str().map_or_else(|| "ok".to_string(), String::from);
            let r = turn.tool("codly_board", json!({})).await;
            let team = r["result"]["team"].as_array().cloned().unwrap_or_default();
            let home = team.iter().find(|m| m["name"] == "Ana").map(|m| !m["at_home_until"].is_null()).unwrap_or(false);
            turn.say(&format!("plan: {plan} | message: {msg} | board: {home}"));
        })
    });
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.say("Hello.");
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    let team = vec![TeamMember { name: "Ana".into(), role: "Backend".into(), ..Default::default() }, TeamMember { name: "Bo".into(), role: "Frontend".into(), ..Default::default() }];
    core.update_settings(Settings { auto_run_plans: false, lead_followups: false, team, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let members = wait_for("the team's agents", 20, async || {
        let s = core.snapshot().await.unwrap();
        let m: Vec<Agent> = s.agents.iter().filter(|a| a.member.is_some()).cloned().collect();
        (m.len() == 2).then_some(m)
    })
    .await;
    let ana = members.iter().find(|a| a.name == "Ana").unwrap().clone();
    let bo = members.iter().find(|a| a.name == "Bo").unwrap().clone();
    let back = now_ms() + 30 * 60_000;
    assert_eq!(core.set_agent_home(&ana.id, Some(back)).unwrap().away_until, Some(back));

    core.send_chat("lead", "add a").await.unwrap();
    let s = wait_for("the Lead's report", 20, async || {
        let s = core.snapshot().await.unwrap();
        chats(&s, "lead").iter().find(|c| c.text.starts_with("plan: ")).map(|c| c.text.clone())
    })
    .await;
    assert!(s.contains("Ana is at home until ") && s.contains("the user can call it in") && s.contains("assign someone at work (Bo)"), "{s}");
    assert!(s.contains("message: Ana is at home until "), "{s}");
    assert!(s.ends_with("board: true"), "{s}");
    assert!(core.snapshot().await.unwrap().plans.is_empty(), "the plan was refused");

    // You are never refused: a task you give it is its own.
    let t = core.create_task("Say hello", "Say hello").await.unwrap();
    let t = core.assign_task(&t.id, Some(&ana.id)).await.unwrap();
    assert_eq!(t.assignee.as_deref(), Some(ana.id.as_str()));

    // The whole team at home: the game never stops the crew from shipping.
    core.set_agent_home(&bo.id, Some(back)).unwrap();
    core.send_chat("lead", "add a again").await.unwrap();
    let s = wait_for("the Lead's second report", 20, async || {
        let s = core.snapshot().await.unwrap();
        chats(&s, "lead").iter().filter(|c| c.text.starts_with("plan: ")).nth(1).map(|c| c.text.clone())
    })
    .await;
    assert!(s.starts_with("plan: ok"), "{s}");

    // The Lead never goes home; called back, Ana is a normal member again.
    let lead = lead_of(&core.snapshot().await.unwrap(), "lead").unwrap().clone();
    assert!(core.set_agent_home(&lead.id, Some(back)).is_err());
    assert_eq!(core.set_agent_home(&ana.id, None).unwrap().away_until, None);
    core.shutdown().await;
}

/// The office says who is tired: the Lead's board shows it and its refusals list the rested
/// first (a hint, never a refusal). A task queued for a member at home waits while someone else
/// is at work; when nobody else is, it calls the member in and the decision says why.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lead_prefers_rested_members_and_a_queued_task_waits_for_home() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let log = Arc::new(Mutex::new(Log::default()));
    let lead: Behavior = Arc::new(move |turn: FakeTurn| {
        Box::pin(async move {
            if turn.prompt.contains("[Codly]") {
                return;
            }
            let plan = json!({ "summary": "Add a.txt", "tasks": [{
                "id": "t1", "title": "Create a.txt", "goal": "Create a.txt", "assignee": "Ana",
                "acceptance_criteria": ["a.txt exists"], "write_scope": ["a.txt"], "depends_on": [], "verify": ["test -f a.txt"],
            }] });
            let r = turn.tool("codly_submit_plan", plan).await;
            let plan = r["error"].as_str().map_or_else(|| "ok".to_string(), String::from);
            let r = turn.tool("codly_board", json!({})).await;
            let team = r["result"]["team"].as_array().cloned().unwrap_or_default();
            let energy = |n: &str| team.iter().find(|m| m["name"] == n).map(|m| m["energy"].to_string()).unwrap_or_default();
            turn.say(&format!("plan: {plan} | bo: {} | cy: {}", energy("Bo"), energy("Cy")));
        })
    });
    let worker: Behavior = Arc::new(|turn: FakeTurn| {
        Box::pin(async move {
            turn.say("Hello.");
        })
    });
    let core = start(dir.path(), FakeSpawner { log: log.clone(), lead, worker }).await;
    let team = ["Ana", "Bo", "Cy"].map(|n| TeamMember { name: n.into(), role: "Backend".into(), ..Default::default() }).to_vec();
    core.update_settings(Settings { auto_run_plans: false, lead_followups: false, team, ..Default::default() }).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let members = wait_for("the team's agents", 20, async || {
        let s = core.snapshot().await.unwrap();
        let m: Vec<Agent> = s.agents.iter().filter(|a| a.member.is_some()).cloned().collect();
        (m.len() == 3).then_some(m)
    })
    .await;
    let id = |n: &str| members.iter().find(|a| a.name == n).unwrap().id.clone();
    let back = now_ms() + 30 * 60_000;
    core.set_agent_home(&id("Ana"), Some(back)).unwrap();
    let bo = core.set_agent_tired(&id("Bo"), true).unwrap();
    let since = bo.tired_since.expect("tired");
    assert_eq!(core.set_agent_tired(&id("Bo"), true).unwrap().tired_since, Some(since), "the same tiredness, not a new one");

    core.send_chat("lead", "add a").await.unwrap();
    let s = wait_for("the Lead's report", 20, async || {
        let s = core.snapshot().await.unwrap();
        chats(&s, "lead").iter().find(|c| c.text.starts_with("plan: ")).map(|c| c.text.clone())
    })
    .await;
    assert!(s.contains("assign someone at work (Cy, Bo (tired))"), "rested first: {s}");
    assert!(s.ends_with("| bo: \"tired, prefer someone rested\" | cy: null"), "{s}");

    // Going home is resting: the hint goes with it.
    assert_eq!(core.set_agent_home(&id("Bo"), Some(back)).unwrap().tired_since, None);

    // A task you queue for Ana waits while Cy is at work: Ana is back in minutes.
    let t = core.create_task("Say hello", "Say hello").await.unwrap();
    core.assign_task(&t.id, Some(&id("Ana"))).await.unwrap();
    let why = wait_for("the task to wait for Ana", 20, async || {
        let s = core.snapshot().await.unwrap();
        s.tasks.iter().find(|x| x.id == t.id).and_then(|x| x.status_reason.clone()).filter(|r| r.contains("is at home until"))
    })
    .await;
    assert!(why.starts_with("waiting: Ana (assigned) is at home until "), "{why}");

    // Cy goes home too: nobody else is free, so Ana is called in for it, and the board says why.
    core.set_agent_home(&id("Cy"), Some(back)).unwrap();
    let s = wait_for("Ana called in for the task", 20, async || {
        let s = core.snapshot().await.unwrap();
        let ana = s.agents.iter().find(|a| a.name == "Ana")?;
        (ana.task_id.as_deref() == Some(t.id.as_str()) && ana.away_until.is_none()).then_some(s)
    })
    .await;
    assert!(s.agents.iter().find(|a| a.name == "Bo").unwrap().away_until.is_some(), "only Ana is called in");
    let events = core.events(None, None, 2000).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
            if decision_type == "route" && summary.contains("Ana") && summary.ends_with("called in from home: nobody else was free"))),
        "the route decision says why"
    );
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_offices_work_at_once_and_share_the_worker_slots() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (temp_repo(&dir.path().join("a")), temp_repo(&dir.path().join("b")));
    let log = Arc::new(Mutex::new(Log::default()));
    // Workers stay on their task: each holds a slot until the test ends.
    let hold: Behavior = Arc::new(|_turn: FakeTurn| Box::pin(std::future::pending::<()>()));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle, worker: hold }).await;
    core.update_settings(Settings { paused: true, max_parallel: 1, ..core.settings() }).await.unwrap();

    // A task in each office; "b" is the one left open.
    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    let ta = core.create_task("Docs for a", "").await.unwrap();
    core.move_task(&ta.id, TaskStatus::Ready).await.unwrap();
    let wa = core.snapshot().await.unwrap().workspace.unwrap().id;
    core.open_workspace(b.to_str().unwrap()).await.unwrap();
    let tb = core.create_task("Docs for b", "").await.unwrap();
    core.move_task(&tb.id, TaskStatus::Ready).await.unwrap();

    let running_b = async || core.snapshot().await.unwrap().tasks.iter().filter(|t| t.id == tb.id && t.status == TaskStatus::Running).count() as i64;
    let running_a = || core.offices().unwrap().iter().find(|o| o.id == wa).map_or(0, |o| o.working);

    // One worker slot for both offices: exactly one task starts, whichever office it is in.
    core.update_settings(Settings { paused: false, ..core.settings() }).await.unwrap();
    wait_for("a task to start", 10, async || (running_b().await + running_a() == 1).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(running_b().await + running_a(), 1, "max_parallel counts every office");

    // Two slots: the other office works too, while "b" stays the open one.
    core.update_settings(Settings { max_parallel: 2, max_per_provider: 2, ..core.settings() }).await.unwrap();
    wait_for("both offices to work", 10, async || (running_b().await == 1 && running_a() == 1).then_some(())).await;
    assert_ne!(core.snapshot().await.unwrap().workspace.unwrap().id, wa);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_office_starts_nothing_while_the_others_work() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (temp_repo(&dir.path().join("a")), temp_repo(&dir.path().join("b")));
    let log = Arc::new(Mutex::new(Log::default()));
    let hold: Behavior = Arc::new(|_turn: FakeTurn| Box::pin(std::future::pending::<()>()));
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let core = start(dir.path(), FakeSpawner { log, lead: idle, worker: hold }).await;
    core.update_settings(Settings { paused: true, max_parallel: 2, max_per_provider: 2, ..core.settings() }).await.unwrap();

    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    let ta = core.create_task("Docs for a", "").await.unwrap();
    core.move_task(&ta.id, TaskStatus::Ready).await.unwrap();
    let wa = core.snapshot().await.unwrap().workspace.unwrap().id;
    core.open_workspace(b.to_str().unwrap()).await.unwrap();
    let tb = core.create_task("Docs for b", "").await.unwrap();
    core.move_task(&tb.id, TaskStatus::Ready).await.unwrap();

    // "a" (in the background) is paused on its own: "b" works, "a" waits.
    core.update_settings(Settings { paused: false, paused_offices: vec![wa.clone()], ..core.settings() }).await.unwrap();
    let running_b = async || core.snapshot().await.unwrap().tasks.iter().any(|t| t.id == tb.id && t.status == TaskStatus::Running);
    let running_a = || core.offices().unwrap().iter().find(|o| o.id == wa).map_or(0, |o| o.working);
    wait_for("b to work", 10, async || running_b().await.then_some(())).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(running_a(), 0, "a paused office starts nothing");

    // Resumed, it works too.
    core.update_settings(Settings { paused_offices: vec![], ..core.settings() }).await.unwrap();
    wait_for("a to work", 10, async || (running_a() == 1).then_some(())).await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_office_keeps_its_code_index() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (temp_repo(&dir.path().join("a")), temp_repo(&dir.path().join("b")));
    let log = Arc::new(Mutex::new(Log::default()));
    let found = Arc::new(Mutex::new(None));
    let seen = found.clone();
    // The worker searches the code (an index loads in the background: it asks for up to 5 s),
    // then stays on its task.
    let worker: Behavior = Arc::new(move |turn: FakeTurn| {
        let seen = seen.clone();
        Box::pin(async move {
            let mut reply = turn.tool("codly_search", json!({ "query": "demo" })).await;
            for _ in 0..50 {
                if reply["ok"] == true {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                reply = turn.tool("codly_search", json!({ "query": "demo" })).await;
            }
            *seen.lock().unwrap() = Some(reply);
            std::future::pending::<()>().await;
        })
    });
    let idle: Behavior = Arc::new(|turn: FakeTurn| Box::pin(async move { turn.say("ok") }));
    let spawner = || FakeSpawner { log: log.clone(), lead: idle.clone(), worker: worker.clone() };
    let core = start_full(dir.path(), spawner(), vec![claude()], true).await;
    core.update_settings(Settings { paused: true, ..core.settings() }).await.unwrap();

    // A task in "a"; "b" is the one left open when its worker starts.
    core.open_workspace(a.to_str().unwrap()).await.unwrap();
    let ta = core.create_task("Docs for a", "").await.unwrap();
    core.move_task(&ta.id, TaskStatus::Ready).await.unwrap();
    core.open_workspace(b.to_str().unwrap()).await.unwrap();
    core.update_settings(Settings { paused: false, ..core.settings() }).await.unwrap();

    let reply = wait_for("the worker's search", 20, async || found.lock().unwrap().clone()).await;
    assert_eq!(reply["ok"], true, "codly_search works in an office that is not the open one: {reply}");
    core.shutdown().await;
    drop(core);

    // A restart opens "b" only: "a" gets its index back when its task goes on.
    *found.lock().unwrap() = None;
    let core = start_full(dir.path(), spawner(), vec![claude()], true).await;
    let reply = wait_for("the worker's search after the restart", 20, async || found.lock().unwrap().clone()).await;
    assert_eq!(reply["ok"], true, "and after a restart: {reply}");
    core.shutdown().await;
}
