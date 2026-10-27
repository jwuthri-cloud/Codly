//! Onboarding commands: thin wrappers over `codly_core::{install, byok}`. `install_cli` takes a
//! provider and a rung, never a command line: the text that reaches the terminal is a constant
//! chosen in the core. The BYOK key only ever travels UI → Keychain; every reply is a status.

use codly_core::byok::{self, ByokStatus};
use codly_core::install::{self, InstallRun, InstallRung};
use codly_core::model::Provider;
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Opens a visible login-shell terminal (attach with `pty_attach`) and types the vendor's
/// install line for (provider, rung) into it. Nothing runs without this explicit call.
#[tauri::command]
pub async fn install_cli(state: State<'_, AppState>, provider: Provider, rung: InstallRung) -> R<InstallRun> {
    // Reject before spawning a shell: unknown/unavailable rungs and scripts off POSIX.
    install::command_for(provider, rung, cfg!(unix)).map_err(err)?;
    let env = state.shell_env().await.clone();
    let home = env.get("HOME").cloned().unwrap_or_else(|| "/".into());
    install::start(&state.pty, std::path::Path::new(&home), &env, provider, rung).map_err(err)
}

/// The switch lives in Settings (the core); whether a key is saved lives in the Keychain.
async fn status(state: &AppState) -> R<ByokStatus> {
    let enabled = state.core().await?.settings().byok_openrouter;
    tokio::task::spawn_blocking(move || byok::global().status(enabled)).await.map_err(err)
}

#[tauri::command]
pub async fn byok_status(state: State<'_, AppState>) -> R<ByokStatus> {
    status(&state).await
}

/// Opt in or out. Off = the key (if any) stays in the Keychain but no agent gets it.
#[tauri::command]
pub async fn byok_configure(state: State<'_, AppState>, enabled: bool) -> R<ByokStatus> {
    let core = state.core().await?;
    let settings = codly_core::model::Settings { byok_openrouter: enabled, ..core.settings() };
    core.update_settings(settings).await.map_err(err)?;
    status(&state).await
}

/// Saves the key in the Keychain. The reply is the status ("key saved"), never the key.
#[tauri::command]
pub async fn byok_save_key(state: State<'_, AppState>, key: String) -> R<ByokStatus> {
    tokio::task::spawn_blocking(move || byok::global().save_key(&key)).await.map_err(err)?.map_err(err)?;
    status(&state).await
}

#[tauri::command]
pub async fn byok_clear_key(state: State<'_, AppState>) -> R<ByokStatus> {
    tokio::task::spawn_blocking(|| byok::global().clear_key()).await.map_err(err)?.map_err(err)?;
    status(&state).await
}
