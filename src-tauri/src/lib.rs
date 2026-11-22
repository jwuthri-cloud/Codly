//! Codly desktop shell: window, plugins, commands and the UI bus. All logic lives in
//! `codly-core`; this crate only wires it to the webview.

mod agent_controls;
mod ask_me;
mod boot;
pub mod cli;
mod commands;
mod diagnostics;
mod dictation;
mod ide_commands;
mod inbound;
mod hire_commands;
mod onboarding_commands;
mod pr_commands;
#[cfg(test)]
mod ipc_tests;
mod logging;
mod ui_bus;
mod web;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use codly_core::blockers::{BlockerSeverity, BlockerStatus};
use codly_core::model::{Approval, EventKind, PlanStatus, TaskStatus};
use codly_core::Core;
use tauri::{Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{OnceCell, watch};

use commands::AppState;
use ui_bus::UiBus;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();

    let serve = match web::from_args(&std::env::args().collect::<Vec<_>>()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // Said here, in the terminal you typed it in: an app that already runs serves for this one.
    if let Some(s) = serve.as_ref().filter(|s| s.remote) {
        eprintln!("{}", web::remote_warning(s));
    }
    let (core_tx, core_rx) = watch::channel(None);
    let (hub_tx, hub_rx) = watch::channel(None);
    let bus = UiBus::new();

    let app = tauri::Builder::default()
        // must be registered first (plugins run in registration order)
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            match web::from_args(&argv) {
                // `codly serve` while the app runs: this app serves too, and opens the browser.
                Ok(Some(s)) if !s.remote => {
                    tauri::async_runtime::spawn(web::run(app.clone(), s, true));
                    return;
                }
                // Not to the network: nobody sees the link this app would print, and its token
                // is never kept. `web::remote_warning` told the terminal to quit Codly first.
                Ok(Some(_)) => tracing::warn!("codly serve --allow-remote was not started: Codly is already open. Quit it, then run the command again."),
                _ => {}
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState { core: core_rx, bus: bus.clone(), pty: Default::default(), shell_env: OnceCell::new() })
        .manage(inbound::InboundHandle { hub: hub_rx })
        .manage(boot::Boot { core: core_tx, hub: hub_tx, bus })
        .setup(move |app| {
            // The window starts hidden (tauri.conf.json): `codly serve` keeps it so, and out of the Dock.
            match serve {
                None => {
                    if let Some(w) = app.get_webview_window("main") {
                        w.show()?;
                    }
                }
                Some(s) => {
                    #[cfg(target_os = "macos")]
                    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(web::run(handle.clone(), s, false));
                    tauri::async_runtime::spawn(async move {
                        let _ = tokio::signal::ctrl_c().await;
                        handle.exit(0);
                    });
                }
            }
            boot::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            boot::retry_core,
            commands::ui_subscribe,
            commands::ui_set_visible,
            commands::detect_clis,
            commands::list_accounts,
            commands::recent_workspaces,
            commands::list_repos,
            commands::open_workspace,
            commands::update_settings,
            commands::send_chat,
            commands::run_plan,
            commands::cancel_plan,
            commands::close_thread,
            commands::closed_requests,
            commands::closed_request,
            commands::resolve_approval,
            commands::approval_rule_offer,
            commands::add_approval_rule,
            commands::approval_rules,
            commands::remove_approval_rule,
            commands::breaker_config,
            commands::set_breaker_config,
            commands::breaker_status,
            commands::reset_breaker,
            commands::plan_spent,
            commands::interrupt_agent,
            agent_controls::agent_control,
            agent_controls::set_agent_block_tools,
            agent_controls::stop_agent_after_step,
            agent_controls::resume_agent,
            agent_controls::steer_agent,
            commands::stop_agent,
            commands::hire_agent,
            commands::list_models,
            commands::set_workspace_team,
            commands::set_request_prefs,
            commands::set_agent_model,
            commands::set_agent_look,
            commands::create_task,
            commands::move_task,
            commands::recheck_task,
            commands::accept_task,
            commands::assign_task,
            commands::task_diff,
            commands::plan_diff,
            commands::send_back_task,
            commands::merge_plan,
            commands::merge_plan_warning,
            commands::undo_merge,
            commands::merge_agent,
            commands::repo_sync,
            commands::commit_checkout,
            commands::push_checkout,
            commands::get_events,
            commands::search_code,
            commands::list_memories,
            commands::add_memory,
            commands::delete_memory,
            commands::list_triples,
            commands::keep_plan,
            commands::pin_memory,
            commands::repo_map,
            commands::read_artifact,
            commands::save_attachment,
            commands::pty_open,
            commands::pty_attach,
            commands::pty_write,
            commands::pty_resize,
            commands::pty_close,
            dictation::dictation_status,
            dictation::dictation_configure,
            dictation::dictation_save_key,
            dictation::dictation_clear_key,
            dictation::dictation_transcribe,
            commands::agent_git,
            commands::agent_checkpoints,
            commands::restore_checkpoint,
            commands::get_triggers,
            commands::set_triggers,
            commands::list_skills,
            commands::skill_catalog,
            commands::install_skill,
            commands::uninstall_skill,
            commands::open_skill_folder,
            ide_commands::ide_info,
            ide_commands::ide_list,
            ide_commands::ide_read,
            ide_commands::ide_write,
            ide_commands::ide_create,
            ide_commands::ide_search,
            ide_commands::ide_changes,
            ide_commands::ide_diff,
            commands::exact_search,
            commands::message_graph,
            commands::memory_file,
            commands::save_memory_file,
            ask_me::list_blockers,
            ask_me::answer_blocker,
            ask_me::dismiss_blocker,
            inbound::inbound_state,
            inbound::inbound_create_webhook,
            inbound::inbound_set_webhook,
            inbound::inbound_revoke_webhook,
            inbound::inbound_set_settings,
            inbound::inbound_set_slack,
            hire_commands::hire_review,
            hire_commands::hire_launch_preview,
            hire_commands::hire_draft,
            hire_commands::hire_from_manifest,
            hire_commands::hire_applicants,
            hire_commands::pick_applicant,
            hire_commands::export_agent,
            hire_commands::export_office,
            hire_commands::trust_agent,
            hire_commands::office_digest,
            hire_commands::set_agent_home,
            hire_commands::note_rest,
            hire_commands::note_hangout,
            hire_commands::set_agent_tired,
            hire_commands::bigger_model,
            hire_commands::ghosts,
            hire_commands::bring_back,
            hire_commands::let_go,
            hire_commands::offices,
            hire_commands::transfer_agent,
            hire_commands::office_shop,
            hire_commands::office_place,
            hire_commands::office_sell,
            hire_commands::wallet,
            hire_commands::dev_grant,
            hire_commands::dev_give_xp,
            hire_commands::dev_reset_office,
            hire_commands::office_lot,
            hire_commands::office_save_lot,
            hire_commands::lots,
            hire_commands::lot_of,
            hire_commands::buy_lot,
            hire_commands::sell_lot,
            hire_commands::move_office,
            hire_commands::close_office,
            commands::board_policy,
            commands::board_sweep,
            commands::board_archive,
            commands::worktree_setup,
            commands::list_connections,
            commands::update_connection,
            commands::storage_report,
            commands::storage_policy,
            commands::storage_clean,
            commands::storage_delete_worktree,
            commands::storage_delete_branch,
            onboarding_commands::install_cli,
            onboarding_commands::byok_status,
            onboarding_commands::byok_configure,
            onboarding_commands::byok_save_key,
            onboarding_commands::byok_clear_key,
            diagnostics::diagnostics,
            commands::cancel_task_tree,
            commands::finish_plan,
            pr_commands::pull_request,
            pr_commands::draft_pull_request,
            pr_commands::create_pull_request,
            pr_commands::push_pr_branch,
            pr_commands::refresh_pull_request,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Codly");

    // The data dir is known from here on: the lines logged so far waited in memory.
    match app.path().app_data_dir() {
        Ok(dir) => match logging::open(&dir.join(logging::DIR)) {
            Ok(()) => tracing::info!("Codly {} started", cli::VERSION),
            Err(e) => tracing::warn!("no log file in {}: {e}", dir.display()),
        },
        Err(e) => tracing::warn!("no log file: {e}"),
    }

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let state = handle.state::<AppState>();
            state.pty.close_all();
            if let Some(Ok(core)) = state.core.borrow().clone() {
                tauri::async_runtime::block_on(core.shutdown());
            }
        }
    });
}

