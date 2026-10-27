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

    async fn shell_env(&self) -> &HashMap<String, String> {
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
pub async fn add_memory(state: State<'_, AppState>, text: String, scope: MemoryScope, kind: MemoryKind) -> R<Memory> {
    state.core().await?.add_memory(&text, scope, kind).await.map_err(err)
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
