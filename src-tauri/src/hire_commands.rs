//! Tauri commands for the add-agent wizard (one per `api.hire*` call in src/lib/api.ts). The
//! three that only judge text (`hire_review`, `hire_launch_preview`, `hire_draft_prompt`) are
//! pure; `hire_from_manifest` is the confirmed step that actually starts an agent.

use codly_core::hire::{self, HireManifest, HireReview, LaunchPreview};
use codly_core::model::Agent;
use tauri::State;

use crate::commands::AppState;

/// Parse and judge a hire file or an AI draft (nothing is started).
#[tauri::command]
pub async fn hire_review(raw: String) -> Result<HireReview, String> {
    Ok(hire::review(&raw))
}

/// The command line an engine choice runs, as the wizard's engine step shows it.
#[tauri::command]
pub async fn hire_launch_preview(provider: String, model: Option<String>, command: Option<String>) -> Result<LaunchPreview, String> {
    Ok(hire::launch_preview(&provider, model.as_deref(), command.as_deref()))
}

/// The message "Generate with AI" sends to the Lead.
#[tauri::command]
pub async fn hire_draft_prompt(brief: String) -> Result<String, String> {
    Ok(hire::draft_prompt(&brief))
}

/// Start the agent the person reviewed and confirmed.
#[tauri::command]
pub async fn hire_from_manifest(state: State<'_, AppState>, manifest: HireManifest, confirm_shell: bool) -> Result<Agent, String> {
    state.core().await?.hire_from_manifest(manifest, confirm_shell).await.map_err(|e| format!("{e:#}"))
}
