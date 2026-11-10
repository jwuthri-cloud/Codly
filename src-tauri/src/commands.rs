//! Tauri commands: thin async wrappers over `codly_core::Core` (one per `api.*` call in
//! src/lib/api.ts). Every command is async so nothing runs on the main thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use codly_core::Core;
use codly_core::model::*;
use codly_core::orchestrator::breaker::BreakerConfig;
use codly_core::orchestrator::BreakerStatus;
use codly_core::pty::PtyManager;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::{OnceCell, watch};

use crate::ui_bus::{UiBatch, UiBus};

pub struct AppState {
    /// None until `Core::start` finishes; then Ok(core) or the startup error.
    pub core: watch::Receiver<Option<Result<Arc<Core>, String>>>,
    pub bus: Arc<UiBus>,
    pub pty: PtyManager,
    /// Login-shell environment for terminals (captured once, off the main thread).
    pub shell_env: OnceCell<HashMap<String, String>>,
}

impl AppState {
    pub async fn core(&self) -> Result<Arc<Core>, String> {
        let mut rx = self.core.clone();
        let ready = rx.wait_for(|c| c.is_some()).await.map_err(|e| e.to_string())?;
        ready.clone().expect("checked is_some")
    }

    pub(crate) async fn shell_env(&self) -> &HashMap<String, String> {
        self.shell_env
            .get_or_init(|| async {
                tokio::task::spawn_blocking(|| {
                    let captured = codly_core::env::capture_login_env(std::time::Duration::from_secs(5));
                    codly_core::env::base_env(captured)
                })
                .await
                .unwrap_or_else(|_| std::env::vars().collect())
            })
            .await
    }
}

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn ui_subscribe(state: State<'_, AppState>, on_batch: Channel<UiBatch>) -> R<Snapshot> {
    let core = state.core().await?;
    state.bus.set_sink(on_batch); // 1) start receiving deltas
    state.bus.visible.store(true, Ordering::Relaxed);
    core.snapshot().await.map_err(err) // 2) then the snapshot; the UI drops rows with id <= as_of
}

