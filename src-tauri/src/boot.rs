//! Starting the core, and starting it again after a failed start.
//!
//! The core starts once at launch. A failed start is kept in `AppState::core`, so every command
//! reports the same reason; "Try again" on the start-up screen (`retry_core`) forgets it and
//! starts the core again, with everything that hangs on it.
//!
//! When the start failed because the database cannot be used, `Boot::problem` says what the
//! screen shows (`start_problem`), and `recover_database` moves the database aside and starts
//! from a backup, or with no data. The file is kept: nothing here deletes it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use codly_core::inbound::InboundHub;
use codly_core::store::backup::{self, StartProblem};
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
    /// Set while the last start is a failed one that the database caused: what the start-up
    /// screen says about it and offers.
    pub problem: Mutex<Option<StartProblem>>,
}

impl Boot {
    fn problem(&self) -> MutexGuard<'_, Option<StartProblem>> {
        self.problem.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn data_dir<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<PathBuf> {
    app.path().app_data_dir().map_err(|e| anyhow::anyhow!("no folder for Codly's data: {e}"))
}

/// Start the core in the background. `AppState::core` gets it, or the reason it failed.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let boot = app.state::<Boot>();
        let started = start_core(&app, &boot).await;
        let problem = match &started {
            Ok(_) => None,
            Err(e) => {
                tracing::error!("core failed to start: {e:#}");
                data_dir(&app).ok().and_then(|dir| backup::problem(&dir.join(codly_core::store::FILE), e))
            }
        };
        *boot.problem() = problem;
        let _ = boot.core.send(Some(started.map_err(|e| format!("{e:#}"))));
    });
}

async fn start_core<R: Runtime>(app: &AppHandle<R>, boot: &Boot) -> anyhow::Result<Arc<Core>> {
    let data_dir = data_dir(app)?;
    let app_exe = std::env::current_exe()?;
    let core = Core::start(CoreConfig { data_dir: data_dir.clone(), app_exe }).await?;
    // With the office rules on (Settings.game_rules), a new consultant's job offer stays up in
    // the office this long (you may pick). Off, it is hired at once.
    core.set_hiring_window(Duration::from_secs(30));
    tauri::async_runtime::spawn(boot.bus.clone().pump(core.subscribe()));
    tauri::async_runtime::spawn(crate::notify::run(app.clone(), core.clone()));
    tauri::async_runtime::spawn(crate::awake::run(core.clone()));
    let _ = boot.hub.send(inbound::start(core.clone(), data_dir).await);
    Ok(core)
}

/// "Try again" on the start-up screen. Only a failed start is started again: a core that runs,
/// or one that is still starting, is left alone.
#[tauri::command]
pub async fn retry_core<R: Runtime>(app: AppHandle<R>, boot: State<'_, Boot>) -> Result<(), String> {
    if take_failed(&boot.core).is_some() {
        start(app);
    }
    Ok(())
}

/// What the start-up screen says and offers when the start failed because the database
/// cannot be used. None = the start did not fail, or it failed for another reason.
#[tauri::command]
pub async fn start_problem(boot: State<'_, Boot>) -> Result<Option<StartProblem>, String> {
    Ok(boot.problem().clone())
}

const NOTHING_TO_RECOVER: &str = "Codly did not fail to start on its database: there is nothing to recover.";

/// Whether the start-up screen offered this way out of `problem`: a start with no data
/// (`backup` None), or a start from the copy at `backup`. A copy the screen did not list is
/// refused, also one that is in the backups folder.
fn offered(problem: Option<&StartProblem>, backup: Option<&str>) -> Result<(), String> {
    let Some(problem) = problem.filter(|p| p.start_empty) else { return Err(NOTHING_TO_RECOVER.into()) };
    match backup {
        Some(path) if !problem.backups.iter().any(|b| b.path == path) => Err(format!("{path} is not a backup that Codly offered to start from.")),
        _ => Ok(()),
    }
}

