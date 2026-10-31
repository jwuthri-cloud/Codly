//! Codly desktop shell: window, plugins, commands and the UI bus. All logic lives in
//! `codly-core`; this crate only wires it to the webview.

mod agent_controls;
mod ask_me;
mod commands;
mod dictation;
mod ide_commands;
mod inbound;
mod hire_commands;
mod onboarding_commands;
#[cfg(test)]
mod ipc_tests;
mod ui_bus;

use std::sync::Arc;

use codly_core::blockers::{BlockerSeverity, BlockerStatus};
use codly_core::model::{EventKind, PlanStatus, TaskStatus};
use codly_core::{Core, CoreConfig};
use tauri::{Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{OnceCell, watch};

use commands::AppState;
use ui_bus::UiBus;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("CODLY_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let (core_tx, core_rx) = watch::channel(None);
    let (hub_tx, hub_rx) = watch::channel(None);
    let bus = UiBus::new();

    let app = tauri::Builder::default()
        // must be registered first (plugins run in registration order)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
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
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let inbound_dir = data_dir.clone();
            let app_exe = std::env::current_exe()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let started = Core::start(CoreConfig { data_dir, app_exe }).await.map_err(|e| format!("{e:#}"));
                if let Ok(core) = &started {
                    // A new consultant's job offer stays up in the office this long (you may pick).
                    core.set_hiring_window(std::time::Duration::from_secs(30));
                    tauri::async_runtime::spawn(bus.clone().pump(core.subscribe()));
                    tauri::async_runtime::spawn(notify_loop(handle, core.clone()));
                    let _ = hub_tx.send(inbound::start(core.clone(), inbound_dir).await);
                } else if let Err(e) = &started {
                    tracing::error!("core failed to start: {e}");
                }
                let _ = core_tx.send(Some(started));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ui_subscribe,
            commands::ui_set_visible,
            commands::detect_clis,
            commands::list_accounts,
            commands::recent_workspaces,
            commands::open_workspace,
            commands::update_settings,
            commands::send_chat,
            commands::run_plan,
            commands::cancel_plan,
            commands::close_thread,
            commands::resolve_approval,
            commands::breaker_config,
            commands::set_breaker_config,
            commands::breaker_status,
            commands::reset_breaker,
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
            commands::assign_task,
            commands::task_diff,
            commands::plan_diff,
            commands::merge_plan,
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
            hire_commands::trust_agent,
            hire_commands::office_digest,
            hire_commands::ghosts,
            hire_commands::bring_back,
            hire_commands::office_shop,
            hire_commands::office_place,
            hire_commands::office_sell,
            commands::board_policy,
            commands::board_sweep,
            commands::board_archive,
            commands::list_connections,
            commands::update_connection,
            onboarding_commands::install_cli,
            onboarding_commands::byok_status,
            onboarding_commands::byok_configure,
            onboarding_commands::byok_save_key,
            onboarding_commands::byok_clear_key,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Codly");

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
async fn notify_loop(app: tauri::AppHandle, core: Arc<Core>) {
    let mut rx = core.subscribe();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        };
        let (title, body) = match &ev.kind {
            EventKind::ApprovalRequested { approval } => {
                let what = match &approval.subject {
                    codly_core::model::ApprovalSubject::Command { command, .. } => format!("run `{}`", truncate(command, 60)),
                    codly_core::model::ApprovalSubject::FileChange { paths, .. } => format!("change {} file(s)", paths.len()),
                    codly_core::model::ApprovalSubject::Question { .. } => "ask you a question".into(),
                    codly_core::model::ApprovalSubject::Tool { name, .. } => format!("use {name}"),
                    codly_core::model::ApprovalSubject::Permissions { .. } => "get more permissions".into(),
                };
                ("An agent needs you".to_string(), format!("It wants to {what}."))
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
            _ => continue,
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
