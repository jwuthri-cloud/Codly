//! Exercises the real commands through Tauri's IPC layer (mock runtime) with the argument
//! keys the frontend sends (src/lib/api.ts), against a real Core in a temp data dir.

use std::sync::Arc;

use codly_core::{Core, CoreConfig};
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tokio::sync::{OnceCell, watch};

use crate::commands::{self, AppState};
use crate::ui_bus::UiBus;

fn invoke(wv: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        wv,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<Value>().unwrap())
}

#[test]
fn commands_round_trip_with_frontend_arg_names() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn retained_sampling() {}\n").unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let core: Arc<Core> = rt
        .block_on(Core::start(CoreConfig { data_dir: tmp.path().join("data"), app_exe: std::env::current_exe().unwrap() }))
        .unwrap();
    let (_tx, rx) = watch::channel(Some(Ok(core)));
    let app = mock_builder()
        .manage(AppState { core: rx, bus: UiBus::new(), pty: Default::default(), shell_env: OnceCell::new() })
        .invoke_handler(tauri::generate_handler![
            commands::detect_clis,
            commands::open_workspace,
            commands::update_settings,
            commands::create_task,
            commands::move_task,
            commands::list_memories,
            commands::add_memory,
            commands::list_triples,
            commands::search_code,
            commands::get_events,
            commands::recent_workspaces,
        ])
        .build(tauri::generate_context!(test = true))
        .unwrap();
    let wv = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let clis = invoke(&wv, "detect_clis", json!({})).unwrap();
    assert!(clis.as_array().unwrap().len() >= 4, "{clis}");

    let ws = invoke(&wv, "open_workspace", json!({ "repoPath": repo.to_str().unwrap() })).unwrap();
    assert_eq!(ws["name"], "repo");

    let task = invoke(&wv, "create_task", json!({ "title": "Board task", "goal": "Try the board" })).unwrap();
    let moved = invoke(&wv, "move_task", json!({ "taskId": task["id"], "status": "cancelled" })).unwrap();
    assert_eq!(moved["status"], "cancelled");

    let m = invoke(&wv, "add_memory", json!({ "text": "We use pnpm.", "scope": "project", "kind": "decision" })).unwrap();
    assert_eq!(m["kind"], "decision");
    let found = invoke(&wv, "list_memories", json!({ "query": "pnpm", "scope": null })).unwrap();
    assert!(!found.as_array().unwrap().is_empty(), "{found}");
    invoke(&wv, "list_triples", json!({ "entity": null })).unwrap();

    let events = invoke(&wv, "get_events", json!({ "agentId": null, "before": null, "limit": 50 })).unwrap();
    assert!(!events.as_array().unwrap().is_empty());
    invoke(&wv, "recent_workspaces", json!({})).unwrap();

    // The index builds in the background; the command must answer either way.
    let _ = invoke(&wv, "search_code", json!({ "query": "retained sampling" }));
    let err = invoke(&wv, "move_task", json!({ "taskId": "nope", "status": "ready" })).unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");
}
