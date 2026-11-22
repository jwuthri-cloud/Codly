//! Starting the core, and starting it again after a failed start.
//!
//! The core starts once at launch. A failed start is kept in `AppState::core`, so every command
//! reports the same reason; "Try again" on the start-up screen (`retry_core`) forgets it and
//! starts the core again, with everything that hangs on it.

use std::sync::Arc;
use std::time::Duration;

use codly_core::inbound::InboundHub;
use codly_core::{Core, CoreConfig};
use tauri::{AppHandle, Manager, Runtime, State};
use tokio::sync::watch;

use crate::inbound;
use crate::ui_bus::UiBus;

/// Where a start of the core puts what it made: the core (or why it failed) for
/// `AppState::core`, the inbound hub for `InboundHandle`.
pub struct Boot {
    pub core: watch::Sender<Option<Result<Arc<Core>, String>>>,
    pub hub: watch::Sender<Option<Arc<InboundHub>>>,
    pub bus: Arc<UiBus>,
}

/// Start the core in the background. `AppState::core` gets it, or the reason it failed.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let boot = app.state::<Boot>();
        let started = start_core(&app, &boot).await.map_err(|e| format!("{e:#}"));
        if let Err(e) = &started {
            tracing::error!("core failed to start: {e}");
        }
        let _ = boot.core.send(Some(started));
    });
}

async fn start_core<R: Runtime>(app: &AppHandle<R>, boot: &Boot) -> anyhow::Result<Arc<Core>> {
    let data_dir = app.path().app_data_dir().map_err(|e| anyhow::anyhow!("no folder for Codly's data: {e}"))?;
    let app_exe = std::env::current_exe()?;
    let core = Core::start(CoreConfig { data_dir: data_dir.clone(), app_exe }).await?;
    // A new consultant's job offer stays up in the office this long (you may pick).
    core.set_hiring_window(Duration::from_secs(30));
    tauri::async_runtime::spawn(boot.bus.clone().pump(core.subscribe()));
    tauri::async_runtime::spawn(crate::notify_loop(app.clone(), core.clone()));
    let _ = boot.hub.send(inbound::start(core.clone(), data_dir).await);
    Ok(core)
}

/// "Try again" on the start-up screen. Only a failed start is started again: a core that runs,
/// or one that is still starting, is left alone.
#[tauri::command]
pub async fn retry_core<R: Runtime>(app: AppHandle<R>, boot: State<'_, Boot>) -> Result<(), String> {
    if clear_failed(&boot.core) {
        start(app);
    }
    Ok(())
}

/// Forget a failed start, so `AppState::core` waits for the next one. False = nothing failed.
fn clear_failed<T>(started: &watch::Sender<Option<Result<T, String>>>) -> bool {
    started.send_if_modified(|s| {
        let failed = matches!(s, Some(Err(_)));
        if failed {
            *s = None;
        }
        failed
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tauri::test::mock_builder;

    use super::*;

    #[test]
    fn only_a_failed_start_is_forgotten() {
        let (tx, rx) = watch::channel::<Option<Result<u8, String>>>(None);
        assert!(!clear_failed(&tx), "still starting: nothing to retry");
        tx.send(Some(Ok(1))).unwrap();
        assert!(!clear_failed(&tx));
        assert_eq!(*rx.borrow(), Some(Ok(1)), "a running core is left alone");
        tx.send(Some(Err("the database is locked".into()))).unwrap();
        assert!(clear_failed(&tx));
        assert_eq!(*rx.borrow(), None, "commands wait for the next start instead of repeating the old error");
        assert!(!clear_failed(&tx), "a second click does not start the core twice");
    }

    /// The command as the UI calls it, through the IPC layer and the real capability list.
    #[test]
    fn ipc_retry_core_leaves_a_starting_core_alone() {
        let (core, core_rx) = watch::channel(None);
        let (hub, _hub_rx) = watch::channel(None);
        let app = mock_builder()
            .manage(Boot { core, hub, bus: UiBus::new() })
            .invoke_handler(tauri::generate_handler![retry_core])
            .build(tauri::generate_context!(test = true))
            .unwrap();
        let wv = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        assert_eq!(crate::ipc_tests::invoke(&wv, "retry_core", json!({})), Ok(Value::Null));
        assert!(core_rx.borrow().is_none(), "nothing was started or changed");
    }
}
