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
    // The CLIs here are the machine's real ones. A refresh of the accounts must not start a
    // signed-in Codex and ask OpenAI for its usage from a test.
    core.set_codex_usage_reads(false);
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
            commands::send_back_plan,
            commands::merge_plan,
            commands::merge_plan_warning,
            commands::undo_merge,
            commands::repo_sync,
            commands::worktree_setup,
            commands::required_checks,
            commands::check_agent,
            commands::agent_check_state,
            commands::workspace_trust,
            commands::resume_work,
            crate::preview_commands::preview,
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
            commands::storage_delete_backup,
            commands::agent_checkpoints,
            commands::restore_checkpoint,
            commands::last_turn_diff,
            commands::agent_diff,
            crate::agent_controls::set_agent_permission_mode,
            crate::agent_controls::compact_agent,
            crate::agent_controls::agent_context_usage,
            crate::session_commands::reconnect_agent_mcp,
            crate::session_commands::set_agent_mcp_enabled,
            crate::session_commands::stop_agent_task,
            crate::takeover_commands::agent_takeover,
            crate::takeover_commands::list_takeovers,
            crate::takeover_commands::take_over_agent,
            crate::takeover_commands::open_takeover_terminal,
            crate::takeover_commands::give_back_agent,
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
            commands::model_catalog,
            crate::merge_commands::merge_state,
            crate::merge_commands::resolve_merge_conflict,
            crate::merge_commands::stop_resolving_merge,
            crate::pr_commands::start_from_github,
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
    // On a finished plan's diff, they become one more task of the plan.
    let e = invoke(&wv, "send_back_plan", json!({ "planId": "nope", "comments": "1. `a.txt`\n   Rename it.", "files": ["a.txt"] })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");

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
    // The backups of Codly's own data: a folder of older copies that was moved aside is
    // listed and can be removed. The database itself is refused.
    let kept = tmp.path().join("data").join("backups.before-5");
    std::fs::create_dir(&kept).unwrap();
    std::fs::write(kept.join("codly-4-v1-daily.sqlite"), b"an older copy").unwrap();
    let storage = invoke(&wv, "storage_report", json!({})).unwrap();
    assert_eq!(storage["set_aside"], json!([{ "path": kept.to_str().unwrap(), "kind": "backups", "moved_at": 5, "bytes": 13 }]));
    let database = tmp.path().join("data").join("codly.sqlite");
    let e = invoke(&wv, "storage_delete_backup", json!({ "path": database.to_str().unwrap() })).unwrap_err();
    assert!(e.to_string().contains("is not a backup of Codly's data") && database.exists(), "{e}");
    assert_eq!(invoke(&wv, "storage_delete_backup", json!({ "path": kept.to_str().unwrap() })), Ok(Value::Null));
    assert!(!kept.exists() && database.exists());

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
    let e = invoke(&wv, "merge_plan", json!({ "planId": "nope", "into": null, "unchecked": false })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "merge_plan", json!({ "planId": "nope", "into": "main", "unchecked": true })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    // A merge that cannot go straight in, with the keys the plan's card and the Git tab send.
    let e = invoke(&wv, "merge_state", json!({ "source": { "kind": "plan", "id": "nope" } })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "resolve_merge_conflict", json!({ "source": { "kind": "agent", "id": "nope" }, "into": null })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "resolve_merge_conflict", json!({ "source": { "kind": "plan", "id": "nope" }, "into": "main" })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "stop_resolving_merge", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");

    // New worktrees: no setup script in this repo, so nothing is on and nothing can be turned on.
    let setup = invoke(&wv, "worktree_setup", json!({ "set": null })).unwrap();
    assert_eq!(setup, json!({ "script": null, "sha": null, "risks": [], "on": false, "changed": false, "problem": null, "include": [] }));
    let e = invoke(&wv, "worktree_setup", json!({ "set": { "allow": "abc" } })).unwrap_err();
    assert!(e.to_string().contains("no .codly/setup.sh"), "{e}");
    assert_eq!(invoke(&wv, "worktree_setup", json!({ "set": { "allow": null } })).unwrap()["on"], false);

    // Required checks: no .codly/checks in this repo, so nothing is on, nothing can be turned
    // on, and "Check the work" names an agent (the key the Git tab sends).
    let checks = invoke(&wv, "required_checks", json!({ "set": null })).unwrap();
    assert_eq!(checks, json!({ "text": null, "sha": null, "commands": [], "on": false, "changed": false, "problem": null }));
    let e = invoke(&wv, "required_checks", json!({ "set": { "allow": "abc" } })).unwrap_err();
    assert!(e.to_string().contains("no .codly/checks"), "{e}");
    assert_eq!(invoke(&wv, "required_checks", json!({ "set": { "allow": null } })).unwrap()["on"], false);
    let e = invoke(&wv, "check_agent", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent"), "{e}");
    let e = invoke(&wv, "agent_check_state", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent"), "{e}");
    // Workspace trust: this repo has no CLI configuration of its own, so there is nothing to
    // decide, nothing to trust, and "open without" changes nothing.
    let nothing = json!({ "state": "nothing", "changed": false, "files": [], "instructions": [], "fingerprint": null, "problem": null, "worktrees": [] });
    assert_eq!(invoke(&wv, "workspace_trust", json!({ "set": null })).unwrap(), nothing);
    let e = invoke(&wv, "workspace_trust", json!({ "set": { "trust": "abc" } })).unwrap_err();
    assert!(e.to_string().contains("changed since you read it"), "{e}");
    assert_eq!(invoke(&wv, "workspace_trust", json!({ "set": { "trust": null } })).unwrap(), nothing);
    // Nothing was under way when this core started: no work waits for a Resume, asked or said.
    assert_eq!(invoke(&wv, "resume_work", json!({ "go": false })).unwrap(), Value::Null);
    assert_eq!(invoke(&wv, "resume_work", json!({ "go": true })).unwrap(), Value::Null);
    // Run… with the keys the frontend sends. No plan ran and no agent has a worktree here, so
    // the core answers that, and no command starts.
    let plan = json!({ "kind": "plan", "id": "nope" });
    let e = invoke(&wv, "preview", json!({ "source": plan, "action": null, "seen": null })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let start = json!({ "do": "start", "command": "echo hi" });
    let e = invoke(&wv, "preview", json!({ "source": plan, "action": start, "seen": { "run": "r1", "lines": 3 } })).unwrap_err();
    assert!(e.to_string().contains("unknown plan"), "{e}");
    let e = invoke(&wv, "preview", json!({ "source": { "kind": "agent", "id": "nope" }, "action": { "do": "stop" }, "seen": null })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    assert!(invoke(&wv, "preview", json!({ "source": plan, "action": { "do": "restart" }, "seen": null })).is_err(), "only start and stop");

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

    // "From issue…" / "From pull request…", with the keys the dialog sends. The core answers
    // before gh is asked: a 1:1 chat is not a request, and a sentence is not a link or a number.
    let from = |thread: &str, kind: &str, reference: &str| json!({ "thread": thread, "kind": kind, "reference": reference, "note": "" });
    let e = invoke(&wv, "start_from_github", from("agent:nope", "issue", "12")).unwrap_err();
    assert!(e.to_string().contains("agent:nope is not a request"), "{e}");
    let e = invoke(&wv, "start_from_github", from("lead", "pull_request", "fix the login bug")).unwrap_err();
    assert!(e.to_string().contains("is not a GitHub pull request"), "{e}");
    assert!(invoke(&wv, "start_from_github", from("lead", "commit", "12")).is_err(), "only an issue or a pull request");

    // Checkpoints, with the keys the Git tab sends: the core answers (no agent here), not the
    // argument check.
    let e = invoke(&wv, "agent_checkpoints", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "restore_checkpoint", json!({ "agentId": "nope", "checkpoint": 3 })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "restore_checkpoint", json!({ "agentId": "nope", "checkpoint": -1 })).unwrap_err();
    assert!(!e.to_string().contains("unknown agent"), "a negative number is refused before the core is asked: {e}");
    // "Restore this file" is the same command with the file's path.
    let e = invoke(&wv, "restore_checkpoint", json!({ "agentId": "nope", "checkpoint": 3, "path": "src/a.rs" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    // "Last turn", with the keys the Git tab and the Changes room send.
    let e = invoke(&wv, "last_turn_diff", json!({ "source": { "kind": "agent", "id": "nope" } })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "last_turn_diff", json!({ "source": { "kind": "task", "id": "nope" } })).unwrap_err();
    assert!(e.to_string().contains("unknown task nope"), "{e}");
    assert!(invoke(&wv, "last_turn_diff", json!({ "source": { "kind": "plan", "id": "nope" } })).is_err(), "only an agent or a task");
    // "All changes" of a 1:1 agent, with the key the Changes room sends.
    let e = invoke(&wv, "agent_diff", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");

    // A Claude agent's mode, "compact now" and context reading, with the keys the agent panel
    // sends: the core answers (no agent here), not the argument check.
    let e = invoke(&wv, "set_agent_permission_mode", json!({ "agentId": "nope", "mode": "plan" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "set_agent_permission_mode", json!({ "agentId": "nope", "mode": "bypassPermissions" })).unwrap_err();
    assert!(!e.to_string().contains("unknown agent"), "a mode Codly does not offer is refused before the core is asked: {e}");
    let e = invoke(&wv, "compact_agent", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "agent_context_usage", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    // "Reconnect" on an MCP server and "Stop" on a background command, with the keys the agent
    // sheet sends: the core answers (no agent here), not the argument check.
    let e = invoke(&wv, "reconnect_agent_mcp", json!({ "agentId": "nope", "server": "linear" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "stop_agent_task", json!({ "agentId": "nope", "taskId": "tid-1" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    assert!(invoke(&wv, "stop_agent_task", json!({ "agentId": "nope" })).is_err(), "the command to stop is required");
    // "Turn off" and "Turn on" on an MCP server, with the keys the agent sheet sends.
    let e = invoke(&wv, "set_agent_mcp_enabled", json!({ "agentId": "nope", "server": "linear", "enabled": false })).unwrap_err();
    assert!(e.to_string().contains("unknown agent nope"), "{e}");
    let e = invoke(&wv, "set_agent_mcp_enabled", json!({ "agentId": "nope", "server": "linear" })).unwrap_err();
    assert!(!e.to_string().contains("unknown agent"), "off or on is required, and is checked before the core is asked: {e}");
    // The model pickers and the agent panel's Compact, with the keys the frontend sends. Claude
    // has no list to ask for, so its catalog is Codly's own and no CLI runs here.
    let catalog = invoke(&wv, "model_catalog", json!({ "provider": "claude" })).unwrap();
    assert_eq!((&catalog["source"], &catalog["problem"]), (&json!("built_in"), &Value::Null), "{catalog}");
    let first = &catalog["models"][0];
    assert!(first["id"].is_string() && first["name"].is_string() && first["efforts"].is_array() && first["is_default"] == false, "{catalog}");
    assert!(invoke(&wv, "model_catalog", json!({})).is_err(), "provider is required");
    let e = invoke(&wv, "compact_agent", json!({ "agentId": "nope" })).unwrap_err();
    assert!(e.to_string().contains("unknown agent"), "{e}");
    // "Open in my terminal", with the key the agent sheet sends: the core answers (no agent
    // here), not the argument check, and no terminal opens for an agent that is not in one.
    for cmd in ["agent_takeover", "take_over_agent", "open_takeover_terminal", "give_back_agent"] {
        let e = invoke(&wv, cmd, json!({ "agentId": "nope" })).unwrap_err();
        assert!(e.to_string().contains("unknown agent nope"), "{cmd}: {e}");
    }
    assert_eq!(invoke(&wv, "list_takeovers", json!({})).unwrap(), json!([]), "no agent is in your terminal");

    // The index builds in the background; the command must answer either way.
    let _ = invoke(&wv, "search_code", json!({ "query": "retained sampling" }));
    let err = invoke(&wv, "move_task", json!({ "taskId": "nope", "status": "ready" })).unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");
}
