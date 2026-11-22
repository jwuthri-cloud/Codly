//! Tauri commands for a merge that cannot go straight in (`api.mergeState` and
//! `api.resolveMergeConflict` in src/lib/api.ts). `merge_state` only reads the repository.
//! `resolve_merge_conflict` changes Codly's own worktree of the plan's branch (or the solo
//! agent's), never the user's checkout.

use codly_core::orchestrator::merge_conflict::MergeState;
use codly_core::orchestrator::pull_request::PrSource;
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn merge_state(state: State<'_, AppState>, source: PrSource) -> R<MergeState> {
    state.core().await?.merge_state(&source).await.map_err(err)
}

#[tauri::command]
pub async fn resolve_merge_conflict(state: State<'_, AppState>, source: PrSource) -> R<MergeState> {
    state.core().await?.resolve_merge_conflict(&source).await.map_err(err)
}
