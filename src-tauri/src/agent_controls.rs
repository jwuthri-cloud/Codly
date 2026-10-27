//! Per-agent live controls (agent panel): thin wrappers over `Core`'s block-tools, stop-after-
//! step, resume and steer-note calls. Kill is the existing `stop_agent` command.

use codly_core::model::ControlState;
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn agent_control(state: State<'_, AppState>, agent_id: String) -> R<ControlState> {
    state.core().await?.agent_control(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn set_agent_block_tools(state: State<'_, AppState>, agent_id: String, on: bool) -> R<ControlState> {
    state.core().await?.set_agent_block_tools(&agent_id, on).await.map_err(err)
}

#[tauri::command]
pub async fn stop_agent_after_step(state: State<'_, AppState>, agent_id: String) -> R<ControlState> {
    state.core().await?.stop_agent_after_step(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn resume_agent(state: State<'_, AppState>, agent_id: String) -> R<ControlState> {
    state.core().await?.resume_agent(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn steer_agent(state: State<'_, AppState>, agent_id: String, text: String) -> R<ControlState> {
    state.core().await?.steer_agent(&agent_id, &text).await.map_err(err)
}
