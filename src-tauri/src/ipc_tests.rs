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

pub(crate) fn invoke(wv: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
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
            commands::list_accounts,
            commands::open_workspace,
            commands::update_settings,
            commands::approval_rule_offer,
            commands::add_approval_rule,
            commands::approval_rules,
            commands::remove_approval_rule,
            commands::create_task,
            commands::move_task,
            commands::cancel_task_tree,
            commands::finish_plan,
            commands::recheck_task,
            commands::accept_task,
            commands::send_back_task,
            commands::merge_plan,
            commands::merge_plan_warning,
            commands::undo_merge,
            commands::repo_sync,
            commands::worktree_setup,
            commands::workspace_trust,
            commands::breaker_config,
            commands::set_breaker_config,
            commands::breaker_status,
            commands::plan_spent,
            commands::list_memories,
            commands::add_memory,
            commands::list_triples,
            commands::search_code,
            commands::get_events,
            commands::closed_requests,
            commands::closed_request,
            commands::recent_workspaces,
            commands::list_repos,
            commands::exact_search,
            commands::message_graph,
            commands::memory_file,
            commands::save_memory_file,
            commands::storage_report,
            commands::storage_policy,
            commands::storage_clean,
            commands::storage_delete_worktree,
            commands::storage_delete_branch,
            commands::agent_checkpoints,
            commands::restore_checkpoint,
            crate::onboarding_commands::install_cli,
            crate::onboarding_commands::byok_status,
            crate::onboarding_commands::byok_configure,
            crate::onboarding_commands::byok_save_key,
            crate::diagnostics::diagnostics,
            crate::pr_commands::pull_request,
            crate::pr_commands::draft_pull_request,
            crate::pr_commands::create_pull_request,
            crate::pr_commands::push_pr_branch,
            crate::pr_commands::refresh_pull_request,
        ])
        .build(tauri::generate_context!(test = true))
        .unwrap();
    let wv = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let clis = invoke(&wv, "detect_clis", json!({})).unwrap();
    assert!(clis.as_array().unwrap().len() >= 4, "{clis}");

    // Settings → Troubleshooting: the report names the app, the core's state and every CLI.
    let d = invoke(&wv, "diagnostics", json!({})).unwrap();
    assert_eq!(d["version"], crate::cli::VERSION);
    let report = d["report"].as_str().unwrap();
    assert!(report.starts_with(&format!("Codly {}\nSystem: ", crate::cli::VERSION)) && report.contains("\nCore: running\n"), "{report}");
    assert_eq!(report.lines().filter(|l| l.starts_with("- ")).count(), clis.as_array().unwrap().len(), "one line per CLI: {report}");
    // This test opens no log file, and the report says so instead of leaving the part out.
    assert!(d["log_file"].is_null() && report.contains("\nNo log file"), "{report}");

    // One account per installed CLI at least (its primary sign-in), the same either way.
    let installed = clis.as_array().unwrap().iter().filter(|c| !c["path"].is_null()).count();
    let cached = invoke(&wv, "list_accounts", json!({ "refresh": false })).unwrap();
    let probed = invoke(&wv, "list_accounts", json!({ "refresh": true })).unwrap();
    for accounts in [&cached, &probed] {
        let list = accounts.as_array().unwrap();
        assert!(list.len() >= installed, "{accounts}");
        assert!(list.iter().all(|a| a["id"].is_string() && a["limits"].is_array()), "{accounts}");
        assert_eq!(list.iter().filter(|a| a["primary"] == true).count(), installed, "one primary sign-in per CLI: {accounts}");
    }
    assert!(invoke(&wv, "list_accounts", json!({})).is_err(), "refresh is required");

    let ws = invoke(&wv, "open_workspace", json!({ "repoPath": repo.to_str().unwrap() })).unwrap();
    assert_eq!(ws["name"], "repo");

    let task = invoke(&wv, "create_task", json!({ "title": "Board task", "goal": "Try the board" })).unwrap();
    let moved = invoke(&wv, "move_task", json!({ "taskId": task["id"], "status": "cancelled" })).unwrap();
    assert_eq!(moved["status"], "cancelled");
    // The task card's Re-run checks and Merge anyway: only for a stopped task.
    let e = invoke(&wv, "recheck_task", json!({ "taskId": task["id"], "allow": [] })).unwrap_err();
    assert!(e.to_string().contains("only a blocked or failed task"), "{e}");
    let e = invoke(&wv, "accept_task", json!({ "taskId": task["id"] })).unwrap_err();
    assert!(e.to_string().contains("can be merged anyway"), "{e}");
    // The diff's Send back: review comments go to a task that can still take them.
    let e = invoke(&wv, "send_back_task", json!({ "taskId": task["id"], "comments": "1. `a.txt:1`\n   Rename it." })).unwrap_err();
    assert!(e.to_string().contains("is cancelled; reopen it first"), "{e}");

    // Cancel with the tasks that wait for it (a board task has none), and Keep what is merged
    // (only a running plan can finish early).
    let other = invoke(&wv, "create_task", json!({ "title": "Another board task", "goal": "" })).unwrap();
    let tree = invoke(&wv, "cancel_task_tree", json!({ "taskId": other["id"] })).unwrap();
    assert!(tree.as_array().is_some_and(|t| t.len() == 1 && t[0]["status"] == "cancelled"), "{tree}");
    let e = invoke(&wv, "finish_plan", json!({ "planId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");

    // Settings → Storage: the office is listed, the rules are saved, and a delete names its
    // office, the path and the commit the list showed. Nothing here is Codly's to delete.
    let storage = invoke(&wv, "storage_report", json!({})).unwrap();
    assert!(storage["offices"][0]["office_id"] == ws["id"] && storage["offices"][0]["worktrees"].is_array() && storage["logs"]["bytes"].is_number(), "{storage}");
    assert_eq!(invoke(&wv, "storage_policy", json!({ "policy": null })).unwrap(), json!({ "log_days": 30, "clean_merged": true }));
    assert_eq!(invoke(&wv, "storage_policy", json!({ "policy": { "log_days": 7, "clean_merged": false } })).unwrap()["log_days"], 7);
    let cleaned = invoke(&wv, "storage_clean", json!({})).unwrap();
    assert_eq!(cleaned, json!({ "worktrees": 0, "branches": 0, "logs": 0, "bytes": 0, "problems": [] }));
    let own = json!({ "officeId": ws["id"], "path": repo.to_str().unwrap(), "head": null, "force": true, "deleteBranch": true });
    let e = invoke(&wv, "storage_delete_worktree", own).unwrap_err();
    assert!(e.to_string().contains("not one of Codly's worktrees"), "{e}");
    let e = invoke(&wv, "storage_delete_branch", json!({ "officeId": ws["id"], "branch": "main", "head": "abc1234", "force": true })).unwrap_err();
    assert!(e.to_string().contains("not one of Codly's branches"), "{e}");

    let m = invoke(&wv, "add_memory", json!({ "text": "We use pnpm.", "scope": "project", "kind": "decision" })).unwrap();
    assert_eq!(m["kind"], "decision");
    let found = invoke(&wv, "list_memories", json!({ "query": "pnpm", "scope": null })).unwrap();
    assert!(!found.as_array().unwrap().is_empty(), "{found}");
    invoke(&wv, "list_triples", json!({ "entity": null })).unwrap();

    // "Always allow in this repo": a rule is only ever saved from a request that waits, so the
    // caller names a request and a kind, never the rule's text.
    assert_eq!(invoke(&wv, "approval_rules", json!({})).unwrap(), json!([]));
    assert_eq!(invoke(&wv, "approval_rule_offer", json!({ "approvalId": "nope" })).unwrap(), json!({ "exact": null, "prefix": null }));
    let e = invoke(&wv, "add_approval_rule", json!({ "approvalId": "nope", "kind": "prefix" })).unwrap_err();
    assert!(e.to_string().contains("unknown approval"), "{e}");
    assert!(invoke(&wv, "add_approval_rule", json!({ "approvalId": "nope", "kind": "cargo test" })).is_err(), "a kind, not a command");
    let e = invoke(&wv, "remove_approval_rule", json!({ "ruleId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("already removed"), "{e}");

    let events = invoke(&wv, "get_events", json!({ "agentId": null, "before": null, "limit": 50 })).unwrap();
    assert!(!events.as_array().unwrap().is_empty());
    // One conversation's page (the transcript's "Load earlier").
    let page = invoke(&wv, "get_events", json!({ "agentId": null, "thread": "lead", "before": null, "limit": 50 })).unwrap();
    assert!(page.is_array(), "{page}");
    // Closed requests: none yet; only an archive thread is read this way.
    assert_eq!(invoke(&wv, "closed_requests", json!({})).unwrap(), json!([]));
    let none = invoke(&wv, "closed_request", json!({ "thread": "closed:lead:1" })).unwrap();
    assert_eq!(none, json!({ "messages": [], "total": 0, "authors": {} }));
    let e = invoke(&wv, "closed_request", json!({ "thread": "lead" })).unwrap_err();
    assert!(e.to_string().contains("not a closed request"), "{e}");
    invoke(&wv, "recent_workspaces", json!({})).unwrap();
    assert!(invoke(&wv, "list_repos", json!({})).unwrap().is_array());

    // Find / Files / Messages lenses: the memory just made is found by its words; no path escapes.
    let hits = invoke(&wv, "exact_search", json!({ "query": "PNPM" })).unwrap();
    assert!(hits.as_array().unwrap().iter().any(|h| h["kind"] == "memory"), "{hits}");
    let graph = invoke(&wv, "message_graph", json!({})).unwrap();
    assert!(graph["nodes"].is_array() && graph["edges"].is_array(), "{graph}");
    assert!(invoke(&wv, "memory_file", json!({ "agentId": "../../etc/passwd" })).is_err());

    // Onboarding: install_cli takes a provider and a rung name, never a command line.
    assert!(invoke(&wv, "install_cli", json!({ "provider": "claude", "rung": "npm; rm -rf ~" })).is_err());
    let e = invoke(&wv, "install_cli", json!({ "provider": "cursor", "rung": "npm" })).unwrap_err();
    assert!(e.to_string().contains("no npm package"), "{e}");
    let e = invoke(&wv, "install_cli", json!({ "provider": "cursor", "rung": "update" })).unwrap_err();
    assert!(e.to_string().contains("no update command"), "{e}");
    // BYOK: a source persists in Settings; Key needs a saved key; a bad key is refused before it
    // reaches the store; the status names sources and "key saved", never a key.
    assert!(invoke(&wv, "byok_configure", json!({ "vendor": "anthropic", "source": { "kind": "key" } })).is_err());
    let on = invoke(&wv, "byok_configure", json!({ "vendor": "openai", "source": { "kind": "command", "command": " mint-token openai " } })).unwrap();
    assert_eq!(on["vendors"][1], json!({ "vendor": "openai", "source": { "kind": "command", "command": "mint-token openai" }, "key_saved": false }));
    assert_eq!(invoke(&wv, "byok_status", json!({})).unwrap()["vendors"][1]["source"]["kind"], "command");
    assert!(invoke(&wv, "byok_save_key", json!({ "vendor": "anthropic", "key": "two words" })).is_err());
    invoke(&wv, "byok_configure", json!({ "vendor": "openai", "source": { "kind": "off" } })).unwrap();

    // Merging and its undo, with the keys the frontend sends. Nothing was merged here, so there
    // is nothing to undo, and a sha the app never showed is refused.
    let sync = invoke(&wv, "repo_sync", json!({})).unwrap();
    assert!(sync["branch"] == "main" && sync["last_merge"].is_null(), "{sync}");
    let e = invoke(&wv, "undo_merge", json!({ "sha": "abc1234" })).unwrap_err();
    assert!(e.to_string().contains("not the last merge"), "{e}");
    let e = invoke(&wv, "merge_plan_warning", json!({ "planId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "merge_plan", json!({ "planId": "nope", "into": null })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");

    // New worktrees: no setup script in this repo, so nothing is on and nothing can be turned on.
    let setup = invoke(&wv, "worktree_setup", json!({ "set": null })).unwrap();
    assert_eq!(setup, json!({ "script": null, "sha": null, "risks": [], "on": false, "changed": false, "problem": null, "include": [] }));
    let e = invoke(&wv, "worktree_setup", json!({ "set": { "allow": "abc" } })).unwrap_err();
    assert!(e.to_string().contains("no .codly/setup.sh"), "{e}");
    assert_eq!(invoke(&wv, "worktree_setup", json!({ "set": { "allow": null } })).unwrap()["on"], false);

    // Workspace trust: this repo has no CLI configuration of its own, so there is nothing to
    // decide, nothing to trust, and "open without" changes nothing.
    let nothing = json!({ "state": "nothing", "changed": false, "files": [], "instructions": [], "fingerprint": null, "problem": null, "worktrees": [] });
    assert_eq!(invoke(&wv, "workspace_trust", json!({ "set": null })).unwrap(), nothing);
    let e = invoke(&wv, "workspace_trust", json!({ "set": { "trust": "abc" } })).unwrap_err();
    assert!(e.to_string().contains("changed since you read it"), "{e}");
    assert_eq!(invoke(&wv, "workspace_trust", json!({ "set": { "trust": null } })).unwrap(), nothing);

    // Budgets: the limits and the status with the fields Settings reads, and a plan's total by
    // the key the plan cards send.
    let cfg = invoke(&wv, "breaker_config", json!({})).unwrap();
    assert_eq!((&cfg["token_budget"], &cfg["cost_budget_usd"]), (&json!(0), &json!(0.0)), "{cfg}");
    let mut wanted = cfg.clone();
    wanted["token_budget"] = json!(2_000_000);
    wanted["cost_budget_usd"] = json!(7.5);
    let saved = invoke(&wv, "set_breaker_config", json!({ "config": wanted })).unwrap();
    assert_eq!((&saved["token_budget"], &saved["cost_budget_usd"]), (&json!(2_000_000), &json!(7.5)), "{saved}");
    let status = invoke(&wv, "breaker_status", json!({})).unwrap();
    assert_eq!(status["used"], json!({ "tokens": 0, "cost_usd": 0.0 }), "{status}");
    assert!(status["today"]["tokens"] == 0 && status["hold"].is_null(), "{status}");
    let e = invoke(&wv, "plan_spent", json!({ "planId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");

    // The pull-request step, with the keys the frontend sends. No plan ran here: nothing is on
    // record, nothing can be drafted, and nothing is pushed.
    let source = json!({ "kind": "plan", "id": "nope" });
    let confirm = json!({ "remote": "origin", "branch": "codly/x/integration", "base": "main" });
    assert!(invoke(&wv, "pull_request", json!({ "source": source })).unwrap().is_null());
    let e = invoke(&wv, "draft_pull_request", json!({ "source": source })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "create_pull_request", json!({ "source": source, "confirm": confirm, "title": "T", "body": "B" })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "push_pr_branch", json!({ "source": source, "confirm": confirm })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "refresh_pull_request", json!({ "source": { "kind": "agent", "id": "nope" } })).unwrap_err();
    assert!(e.to_string().contains("has not opened a pull request"), "{e}");
    assert!(invoke(&wv, "draft_pull_request", json!({ "source": { "kind": "branch", "id": "main" } })).is_err(), "only a plan or a solo agent");

    // Checkpoints, with the keys the Git tab sends: the core answers (no agent here), not the
    // argument check.
    let e = invoke(&wv, "agent_checkpoints", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "restore_checkpoint", json!({ "agentId": "nope", "checkpoint": 3 })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "restore_checkpoint", json!({ "agentId": "nope", "checkpoint": -1 })).unwrap_err();
    assert!(!e.to_string().contains("unknown agent"), "a negative number is refused before the core is asked: {e}");

    // The index builds in the background; the command must answer either way.
    let _ = invoke(&wv, "search_code", json!({ "query": "retained sampling" }));
    let err = invoke(&wv, "move_task", json!({ "taskId": "nope", "status": "ready" })).unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");
}
