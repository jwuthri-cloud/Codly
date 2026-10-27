//! The Core facade: the only API the Tauri shell calls. It owns the store, the agent
//! actors, the Lead (supervisor), the scheduler, the gates and the approval broker.
//!
//! CONTRACT: the public method signatures of `Core` below are fixed (src-tauri is written
//! against them). Implementations live in sibling modules:
//! - `ingest`   the one event pipeline: persist, project agents, react (gates, approvals, chat)
//! - `runs`     spawning agent runs (the `Spawner` seam), prompts, the live-run table
//! - `lead`     the supervisors: one Lead per request thread (≤ 3 live), board header, follow-ups
//! - `plans`    submit/run/cancel plans, the scheduler loop, starting workers
//! - `gate`     commit → verify → scope → review → merge queue → done
//! - `tools`    the MCP `ToolHandler` (codly_* tools)
//! - `workspace` open a repo + index, snapshot, solo agents, approvals, diffs, merge_plan
//! - `accounts` the sign-in registry (several accounts per CLI), usage windows, account choice
//! - `team`     standing team members (per-repo agents, stable worktrees) and consultants
//! - pure, unit-tested policy: `planner`, `scheduler`, `approvals`, `scope`, `names`, `prompts`

pub mod accounts;
pub mod approvals;
pub mod agent_git;
mod gate;
mod ingest;
mod lead;
pub mod names;
pub mod planner;
mod plans;
pub mod prompts;
mod runs;
pub mod scheduler;
pub mod scope;
mod team;
mod tools;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Notify, broadcast, mpsc};

use crate::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use crate::git::Git;
use crate::index::Index;
use crate::mcp::McpServer;
use crate::memory::{self, MemoryView};
use crate::model::*;
use crate::store::{self, Store};

#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// App data dir: codly.sqlite, runs/<run_id>/raw.ndjson, worktrees/, index/, mcp.sock.
    pub data_dir: PathBuf,
    /// Path of the running Codly executable (agents launch `<exe> mcp` as their MCP server).
    pub app_exe: PathBuf,
}

pub type SpawnFuture = Pin<Box<dyn Future<Output = anyhow::Result<AgentHandle>> + Send>>;

/// Starts agent runs. The app uses the real CLI adapters; tests substitute fake agents.
pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture;
}

struct CliSpawner;

impl Spawner for CliSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        Box::pin(crate::adapters::spawn(spec, events))
    }
}

/// Seams for `Core::start_with` (tests, tools). `Core::start` = real env, CLIs and index.
#[derive(Default)]
pub struct StartOptions {
    /// None = the real adapters.
    pub spawner: Option<Arc<dyn Spawner>>,
    /// Child base environment. None = capture the login shell env (5 s timeout).
    pub env: Option<HashMap<String, String>>,
    /// CLI detection result. None = detect; Some = fixed (redetect returns it unchanged).
    pub clis: Option<Vec<CliInfo>>,
    /// Build and use the code index for opened workspaces.
    pub index: bool,
}

pub struct Core {
    inner: Arc<Inner>,
}

/// The index synchronizes itself (separate reader and writer connections), so searches
/// never wait behind a refresh.
type SharedIndex = Arc<Index>;

/// One running agent process (an adapter actor).
struct Live {
    handle: AgentHandle,
    run_id: Id,
    role: Role,
    provider: Provider,
    /// The model it was started with (the agent row holds the one the CLI reports).
    model: Option<String>,
    /// `AccountInfo::id` it runs on (None = no account detected for its CLI).
    account: Option<String>,
    task_id: Option<Id>,
    workspace_id: Option<Id>,
    cwd: PathBuf,
    /// Lead, reviewers and between-task chats: never auto-approve anything that writes.
    read_only: bool,
    token: String,
    /// Between TurnStarted and TurnCompleted.
    turn_active: bool,
    /// Prompts sent but not started yet (the adapter queues them; Codex steers instead).
    queued: u32,
    /// Codly asked it to shut down (task done, user stop, app exit).
    stopping: bool,
    /// Turns completed in this run (the gate checks no turn ended while it ran).
    turns: u64,
    /// Last assistant message of the current or last turn.
    last_text: Option<String>,
    turn_messages: u32,
    /// First line of the latest user message (solo commit messages).
    last_prompt: Option<String>,
    /// Lead messages (codly_message_member) whose answer goes back to that Lead.
    answers: Vec<Answer>,
    /// Started only to answer the Lead: shut down once every answer went out.
    answer_only: bool,
}

