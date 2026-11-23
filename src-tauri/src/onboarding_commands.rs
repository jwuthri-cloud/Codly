//! Onboarding commands: thin wrappers over `codly_core::{install, byok}`. `install_cli` takes a
//! provider and a rung, never a command line: the text that reaches the terminal is a constant
//! chosen in the core. A BYOK key only ever travels UI → secret store; every reply is a status.

use codly_core::byok::{self, ByokSource, ByokStatus, Vendor};
use codly_core::install::{self, InstallRun, InstallRung};
use codly_core::model::Provider;
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Opens a visible login-shell terminal (attach with `pty_attach`) and types the vendor's
/// install or update line for (provider, rung) into it. Nothing runs without this explicit call.
#[tauri::command]
pub async fn install_cli(state: State<'_, AppState>, provider: Provider, rung: InstallRung) -> R<InstallRun> {
    // Reject before spawning a shell: unknown/unavailable rungs and scripts off POSIX.
    install::command_for(provider, rung, cfg!(unix)).map_err(err)?;
    let mut env = state.shell_env().await.clone();
    codly_core::env::refresh_path(&mut env); // an update line must find the CLI installed a moment ago
    let home = env.get("HOME").cloned().unwrap_or_else(|| "/".into());
    install::start(&state.pty, std::path::Path::new(&home), &env, provider, rung).map_err(err)
}

/// Sources live in Settings (the core); whether a key is saved lives in the secret store.
async fn status(state: &AppState) -> R<ByokStatus> {
    let settings = state.core().await?.settings().byok;
    tokio::task::spawn_blocking(move || byok::global().status(&settings)).await.map_err(err)
}

#[tauri::command]
pub async fn byok_status(state: State<'_, AppState>) -> R<ByokStatus> {
    status(&state).await
}

/// Pick where a vendor's credential comes from: off, the saved key, or a token command.
/// Off keeps the key (if any) in the store, but no agent gets it.
#[tauri::command]
pub async fn byok_configure(state: State<'_, AppState>, vendor: Vendor, source: ByokSource) -> R<ByokStatus> {
    let saved = tokio::task::spawn_blocking(move || byok::global().key(vendor).is_some()).await.map_err(err)?;
    let source = byok::validate_source(source, saved).map_err(err)?;
    let core = state.core().await?;
    let mut settings = core.settings();
    settings.byok.set(vendor, source);
    core.update_settings(settings).await.map_err(err)?;
    core.byok_changed();
    status(&state).await
}

/// Saves a vendor's key. The reply is the status ("key saved"), never the key.
#[tauri::command]
pub async fn byok_save_key(state: State<'_, AppState>, vendor: Vendor, key: String) -> R<ByokStatus> {
    tokio::task::spawn_blocking(move || byok::global().save_key(vendor, &key)).await.map_err(err)?.map_err(err)?;
    state.core().await?.byok_changed();
    status(&state).await
}

/// Removes a vendor's key; a vendor that used it is switched off, so nothing waits on a missing key.
#[tauri::command]
pub async fn byok_clear_key(state: State<'_, AppState>, vendor: Vendor) -> R<ByokStatus> {
    tokio::task::spawn_blocking(move || byok::global().clear_key(vendor)).await.map_err(err)?.map_err(err)?;
    let core = state.core().await?;
    let mut settings = core.settings();
    if *settings.byok.get(vendor) == ByokSource::Key {
        settings.byok.set(vendor, ByokSource::Off);
        core.update_settings(settings).await.map_err(err)?;
    }
    core.byok_changed();
    status(&state).await
}
