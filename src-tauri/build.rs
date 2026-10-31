fn main() {
    // Declaring an app manifest makes every app command subject to capability checks
    // (without it, local app commands are allowed implicitly).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "agent_control", "set_agent_block_tools", "stop_agent_after_step", "resume_agent", "steer_agent",
        "ui_subscribe", "ui_set_visible", "detect_clis", "list_accounts", "recent_workspaces", "open_workspace", "update_settings",
        "send_chat", "run_plan", "cancel_plan", "close_thread", "resolve_approval", "interrupt_agent", "stop_agent", "hire_agent", "list_models", "set_agent_model", "set_agent_look", "set_workspace_team", "set_request_prefs",
        "breaker_config", "set_breaker_config", "breaker_status", "reset_breaker",
        "create_task", "move_task", "assign_task", "task_diff", "plan_diff", "merge_plan", "get_events", "search_code",
        "list_memories", "add_memory", "delete_memory", "list_triples", "repo_map", "read_artifact", "keep_plan", "pin_memory", "pty_open", "pty_attach", "pty_write",
        "pty_resize", "pty_close",
        "dictation_status", "dictation_configure", "dictation_save_key", "dictation_clear_key", "dictation_transcribe",
        "exact_search", "message_graph", "memory_file", "save_memory_file",
        "list_blockers", "answer_blocker", "dismiss_blocker",
        "inbound_state", "inbound_create_webhook", "inbound_set_webhook", "inbound_revoke_webhook", "inbound_set_settings", "inbound_set_slack",
        "hire_review", "hire_launch_preview", "hire_draft_prompt", "hire_from_manifest", "hire_applicants", "pick_applicant", "export_agent", "export_office", "ghosts", "bring_back", "offices", "transfer_agent", "office_shop", "office_place", "office_sell", "office_lot", "office_save_lot", "lots", "lot_of", "buy_lot", "sell_lot", "move_office",
        "wallet", "dev_grant", "dev_give_xp", "dev_reset_office",
        "agent_git", "get_triggers", "set_triggers", "list_skills", "skill_catalog", "install_skill", "uninstall_skill", "open_skill_folder", "ide_info", "ide_list", "ide_read", "ide_write", "ide_create", "ide_search", "ide_changes", "ide_diff",
        "board_policy", "board_sweep", "board_archive", "list_connections", "update_connection",
        "save_attachment", "hire_draft", "trust_agent", "office_digest", "set_agent_home", "note_rest", "set_agent_tired", "bigger_model",
        "install_cli", "byok_status", "byok_configure", "byok_save_key", "byok_clear_key",
    ])))
    .expect("tauri-build failed");
}