/// A member's reply the Lead of `thread` waits for: the reply of turn number `due`.
struct Answer {
    due: u64,
    ws_id: Id,
    thread: String,
}

struct Inner {
    cfg: CoreConfig,
    store: Store,
    bus: broadcast::Sender<EventRow>,
    events_tx: mpsc::UnboundedSender<AdapterEvent>,
    spawner: Arc<dyn Spawner>,
    mcp: OnceLock<Arc<McpServer>>,
    git: Git,
    base_env: HashMap<String, String>,
    clis: RwLock<Vec<CliInfo>>,
    fixed_clis: bool,
    settings: RwLock<Settings>,
    ws: RwLock<Option<Workspace>>,
    index: RwLock<Option<SharedIndex>>,
    use_index: bool,
    live: Mutex<HashMap<Id, Live>>,
    /// Serializes git mutations of the repository (worktrees, refs, commits, merges).
    repo_lock: tokio::sync::Mutex<()>,
    /// The merge queue: one merge into an integration branch at a time.
    merge_lock: tokio::sync::Mutex<()>,
    sched_lock: tokio::sync::Mutex<()>,
    /// Serializes Lead creation, spawning, eviction and delivery of queued Lead turns.
    lead_lock: tokio::sync::Mutex<()>,
    kick: Notify,
    /// Tasks whose gate is running (one per task at a time) → "run it again when done"
    /// (a worker turn ended while it ran).
    gating: Mutex<HashMap<Id, bool>>,
    /// Fix rounds used per task (verify, scope, review and conflict feedback); a user retry resets them.
    fix_rounds: Mutex<HashMap<Id, u32>>,
    /// Reminders sent to a reviewer run that ended without a verdict.
    review_nudges: Mutex<HashMap<Id, u32>>,
    /// Feedback for the next worker run of a task whose worker was not alive.
    pending_feedback: Mutex<HashMap<Id, String>>,
    /// The request threads' Leads: per-Lead turn queues (codly_ask_lead forwarding), LRU
    /// use, and turns waiting for a free Lead slot.
    leads: Mutex<lead::LeadPool>,
    shutting_down: AtomicBool,
    /// Set when memory::derive_triples is still a stub, to stop calling it.
    kg_off: AtomicBool,
    /// Agent names and task keys by id, for readable knowledge-graph entities.
    labels: Mutex<HashMap<Id, String>>,
    /// Every sign-in of every installed CLI, in your order (`limits` empty: see `quota`).
    accounts: RwLock<Vec<AccountInfo>>,
    /// Latest usage windows per account id (from RateLimit events; persisted in `kv`).
    quota: Mutex<HashMap<String, Vec<UsageWindow>>>,
    /// Artifact files already announced (absolute path + size), so rewrites re-announce.
    artifacts_seen: Mutex<HashSet<String>>,
    /// Model names per CLI (`env::list_models`), for the Lead's consultant choices.
    models: Mutex<HashMap<Provider, Vec<String>>>,
}

// ───────────── poison-tolerant locks: a panic elsewhere must not wedge the core ─────────────

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}
fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Run code from a module that may still be a stub (index, memory and env are built in
/// parallel) or may have a bug, without letting a panic escape into the orchestrator.
/// ponytail: keep as a safety net; drop when those modules have proven themselves.
fn guard<T>(what: &str, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
            Err(anyhow!("{what} unavailable ({msg})"))
        }
    }
}

fn is_stub(e: &anyhow::Error) -> bool {
    e.to_string().contains("not yet implemented")
}

fn row(agent: Option<&str>, task: Option<&str>, run: Option<&str>, kind: EventKind) -> EventRow {
    EventRow {
        id: 0,
        ts: now_ms(),
        agent_id: agent.map(String::from),
        task_id: task.map(String::from),
        run_id: run.map(String::from),
        kind,
    }
}

async fn capture_env() -> HashMap<String, String> {
    let job = tokio::task::spawn_blocking(|| crate::env::capture_login_env(Duration::from_secs(5)));
    let captured = match tokio::time::timeout(Duration::from_secs(6), job).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::warn!("login env capture unavailable: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("login env capture timed out");
            None
        }
    };
    guard("env::base_env", || Ok(crate::env::base_env(captured))).unwrap_or_else(|e| {
        tracing::warn!("{e}; using the process environment");
        std::env::vars().collect()
    })
}

