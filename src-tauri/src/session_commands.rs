//! What the agent sheet does to the things that run beside a live Claude agent's own calls:
//! "Reconnect" on one of its MCP servers and "Stop" on a command it left running in the
//! background. Thin wrappers over `Core` (orchestrator/live_session.rs).

use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn reconnect_agent_mcp(state: State<'_, AppState>, agent_id: String, server: String) -> R<()> {
    state.core().await?.reconnect_agent_mcp(&agent_id, &server).await.map_err(err)
}

#[tauri::command]
pub async fn stop_agent_task(state: State<'_, AppState>, agent_id: String, task_id: String) -> R<()> {
    state.core().await?.stop_agent_task(&agent_id, &task_id).await.map_err(err)
}
