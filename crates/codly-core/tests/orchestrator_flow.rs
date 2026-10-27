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

async fn start(root: &Path, spawner: FakeSpawner) -> Arc<Core> {
    start_with_clis(root, spawner, vec![claude()]).await
}

async fn start_with_clis(root: &Path, spawner: FakeSpawner, clis: Vec<CliInfo>) -> Arc<Core> {
    start_full(root, spawner, clis, false).await
}

async fn start_full(root: &Path, spawner: FakeSpawner, clis: Vec<CliInfo>, index: bool) -> Arc<Core> {
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

    // A dirty checkout is refused.
    std::fs::write(repo.join("README.md"), "changed\n").unwrap();
    let err = core.merge_plan(&plan.id).await.unwrap_err().to_string();
    assert!(err.contains("uncommitted changes"), "{err}");
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
        let ev = |kind| EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind };
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
            archived: false,
            usage: Usage::default(),
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
    let agent = snap.agents.iter().find(|a| a.id == agent_id).unwrap();
    assert_eq!(agent.task_id, None);

    let store = Store::open(&data.join("codly.sqlite")).unwrap();
    assert_eq!(store.run(&run_id).unwrap().unwrap().status, "lost");
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
    send(core.add_memory("m", MemoryScope::Project, MemoryKind::Fact));
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
        let ev = |kind| EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind };
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
    let ada = snap.agents.iter().find(|a| a.id == agent_id).unwrap();
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