async fn detect_clis(base: &HashMap<String, String>) -> Vec<CliInfo> {
    let base = base.clone();
    let job = tokio::spawn(async move { crate::env::detect_all(&base).await });
    match tokio::time::timeout(Duration::from_secs(30), job).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!("CLI detection unavailable: {e}");
            vec![]
        }
        Err(_) => {
            tracing::warn!("CLI detection timed out");
            vec![]
        }
    }
}

/// Settings' accounts as ids will see them: homes absolute (`~` expanded, no trailing slash;
/// empty = the default sign-in), none for CLIs without an account variable, labels trimmed,
/// one entry per sign-in (the first wins).
fn normalize_accounts(list: Vec<AccountConfig>, base: &HashMap<String, String>) -> anyhow::Result<Vec<AccountConfig>> {
    let mut out: Vec<AccountConfig> = vec![];
    for mut a in list {
        a.home = match a.home.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
            _ if crate::env::account_var(a.provider).is_none() => None,
            None => None,
            Some(h) => Some(
                crate::env::normalize_home(h, base)
                    .ok_or_else(|| anyhow!("the {} folder '{h}' must be an absolute path or start with ~/", a.provider.label()))?,
            ),
        };
        a.label = a.label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
        if !out.iter().any(|x| x.provider == a.provider && x.home == a.home) {
            out.push(a);
        }
    }
    Ok(out)
}

impl Core {
    /// Open the DB (migrate), capture the login env, detect CLIs, start the MCP server,
    /// reconcile runs left over from a crash, reopen the last workspace if any.
    pub async fn start(cfg: CoreConfig) -> anyhow::Result<Arc<Core>> {
        Core::start_with(cfg, StartOptions { index: true, ..Default::default() }).await
    }

    /// `start` with injectable seams (fake agents, fixed env and CLIs, no index).
    pub async fn start_with(cfg: CoreConfig, opts: StartOptions) -> anyhow::Result<Arc<Core>> {
        for d in ["runs", "worktrees", "index"] {
            std::fs::create_dir_all(cfg.data_dir.join(d)).with_context(|| format!("create {}", cfg.data_dir.join(d).display()))?;
        }
        // Other runs' MCP tokens and transcripts: owner-only, and never an agent's to read.
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(cfg.data_dir.join("runs"), std::fs::Permissions::from_mode(0o700))?;
        }
        approvals::protect_dir(&cfg.data_dir);
        let store = Store::open(&cfg.data_dir.join("codly.sqlite"))?;
        let settings = store.settings()?;
        let base_env = match opts.env {
            Some(e) => e,
            None => capture_env().await,
        };
        let git = Git::detect(base_env.clone());
        let fixed_clis = opts.clis.is_some();
        let clis = match opts.clis {
            Some(c) => c,
            None => detect_clis(&base_env).await,
        };
        // Fixed CLIs (tests) are not probed: every account takes its CLI's state.
        let accounts = accounts::detect(&base_env, &clis, &settings.accounts, !fixed_clis, None).await.unwrap_or_default();
        let quota = accounts::stored_limits(&store);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (bus, _) = broadcast::channel(4096);
        let inner = Arc::new(Inner {
            cfg: cfg.clone(),
            store,
            bus,
            events_tx,
            spawner: opts.spawner.unwrap_or_else(|| Arc::new(CliSpawner)),
            mcp: OnceLock::new(),
            git,
            base_env,
            clis: RwLock::new(clis),
            fixed_clis,
            settings: RwLock::new(settings),
            ws: RwLock::new(None),
            index: RwLock::new(None),
            use_index: opts.index,
            live: Mutex::new(HashMap::new()),
            repo_lock: tokio::sync::Mutex::new(()),
            merge_lock: tokio::sync::Mutex::new(()),
            sched_lock: tokio::sync::Mutex::new(()),
            lead_lock: tokio::sync::Mutex::new(()),
            kick: Notify::new(),
            gating: Mutex::new(HashMap::new()),
            fix_rounds: Mutex::new(HashMap::new()),
            review_nudges: Mutex::new(HashMap::new()),
            pending_feedback: Mutex::new(HashMap::new()),
            leads: Mutex::new(lead::LeadPool::default()),
            shutting_down: AtomicBool::new(false),
            kg_off: AtomicBool::new(false),
            labels: Mutex::new(HashMap::new()),
            accounts: RwLock::new(accounts),
            quota: Mutex::new(quota),
            artifacts_seen: Mutex::new(HashSet::new()),
            models: Mutex::new(HashMap::new()),
        });
        let mcp = McpServer::start(cfg.data_dir.join("mcp.sock"), tools::handler(Arc::downgrade(&inner))).await?;
        let _ = inner.mcp.set(mcp);
        inner.recover().await;
        tokio::spawn(ingest::run(inner.clone(), events_rx));
        tokio::spawn(plans::scheduler_loop(inner.clone()));
        match inner.store.last_workspace() {
            Ok(Some(ws)) => {
                if let Err(e) = inner.open_workspace(&ws.repo_path).await {
                    tracing::warn!("could not reopen {}: {e}", ws.repo_path);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("reading the last workspace failed: {e}"),
        }
        Ok(Arc::new(Core { inner }))
    }

    /// Live stream of every event: persisted rows (id > 0) and ephemeral deltas (id = 0).
    pub fn subscribe(&self) -> broadcast::Receiver<EventRow> {
        self.inner.bus.subscribe()
    }

    /// Full state for a (re)connecting UI. `as_of` = last persisted event id.
    pub async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        self.inner.snapshot().await
    }

