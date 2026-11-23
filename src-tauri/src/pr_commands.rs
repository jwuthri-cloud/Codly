//! Tauri commands for the pull-request step (one per `api.*PullRequest` / `api.pushPrBranch`
//! call in src/lib/api.ts). `create_pull_request` and `push_pr_branch` push one of Codly's own
//! `codly/…` branches, and only what `confirm` names: the remote, the branch and the base the
//! user saw in the dialog. `start_from_github` is the way in from GitHub (`api.startFromGithub`):
//! it reads an issue or a pull request with the user's `gh` and writes nothing there.

use codly_core::orchestrator::intake::{Intake, IntakeKind};
use codly_core::orchestrator::pull_request::{PrConfirm, PrDraft, PrPushed, PrSource, PullRequest};
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn pull_request(state: State<'_, AppState>, source: PrSource) -> R<Option<PullRequest>> {
    state.core().await?.pull_request(&source).await.map_err(err)
}

#[tauri::command]
pub async fn draft_pull_request(state: State<'_, AppState>, source: PrSource) -> R<PrDraft> {
    state.core().await?.draft_pull_request(&source).await.map_err(err)
}

#[tauri::command]
pub async fn create_pull_request(state: State<'_, AppState>, source: PrSource, confirm: PrConfirm, title: String, body: String) -> R<PullRequest> {
    state.core().await?.create_pull_request(&source, &confirm, &title, &body).await.map_err(err)
}

#[tauri::command]
pub async fn push_pr_branch(state: State<'_, AppState>, source: PrSource, confirm: PrConfirm) -> R<PrPushed> {
    state.core().await?.push_pr_branch(&source, &confirm).await.map_err(err)
}

#[tauri::command]
pub async fn refresh_pull_request(state: State<'_, AppState>, source: PrSource) -> R<PullRequest> {
    state.core().await?.refresh_pull_request(&source).await.map_err(err)
}

#[tauri::command]
pub async fn start_from_github(state: State<'_, AppState>, thread: String, kind: IntakeKind, reference: String, note: String) -> R<Intake> {
    state.core().await?.start_from_github(&thread, kind, &reference, &note).await.map_err(err)
}
