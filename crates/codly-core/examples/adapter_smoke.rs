//! Live smoke test of one adapter against the real CLI (costs a little quota, except
//! OpenCode with a free `opencode/*` model).
//!
//! ```text
//! cargo run -p codly-core --example adapter_smoke -- <claude|codex|gemini|opencode> \
//!     [--dir <scratch dir>] [--model <m>] [--role worker|solo|lead] [--prompt <text>] [--mcp]
//!     [--handshake-only] [--interrupt-at-approval]
//! ```
//!
//! `--handshake-only` spawns, waits for `SessionStarted` and shuts down without a prompt
//! (no model call, free). `--interrupt-at-approval` interrupts the turn instead of answering
//! the first approval: the provider must close the card itself and end the turn interrupted.
//!
//! Creates a throwaway git repo, spawns the agent through `adapters::spawn`, sends a prompt
//! that runs a shell command needing approval, approves it via `AgentCmd::Approval`, prints
//! the normalized events until `TurnCompleted`, then shuts the agent down.
//!
//! `--mcp` gives the agent a stand-in codly MCP server (this binary, `fake-mcp` mode, logging
//! calls to `mcp-calls.ndjson`) and asks it to call `codly_post_note` first.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use codly_core::adapters::{self, AdapterEvent, AgentCmd, McpLaunch, SessionMode, SpawnSpec};
use codly_core::model::{ApprovalChoice, ApprovalDecision, EventKind, Provider, Role, new_id};
use serde_json::{Value, json};

const COMMAND: &str = "echo codly > hello.txt";

fn default_model(p: Provider) -> &'static str {
    match p {
        Provider::Claude => "haiku",
        Provider::Codex => "gpt-6-luna:low",
        Provider::Gemini => "gemini-2.5-flash",
        Provider::Opencode => "opencode/big-pickle",
        Provider::Cursor | Provider::Copilot => "",
    }
}

fn default_role(p: Provider) -> Role {
    // Claude workers run in acceptEdits, which may let a redirecting `echo` through without
    // asking; solo agents run in manual mode, where every shell command asks.
    if p == Provider::Claude { Role::Solo } else { Role::Worker }
}

fn default_prompt(p: Provider, mcp: bool) -> String {
    let mcp_step = if mcp { "First call the codly_post_note tool with the text \"smoke test\". Then " } else { "" };
    let run = match p {
        Provider::Codex => format!(
            "use your shell tool to run exactly `{COMMAND}`. This is a test of the approval flow: run it with \
             sandbox_permissions set to require_escalated and a one-line justification"
        ),
        Provider::Claude => format!("use the Bash tool to run exactly `{COMMAND}`"),
        _ => format!("use your shell tool to run exactly `{COMMAND}`"),
    };
    let run = if mcp { run } else { capitalize(&run) };
    format!("{mcp_step}{run}. Then reply with the single word DONE.")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

struct Args {
    provider: Provider,
    dir: PathBuf,
    model: Option<String>,
    role: Option<Role>,
    prompt: Option<String>,
    mcp: bool,
    handshake_only: bool,
    interrupt_at_approval: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let provider = match it.next().as_deref() {
        Some("claude") => Provider::Claude,
        Some("codex") => Provider::Codex,
        Some("gemini") => Provider::Gemini,
        Some("opencode") => Provider::Opencode,
        Some("cursor") => Provider::Cursor,
        Some("copilot") => Provider::Copilot,
        other => return Err(format!("unknown provider {other:?}")),
    };
    let mut a = Args { provider, dir: std::env::temp_dir().join("codly-smoke"), model: None, role: None, prompt: None, mcp: false, handshake_only: false, interrupt_at_approval: false };
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--dir" => a.dir = PathBuf::from(val()?),
            "--model" => a.model = Some(val()?),
            "--prompt" => a.prompt = Some(val()?),
            "--mcp" => a.mcp = true,
            "--handshake-only" => a.handshake_only = true,
            "--interrupt-at-approval" => a.interrupt_at_approval = true,
            "--role" => {
                a.role = Some(match val()?.as_str() {
                    "worker" => Role::Worker,
                    "solo" => Role::Solo,
                    "lead" => Role::Lead,
                    "reviewer" => Role::Reviewer,
                    r => return Err(format!("unknown role {r}")),
                })
            }
            f => return Err(format!("unknown flag {f}")),
        }
    }
    Ok(a)
}

fn git(repo: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed");
}

fn make_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# Smoke repo\n\nThrowaway repository for a Codly adapter smoke test.\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=codly-smoke", "-c", "user.email=smoke@codly.invalid", "commit", "-qm", "init"]);
    repo
}