    pub async fn redetect_clis(&self) -> anyhow::Result<Vec<CliInfo>> {
        let i = &self.inner;
        if !i.fixed_clis {
            let clis = detect_clis(&i.base_env).await;
            *write(&i.clis) = clis;
        }
        i.redetect_accounts(false).await;
        Ok(read(&i.clis).clone())
    }

    /// Every sign-in of every installed CLI, in your order, with the usage windows its agents
    /// last reported (`limits`). `refresh` = ask each CLI again who is signed in (≤ 5 s).
    pub async fn accounts(&self, refresh: bool) -> anyhow::Result<Vec<AccountInfo>> {
        if refresh {
            self.inner.redetect_accounts(false).await;
        }
        Ok(self.inner.accounts())
    }

    pub async fn recent_workspaces(&self) -> anyhow::Result<Vec<RecentWorkspace>> {
        self.inner.store.recent_workspaces(20)
    }

    /// Open (or create) the workspace for a git repo. Starts the Lead agent lazily on
    /// first chat; indexes the repo in the background.
    pub async fn open_workspace(&self, repo_path: &str) -> anyhow::Result<Workspace> {
        self.inner.open_workspace(repo_path).await
    }

    pub async fn update_settings(&self, settings: Settings) -> anyhow::Result<Settings> {
        let mut s = settings;
        s.max_parallel = s.max_parallel.clamp(1, 8);
        s.max_per_provider = s.max_per_provider.clamp(1, s.max_parallel);
        let mut seen = HashSet::new();
        s.worker_providers.retain(|p| seen.insert(*p));
        s.accounts = normalize_accounts(std::mem::take(&mut s.accounts), &self.inner.base_env)?;
        s.lead_account = s.lead_account.filter(|a| !a.trim().is_empty());
        let repo_members = self.inner.workspace().map(|w| self.inner.store.workspace_team(&w.id)).transpose()?.unwrap_or_default().members;
        s.team = team::normalize_team(std::mem::take(&mut s.team), &repo_members)?;
        let accounts_changed = self.inner.settings().accounts != s.accounts;
        let team_changed = self.inner.settings().team != s.team;
        self.inner.store.save_settings(&s)?;
        *write(&self.inner.settings) = s.clone();
        if accounts_changed {
            self.inner.redetect_accounts(true).await; // only a newly listed home is probed
        }
        if team_changed && let Some(ws) = self.inner.workspace() {
            self.inner.sync_team(&ws)?;
        }
        self.inner.kick();
        Ok(s)
    }

    /// The open repo's changes to the global team (switch members off, add its own).
    pub async fn set_workspace_team(&self, team: WorkspaceTeam) -> anyhow::Result<WorkspaceTeam> {
        self.inner.set_workspace_team(team)
    }

    /// thread = "lead" or "lead:<id>" → a turn for that request thread's own Lead (spawned
    /// on demand; at most 3 Leads live, a 4th waits for a free slot);
    /// thread = "agent:<id>" → a 1:1 turn for that agent (personal chat).
    /// Returns once the message is recorded and handed over, never after the agent's turn.
    pub async fn send_chat(&self, thread: &str, text: &str) -> anyhow::Result<()> {
        let text = text.trim();
        if text.is_empty() {
            bail!("empty message");
        }
        if is_lead_thread(thread) {
            if thread.strip_prefix("lead:").is_some_and(|id| id.trim().is_empty()) {
                bail!("unknown thread {thread}");
            }
            return self.inner.chat_lead(thread, text).await;
        }
        let Some(agent_id) = thread.strip_prefix("agent:") else { bail!("unknown thread {thread}") };
        self.inner.chat_agent(agent_id, text).await
    }

