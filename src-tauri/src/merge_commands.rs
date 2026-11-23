//! Tauri commands for a merge that cannot go straight in (`api.mergeState`,
//! `api.resolveMergeConflict` and `api.stopResolvingMerge` in src/lib/api.ts). `merge_state`
//! only reads the repository. `resolve_merge_conflict` and `stop_resolving_merge` change
//! Codly's own worktree of the plan's branch (or the solo agent's), never the user's checkout.

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

/// `into`: the branch the user confirmed after `merge_plan_warning`, for a plan that did not
/// start from the checked-out branch.
#[tauri::command]
pub async fn resolve_merge_conflict(state: State<'_, AppState>, source: PrSource, into: Option<String>) -> R<MergeState> {
    let core = state.core().await?;
    match into {
        Some(branch) => core.resolve_merge_conflict_into(&source, &branch).await,
        None => core.resolve_merge_conflict(&source).await,
    }
    .map_err(err)
}

/// Stop a solo agent's conflict resolution: the merge is taken out of the agent's worktree.
#[tauri::command]
pub async fn stop_resolving_merge(state: State<'_, AppState>, agent_id: String) -> R<MergeState> {
    state.core().await?.stop_resolving_merge(&agent_id).await.map_err(err)
}
