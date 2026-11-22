//! The Tauri command behind Run… on a finished plan and on an agent's Git tab (`api.preview` in
//! src/lib/api.ts). The command line is the one the user typed or confirmed there; the core
//! runs nothing without a `start`, and ends what it started on `stop`, at a merge and at exit.

use codly_core::orchestrator::preview::{Preview, PreviewAction, PreviewSeen, PreviewSource};
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// The preview of a plan's or an agent's worktree, after `action` (start your command, or stop
/// it) when one is given. `seen` says which output lines the page has already.
#[tauri::command]
pub async fn preview(state: State<'_, AppState>, source: PreviewSource, action: Option<PreviewAction>, seen: Option<PreviewSeen>) -> R<Preview> {
    state.core().await?.preview(&source, action, seen.as_ref()).await.map_err(err)
}