    /// Approve a plan the Lead submitted: create the integration branch and start scheduling.
    pub async fn run_plan(&self, plan_id: &str) -> anyhow::Result<()> {
        self.inner.run_plan(plan_id).await
    }

    pub async fn cancel_plan(&self, plan_id: &str) -> anyhow::Result<()> {
        self.inner.cancel_plan(plan_id).await
    }

    /// Close a request thread: its Lead stops, its unfinished plans are cancelled and it leaves
    /// the tabs (archived as "closed:<thread>:<ms>", nothing deleted). Reload the snapshot after.
    pub async fn close_thread(&self, thread: &str) -> anyhow::Result<()> {
        self.inner.close_thread(thread).await
    }

    pub async fn resolve_approval(&self, approval_id: &str, decision: ApprovalDecision) -> anyhow::Result<()> {
        self.inner.resolve_approval(approval_id, decision, "user")
    }

    pub async fn interrupt_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        let live = lock(&self.inner.live);
        let l = live.get(agent_id).ok_or_else(|| anyhow!("that agent is not running"))?;
        if !l.handle.send(AgentCmd::Interrupt) {
            bail!("that agent's process is gone");
        }
        Ok(())
    }

    pub async fn stop_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        self.inner.stop_agent(agent_id).await
    }

    /// Create a Solo agent for 1:1 chat ("hire").
    /// `title`: one of `ROLE_TITLES` (its badge), or None.
    pub async fn hire_agent(&self, provider: Provider, model: Option<String>, title: Option<String>) -> anyhow::Result<Agent> {
        self.inner.hire_agent(provider, model, title).await
    }

    /// Switch a 1:1 agent to another model (None = the CLI's default).
    pub async fn set_agent_model(&self, agent_id: &str, model: Option<String>) -> anyhow::Result<Agent> {
        self.inner.set_agent_model(agent_id, model).await
    }

    /// Models a CLI can run, for the start-agent picker (empty when it takes any name).
    pub async fn list_models(&self, provider: Provider) -> anyhow::Result<Vec<String>> {
        crate::env::list_models(provider, &self.inner.base_env).await.map_err(|e| anyhow::anyhow!(e))
    }

    /// Ad-hoc task on the shared board (no plan). Runs when moved to Ready or assigned.
    pub async fn create_task(&self, title: &str, goal: &str) -> anyhow::Result<Task> {
        self.inner.create_task(title, goal)
    }

    /// Kanban drag-and-drop. Only user-legal transitions (e.g. backlog ↔ ready, cancel, retry).
    pub async fn move_task(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<Task> {
        self.inner.move_task(task_id, status).await
    }

    /// Drag an agent onto a task (or None to unassign).
    pub async fn assign_task(&self, task_id: &str, agent_id: Option<&str>) -> anyhow::Result<Task> {
        self.inner.assign_task(task_id, agent_id)
    }

    /// Unified diff of a task branch against its base.
    pub async fn task_diff(&self, task_id: &str) -> anyhow::Result<String> {
        self.inner.task_diff(task_id).await
    }

    /// Unified diff of the plan's integration branch against the plan base.
    pub async fn plan_diff(&self, plan_id: &str) -> anyhow::Result<String> {
        self.inner.plan_diff(plan_id).await
    }

    /// Merge the plan's integration branch into the user's current branch (refuses if the
    /// main checkout is dirty). Returns the new HEAD sha.
    pub async fn merge_plan(&self, plan_id: &str) -> anyhow::Result<String> {
        self.inner.merge_plan(plan_id).await
    }

    /// The user keeps the plan's integration branch instead of merging it.
    pub async fn keep_plan(&self, plan_id: &str) -> anyhow::Result<Plan> {
        self.inner.update_plan(plan_id, |p| {
            p.kept = true;
            Ok(())
        })
    }

    /// Pin or unpin a memory (pinned memories always make the memory pack).
    pub async fn pin_memory(&self, id: &str, pinned: bool) -> anyhow::Result<()> {
        self.inner.store.with(|db| guard("memory", || memory::set_pinned(db, id, pinned)))
    }

    /// Paged history (newest first) for the timeline / inspector.
    pub async fn events(&self, agent_id: Option<&str>, before: Option<i64>, limit: i64) -> anyhow::Result<Vec<EventRow>> {
        let ws = self.inner.workspace().map(|w| w.id);
        self.inner.store.events(ws.as_deref(), agent_id, before, limit)
    }

    /// Code search over the workspace index (the same engine as the codly_search tool).
    pub async fn search(&self, query: &str) -> anyhow::Result<SearchResult> {
        self.inner.search(query, 12).await
    }

    pub async fn memories(&self, query: Option<&str>, scope: Option<MemoryScope>) -> anyhow::Result<Vec<Memory>> {
        let ws = self.inner.workspace().map(|w| w.id);
        let view = MemoryView { workspace_id: ws.as_deref(), agent_id: None, task_ids: &[] };
        self.inner.store.with(|db| {
            guard("memory", || match query.map(str::trim).filter(|q| !q.is_empty()) {
                Some(q) => memory::search_memories(db, &view, q, scope, 100),
                None => memory::list_memories(db, &view, scope, 200),
            })
        })
    }

    pub async fn add_memory(&self, text: &str, scope: MemoryScope, kind: MemoryKind) -> anyhow::Result<Memory> {
        let ws = self.inner.workspace().map(|w| w.id);
        self.inner.add_memory(ws.as_deref(), scope, None, kind, text, "user", None, 0.7)
    }

    pub async fn delete_memory(&self, id: &str) -> anyhow::Result<()> {
        self.inner.store.with(|db| guard("memory", || memory::delete_memory(db, id)))
    }

    pub async fn triples(&self, entity: Option<&str>) -> anyhow::Result<Vec<Triple>> {
        let ws = self.inner.workspace().map(|w| w.id);
        self.inner.store.with(|db| guard("knowledge graph", || memory::query_triples(db, ws.as_deref(), entity, None, 500)))
    }

    /// Files of the open workspace for the 3D codebase city (from the code index).
    pub async fn repo_map(&self) -> anyhow::Result<Vec<RepoFile>> {
        let index = self.inner.index.read().map_err(|_| anyhow!("index lock poisoned"))?.clone();
        match index {
            Some(ix) => tokio::task::spawn_blocking(move || ix.files()).await?,
            None => Ok(Vec::new()),
        }
    }

    /// Bytes and MIME type of a recorded artifact. Only files Codly announced in an
    /// `artifact_added` event can be read, so the UI cannot be used to read arbitrary files.
    pub async fn read_artifact(&self, id: &str) -> anyhow::Result<(Vec<u8>, String)> {
        let a = self.inner.store.artifact(id)?.ok_or_else(|| anyhow!("unknown artifact {id}"))?;
        let bytes = tokio::fs::read(&a.abs_path).await.with_context(|| format!("reading {}", a.path))?;
        Ok((bytes, a.mime))
    }

    /// Stop every agent (shutdown ladder) and flush the store. Called on app exit.
    pub async fn shutdown(&self) {
        self.inner.shutdown().await
    }
}