#[tauri::command]
pub async fn ui_set_visible(state: State<'_, AppState>, visible: bool) -> R<()> {
    state.bus.visible.store(visible, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn detect_clis(state: State<'_, AppState>) -> R<Vec<CliInfo>> {
    state.core().await?.redetect_clis().await.map_err(err)
}

/// Every sign-in of every CLI with its usage windows; `refresh` re-probes who is signed in.
#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>, refresh: bool) -> R<Vec<AccountInfo>> {
    state.core().await?.accounts(refresh).await.map_err(err)
}

#[tauri::command]
pub async fn recent_workspaces(state: State<'_, AppState>) -> R<Vec<RecentWorkspace>> {
    state.core().await?.recent_workspaces().await.map_err(err)
}

/// The git repositories under the home folder, newest first: the browser's "Open repository"
/// picker (`codly serve` cannot show a folder dialog). Three folders deep, at most 200.
#[tauri::command]
pub async fn list_repos() -> R<Vec<codly_core::repos::RepoEntry>> {
    let home = codly_core::skills::home_dir().map_err(err)?;
    tokio::task::spawn_blocking(move || codly_core::repos::find_repos(&home, 3, 200)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_workspace(state: State<'_, AppState>, repo_path: String) -> R<Workspace> {
    state.core().await?.open_workspace(&repo_path).await.map_err(err)
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: Settings) -> R<Settings> {
    state.core().await?.update_settings(settings).await.map_err(err)
}

#[tauri::command]
pub async fn send_chat(state: State<'_, AppState>, thread: String, text: String) -> R<()> {
    state.core().await?.send_chat(&thread, &text).await.map_err(err)
}

#[tauri::command]
pub async fn run_plan(state: State<'_, AppState>, plan_id: String) -> R<()> {
    state.core().await?.run_plan(&plan_id).await.map_err(err)
}

#[tauri::command]
pub async fn cancel_plan(state: State<'_, AppState>, plan_id: String) -> R<()> {
    state.core().await?.cancel_plan(&plan_id).await.map_err(err)
}

#[tauri::command]
pub async fn close_thread(state: State<'_, AppState>, thread: String) -> R<()> {
    state.core().await?.close_thread(&thread).await.map_err(err)
}

#[tauri::command]
pub async fn resolve_approval(state: State<'_, AppState>, approval_id: String, decision: ApprovalDecision) -> R<()> {
    state.core().await?.resolve_approval(&approval_id, decision).await.map_err(err)
}

#[tauri::command]
pub async fn interrupt_agent(state: State<'_, AppState>, agent_id: String) -> R<()> {
    state.core().await?.interrupt_agent(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn stop_agent(state: State<'_, AppState>, agent_id: String) -> R<()> {
    state.core().await?.stop_agent(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn hire_agent(state: State<'_, AppState>, provider: Provider, model: Option<String>, title: Option<String>) -> R<Agent> {
    state.core().await?.hire_agent(provider, model, title).await.map_err(err)
}

#[tauri::command]
pub async fn set_request_prefs(state: State<'_, AppState>, thread: String, prefs: RequestPrefs) -> R<RequestPrefs> {
    state.core().await?.set_request_prefs(&thread, prefs).await.map_err(err)
}

#[tauri::command]
pub async fn set_workspace_team(state: State<'_, AppState>, team: WorkspaceTeam) -> R<WorkspaceTeam> {
    state.core().await?.set_workspace_team(team).await.map_err(err)
}

#[tauri::command]
pub async fn list_models(state: State<'_, AppState>, provider: Provider) -> R<Vec<String>> {
    state.core().await?.list_models(provider).await.map_err(err)
}

#[tauri::command]
pub async fn set_agent_model(state: State<'_, AppState>, agent_id: String, model: Option<String>) -> R<Agent> {
    state.core().await?.set_agent_model(&agent_id, model).await.map_err(err)
}

#[tauri::command]
pub async fn set_agent_look(state: State<'_, AppState>, agent_id: String, look_seed: String) -> R<Agent> {
    state.core().await?.set_agent_look(&agent_id, &look_seed).await.map_err(err)
}

#[tauri::command]
pub async fn create_task(state: State<'_, AppState>, title: String, goal: String) -> R<Task> {
    state.core().await?.create_task(&title, &goal).await.map_err(err)
}

#[tauri::command]
pub async fn move_task(state: State<'_, AppState>, task_id: String, status: TaskStatus) -> R<Task> {
    state.core().await?.move_task(&task_id, status).await.map_err(err)
}

#[tauri::command]
pub async fn assign_task(state: State<'_, AppState>, task_id: String, agent_id: Option<String>) -> R<Task> {
    state.core().await?.assign_task(&task_id, agent_id.as_deref()).await.map_err(err)
}

#[tauri::command]
pub async fn task_diff(state: State<'_, AppState>, task_id: String) -> R<String> {
    state.core().await?.task_diff(&task_id).await.map_err(err)
}

#[tauri::command]
pub async fn plan_diff(state: State<'_, AppState>, plan_id: String) -> R<String> {
    state.core().await?.plan_diff(&plan_id).await.map_err(err)
}

#[tauri::command]
pub async fn merge_plan(state: State<'_, AppState>, plan_id: String) -> R<String> {
    state.core().await?.merge_plan(&plan_id).await.map_err(err)
}

#[tauri::command]
pub async fn get_events(state: State<'_, AppState>, agent_id: Option<String>, before: Option<i64>, limit: i64) -> R<Vec<EventRow>> {
    state.core().await?.events(agent_id.as_deref(), before, limit.clamp(1, 5000)).await.map_err(err)
}

#[tauri::command]
pub async fn search_code(state: State<'_, AppState>, query: String) -> R<SearchResult> {
    state.core().await?.search(&query).await.map_err(err)
}

#[tauri::command]
pub async fn list_memories(state: State<'_, AppState>, query: Option<String>, scope: Option<MemoryScope>) -> R<Vec<Memory>> {
    state.core().await?.memories(query.as_deref(), scope).await.map_err(err)
}

#[tauri::command]
pub async fn add_memory(
    state: State<'_, AppState>,
    text: String,
    scope: MemoryScope,
    kind: MemoryKind,
    title: Option<String>,
    tags: Option<Vec<String>>,
) -> R<Memory> {
    let meta = MemoryMeta { title: title.unwrap_or_default(), tags: tags.unwrap_or_default(), refs: Vec::new() };
    state.core().await?.add_memory(&text, scope, kind, meta).await.map_err(err)
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, AppState>, id: String) -> R<()> {
    state.core().await?.delete_memory(&id).await.map_err(err)
}

#[tauri::command]
pub async fn list_triples(state: State<'_, AppState>, entity: Option<String>) -> R<Vec<Triple>> {
    state.core().await?.triples(entity.as_deref()).await.map_err(err)
}

#[tauri::command]
pub async fn keep_plan(state: State<'_, AppState>, plan_id: String) -> R<Plan> {
    state.core().await?.keep_plan(&plan_id).await.map_err(err)
}

#[tauri::command]
pub async fn pin_memory(state: State<'_, AppState>, id: String, pinned: bool) -> R<()> {
    state.core().await?.pin_memory(&id, pinned).await.map_err(err)
}

#[tauri::command]
pub async fn repo_map(state: State<'_, AppState>) -> R<Vec<RepoFile>> {
    state.core().await?.repo_map().await.map_err(err)
}

/// Raw bytes of a recorded artifact (an ArrayBuffer in JS; the MIME type is on the Artifact).
#[tauri::command]
pub async fn read_artifact(state: State<'_, AppState>, artifact_id: String) -> R<tauri::ipc::Response> {
    let (bytes, _mime) = state.core().await?.read_artifact(&artifact_id).await.map_err(err)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// A pasted or dropped file: the raw bytes are the request body, its name an `x-name`
/// header (URI-encoded). Returns the saved absolute path.
#[tauri::command]
pub async fn save_attachment(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> R<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("expected the file's bytes".into()) };
    let name = request.headers().get("x-name").and_then(|v| v.to_str().ok()).map(uri_decode).unwrap_or_default();
    state.core().await?.save_attachment(&name, bytes.clone()).await.map_err(err)
}

/// Percent-decoding for the `x-name` header (lossy on malformed input).
fn uri_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
pub async fn pty_open(state: State<'_, AppState>, cwd: Option<String>) -> R<String> {
    let env = state.shell_env().await.clone();
    let cwd = cwd
        .filter(|c| std::path::Path::new(c).is_dir())
        .or_else(|| env.get("HOME").cloned())
        .unwrap_or_else(|| "/".into());
    state.pty.open(std::path::Path::new(&cwd), &env).map_err(err)
}

#[tauri::command]
pub async fn pty_attach(state: State<'_, AppState>, pty_id: String, on_data: Channel<InvokeResponseBody>) -> R<()> {
    state
        .pty
        .attach(&pty_id, Box::new(move |bytes| { let _ = on_data.send(InvokeResponseBody::Raw(bytes)); }))
        .map_err(err)
}

#[tauri::command]
pub async fn pty_write(state: State<'_, AppState>, pty_id: String, data: String) -> R<()> {
    state.pty.write(&pty_id, data.as_bytes()).map_err(err)
}

#[tauri::command]
pub async fn pty_resize(state: State<'_, AppState>, pty_id: String, cols: u16, rows: u16) -> R<()> {
    state.pty.resize(&pty_id, cols.max(2), rows.max(1)).map_err(err)
}

#[tauri::command]
pub async fn pty_close(state: State<'_, AppState>, pty_id: String) -> R<()> {
    state.pty.close(&pty_id);
    Ok(())
}

/// Read-only git state (status, branches, log) of an agent's worktree: the Git tab.
#[tauri::command]
pub async fn agent_git(state: State<'_, AppState>, agent_id: String) -> R<codly_core::orchestrator::agent_git::AgentGit> {
    state.core().await?.agent_git(&agent_id).await.map_err(err)
}

/// Triggers: schedules, context rules and network mode, with the log of what they did.
#[tauri::command]
pub async fn get_triggers(state: State<'_, AppState>) -> R<codly_core::triggers::TriggersConfig> {
    state.core().await?.triggers().await.map_err(err)
}

#[tauri::command]
pub async fn set_triggers(state: State<'_, AppState>, config: codly_core::triggers::TriggersConfig) -> R<codly_core::triggers::TriggersConfig> {
    state.core().await?.set_triggers(config).await.map_err(err)
}

// ───────────────────────────── skills manager ─────────────────────────────
// Blocking filesystem/git work, so each runs on the blocking pool. `project` is the open repo.

async fn skills_blocking<T: Send + 'static>(f: impl FnOnce(std::path::PathBuf) -> anyhow::Result<T> + Send + 'static) -> R<T> {
    tokio::task::spawn_blocking(move || f(codly_core::skills::home_dir()?))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[tauri::command]
pub async fn list_skills(project: Option<String>) -> R<Vec<codly_core::skills::Skill>> {
    skills_blocking(move |home| Ok(codly_core::skills::list(&home, project.as_deref().map(std::path::Path::new)))).await
}

#[tauri::command]
pub async fn skill_catalog() -> R<Vec<codly_core::skills::CatalogSkill>> {
    skills_blocking(|home| Ok(codly_core::skills::catalog(&home))).await
}

#[tauri::command]
pub async fn install_skill(id: String) -> R<codly_core::skills::Skill> {
    skills_blocking(move |home| codly_core::skills::install(&id, &home)).await
}

#[tauri::command]
pub async fn uninstall_skill(path: String, project: Option<String>) -> R<()> {
    skills_blocking(move |home| codly_core::skills::uninstall(std::path::Path::new(&path), &home, project.as_deref().map(std::path::Path::new))).await
}

#[tauri::command]
pub async fn open_skill_folder(path: String, project: Option<String>) -> R<()> {
    skills_blocking(move |home| codly_core::skills::open_folder(std::path::Path::new(&path), &home, project.as_deref().map(std::path::Path::new))).await
}

/// Literal text search across the board, tasks and memories (the Find lens).
#[tauri::command]
pub async fn exact_search(state: State<'_, AppState>, query: String) -> R<Vec<codly_core::memory::panel::ExactHit>> {
    state.core().await?.exact_search(&query).await.map_err(err)
}

/// Agents, and the messages between them by act (the Messages lens).
#[tauri::command]
pub async fn message_graph(state: State<'_, AppState>) -> R<codly_core::memory::panel::MessageGraph> {
    state.core().await?.message_graph().await.map_err(err)
}

/// One agent's memory file; the core resolves the path from the agent id, never from the UI.
#[tauri::command]
pub async fn memory_file(state: State<'_, AppState>, agent_id: String) -> R<codly_core::memory::panel::MemoryFile> {
    state.core().await?.memory_file(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn save_memory_file(state: State<'_, AppState>, agent_id: String, text: String) -> R<codly_core::memory::panel::MemoryFile> {
    state.core().await?.save_memory_file(&agent_id, &text).await.map_err(err)
}

// ── circuit breaker (Settings → Autonomy & Budgets) ──

#[tauri::command]
pub async fn breaker_config(state: State<'_, AppState>) -> R<BreakerConfig> {
    state.core().await?.breaker_config().await.map_err(err)
}

#[tauri::command]
pub async fn set_breaker_config(state: State<'_, AppState>, config: BreakerConfig) -> R<BreakerConfig> {
    state.core().await?.set_breaker_config(config).await.map_err(err)
}

#[tauri::command]
pub async fn breaker_status(state: State<'_, AppState>) -> R<BreakerStatus> {
    state.core().await?.breaker_status().await.map_err(err)
}

#[tauri::command]
pub async fn reset_breaker(state: State<'_, AppState>) -> R<()> {
    state.core().await?.reset_breaker().await.map_err(err)
}

// ───────────── housekeeping: board archive, default MCP servers ─────────────

/// The board's archiving thresholds; pass `policy` to change them.
#[tauri::command]
pub async fn board_policy(state: State<'_, AppState>, policy: Option<codly_core::housekeeping::archive::ArchivePolicy>) -> R<codly_core::housekeeping::archive::ArchivePolicy> {
    state.core().await?.board_policy(policy).await.map_err(err)
}

/// Archive what is due; returns what is flagged and which tasks are archived.
#[tauri::command]
pub async fn board_sweep(state: State<'_, AppState>) -> R<codly_core::orchestrator::housekeeping::SweepReport> {
    state.core().await?.board_sweep().await.map_err(err)
}

/// Archived tasks matching `query` (all when empty).
#[tauri::command]
pub async fn board_archive(state: State<'_, AppState>, query: Option<String>) -> R<Vec<codly_core::housekeeping::archive::ArchivedItem>> {
    state.core().await?.board_archive(query.as_deref()).await.map_err(err)
}

#[tauri::command]
pub async fn list_connections(state: State<'_, AppState>) -> R<Vec<codly_core::housekeeping::connections::Connection>> {
    state.core().await?.connections().await.map_err(err)
}

/// Switch a default MCP server and/or store its key (Keychain). Applies to the next spawn.
#[tauri::command]
pub async fn update_connection(
    state: State<'_, AppState>,
    id: String,
    enabled: Option<bool>,
    secret: Option<codly_core::orchestrator::housekeeping::ConnectionSecret>,
) -> R<Vec<codly_core::housekeeping::connections::Connection>> {
    state.core().await?.update_connection(&id, enabled, secret).await.map_err(err)
}