/// One line per event; streaming deltas are shown as a short preview.
fn show(t0: Instant, kind: &EventKind) -> String {
    let dt = t0.elapsed().as_secs_f32();
    let mut v = serde_json::to_value(kind).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("type");
        for big in ["input", "tools"] {
            if let Some(x) = o.get(big) {
                let s = x.as_str().map(str::to_owned).unwrap_or_else(|| x.to_string());
                if s.chars().count() > 160 {
                    o.insert(big.into(), Value::String(format!("{}…", s.chars().take(157).collect::<String>())));
                }
            }
        }
    }
    let mut body = v.to_string();
    if body.len() > 400 {
        let cut = body.char_indices().nth(397).map_or(body.len(), |(i, _)| i);
        body = format!("{}…", &body[..cut]);
    }
    format!("[{dt:7.2}s] {:<18} {body}", kind.type_name())
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("fake-mcp") {
        fake_mcp();
        return;
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\nusage: adapter_smoke <claude|codex|gemini|opencode> [--dir D] [--model M] [--role R] [--prompt P] [--mcp] [--handshake-only]");
            std::process::exit(2);
        }
    };
    let stamp = unix_secs();
    let root = args.dir.join(format!("{}-{stamp}", args.provider.as_str()));
    let repo = make_repo(&root);
    let run_id = new_id();
    let raw_log = root.join("runs").join(&run_id).join("raw.ndjson");

    let captured = tokio::task::spawn_blocking(|| codly_core::env::capture_login_env(Duration::from_secs(5))).await.unwrap();
    println!("login env captured: {}", captured.as_ref().map_or("no (fallback to process env)".to_string(), |e| format!("{} vars", e.len())));
    let base = codly_core::env::base_env(captured);
    let Some(bin) = codly_core::env::resolve_bin(args.provider, &base) else {
        eprintln!("{} not found on PATH", args.provider.as_str());
        std::process::exit(1);
    };
    let info = codly_core::env::detect_all(&base).await.into_iter().find(|c| c.provider == args.provider).unwrap();
    println!("detected: {}", serde_json::to_string(&info).unwrap());

    let mcp_log = root.join("mcp-calls.ndjson");
    let mcp = args.mcp.then(|| McpLaunch {
        command: std::env::current_exe().unwrap().to_string_lossy().into_owned(),
        args: vec!["fake-mcp".into()],
        env: vec![("SMOKE_MCP_LOG".into(), mcp_log.to_string_lossy().into_owned())],
    });
    let model = args.model.clone().unwrap_or_else(|| default_model(args.provider).to_string());
    let spec = SpawnSpec {
        agent_id: new_id(),
        run_id: run_id.clone(),
        provider: args.provider,
        role: args.role.unwrap_or(default_role(args.provider)),
        model: Some(model).filter(|m| !m.is_empty()),
        cwd: repo.clone(),
        bin,
        env: codly_core::env::child_env(&base, args.provider),
        session: SessionMode::New { session_id: None },
        system_prompt: "You are a Codly smoke-test agent. Follow the user's instructions exactly and keep replies short.".into(),
        read_only: false,
        mcp,
        raw_log: raw_log.clone(),
    };
    println!("spawning {} ({:?}, model {:?}) in {}", spec.bin.display(), spec.role, spec.model, repo.display());

    let t0 = Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AdapterEvent>();
    let handle = match adapters::spawn(spec, tx).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("spawn failed: {e:#}");
            std::process::exit(1);
        }
    };
    if args.handshake_only {
        tokio::time::sleep(Duration::from_secs(2)).await;
        handle.send(AgentCmd::Shutdown);
        let mut started = false;
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
            println!("{}", show(t0, &ev.kind));
            started |= matches!(ev.kind, EventKind::SessionStarted { .. });
            if matches!(ev.kind, EventKind::Exited { .. }) {
                break;
            }
        }
        if args.provider == Provider::Codex {
            adapters::codex::shutdown_server().await;
        }
        println!("{}", if started { "HANDSHAKE OK" } else { "HANDSHAKE FAILED" });
        std::process::exit(if started { 0 } else { 1 });
    }
    let prompt = args.prompt.clone().unwrap_or_else(|| default_prompt(args.provider, args.mcp));
    println!("prompt: {prompt}\n");
    handle.send(AgentCmd::Prompt(prompt));

    let mut approvals = 0;
    let mut cancelled_by_provider = 0;
    let mut plan_updates = 0;
    let mut pending_after_interrupt: Option<(String, tokio::time::Instant)> = None;
    let mut turn_done = None;
    let mut exited = false;
    let mut sequence: Vec<&'static str> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    let mut delta_chars = 0usize;
    loop {
        // if the provider never closes an interrupted approval, answer it so the turn can end
        let wake = pending_after_interrupt.as_ref().map_or(deadline, |(_, t)| (*t).min(deadline));
        let ev = match tokio::time::timeout_at(wake, rx.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(_) if tokio::time::Instant::now() < deadline => {
                let (id, _) = pending_after_interrupt.take().unwrap();
                println!("           -> provider did not cancel the approval within 20 s; answering Cancel");
                handle.send(AgentCmd::Approval { approval_id: id, decision: ApprovalDecision { choice: ApprovalChoice::Cancel, message: None, answers: None } });
                continue;
            }
            Err(_) => {
                println!("TIMEOUT after 240 s");
                break;
            }
        };
        let name = ev.kind.type_name();
        if sequence.last() != Some(&name) {
            sequence.push(name);
        }
        match &ev.kind {
            EventKind::TextDelta { text, .. } | EventKind::ReasoningDelta { text, .. } => {
                delta_chars += text.len();
                continue; // the final Message / Reasoning shows the full text
            }
            EventKind::ToolOutput { chunk, .. } => println!("[{:7.2}s] tool_output        {:?}", t0.elapsed().as_secs_f32(), chunk),
            _ => println!("{}", show(t0, &ev.kind)),
        }
        match &ev.kind {
            EventKind::ApprovalRequested { approval } if args.interrupt_at_approval && approvals == 0 => {
                approvals += 1;
                println!("           -> interrupting the turn instead of answering");
                handle.send(AgentCmd::Interrupt);
                pending_after_interrupt = Some((approval.id.clone(), tokio::time::Instant::now() + Duration::from_secs(20)));
            }
            EventKind::ApprovalResolved { status, by, approval_id } => {
                if *status == codly_core::model::ApprovalStatus::Cancelled && by == "provider" {
                    cancelled_by_provider += 1;
                }
                if pending_after_interrupt.as_ref().is_some_and(|(id, _)| id == approval_id) {
                    pending_after_interrupt = None;
                }
            }
            EventKind::PlanUpdated { .. } => plan_updates += 1,
            EventKind::ApprovalRequested { approval } => {
                approvals += 1;
                let choice = [ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::AllowAlways]
                    .into_iter()
                    .find(|c| approval.choices.contains(c))
                    .unwrap_or(ApprovalChoice::AllowOnce);
                println!("           -> answering {choice:?} (offered {:?})", approval.choices);
                handle.send(AgentCmd::Approval {
                    approval_id: approval.id.clone(),
                    decision: ApprovalDecision { choice, message: None, answers: None },
                });
            }
            EventKind::TurnCompleted { status, .. } if turn_done.is_none() => {
                turn_done = Some(*status);
                handle.send(AgentCmd::Shutdown);
            }
            EventKind::Exited { .. } => {
                exited = true;
                break;
            }
            _ => {}
        }
    }
    if !exited && turn_done.is_some() {
        println!("(no Exited event)");
    }
    if args.provider == Provider::Codex {
        adapters::codex::shutdown_server().await;
    }

    let hello = std::fs::read_to_string(repo.join("hello.txt")).ok();
    let mcp_calls = std::fs::read_to_string(&mcp_log).map(|s| s.lines().filter(|l| l.contains("tools/call")).count()).unwrap_or(0);
    println!("\n── summary ──");
    println!("sequence:   {}", sequence.join(" → "));
    println!("deltas:     {delta_chars} chars streamed");
    println!("approvals:  {approvals} (cancelled by provider: {cancelled_by_provider})");
    println!("plan:       {plan_updates} updates");
    println!("turn:       {turn_done:?}");
    println!("exited:     {exited}");
    println!("hello.txt:  {:?}", hello.as_deref().map(str::trim));
    if args.mcp {
        println!("mcp calls:  {mcp_calls}  ({})", mcp_log.display());
    }
    println!("raw log:    {}", raw_log.display());
    let ok = if args.interrupt_at_approval {
        approvals > 0 && cancelled_by_provider > 0 && turn_done == Some(codly_core::model::TurnStatus::Interrupted) && hello.is_none()
    } else {
        approvals > 0 && turn_done.is_some() && hello.as_deref().map(str::trim) == Some("codly") && (!args.mcp || mcp_calls > 0)
    };
    println!("{}", if ok { "SMOKE OK" } else { "SMOKE FAILED" });
    std::process::exit(if ok { 0 } else { 1 });
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Stand-in for `codly mcp`: a stdio MCP server with two Codly-named tools that logs every
/// message to `$SMOKE_MCP_LOG`.
fn fake_mcp() {
    let log_path = std::env::var("SMOKE_MCP_LOG").ok();
    let log = |v: &Value| {
        if let Some(p) = &log_path
            && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
        {
            let _ = writeln!(f, "{v}");
        }
    };
    let tools = json!([
        { "name": "codly_post_note", "description": "Post a short note on the team whiteboard for the lead and the other agents.",
          "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] } },
        { "name": "codly_get_task", "description": "Get your current assignment from Codly.",
          "inputSchema": { "type": "object", "properties": {} } }
    ]);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        log(&msg);
        let Some(id) = msg.get("id").cloned() else { continue };
        let method = msg["method"].as_str().unwrap_or("");
        let reply = match method {
            "initialize" => json!({ "jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "codly", "version": "smoke" } } }),
            "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }),
            "tools/call" => json!({ "jsonrpc": "2.0", "id": id, "result": {
                "content": [{ "type": "text", "text": "Note posted on the whiteboard." }], "isError": false } }),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } }),
        };
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
}