/// "Start from the last backup" (`backup`: its path) and "Start with no data" (None) on the
/// start-up screen. The database that cannot be used is moved aside in its folder, never
/// deleted, and the core starts again. Only after a start that failed on the database: then
/// nothing has the file open, and a running Codly can never be made to drop its data. And
/// only what `start_problem` offered for that start.
#[tauri::command]
pub async fn recover_database<R: Runtime>(app: AppHandle<R>, boot: State<'_, Boot>, backup: Option<String>) -> Result<(), String> {
    let db = data_dir(&app).map_err(|e| e.to_string())?.join(codly_core::store::FILE);
    offered(boot.problem().as_ref(), backup.as_deref())?;
    let Some(why) = take_failed(&boot.core) else { return Err(NOTHING_TO_RECOVER.into()) };
    let moved = tokio::task::spawn_blocking(move || backup::recover(&db, backup.as_deref().map(std::path::Path::new), codly_core::model::now_ms()))
        .await
        .map_err(|e| e.to_string())
        .and_then(|moved| moved.map_err(|e| format!("{e:#}")));
    match moved {
        Ok(aside) => {
            tracing::warn!("the database that could not be used was moved to {}", aside.display());
            start(app);
            Ok(())
        }
        Err(e) => {
            // Still the failed start it was: the screen stays, with its reason.
            let _ = boot.core.send(Some(Err(why)));
            Err(e)
        }
    }
}

/// Forget a failed start, so `AppState::core` waits for the next one. Returns why it failed;
/// None = nothing failed.
fn take_failed<T>(started: &watch::Sender<Option<Result<T, String>>>) -> Option<String> {
    let mut why = None;
    started.send_if_modified(|s| {
        if let Some(Err(e)) = s {
            why = Some(std::mem::take(e));
            *s = None;
        }
        why.is_some()
    });
    why
}

#[cfg(test)]
mod tests {
    use codly_core::store::backup::{BackupKind, DbBackup};
    use serde_json::{Value, json};
    use tauri::test::mock_builder;

    use super::*;

    #[test]
    fn only_a_failed_start_is_forgotten() {
        let (tx, rx) = watch::channel::<Option<Result<u8, String>>>(None);
        assert_eq!(take_failed(&tx), None, "still starting: nothing to retry");
        tx.send(Some(Ok(1))).unwrap();
        assert_eq!(take_failed(&tx), None);
        assert_eq!(*rx.borrow(), Some(Ok(1)), "a running core is left alone");
        tx.send(Some(Err("the database is locked".into()))).unwrap();
        assert_eq!(take_failed(&tx).as_deref(), Some("the database is locked"));
        assert_eq!(*rx.borrow(), None, "commands wait for the next start instead of repeating the old error");
        assert_eq!(take_failed(&tx), None, "a second click does not start the core twice");
    }

    /// The command as the UI calls it, through the IPC layer and the real capability list.
    #[test]
    fn ipc_retry_core_leaves_a_starting_core_alone() {
        let (core, core_rx) = watch::channel(None);
        let (hub, _hub_rx) = watch::channel(None);
        let app = mock_builder()
            .manage(Boot { core, hub, bus: UiBus::new(), problem: Default::default() })
            .invoke_handler(tauri::generate_handler![retry_core])
            .build(tauri::generate_context!(test = true))
            .unwrap();
        let wv = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        assert_eq!(crate::ipc_tests::invoke(&wv, "retry_core", json!({})), Ok(Value::Null));
        assert!(core_rx.borrow().is_none(), "nothing was started or changed");
    }

