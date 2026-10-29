//! Tauri commands for the add-agent wizard (one per `api.hire*` call in src/lib/api.ts). The
//! three that only judge text (`hire_review`, `hire_launch_preview`) are
//! pure; `hire_from_manifest` is the confirmed step that actually starts an agent.

use codly_core::hire::{self, HireManifest, HireReview, LaunchPreview};
use codly_core::model::{Agent, Applicant, Shop};
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

/// "Generate with AI": a schema-enforced manifest draft (nothing is started).
#[tauri::command]
pub async fn hire_draft(state: State<'_, AppState>, brief: String) -> Result<HireReview, String> {
    state.core().await?.hire_draft(&brief).await.map_err(|e| format!("{e:#}"))
}

/// Start the agent the person reviewed and confirmed.
#[tauri::command]
pub async fn hire_from_manifest(state: State<'_, AppState>, manifest: HireManifest, confirm_shell: bool) -> Result<Agent, String> {
    state.core().await?.hire_from_manifest(manifest, confirm_shell).await.map_err(|e| format!("{e:#}"))
}

/// Applicants for the add-agent wizard to pick from (nothing is started).
#[tauri::command]
pub async fn hire_applicants(state: State<'_, AppState>, n: usize) -> Result<Vec<Applicant>, String> {
    state.core().await?.applicants(n).map_err(|e| format!("{e:#}"))
}

/// You pick who gets the job a new consultant's offer is for.
#[tauri::command]
pub async fn pick_applicant(state: State<'_, AppState>, offer_id: String, index: usize) -> Result<(), String> {
    state.core().await?.pick_applicant(&offer_id, index).map_err(|e| format!("{e:#}"))
}

/// An agent as a `codly/agent@1` file (career, traits and memory included), for another repo.
#[tauri::command]
pub async fn export_agent(state: State<'_, AppState>, agent_id: String) -> Result<String, String> {
    state.core().await?.export_agent(&agent_id).map_err(|e| format!("{e:#}"))
}

/// Workers let go in the open repo, who keep their career (the office's ghosts).
#[tauri::command]
pub async fn ghosts(state: State<'_, AppState>) -> Result<Vec<Agent>, String> {
    state.core().await?.ghosts().map_err(|e| format!("{e:#}"))
}

/// Bring a ghost back as a regular worker.
#[tauri::command]
pub async fn bring_back(state: State<'_, AppState>, agent_id: String) -> Result<Agent, String> {
    state.core().await?.bring_back(&agent_id).map_err(|e| format!("{e:#}"))
}

/// Build/Buy: budget (XP earned), catalog and what the office owns.
#[tauri::command]
pub async fn office_shop(state: State<'_, AppState>) -> Result<Shop, String> {
    state.core().await?.shop().map_err(|e| format!("{e:#}"))
}

/// Buy a piece of furniture, or sell it back.
#[tauri::command]
pub async fn office_trade(state: State<'_, AppState>, item: String, buy: bool) -> Result<Shop, String> {
    state.core().await?.trade(&item, buy).map_err(|e| format!("{e:#}"))
}
