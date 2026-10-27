//! Tauri commands for the workspace IDE overlay (one per `api.ide*` call in src/lib/api.ts).
//! Paths are root-relative strings; `codly_core::ide::confine` decides what they may reach.

use codly_core::ide::{IdeChange, IdeEntry, IdeFile, IdeInfo, IdeSearch};
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn ide_info(state: State<'_, AppState>, agent_id: String) -> R<IdeInfo> {
    state.core().await?.ide_info(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn ide_list(state: State<'_, AppState>, agent_id: String, path: String) -> R<Vec<IdeEntry>> {
    state.core().await?.ide_list(&agent_id, &path).await.map_err(err)
}

#[tauri::command]
pub async fn ide_read(state: State<'_, AppState>, agent_id: String, path: String) -> R<IdeFile> {
    state.core().await?.ide_read(&agent_id, &path).await.map_err(err)
}

#[tauri::command]
pub async fn ide_write(state: State<'_, AppState>, agent_id: String, path: String, content: String) -> R<()> {
    state.core().await?.ide_write(&agent_id, &path, content).await.map_err(err)
}

#[tauri::command]
pub async fn ide_create(state: State<'_, AppState>, agent_id: String, path: String) -> R<()> {
    state.core().await?.ide_create(&agent_id, &path).await.map_err(err)
}

#[tauri::command]
pub async fn ide_search(state: State<'_, AppState>, agent_id: String, query: String) -> R<IdeSearch> {
    state.core().await?.ide_search(&agent_id, &query).await.map_err(err)
}

#[tauri::command]
pub async fn ide_changes(state: State<'_, AppState>, agent_id: String) -> R<Vec<IdeChange>> {
    state.core().await?.ide_changes(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn ide_diff(state: State<'_, AppState>, agent_id: String, path: String) -> R<String> {
    state.core().await?.ide_diff(&agent_id, &path).await.map_err(err)
}