impl Inner {
    fn kick(&self) {
        self.kick.notify_one();
    }

    fn settings(&self) -> Settings {
        read(&self.settings).clone()
    }

    fn clis(&self) -> Vec<CliInfo> {
        read(&self.clis).clone()
    }

    fn workspace(&self) -> Option<Workspace> {
        read(&self.ws).clone()
    }

    fn require_ws(&self) -> anyhow::Result<Workspace> {
        self.workspace().ok_or_else(|| anyhow!("open a repository first"))
    }

    fn index(&self) -> Option<SharedIndex> {
        read(&self.index).clone()
    }

    fn is_live(&self, agent_id: &str) -> bool {
        lock(&self.live).contains_key(agent_id)
    }

    fn data(&self, rel: &str) -> PathBuf {
        self.cfg.data_dir.join(rel)
    }

    /// `<data>/worktrees/<workspace>/` — outside the repo, so the repo watcher and agents'
    /// globbing never see other worktrees.
    fn worktree_root(&self, ws: &Workspace) -> PathBuf {
        let short = &ws.id[ws.id.len().saturating_sub(6)..];
        self.data("worktrees").join(format!("{}-{short}", names::slug(&ws.name, 24)))
    }

    // ───────────── events ─────────────

    /// Persist (unless ephemeral) and broadcast one event.
    fn emit(&self, ws: Option<&str>, mut ev: EventRow) -> anyhow::Result<EventRow> {
        if !ev.kind.is_ephemeral() {
            ev.id = self.store.append_event(ws, &ev)?;
        }
        self.publish(ws, &ev);
        Ok(ev)
    }

