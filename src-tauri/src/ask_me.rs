//! Tauri commands for the "Ask me" inbox (one per `api.*` call in src/lib/api.ts).

use codly_core::blockers::Blocker;
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Every card of the open repo, oldest first; long-waiting ones come back flagged.
#[tauri::command]
pub async fn list_blockers(state: State<'_, AppState>) -> R<Vec<Blocker>> {
    state.core().await?.list_blockers().await.map_err(err)
}

/// "Respond & unblock": answer a card and resume the agent that waits for it.
#[tauri::command]
pub async fn answer_blocker(state: State<'_, AppState>, blocker_id: String, answer: String) -> R<Blocker> {
    state.core().await?.answer_blocker(&blocker_id, &answer).await.map_err(err)
}

#[tauri::command]
pub async fn dismiss_blocker(state: State<'_, AppState>, blocker_id: String) -> R<Blocker> {
    state.core().await?.dismiss_blocker(&blocker_id).await.map_err(err)
}