    /// The recovery commands as the UI calls them. Only the refusals: a recovery that goes
    /// through moves the database of the app's own data folder, which a test must not touch
    /// (the move itself is tested in `codly_core::store::backup`).
    #[test]
    fn ipc_recovery_is_refused_unless_the_start_failed_on_the_database() {
        let (core, core_rx) = watch::channel(None);
        let (hub, _hub_rx) = watch::channel(None);
        let app = mock_builder()
            .manage(Boot { core, hub, bus: UiBus::new(), problem: Default::default() })
            .invoke_handler(tauri::generate_handler![start_problem, recover_database])
            .build(tauri::generate_context!(test = true))
            .unwrap();
        let wv = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let recover = |backup: Value| crate::ipc_tests::invoke(&wv, "recover_database", json!({ "backup": backup }));
        let refused = Err(json!("Codly did not fail to start on its database: there is nothing to recover."));

        // Still starting.
        assert_eq!(crate::ipc_tests::invoke(&wv, "start_problem", json!({})), Ok(Value::Null));
        assert_eq!(recover(Value::Null), refused);
        // A start that failed for another reason: the database is not the problem.
        let boot = app.state::<Boot>();
        assert!(boot.core.send(Some(Err("no login shell".into()))).is_ok());
        assert_eq!(recover(json!("/data/backups/codly-1-v1-daily.sqlite")), refused);
        // The database is fine and only its copy before an update failed: nothing to move.
        let problem = StartProblem { message: "Codly could not copy its data".into(), database: "/data/codly.sqlite".into(), backups: vec![], start_empty: false };
        *boot.problem() = Some(problem.clone());
        assert_eq!(crate::ipc_tests::invoke(&wv, "start_problem", json!({})), Ok(serde_json::to_value(&problem).unwrap()));
        assert_eq!(recover(Value::Null), refused);
        // The database is damaged and the screen offers one copy. Another file is refused,
        // also one in the backups folder: nothing is moved for it.
        let copy = DbBackup { path: "/data/backups/codly-2-v1-daily.sqlite".into(), kind: BackupKind::Daily, created_at: 2, schema: 1, bytes: 1 };
        *boot.problem() = Some(StartProblem { message: "Codly's database is damaged".into(), backups: vec![copy], start_empty: true, ..problem });
        for other in ["/data/backups/codly-1-v1-daily.sqlite", "/data/codly.sqlite", "/data/backups/../codly-2-v1-daily.sqlite"] {
            assert_eq!(recover(json!(other)), Err(json!(format!("{other} is not a backup that Codly offered to start from."))));
        }
        assert!(matches!(&*core_rx.borrow(), Some(Err(e)) if e == "no login shell"), "the failed start is kept as it was");
    }

    #[test]
    fn only_what_the_screen_offered_is_a_way_out() {
        let copy = |at: i64| DbBackup { path: format!("/data/backups/codly-{at}-v1-daily.sqlite"), kind: BackupKind::Daily, created_at: at, schema: 1, bytes: 1 };
        let damaged = StartProblem { message: "damaged".into(), database: "/data/codly.sqlite".into(), backups: vec![copy(2), copy(1)], start_empty: true };
        assert_eq!(offered(Some(&damaged), None), Ok(()));
        assert_eq!(offered(Some(&damaged), Some(&copy(1).path)), Ok(()));
        assert_eq!(offered(Some(&damaged), Some(&copy(3).path)), Err(format!("{} is not a backup that Codly offered to start from.", copy(3).path)));
        // The update of the data failed: no copy is offered, only a start with no data.
        let update_failed = StartProblem { backups: vec![], ..damaged.clone() };
        assert_eq!(offered(Some(&update_failed), None), Ok(()));
        assert!(offered(Some(&update_failed), Some(&copy(2).path)).is_err());
        // Nothing to move: the start did not fail on the database, or only its copy failed.
        let copy_failed = StartProblem { start_empty: false, ..damaged };
        for backup in [None, Some(copy(2).path)] {
            assert_eq!(offered(None, backup.as_deref()), Err(NOTHING_TO_RECOVER.into()));
            assert_eq!(offered(Some(&copy_failed), backup.as_deref()), Err(NOTHING_TO_RECOVER.into()));
        }
    }
}