/// Native notifications come from Rust: the webview may be suspended while hidden, and an
/// approval must never wait silently. Only when the window is not focused.
async fn notify_loop<R: tauri::Runtime>(app: tauri::AppHandle<R>, core: Arc<Core>) {
    let mut rx = core.subscribe();
    // Approvals not answered yet. The broker allows most of them by itself right away: only one
    // it leaves to you ("asking you: …") is worth a notification.
    let mut asked: HashMap<String, Approval> = HashMap::new();
    let mut stops = Stops::default();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        };
        // Looked at for every event: it must see a stop end to tell the next one.
        let stop = stops.notice(&ev.kind);
        let (title, body) = match &ev.kind {
            EventKind::ApprovalRequested { approval } => {
                asked.insert(approval.id.clone(), approval.clone());
                continue;
            }
            EventKind::ApprovalResolved { approval_id, .. } => {
                asked.remove(approval_id);
                continue;
            }
            EventKind::Decision { decision_type, summary, detail, .. } if decision_type == "approval_policy" && summary.starts_with("asking you") => {
                let Some(approval) = detail["approval_id"].as_str().and_then(|id| asked.remove(id)) else { continue };
                let what = match &approval.subject {
                    codly_core::model::ApprovalSubject::Command { command, .. } => format!("run `{}`", truncate(command, 60)),
                    codly_core::model::ApprovalSubject::FileChange { paths, .. } => format!("change {} file(s)", paths.len()),
                    codly_core::model::ApprovalSubject::Question { .. } => "ask you a question".into(),
                    codly_core::model::ApprovalSubject::Tool { name, .. } => format!("use {name}"),
                    codly_core::model::ApprovalSubject::Permissions { .. } => "get more permissions".into(),
                };
                // Who, and in which office: it may not be the one on screen.
                let (office, agent) = core.names_for(ev.workspace_id.as_deref(), Some(&approval.agent_id));
                let agent = agent.unwrap_or_else(|| "An agent".into());
                let title = office.map_or_else(|| format!("{agent} needs you"), |o| format!("{o}: {agent} needs you"));
                (title, format!("It wants to {what}."))
            }
            // A new blocking question, or one that has waited for days.
            EventKind::BlockerChanged { blocker }
                if blocker.status == BlockerStatus::Open && blocker.severity == BlockerSeverity::Blocking && blocker.updated_at == blocker.reminded_at.unwrap_or(blocker.created_at) =>
            {
                let title = if blocker.reminded_at.is_some() { "Still waiting for your answer" } else { "An agent needs a decision" };
                (title.to_string(), truncate(&blocker.title, 120))
            }
            EventKind::PlanChanged { plan } if plan.status == PlanStatus::Done => {
                ("Plan done".to_string(), truncate(&plan.summary, 120))
            }
            EventKind::TaskChanged { task } if task.status == TaskStatus::Failed => {
                ("Task failed".to_string(), format!("{} — {}", task.key, truncate(&task.title, 80)))
            }
            // The circuit breaker held an agent back or stopped it: its work waits for you.
            EventKind::Decision { decision_type, summary, detail, .. }
                if decision_type == "breaker" && matches!(detail["action"].as_str(), Some("constrained" | "stopped")) =>
            {
                ("An agent needs a look".to_string(), truncate(summary, 160))
            }
            // The watchdog saw an agent that gives no output or sits idle on its task, or it
            // ended a silent turn or run. (Asking an agent to continue needs no look from you.)
            EventKind::Decision { decision_type, summary, detail, .. }
                if decision_type == "watchdog" && matches!(detail["action"].as_str(), Some("flagged" | "idle" | "interrupted" | "stopped")) =>
            {
                ("An agent needs a look".to_string(), truncate(summary, 160))
            }
            // A budget is used up: no new task starts until you raise it or reset the count.
            EventKind::Decision { decision_type, summary, detail, .. } if decision_type == "breaker" && detail["action"] == "budget" => {
                ("Budget reached".to_string(), truncate(summary, 160))
            }
            // Codly blocked a task, or a plan failed: that work waits for you.
            _ => match stop {
                Some(notice) => notice,
                None => continue,
            },
        };
        let focused = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
        if !focused {
            let _ = app.notification().builder().title(title).body(body).show();
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// Work that stopped and waits for you: a task Codly blocked, a plan that failed. Each stop
/// is told once, however many events show it, and again only after it ended.
#[derive(Default)]
struct Stops {
    tasks: HashSet<String>,
    plans: HashSet<String>,
}

impl Stops {
    /// The notification (title, body) for this event, if it is the first to show a stop.
    fn notice(&mut self, kind: &EventKind) -> Option<(String, String)> {
        match kind {
            EventKind::TaskChanged { task } => {
                let reason = task.status_reason.as_deref().unwrap_or_default();
                // Not a task you stopped yourself, and not one that asks you a question (its
                // card has a notification of its own).
                let yours = ["stopped by", "review stopped by", "waiting for you:"].iter().any(|p| reason.starts_with(p));
                if task.status != TaskStatus::Blocked || yours {
                    self.tasks.remove(&task.id);
                    return None;
                }
                let why = truncate(reason.strip_prefix("blocked: ").unwrap_or(reason), 100);
                self.tasks.insert(task.id.clone()).then(|| ("Task blocked".to_string(), format!("{} — {}: {why}", task.key, truncate(&task.title, 60))))
            }
            EventKind::PlanChanged { plan } => {
                if plan.status != PlanStatus::Failed {
                    self.plans.remove(&plan.id);
                    return None;
                }
                self.plans.insert(plan.id.clone()).then(|| ("Plan failed".to_string(), truncate(&plan.summary, 120)))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod stop_notice_tests {
    use super::*;
    use serde_json::json;

    fn task(status: &str, reason: &str) -> EventKind {
        let task = serde_json::from_value(json!({
            "id": "t", "plan_id": null, "key": "t1", "title": "Add sign-in", "goal": "", "acceptance": [], "write_scope": [],
            "verify": [], "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
            "attempt": 1, "status_reason": reason, "summary": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap();
        EventKind::TaskChanged { task }
    }

    fn plan(status: &str) -> EventKind {
        let plan = serde_json::from_value(json!({
            "id": "p", "workspace_id": "ws", "request": "", "summary": "Sign-in page", "assumptions": [], "open_questions": [],
            "status": status, "base_commit": null, "integration_branch": null, "integration_worktree": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap();
        EventKind::PlanChanged { plan }
    }

    #[test]
    fn a_task_codly_blocked_is_told_once_and_again_after_a_retry() {
        let mut stops = Stops::default();
        let blocked = task("blocked", "blocked: could not start the worker: Claude Code is not signed in");
        let notice = stops.notice(&blocked).expect("a blocked task is worth a notification");
        assert_eq!(notice, ("Task blocked".to_string(), "t1 — Add sign-in: could not start the worker: Claude Code is not signed in".to_string()));
        assert_eq!(stops.notice(&blocked), None, "another change of the same blocked task says nothing");
        assert_eq!(stops.notice(&task("ready", "")), None);
        assert!(stops.notice(&blocked).is_some(), "blocked again after your Retry: told again");
    }

    #[test]
    fn a_task_you_stopped_or_that_asks_you_has_no_blocked_notice() {
        let mut stops = Stops::default();
        for reason in ["stopped by you; Retry to resume", "review stopped by you; Retry to re-check", "waiting for you: Which provider?"] {
            assert_eq!(stops.notice(&task("blocked", reason)), None, "{reason}");
        }
        assert_eq!(stops.notice(&task("failed", "failed: checks")), None, "a failed task has its own notification");
    }

    #[test]
    fn a_failed_plan_is_told_once() {
        let mut stops = Stops::default();
        assert_eq!(stops.notice(&plan("running")), None);
        assert_eq!(stops.notice(&plan("failed")), Some(("Plan failed".to_string(), "Sign-in page".to_string())));
        assert_eq!(stops.notice(&plan("failed")), None);
    }
}
