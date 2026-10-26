//! End-to-end run against the real CLIs (costs a little subscription quota):
//! a scratch repo → the Lead plans → workers implement in worktrees → Codly verifies and
//! merges → plan done. Approvals are auto-answered "allow once" and logged, standing in
//! for the user.
//!
//! cargo run -p codly-core --example e2e -- [scratch_dir] [lead=claude] [workers=codex,claude]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use codly_core::model::*;
use codly_core::{Core, CoreConfig};

fn sh(dir: &Path, args: &[&str]) {
    let ok = Command::new(args[0]).args(&args[1..]).current_dir(dir).status().map(|s| s.success()).unwrap_or(false);
    assert!(ok, "command failed: {args:?}");
}

fn make_repo(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    std::fs::write(dir.join("package.json"), r#"{ "name": "e2e-math", "type": "module", "scripts": { "test": "node --test" } }"#).unwrap();
    std::fs::write(dir.join("src/math.js"), "export function add(a, b) {\n  return a + b;\n}\n").unwrap();
    std::fs::write(
        dir.join("test/math.test.js"),
        "import { test } from 'node:test';\nimport assert from 'node:assert/strict';\nimport { add } from '../src/math.js';\n\ntest('add', () => {\n  assert.equal(add(2, 3), 5);\n});\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# e2e-math\n\nTiny math library. Run tests with `node --test`.\n").unwrap();
    sh(dir, &["git", "init", "-q", "-b", "main"]);
    sh(dir, &["git", "add", "-A"]);
    sh(dir, &["git", "-c", "user.name=e2e", "-c", "user.email=e2e@example.com", "commit", "-q", "-m", "init"]);
}

fn provider(s: &str) -> Provider {
    match s {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        "gemini" => Provider::Gemini,
        "opencode" => Provider::Opencode,
        other => panic!("unknown provider {other}"),
    }
}

fn short(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n { format!("{}…", one.chars().take(n).collect::<String>()) } else { one }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scratch = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "/tmp/codly-e2e".into()));
    let lead = provider(args.get(2).map(String::as_str).unwrap_or("claude"));
    let workers: Vec<Provider> =
        args.get(3).map(String::as_str).unwrap_or("codex,claude").split(',').map(provider).collect();

    let repo = scratch.join("repo");
    let data = scratch.join("data");
    make_repo(&repo);
    let _ = std::fs::remove_dir_all(&data);
    let app_exe = std::env::current_dir()?.join("target/debug/codly");
    anyhow::ensure!(app_exe.exists(), "build the app first: cargo build -p codly ({})", app_exe.display());

    let core = Core::start(CoreConfig { data_dir: data.clone(), app_exe }).await?;
    let ws = core.open_workspace(repo.to_str().unwrap()).await?;
    println!("workspace {} ({})", ws.name, ws.repo_path);
    for c in core.redetect_clis().await? {
        println!("  cli {:?}: {:?} logged_in={:?} billing={:?} {}", c.provider, c.version, c.logged_in, c.billing, c.problem.unwrap_or_default());
    }

    let mut settings = core.snapshot().await?.settings;
    settings.lead_provider = Some(lead);
    settings.lead_model = Some(if lead == Provider::Claude { "sonnet".into() } else { String::new() }).filter(|m| !m.is_empty());
    settings.worker_providers = workers;
    settings.auto_run_plans = true;
    settings.approval_mode = ApprovalMode::Balanced;
    settings.review_gate = std::env::var("E2E_REVIEW").is_ok();
    core.update_settings(settings).await?;

    let mut rx = core.subscribe();
    let request = std::env::var("E2E_REQUEST").unwrap_or_else(|_| {
        "Add `multiply(a, b)` and `subtract(a, b)` to src/math.js, each with a test in test/math.test.js. \
         Verify with `node --test`."
            .into()
    });
    let request = request.as_str();
    println!("\n> lead: {request}\n");
    core.send_chat("lead", request).await?;
    // Optional second request in parallel, on its own Lead.
    let request2 = std::env::var("E2E_REQUEST2").ok();
    if let Some(r2) = &request2 {
        println!("> lead:b: {r2}\n");
        core.send_chat("lead:b", r2).await?;
    }
    let expected_plans = if request2.is_some() { 2 } else { 1 };
    let mut done_plans: std::collections::HashSet<String> = Default::default();

    let started = Instant::now();
    let deadline = started + Duration::from_secs(25 * 60);
    let mut plan_id: Option<String> = None;
    let mut outcome = "timeout";
    while Instant::now() < deadline {
        let ev = match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(ev)) => ev,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                println!("  (lagged {n})");
                continue;
            }
            Ok(Err(_)) => break,
            Err(_) => continue,
        };
        let t = started.elapsed().as_secs();
        let who = ev.agent_id.as_deref().map(|a| &a[a.len().saturating_sub(4)..]).unwrap_or("----");
        match &ev.kind {
            EventKind::TextDelta { .. } | EventKind::ReasoningDelta { .. } | EventKind::ToolOutput { .. } | EventKind::UsageUpdated { .. } => {}
            EventKind::ApprovalRequested { approval } => {
                println!("[{t:>4}s] {who} APPROVAL {:?} risks={:?} choices={:?}", approval.subject, approval.risks, approval.choices);
                let decision = ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None };
                core.resolve_approval(&approval.id, decision).await?;
            }
            EventKind::ToolStarted { name, activity, target, .. } => {
                println!("[{t:>4}s] {who} tool {name} ({activity:?}) {}", short(&format!("{:?}{:?}{:?}", target.path, target.command, target.query), 90));
            }
            EventKind::ToolCompleted { ok, exit_code, tests_passed, tests_failed, .. } => {
                println!("[{t:>4}s] {who}   → ok={ok} exit={exit_code:?} tests={tests_passed:?}/{tests_failed:?}");
            }
            EventKind::Message { text, phase, .. } => println!("[{t:>4}s] {who} said ({phase:?}): {}", short(text, 160)),
            EventKind::Chat { message } => println!("[{t:>4}s] chat[{}] {}: {}", message.thread, message.author, short(&message.text, 200)),
            EventKind::PlanChanged { plan } => {
                println!("[{t:>4}s] PLAN {:?} {}", plan.status, short(&plan.summary, 120));
                plan_id = Some(plan.id.clone());
                match plan.status {
                    PlanStatus::Done => {
                        done_plans.insert(plan.id.clone());
                        if done_plans.len() >= expected_plans { outcome = "done"; break; }
                    }
                    PlanStatus::Failed | PlanStatus::Cancelled => { outcome = "failed"; break; }
                    _ => {}
                }
            }
            EventKind::TaskChanged { task } => println!(
                "[{t:>4}s] TASK {} {:?} {} {}",
                task.key, task.status, short(&task.title, 60), task.status_reason.as_deref().unwrap_or("")
            ),
            EventKind::Decision { decision_type, summary, .. } => println!("[{t:>4}s] decision[{decision_type}] {}", short(summary, 140)),
            EventKind::Verified { command, ok, .. } => println!("[{t:>4}s] VERIFY {command} ok={ok}"),
            EventKind::Committed { sha, files, .. } => println!("[{t:>4}s] COMMIT {} {files:?}", &sha[..sha.len().min(8)]),
            EventKind::Merged { sha, branch } => println!("[{t:>4}s] MERGED {} into {branch}", &sha[..sha.len().min(8)]),
            EventKind::Error { message, fatal } => println!("[{t:>4}s] {who} ERROR fatal={fatal}: {}", short(message, 200)),
            EventKind::TurnCompleted { status, .. } => println!("[{t:>4}s] {who} turn {status:?}"),
            EventKind::AgentChanged { agent } => {
                println!("[{t:>4}s] agent {} {:?} {:?} {:?} {}", agent.name, agent.provider, agent.role, agent.status, agent.activity_label.as_deref().unwrap_or(""))
            }
            EventKind::MemoryAdded { memory } => println!("[{t:>4}s] MEMORY {:?}/{:?}: {}", memory.scope, memory.kind, short(&memory.text, 120)),
            EventKind::TripleAdded { triple } => println!("[{t:>4}s] KG {} —{}→ {}", triple.subject, triple.predicate, triple.object),
            EventKind::Retrieval { query, confidence, citations } => println!("[{t:>4}s] RETRIEVAL {confidence:?} {} cites for {}", citations.len(), short(query, 60)),
            other => println!("[{t:>4}s] {who} {}", other.type_name()),
        }
    }
    println!("\n== outcome: {outcome} after {}s", started.elapsed().as_secs());
    if let Some(p) = plan_id {
        let diff = core.plan_diff(&p).await.unwrap_or_else(|e| format!("(no diff: {e})"));
        println!("== plan diff ({} lines)\n{}", diff.lines().count(), diff.lines().take(80).collect::<Vec<_>>().join("\n"));
    }
    let snap = core.snapshot().await?;
    println!("== agents: {}", snap.agents.iter().map(|a| format!("{}({:?},{:?})", a.name, a.provider, a.role)).collect::<Vec<_>>().join(", "));
    println!("== memories: {}, triples: {}, events: {}", snap.memories.len(), snap.triples.len(), snap.as_of);
    core.shutdown().await;
    Ok(())
}
