//! Asking before a quit that stops agents in the middle of their work (`Settings::confirm_quit`).
//!
//! Agents run as children of Codly: a quit stops them. Their sessions are kept, and the work
//! can go on at the next start: at once, or after your Resume there (Settings → "Resume work
//! when Codly opens"). Nothing is lost, but a long run is cut short, so Codly asks once before
//! that happens. Only a quit you ask for in the app is questioned: Quit in the menu (Cmd+Q)
//! and the window's close button. `codly serve` stopped from its terminal quits at once, and
//! so does a quit that macOS itself carries out (Quit in the Dock menu, a logout): those end
//! the process without asking the app.

use std::sync::atomic::{AtomicBool, Ordering};

use codly_core::orchestrator::activity::Activity;
use tauri::menu::{MenuItem, MenuItemKind};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::commands::AppState;

/// True while the question is on screen: asking to quit again does not ask it twice.
#[derive(Default)]
pub struct Asking(AtomicBool);

/// The id of Codly's own Quit in the menu bar (`own_menu_quit`).
const MENU_QUIT: &str = "codly-quit";

/// macOS: make the Quit of the app menu Codly's own. The system's Quit item ends the process
/// without asking the app (no exit request reaches it), so it could never be questioned. The
/// app menu is the first menu of the bar and Quit is its last entry; a menu bar that is not
/// built that way is left as it is.
pub fn own_menu_quit<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(MenuItemKind::Submenu(app_menu)) = app.menu().map(|bar| bar.items()).transpose()?.and_then(|menus| menus.into_iter().next()) else {
        return Ok(());
    };
    let Some(MenuItemKind::Predefined(system_quit)) = app_menu.items()?.into_iter().next_back() else { return Ok(()) };
    if !system_quit.text()?.to_lowercase().contains("quit") {
        return Ok(());
    }
    app_menu.remove(&system_quit)?;
    app_menu.append(&MenuItem::with_id(app, MENU_QUIT, "Quit Codly", true, Some("CmdOrCtrl+Q"))?)
}

/// A click in the menu bar: Codly's own Quit quits, after the question if one is due.
pub fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if id == MENU_QUIT && may_quit(app) {
        app.exit(0);
    }
}

/// What to ask before Codly quits with this work under way. None: nothing would be cut short,
/// or you turned the question off (`confirm` is the Settings switch).
fn question(confirm: bool, now: Activity) -> Option<String> {
    if !confirm || !now.busy() {
        return None;
    }
    let what = match now.agents {
        0 if now.checks > 0 => "Codly is checking a task’s work. The checks stop now and can run again when you open Codly again.".to_string(),
        0 => "A plan is running. It stops now and can go on when you open Codly again.".to_string(),
        1 => "1 agent is working. It stops now and can go on when you open Codly again.".to_string(),
        n => format!("{n} agents are working. They stop now and can go on when you open Codly again."),
    };
    Some(format!("{what}\n\nTo quit without this question, turn off “Ask before quitting while agents work” in Settings."))
}

/// You asked Codly to quit. True = it may quit now. False = work is under way and the question
/// is on screen: Codly quits when you say so there, and stays open otherwise.
pub fn may_quit<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(Ok(core)) = app.state::<AppState>().core.borrow().clone() else {
        return true; // the core never started: nothing runs
    };
    let now = match core.activity() {
        Ok(now) => now,
        Err(e) => {
            tracing::warn!("quitting without the question, cannot tell whether agents work: {e:#}");
            return true;
        }
    };
    let Some(text) = question(core.settings().confirm_quit, now) else { return true };
    if app.state::<Asking>().0.swap(true, Ordering::SeqCst) {
        return false;
    }
    let handle = app.clone();
    app.dialog()
        .message(text)
        .title("Quit Codly?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()))
        .show(move |quit| {
            handle.state::<Asking>().0.store(false, Ordering::SeqCst);
            if quit {
                // An exit Codly asks for itself is not questioned again.
                handle.exit(0);
            }
        });
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINT: &str = "\n\nTo quit without this question, turn off “Ask before quitting while agents work” in Settings.";

    #[test]
    fn a_quit_is_questioned_only_while_work_is_under_way() {
        assert_eq!(question(true, Activity::default()), None, "nothing runs: Codly quits at once");
        let three = Activity { agents: 3, checks: 1, plans: 1 };
        assert_eq!(question(true, three), Some(format!("3 agents are working. They stop now and can go on when you open Codly again.{HINT}")));
        assert_eq!(question(false, three), None, "you turned the question off");
    }

    /// The next start may ask before the work goes on (Settings → "Resume work when Codly
    /// opens"): the question does not say that it goes on by itself.
    #[test]
    fn the_question_says_what_stops_and_promises_no_resume_by_itself() {
        let ask = |now| question(true, now).unwrap();
        let one = ask(Activity { agents: 1, ..Default::default() });
        let checks = ask(Activity { checks: 2, plans: 1, ..Default::default() });
        let plan = ask(Activity { plans: 1, ..Default::default() });
        assert!(one.starts_with("1 agent is working. It stops now and can go on when you open Codly again."), "{one}");
        assert!(checks.starts_with("Codly is checking a task’s work. The checks stop now and can run again when you open Codly again."), "{checks}");
        assert!(plan.starts_with("A plan is running. It stops now and can go on when you open Codly again."), "{plan}");
    }
}
