//! Tauri commands for the add-agent wizard (one per `api.hire*` call in src/lib/api.ts). The
//! three that only judge text (`hire_review`, `hire_launch_preview`) are
//! pure; `hire_from_manifest` is the confirmed step that actually starts an agent.

use codly_core::hire::{self, HireManifest, HireReview, LaunchPreview};
use codly_core::model::{Agent, Applicant, Digest, Lot, ModelOffer, Placed, Rank, Shop, Wallet};
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

/// Trust an agent up to a rank's perks (Intern takes them back).
#[tauri::command]
pub async fn trust_agent(state: State<'_, AppState>, agent_id: String, rank: Rank) -> Result<Agent, String> {
    state.core().await?.trust_agent(&agent_id, rank).map_err(|e| format!("{e:#}"))
}

/// The office sends an agent home until `until` (ms), or calls it back (None).
#[tauri::command]
pub async fn set_agent_home(state: State<'_, AppState>, agent_id: String, until: Option<i64>) -> Result<Agent, String> {
    state.core().await?.set_agent_home(&agent_id, until).map_err(|e| format!("{e:#}"))
}

/// The office reports an agent tired (energy under 30%) or rested again: a hint for the Lead.
#[tauri::command]
pub async fn set_agent_tired(state: State<'_, AppState>, agent_id: String, tired: bool) -> Result<Agent, String> {
    state.core().await?.set_agent_tired(&agent_id, tired).map_err(|e| format!("{e:#}"))
}

/// The model of its CLI with the biggest context window, when bigger than its own (a new Legend's offer).
#[tauri::command]
pub async fn bigger_model(state: State<'_, AppState>, agent_id: String) -> Result<Option<ModelOffer>, String> {
    state.core().await?.bigger_model(&agent_id).await.map_err(|e| format!("{e:#}"))
}

/// The office reports a shift worked and rested from, or a sickness (stamina).
#[tauri::command]
pub async fn note_rest(state: State<'_, AppState>, agent_id: String, sick: bool) -> Result<Agent, String> {
    state.core().await?.note_rest(&agent_id, sick).map_err(|e| format!("{e:#}"))
}

/// What the office did since `since` (ms): the daily edition and the morning standup.
#[tauri::command]
pub async fn office_digest(state: State<'_, AppState>, since: i64) -> Result<Digest, String> {
    state.core().await?.digest(since).map_err(|e| format!("{e:#}"))
}

/// Build/Buy: the wallet and what the office owns.
#[tauri::command]
pub async fn office_shop(state: State<'_, AppState>) -> Result<Shop, String> {
    state.core().await?.shop().map_err(|e| format!("{e:#}"))
}

/// Buy a piece of furniture and put it down, or move one already bought.
#[tauri::command]
pub async fn office_place(state: State<'_, AppState>, piece: Placed) -> Result<Shop, String> {
    state.core().await?.place(piece).map_err(|e| format!("{e:#}"))
}

/// Sell a piece of furniture back.
#[tauri::command]
pub async fn office_sell(state: State<'_, AppState>, uid: String) -> Result<Shop, String> {
    state.core().await?.sell(&uid).map_err(|e| format!("{e:#}"))
}

/// The player's coins (🪙), across every repo.
#[tauri::command]
pub async fn wallet(state: State<'_, AppState>) -> Result<Wallet, String> {
    state.core().await?.wallet().map_err(|e| format!("{e:#}"))
}

/// Developer panel (Developer mode only): give coins; flags the wallet for good.
#[tauri::command]
pub async fn dev_grant(state: State<'_, AppState>, amount: i64) -> Result<Wallet, String> {
    state.core().await?.dev_grant(amount).map_err(|e| format!("{e:#}"))
}

/// Developer panel (Developer mode only): give an agent XP; its rank follows.
#[tauri::command]
pub async fn dev_give_xp(state: State<'_, AppState>, agent_id: String, xp: i64) -> Result<Agent, String> {
    state.core().await?.dev_give_xp(&agent_id, xp).map_err(|e| format!("{e:#}"))
}

/// Developer panel (Developer mode only): empty the open repo's office, refunded.
#[tauri::command]
pub async fn dev_reset_office(state: State<'_, AppState>) -> Result<Shop, String> {
    state.core().await?.dev_reset_office().map_err(|e| format!("{e:#}"))
}

/// Build mode: the open repo's lot (made on the first read).
#[tauri::command]
pub async fn office_lot(state: State<'_, AppState>) -> Result<Lot, String> {
    state.core().await?.lot().map_err(|e| format!("{e:#}"))
}

/// Build mode: check and store the open repo's lot.
#[tauri::command]
pub async fn office_save_lot(state: State<'_, AppState>, lot: Lot) -> Result<Lot, String> {
    state.core().await?.save_lot(lot).map_err(|e| format!("{e:#}"))
}