    fn decide(&self, ws: Option<&str>, task_id: Option<&str>, kind: &str, summary: impl Into<String>, detail: Value) -> anyhow::Result<Id> {
        self.decide_by(ws, task_id, kind, "rule", summary, detail)
    }

    fn decide_by(
        &self,
        ws: Option<&str>,
        task_id: Option<&str>,
        kind: &str,
        method: &str,
        summary: impl Into<String>,
        detail: Value,
    ) -> anyhow::Result<Id> {
        let id = new_id();
        let kind =
            EventKind::Decision { id: id.clone(), decision_type: kind.into(), method: method.into(), summary: summary.into(), detail };
        self.emit(ws, row(None, task_id, None, kind))?;
        Ok(id)
    }

    fn chat(&self, ws: Option<&str>, thread: &str, author: &str, text: &str, attachment: Option<Value>) -> anyhow::Result<EventRow> {
        let message =
            ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: text.into(), attachment, ts: now_ms() };
        // Attributed to the agent of a 1:1 thread, or to the agent who wrote it (the Lead).
        let agent = thread.strip_prefix("agent:").or((author != "user" && author != "codly").then_some(author));
        self.emit(ws, row(agent, None, None, EventKind::Chat { message }))
    }

    /// Load-modify-save one JSON projection row atomically (one store transaction), emitting
    /// the change event only if something changed.
    fn update_row<T: Serialize + DeserializeOwned>(
        &self,
        table: &str,
        id: &str,
        f: impl FnOnce(&mut T) -> anyhow::Result<()>,
        wrap: impl FnOnce(&T) -> EventRow,
        touch: impl FnOnce(&mut T),
    ) -> anyhow::Result<T> {
        let (value, ev) = self.store.tx(|tx| {
            let (json, ws): (String, Option<String>) = tx
                .query_row(&format!("SELECT json, workspace_id FROM {table} WHERE id = ?1"), [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .ok_or_else(|| anyhow!("unknown {table} {id}"))?;
            let mut v: T = serde_json::from_str(&json)?;
            let before = serde_json::to_value(&v)?;
            f(&mut v)?;
            if serde_json::to_value(&v)? == before {
                return Ok((v, None));
            }
            touch(&mut v);
            let mut ev = wrap(&v);
            ev.id = store::insert_event(tx, ws.as_deref(), &ev)?;
            Ok((v, Some((ev, ws))))
        })?;
        if let Some((ev, ws)) = ev {
            self.publish(ws.as_deref(), &ev);
        }
        Ok(value)
    }

    /// Codly's own task transitions. A cancelled or done task is final for Codly: an
    /// in-flight gate or run can never bring it back (only the user can, via `move_task`).
    fn update_task(&self, id: &str, f: impl FnOnce(&mut Task) -> anyhow::Result<()>) -> anyhow::Result<Task> {
        self.update_task_as(id, false, f)
    }

    fn update_task_as(&self, id: &str, by_user: bool, f: impl FnOnce(&mut Task) -> anyhow::Result<()>) -> anyhow::Result<Task> {
        self.update_row(
            "task",
            id,
            |t: &mut Task| {
                let was = t.status;
                f(t)?;
                if !by_user && t.status != was && matches!(was, TaskStatus::Cancelled | TaskStatus::Done) {
                    bail!("task {} is {}", t.key, store::enum_str(&was));
                }
                Ok(())
            },
            |t: &Task| row(t.assignee.as_deref(), Some(&t.id), None, EventKind::TaskChanged { task: t.clone() }),
            |t| t.updated_at = now_ms(),
        )
    }

    fn update_agent(&self, id: &str, f: impl FnOnce(&mut Agent)) -> anyhow::Result<Agent> {
        self.update_row(
            "agent",
            id,
            |a| {
                f(a);
                Ok(())
            },
            |a: &Agent| row(Some(&a.id), a.task_id.as_deref(), a.run_id.as_deref(), EventKind::AgentChanged { agent: a.clone() }),
            |a| a.updated_at = now_ms(),
        )
    }

    fn update_plan(&self, id: &str, f: impl FnOnce(&mut Plan) -> anyhow::Result<()>) -> anyhow::Result<Plan> {
        self.update_row(
            "plan",
            id,
            f,
            |p: &Plan| row(None, None, None, EventKind::PlanChanged { plan: p.clone() }),
            |p| p.updated_at = now_ms(),
        )
    }

    fn task(&self, id: &str) -> anyhow::Result<Task> {
        self.store.task(id)?.ok_or_else(|| anyhow!("unknown task {id}"))
    }

    fn agent(&self, id: &str) -> anyhow::Result<Agent> {
        self.store.agent(id)?.ok_or_else(|| anyhow!("unknown agent {id}"))
    }

    fn plan(&self, id: &str) -> anyhow::Result<Plan> {
        self.store.plan(id)?.ok_or_else(|| anyhow!("unknown plan {id}"))
    }

    /// A new agent character (persisted with an AgentChanged event).
    fn create_agent(
        &self,
        ws: &str,
        name: &str,
        provider: Provider,
        role: Role,
        color: i64,
        model: Option<String>,
    ) -> anyhow::Result<Agent> {
        self.insert_agent(ws, self.new_agent(name, provider, role, color, model))
    }

    fn insert_agent(&self, ws: &str, agent: Agent) -> anyhow::Result<Agent> {
        self.emit(Some(ws), row(Some(&agent.id), None, None, EventKind::AgentChanged { agent: agent.clone() }))?;
        Ok(agent)
    }

    /// An agent row not persisted yet (`insert_agent` persists it). Its account (and billing
    /// badge) is set when a run starts.
    fn new_agent(&self, name: &str, provider: Provider, role: Role, color: i64, model: Option<String>) -> Agent {
        let billing = self.clis().iter().find(|c| c.provider == provider).map_or(Billing::Unknown, |c| c.billing);
        let id = new_id();
        Agent {
            id: id.clone(),
            name: name.into(),
            provider,
            model,
            role,
            status: AgentStatus::Idle,
            activity: None,
            activity_label: None,
            task_id: None,
            run_id: None,
            session_id: None,
            worktree: None,
            billing,
            account: None,
            color,
            look_seed: id.clone(),
            thread: None,
            member: None,
            title: None,
            consultant: false,
            archived: false,
            usage: Usage::default(),
            created_at: now_ms(),
            updated_at: now_ms(),
        }
    }

    // ───────────── memory (through crate::memory) ─────────────

    #[allow(clippy::too_many_arguments)]
    fn add_memory(
        &self,
        ws: Option<&str>,
        scope: MemoryScope,
        scope_id: Option<&str>,
        kind: MemoryKind,
        text: &str,
        author: &str,
        source_event: Option<i64>,
        importance: f64,
    ) -> anyhow::Result<Memory> {
        let text = text.trim();
        if text.is_empty() {
            bail!("memory text is empty");
        }
        let m = Memory {
            id: new_id(),
            workspace_id: if scope == MemoryScope::Global { None } else { ws.map(String::from) },
            scope,
            scope_id: scope_id.map(String::from),
            kind,
            text: prompts::clip(text, 2000),
            author: author.into(),
            source_event,
            importance,
            pinned: false,
            superseded_by: None,
            created_at: now_ms(),
            accessed_at: now_ms(),
        };
        let agent = (author != "user" && author != "codly").then_some(author);
        let ev = row(agent, if scope == MemoryScope::Task { scope_id } else { None }, None, EventKind::MemoryAdded { memory: m.clone() });
        let ev = self.store.tx(|tx| {
            // The event is the record; the memory module's table is its searchable projection.
            // ponytail: while that module is a stub, keep the event (the blackboard reads events).
            if let Err(e) = guard("memory", || memory::add_memory(tx, &m)) {
                if !is_stub(&e) {
                    return Err(e);
                }
                tracing::debug!("{e}");
            }
            let mut ev = ev;
            ev.id = store::insert_event(tx, ws, &ev)?;
            Ok(ev)
        })?;
        self.publish(ws, &ev);
        Ok(m)
    }

    fn add_triple(&self, ws: Option<&str>, mut t: Triple, agent: Option<&str>) -> anyhow::Result<Option<Triple>> {
        if t.id.is_empty() {
            t.id = new_id();
        }
        if t.valid_from == 0 {
            t.valid_from = now_ms();
        }
        if t.workspace_id.is_none() {
            t.workspace_id = ws.map(String::from);
        }
        let ev = self.store.tx(|tx| {
            let Some(added) = guard("knowledge graph", || memory::add_triple(tx, &t))? else { return Ok(None) };
            let mut ev = row(agent, None, None, EventKind::TripleAdded { triple: added.clone() });
            ev.id = store::insert_event(tx, ws, &ev)?;
            Ok(Some((added, ev)))
        })?;
        Ok(ev.map(|(t, ev)| {
            self.publish(ws, &ev);
            t
        }))
    }
}
