//! Codly desktop shell: window, plugins, commands and the UI bus. All logic lives in
//! `codly-core`; this crate only wires it to the webview.

mod agent_controls;
mod ask_me;
mod awake;
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
mod quit;
mod takeover_commands;
mod preview_commands;
#[cfg(test)]
mod ipc_tests;
mod logging;
mod notify;
mod merge_commands;
mod ui_bus;
mod web;

use tauri::{Manager, RunEvent, WindowEvent};
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
        .manage(boot::Boot { core: core_tx, hub: hub_tx, bus, problem: Default::default() })
        .manage(quit::Asking::default())
        .on_menu_event(|app, event| quit::on_menu(app, event.id().as_ref()))
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
            // Cmd+Q asks first while agents work. Elsewhere there is no app menu to change.
            if cfg!(target_os = "macos")
                && let Err(e) = quit::own_menu_quit(app.handle())
            {
                tracing::warn!("Quit in the menu does not ask before it stops agents: {e}");
            }
            boot::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            boot::retry_core,
            boot::start_problem,
            boot::recover_database,
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
            agent_controls::set_agent_block_tools,
            agent_controls::stop_agent_after_step,
            agent_controls::resume_agent,
            agent_controls::steer_agent,
            agent_controls::set_agent_permission_mode,
            agent_controls::compact_agent,
            agent_controls::agent_context_usage,
            takeover_commands::agent_takeover,
            takeover_commands::list_takeovers,
            takeover_commands::take_over_agent,
            takeover_commands::open_takeover_terminal,
            takeover_commands::give_back_agent,
            commands::stop_agent,
            commands::list_models,
            commands::model_catalog,
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
            commands::required_checks,
            commands::check_agent,
            commands::agent_check_state,
            commands::workspace_trust,
            preview_commands::preview,
            commands::list_connections,
            commands::update_connection,
            commands::storage_report,
            commands::storage_policy,
            commands::storage_clean,
            commands::storage_delete_worktree,
            commands::storage_delete_branch,
            commands::storage_delete_backup,
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
            merge_commands::merge_state,
            merge_commands::resolve_merge_conflict,
            merge_commands::stop_resolving_merge,
            pr_commands::start_from_github,
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

    app.run(|handle, event| match event {
        // Closing the window quits Codly: asked first while agents work (`quit.rs`).
        RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. } if label == "main" => {
            if !quit::may_quit(handle) {
                api.prevent_close();
            }
        }
        RunEvent::Exit => {
            let state = handle.state::<AppState>();
            state.pty.close_all();
            if let Some(Ok(core)) = state.core.borrow().clone() {
                tauri::async_runtime::block_on(core.shutdown());
            }
        }
        _ => {}
    });
}
