fn main() {
    // Declaring an app manifest makes every app command subject to capability checks
    // (without it, local app commands are allowed implicitly).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "ui_subscribe", "ui_set_visible", "detect_clis", "list_accounts", "recent_workspaces", "open_workspace", "update_settings",
        "send_chat", "run_plan", "cancel_plan", "close_thread", "resolve_approval", "interrupt_agent", "stop_agent", "hire_agent", "list_models", "set_agent_model", "set_workspace_team",
        "breaker_config", "set_breaker_config", "breaker_status", "reset_breaker",
        "create_task", "move_task", "assign_task", "task_diff", "plan_diff", "merge_plan", "get_events", "search_code",
        "list_memories", "add_memory", "delete_memory", "list_triples", "repo_map", "read_artifact", "keep_plan", "pin_memory", "pty_open", "pty_attach", "pty_write",
        "pty_resize", "pty_close",
        "dictation_status", "dictation_configure", "dictation_save_key", "dictation_clear_key", "dictation_transcribe",
        "exact_search", "message_graph", "memory_file", "save_memory_file",
    ])))
    .expect("tauri-build failed");
}
