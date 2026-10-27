//! Free Flow commands: thin wrappers over `codly_core::dictation`. Config and key handling
//! live in the core; this file only finds the data dir and keeps blocking work (the Keychain
//! tool) off the async runtime.

use codly_core::dictation::{CurlTransport, Dictation, DictationStatus, Keychain, decode_base64};
use tauri::{AppHandle, Manager};

type R<T> = Result<T, String>;

fn service(app: &AppHandle) -> R<Dictation<Keychain, CurlTransport>> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(Dictation::new(dir, Keychain, CurlTransport))
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn dictation_status(app: AppHandle) -> R<DictationStatus> {
    let d = service(&app)?;
    tokio::task::spawn_blocking(move || d.status()).await.map_err(err)
}

#[tauri::command]
pub async fn dictation_configure(app: AppHandle, enabled: Option<bool>, model: Option<String>) -> R<DictationStatus> {
    let d = service(&app)?;
    tokio::task::spawn_blocking(move || d.configure(enabled, model)).await.map_err(err)?.map_err(err)
}

/// Saves the key in the Keychain. The reply is the status ("key saved"), never the key.
#[tauri::command]
pub async fn dictation_save_key(app: AppHandle, key: String) -> R<DictationStatus> {
    let d = service(&app)?;
    tokio::task::spawn_blocking(move || d.save_key(&key)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn dictation_clear_key(app: AppHandle) -> R<DictationStatus> {
    let d = service(&app)?;
    tokio::task::spawn_blocking(move || d.clear_key()).await.map_err(err)?.map_err(err)
}

/// One recording (base64, with its MIME type) → its transcript. Sent to Groq only from here.
#[tauri::command]
pub async fn dictation_transcribe(app: AppHandle, audio: String, mime: String) -> R<String> {
    let d = service(&app)?;
    let bytes = decode_base64(&audio).map_err(err)?;
    d.transcribe(bytes, &mime).await.map_err(err)
}
