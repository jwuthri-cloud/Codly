//! Claude Code adapter: one long-lived `claude -p` process per agent, NDJSON stream-json on
//! stdout and the control protocol on stdin — the wire protocol both official Agent SDKs
//! speak (cli-claude-codex §2.2–2.9, mapping §5.3.1).
//!
//! Verified live on 2.1.309 (CRITIQUE §10):
//! - `can_use_tool` arrives as a `control_request` and is answered with a `control_response`.
//! - MCP tools are deferred behind ToolSearch only when `ToolSearch` is in `--tools`; Codly
//!   omits it and also marks its server `alwaysLoad`, so `mcp__codly__*` is always callable.
//! - `system/init` (with the real tool list) arrives only after the first user message, so
//!   `SessionStarted` is emitted twice: at spawn from the `initialize` handshake (tools empty)
//!   and once more from `system/init`. Treat it as an upsert.
//!
//! Prompts are queued Codly-side while a turn runs, so every `AgentCmd::Prompt` is exactly one
//! turn with its own `TurnStarted` and `TurnCompleted`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use regex::Regex;
use serde_json::{Value, json};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use super::classify::{self, OUTPUT_MAX};
use super::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec, attachments, startup};
use crate::model::{
    ActivityKind, AgentStatus, Approval, ApprovalChoice, ApprovalDecision, ApprovalStatus, ApprovalSubject, Billing, EventKind,
    Id, MessagePhase, PlanStep, Provider, Role, TurnStatus, Usage, new_id, now_ms,
};
use crate::proc::{self, Inbound, RawLog, StderrTail};

/// Explicit worker tool list (CRITIQUE D5/G5): the default `-p` set includes cross-session
/// messaging, cron, remote triggers and worktree tools that fight Codly, so they stay out:
/// recurring work is Codly's Schedules, plans are `codly_submit_plan`, questions are
/// `codly_ask_user`. Grep/Glob and the Task* plan tools exist only when named. Monitor and
/// TaskStop are how a background command is watched and stopped; Skill and LSP are the
/// user's skills and code navigation. ToolSearch joins only with the user's own setup
/// (`TOOL_SEARCH`): with it Claude defers MCP tools and the Task* tools behind a search step
/// (free probe, §10), which Codly's own tools avoid with `alwaysLoad`.
pub const WORKER_TOOLS: &[&str] = &[
    "Bash", "Read", "Edit", "Write", "Grep", "Glob", "TaskCreate", "TaskUpdate", "TaskList", "TaskGet", "WebFetch", "WebSearch", "Agent", "Monitor",
    "TaskStop", "Skill", "LSP",
];
/// Lead, reviewers and other read-only roles: no Bash, no edits.
pub const READ_ONLY_TOOLS: &[&str] = &["Read", "Grep", "Glob", "WebFetch", "WebSearch", "Skill", "LSP"];
/// Tools whose presence depends on the CLI's version or the user's setup (LSP needs a language
/// server, Skill needs skills): asked for, but never reported as "not enabled".
const OPTIONAL_TOOLS: &[&str] = &["Monitor", "TaskStop", "Skill", "LSP", TOOL_SEARCH];
/// Needed to reach the user's own MCP servers when there are many of them.
const TOOL_SEARCH: &str = "ToolSearch";

const INIT_TIMEOUT: Duration = Duration::from_secs(90);

/// How long a turn has to end after an interrupt request. Past it the CLI is stuck: its process
/// is ended (stdin closed, then SIGTERM, then SIGKILL) and the session resumes with the next run.
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(8);

pub fn tools_for(spec: &SpawnSpec) -> Vec<&'static str> {
    let mut t = if spec.read_only { READ_ONLY_TOOLS } else { WORKER_TOOLS }.to_vec();
    if spec.use_user_setup {
        t.push(TOOL_SEARCH);
    }
    t
}

/// Workers edit freely inside their own worktree (Bash still asks). Solo agents work in the
/// user's checkout and read-only roles have nothing to accept, so both ask for everything.
pub fn permission_mode(spec: &SpawnSpec) -> &'static str {
    if spec.role == Role::Worker && !spec.read_only { "acceptEdits" } else { "manual" }
}

/// The `--mcp-config` document for the codly server (or None without MCP).
pub fn mcp_config(spec: &SpawnSpec) -> Option<Value> {
    let m = spec.mcp.as_ref()?;
    let env: serde_json::Map<String, Value> = m.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
    let mut doc = json!({ "mcpServers": { "codly": {
        "type": "stdio", "command": m.command, "args": m.args, "env": env,
        // keep codly tools loaded even if a future default defers MCP tools
        "alwaysLoad": true
    }}});
    // The user's default servers (Settings → Connections), as enabled when this run started.
    for x in crate::housekeeping::connections::load_ready(&spec.raw_log) {
        let env: serde_json::Map<String, Value> = x.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        doc["mcpServers"][&x.name] = json!({ "type": "stdio", "command": x.command, "args": x.args, "env": env });
    }
    Some(doc)
}

/// `--allowedTools` patterns for the default servers that only read; the others ask first.
fn default_mcp_allowed(spec: &SpawnSpec) -> Vec<String> {
    crate::housekeeping::connections::load_ready(&spec.raw_log).into_iter().filter(|x| x.read_only).map(|x| format!("mcp__{}__*", x.name)).collect()
}

/// The full `claude` argv. `mcp_config_path` is a 0600 file holding [`mcp_config`] (a file,
/// not an inline string, so the per-run MCP token never shows up in `ps`).
pub fn build_args(spec: &SpawnSpec, session_id: &str, mcp_config_path: Option<&Path>) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        permission_mode(spec),
    ]
    .map(String::from)
    .into();
    match &spec.session {
        SessionMode::Resume { session_id } => a.extend(["--resume".into(), session_id.clone()]),
        SessionMode::New { .. } => a.extend(["--session-id".into(), session_id.to_string()]),
    }
    a.extend(["--tools".into(), tools_for(spec).join(",")]);
    // Strict = only Codly's server and the Connections; the user's own setup adds their MCP
    // servers, plugins and project settings, as in their own Claude Code.
    if !spec.use_user_setup {
        a.push("--strict-mcp-config".into());
    }
    if let Some(p) = mcp_config_path {
        a.extend(["--mcp-config".into(), p.to_string_lossy().into_owned()]);
        let mut allowed = vec!["mcp__codly__*".to_string()];
        allowed.extend(default_mcp_allowed(spec));
        a.extend(["--allowedTools".into(), allowed.join(",")]);
    }
    a.extend(["--setting-sources".into(), if spec.use_user_setup { "user,project,local" } else { "user,local" }.into()]);
    if !spec.system_prompt.trim().is_empty() {
        a.extend(["--append-system-prompt".into(), spec.system_prompt.clone()]);
    }
    let (model, effort) = classify::split_model_effort(spec.model.as_deref());
    if let Some(m) = model {
        a.extend(["--model".into(), m]);
    }
    if let Some(e) = effort {
        a.extend(["--effort".into(), e]);
    }
    // BYOK token command (byok.rs): Claude runs it for its API key and re-runs it on expiry.
    if let Some(cmd) = &spec.byok_command {
        a.extend(["--settings".into(), serde_json::json!({ "apiKeyHelper": cmd }).to_string()]);
    }
    let role = format!("{:?}", spec.role).to_lowercase();
    let short: String = spec.agent_id.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    a.extend(["-n".into(), format!("codly:{role}-{short}")]);
    a
}

pub async fn spawn(spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> anyhow::Result<AgentHandle> {
    let session_id = match &spec.session {
        SessionMode::New { session_id: Some(id) } => id.clone(),
        SessionMode::New { session_id: None } => uuid::Uuid::new_v4().to_string(),
        SessionMode::Resume { session_id } => session_id.clone(),
    };
    let raw = Arc::new(RawLog::open(&spec.raw_log).with_context(|| format!("cannot open raw log {}", spec.raw_log.display()))?);
    let mcp_file = match mcp_config(&spec) {
        Some(doc) => Some(PrivateFile::write(&spec.raw_log.with_file_name(MCP_CONFIG_FILE), &doc.to_string())?),
        None => None,
    };
    let args = build_args(&spec, &session_id, mcp_file.as_ref().map(|f| f.0.as_path()));
    startup::check_cwd(&spec.cwd)?;
    let mut child = proc::command(&spec.bin, &args, &spec.cwd, &spec.env)
        .spawn()
        .with_context(|| format!("failed to start {}", spec.bin.display()))?;
    let pgid = child.id().map(|p| p as i32).unwrap_or(0);
    proc::record_group(&spec.raw_log, pgid, now_ms());
    let stdin = child.stdin.take().context("no stdin")?;
    let lines = proc::spawn_json_reader(child.stdout.take().context("no stdout")?, raw.clone());
    let stderr = StderrTail::spawn(child.stderr.take().context("no stderr")?, 64 << 10);
    let (exit_tx, exit_rx) = oneshot::channel();
    // Also ends what the CLI left running in its group (a dev server, a watcher) once it is gone.
    proc::reap(child, move |code| {
        let _ = exit_tx.send(code);
    });

    let (tx, cmds) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let norm = ClaudeNormalizer::new(&spec, session_id.clone());
    let actor = Actor {
        agent_id: spec.agent_id.clone(),
        run_id: spec.run_id.clone(),
        events,
        stdin: Some(stdin),
        raw,
        pgid,
        stderr,
        norm,
        queue: VecDeque::new(),
        turn_active: false,
        next_req: 0,
        stage: Stage::Running,
        deadline: None,
        start_failed: false,
        interrupt_failed: false,
        _mcp_file: mcp_file,
    };
    tokio::spawn(actor.run(cmds, lines, exit_rx, ready_tx));
    match tokio::time::timeout(INIT_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(()))) => Ok(AgentHandle { agent_id: spec.agent_id, run_id: spec.run_id, tx }),
        Ok(Ok(Err(e))) => Err(startup::explain(&spec, None, e)),
        Ok(Err(_)) => Err(anyhow!("claude adapter stopped during startup")),
        Err(_) => {
            let _ = tx.send(AgentCmd::Shutdown);
            Err(anyhow!("claude did not answer the initialize request within {} s", INIT_TIMEOUT.as_secs()))
        }
    }
}

/// Name of the `--mcp-config` document in a run's folder.
const MCP_CONFIG_FILE: &str = "claude-mcp.json";

/// A file with this run's keys in it (the `--mcp-config` document: the run's Codly token and
/// the user's Connection keys). Only the user can read it, and it is removed when this value
/// is dropped: when the run is over, or when its start failed.
struct PrivateFile(PathBuf);

impl PrivateFile {
    fn write(path: &Path, contents: &str) -> anyhow::Result<PrivateFile> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        let file = PrivateFile(path.to_path_buf());
        f.write_all(contents.as_bytes())?;
        Ok(file)
    }
}

impl Drop for PrivateFile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {e}", self.0.display());
        }
    }
}

/// Remove the `--mcp-config` documents that earlier runs left under `runs_dir`: a run whose
/// end Codly did not see (a crash, a version that kept the file) cannot remove its own. For
/// the start of the app, before any run.
pub fn remove_stale_mcp_configs(runs_dir: &Path) {
    let Ok(runs) = std::fs::read_dir(runs_dir) else { return };
    for run in runs.flatten() {
        let file = run.path().join(MCP_CONFIG_FILE);
        if let Err(e) = std::fs::remove_file(&file)
            && !matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory)
        {
            tracing::warn!("cannot remove {}: {e}", file.display());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Running,
    /// Shutdown asked while a turn ran: interrupt sent, waiting for its `result`.
    Interrupting,
    /// stdin closed, waiting for the process to exit.
    Closing,
    /// SIGTERM → SIGKILL ladder running.
    Killing,
}

struct Actor {
    agent_id: Id,
    run_id: Id,
    events: mpsc::UnboundedSender<AdapterEvent>,
    stdin: Option<ChildStdin>,
    raw: Arc<RawLog>,
    pgid: i32,
    stderr: StderrTail,
    norm: ClaudeNormalizer,
    queue: VecDeque<String>,
    turn_active: bool,
    next_req: u64,
    stage: Stage,
    /// While running: when an interrupted turn must have ended. In a shutdown: the stage's limit.
    deadline: Option<tokio::time::Instant>,
    start_failed: bool,
    /// An interrupt got no result within `INTERRUPT_TIMEOUT`, so the process is being ended.
    interrupt_failed: bool,
    /// Held, never read: dropping the actor removes the file (`PrivateFile`).
    _mcp_file: Option<PrivateFile>,
}

impl Actor {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.agent_id.clone(), run_id: self.run_id.clone(), kind });
    }

    async fn write(&mut self, msg: &Value) -> bool {
        let Some(stdin) = self.stdin.as_mut() else { return false };
        match proc::write_json_within(stdin, &self.raw, msg, proc::WRITE_TIMEOUT).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("claude stdin write failed: {e}");
                self.stdin = None;
                if e.kind() == std::io::ErrorKind::TimedOut {
                    self.stopped_reading();
                }
                false
            }
        }
    }

    /// The CLI no longer reads its stdin: nothing can be said to it, not even "stop". Its
    /// process is ended; the exit is reported as any other, and the next run resumes the session.
    fn stopped_reading(&self) {
        self.raw.note("claude stopped reading its input: ending its process");
        self.emit(EventKind::Error { message: "Claude stopped reading its input, so Codly ended its process.".into(), fatal: false });
        tokio::spawn(proc::kill_group(self.pgid, Duration::from_secs(2)));
    }

    async fn control(&mut self, subtype: &str, mut request: Value) -> String {
        self.next_req += 1;
        let id = format!("codly-{}", self.next_req);
        request["subtype"] = json!(subtype);
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request })).await;
        id
    }

    async fn run(
        mut self,
        mut cmds: mpsc::UnboundedReceiver<AgentCmd>,
        mut lines: mpsc::UnboundedReceiver<Inbound>,
        mut exit_rx: oneshot::Receiver<Option<i64>>,
        ready: oneshot::Sender<anyhow::Result<()>>,
    ) {
        let mut ready = Some(ready);
        let init_id = self.control("initialize", json!({ "hooks": null })).await;
        let mut cmds_open = true;
        loop {
            let deadline = self.deadline;
            let timer = async move {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                inbound = lines.recv() => match inbound {
                    Some(Inbound::Json(v)) => self.on_json(v, &init_id, &mut ready).await,
                    Some(Inbound::Text(_)) => self.emit(EventKind::Unknown { raw_type: "non_json".into() }),
                    Some(Inbound::TooLong(n)) => self.emit(EventKind::Error {
                        message: format!("dropped a {n}-byte line from claude (over the 64 MiB cap)"),
                        fatal: false,
                    }),
                    Some(Inbound::Eof) | None => break,
                },
                cmd = cmds.recv(), if cmds_open => match cmd {
                    Some(cmd) => self.on_cmd(cmd).await,
                    // every handle dropped: nobody can talk to this agent any more
                    None => { cmds_open = false; self.begin_shutdown().await }
                },
                _ = timer => self.on_deadline().await,
            }
        }

        // stdout closed: the process is exiting
        let code = match tokio::time::timeout(Duration::from_secs(5), &mut exit_rx).await {
            Ok(Ok(code)) => code,
            _ => {
                proc::kill_group(self.pgid, Duration::from_secs(1)).await;
                exit_rx.await.ok().flatten()
            }
        };
        let tail = self.stderr.tail(2000);
        if let Some(r) = ready.take() {
            let detail = if tail.is_empty() { String::new() } else { format!(": {tail}") };
            let _ = r.send(Err(anyhow!("claude exited during startup ({}){detail}", startup::exit_text(code))));
            return;
        }
        if self.start_failed {
            return;
        }
        for e in self.norm.cancel_all_approvals() {
            self.emit(e);
        }
        if self.turn_active {
            let (status, result_text) = self.unfinished_turn();
            self.emit(EventKind::TurnCompleted { status, result_text, duration_ms: None });
        }
        if self.stage == Stage::Running {
            let detail = if tail.is_empty() { String::new() } else { format!(": {tail}") };
            self.emit(EventKind::Error { message: format!("claude exited unexpectedly (code {code:?}){detail}"), fatal: true });
        }
        self.emit(EventKind::StatusChanged { status: AgentStatus::Offline });
        self.emit(EventKind::Exited { code });
    }

    async fn on_json(&mut self, v: Value, init_id: &str, ready: &mut Option<oneshot::Sender<anyhow::Result<()>>>) {
        if v["type"] == "control_response" && !self.norm.asked(&v["response"]) {
            let resp = &v["response"];
            if resp["request_id"].as_str() == Some(init_id) {
                let Some(r) = ready.take() else { return };
                if resp["subtype"] == "success" {
                    let started = self.norm.on_initialize(&resp["response"]);
                    self.emit(started);
                    let _ = r.send(Ok(()));
                } else {
                    let _ = r.send(Err(anyhow!("claude rejected the initialize request: {}", resp["error"])));
                    self.start_failed = true; // spawn() returned the error: no events for this run
                    self.begin_shutdown().await;
                }
            } else if resp["subtype"] == "error" {
                self.emit(EventKind::Error { message: format!("claude control request failed: {}", resp["error"]), fatal: false });
            }
            return;
        }
        let events = self.norm.on_message(&v);
        let turn_done = events.iter().any(|e| matches!(e, EventKind::TurnCompleted { .. }));
        for e in events {
            self.emit(e);
        }
        for reply in std::mem::take(&mut self.norm.outbox) {
            self.write(&reply).await;
        }
        if turn_done {
            self.turn_active = false;
            if self.stage == Stage::Interrupting {
                self.close_stdin();
            } else if self.stage == Stage::Running {
                self.deadline = None; // an interrupted turn ended in time
                if let Some(next) = self.queue.pop_front() {
                    self.send_prompt(next).await;
                }
            }
        }
    }

    /// How a turn that is still open when the process exits ends: interrupted when Codly ended
    /// the process because the turn did not stop, failed otherwise.
    fn unfinished_turn(&self) -> (TurnStatus, Option<String>) {
        if self.interrupt_failed {
            (TurnStatus::Interrupted, None)
        } else {
            (TurnStatus::Failed, Some("Claude exited before the turn finished".into()))
        }
    }

    async fn on_cmd(&mut self, cmd: AgentCmd) {
        match cmd {
            AgentCmd::Prompt(text) => {
                if self.stage != Stage::Running {
                    return;
                }
                if self.turn_active {
                    self.queue.push_back(text);
                } else {
                    self.send_prompt(text).await;
                }
            }
            AgentCmd::Interrupt => {
                if self.stage != Stage::Running {
                    return; // a shutdown is under way: it interrupts the turn itself
                }
                // Sent even when no turn seems to run: Claude answers an idle interrupt at once
                // (`still_queued: []`), so "stop" is never dropped on a flag that went stale.
                self.norm.interrupt_requested = self.turn_active;
                self.control("interrupt", json!({})).await;
                if self.turn_active {
                    self.deadline = Some(tokio::time::Instant::now() + INTERRUPT_TIMEOUT);
                }
            }
            AgentCmd::Approval { approval_id, decision } => match self.norm.answer(&approval_id, &decision) {
                Some((reply, events)) => {
                    if self.write(&reply).await {
                        for e in events {
                            self.emit(e);
                        }
                    } else {
                        // The answer never reached the CLI: the card must not say it did.
                        self.emit(EventKind::ApprovalResolved { approval_id, status: ApprovalStatus::Expired, by: "provider".into() });
                    }
                }
                None => tracing::warn!("claude: no pending approval {approval_id}"),
            },
            AgentCmd::Shutdown => self.begin_shutdown().await,
        }
    }

    async fn send_prompt(&mut self, text: String) {
        // Attached images go as image blocks before the text, which keeps their paths.
        let attached = attachments::images(&text).await;
        if let Some(note) = attached.note {
            self.emit(EventKind::Error { message: note, fatal: false });
        }
        let msg = json!({
            "type": "user", "session_id": "", "parent_tool_use_id": null,
            "message": { "role": "user", "content": attachments::claude_content(&text, &attached.images) },
        });
        if !self.write(&msg).await {
            self.emit(EventKind::Error { message: "could not send the prompt: claude's stdin is closed".into(), fatal: false });
            return;
        }
        self.turn_active = true;
        for e in self.norm.turn_started(&text) {
            self.emit(e);
        }
    }

    /// Shutdown ladder: interrupt (≤ 5 s for the result) → close stdin (≤ 3 s) → SIGTERM → SIGKILL.
    async fn begin_shutdown(&mut self) {
        if self.stage != Stage::Running {
            return;
        }
        self.queue.clear();
        if self.turn_active {
            self.norm.interrupt_requested = true;
            self.control("interrupt", json!({})).await;
            self.stage = Stage::Interrupting;
            self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
        } else {
            self.close_stdin();
        }
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
        self.stage = Stage::Closing;
        self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(3));
    }

    async fn on_deadline(&mut self) {
        match self.stage {
            Stage::Interrupting => self.close_stdin(),
            Stage::Closing => {
                self.stage = Stage::Killing;
                self.deadline = None;
                let pgid = self.pgid;
                tokio::spawn(proc::kill_group(pgid, Duration::from_secs(3)));
            }
            // The turn did not end after the interrupt: the CLI is stuck in a tool or a request
            // that ignores it. End the process; the session resumes with the next run.
            Stage::Running => {
                self.interrupt_failed = true;
                let unsent = super::unsent_note(self.queue.len());
                self.queue.clear();
                self.emit(EventKind::Error {
                    message: format!(
                        "Claude did not stop within {} s, so Codly ended its process. The session is kept: its next run resumes it.{unsent}",
                        INTERRUPT_TIMEOUT.as_secs()
                    ),
                    fatal: false,
                });
                self.emit(super::gave_up("Claude", INTERRUPT_TIMEOUT));
                self.close_stdin();
            }
            Stage::Killing => self.deadline = None,
        }
    }
}

// ───────────────────────────── normalization (pure) ─────────────────────────────

struct ToolInfo {
    name: String,
    input: Value,
    activity: ActivityKind,
    started: Instant,
    /// The subagent that made this call (the id of the `Agent` call that started it).
    helper: Option<String>,
}

/// A subagent: a helper the agent started with one `Agent` tool call. Its own lines carry
/// that call's id as `parent_tool_use_id`, and `system/task_*` lines tell how it goes
/// (cli-claude-stream-json-2.1.311 §4).
struct Helper {
    description: String,
    /// Its kind ("general-purpose"), once a `SubAgentProgress` said it.
    name: Option<String>,
    /// It works on after its `Agent` call returned, also past the end of the turn.
    background: bool,
    done: bool,
}

/// The id of the one `mcp_status` request a run sends (the actor's own ids are `codly-<n>`).
const MCP_STATUS_REQUEST: &str = "codly-mcp-status";

struct PendingApproval {
    request_id: String,
    tool_use_id: Option<String>,
    tool_name: String,
    input: Value,
    suggestions: Vec<Value>,
}

#[derive(Clone)]
struct PlanItem {
    id: String,
    text: String,
    status: String,
}

/// An API failure Codly can explain: what happened and what to do about it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ApiFailure {
    SignedOut,
    OrgNotAllowed,
    Billing,
    OnHold,
    ModelNotFound,
    CloudCredentials,
    RateLimit,
    Overloaded,
    ServerError,
}

impl ApiFailure {
    /// From the CLI's own category (`system/api_retry.error`), else from the HTTP status
    /// (`api_retry.error_status`, `result.api_error_status`).
    fn of(kind: &str, status: Option<i64>) -> Option<Self> {
        Some(match (kind, status) {
            ("authentication_failed", _) => Self::SignedOut,
            ("oauth_org_not_allowed", _) => Self::OrgNotAllowed,
            ("billing_error", _) => Self::Billing,
            ("account_on_hold", _) => Self::OnHold,
            ("model_not_found", _) => Self::ModelNotFound,
            ("cloud_credential_error", _) => Self::CloudCredentials,
            ("rate_limit", _) => Self::RateLimit,
            ("overloaded", _) => Self::Overloaded,
            ("server_error", _) => Self::ServerError,
            (_, Some(401)) => Self::SignedOut,
            (_, Some(402)) => Self::Billing,
            (_, Some(429)) => Self::RateLimit,
            (_, Some(529)) => Self::Overloaded,
            (_, Some(500..=599)) => Self::ServerError,
            _ => return None,
        })
    }

    /// Waiting cannot fix it: the user must act.
    fn needs_you(self) -> bool {
        !matches!(self, Self::RateLimit | Self::Overloaded | Self::ServerError)
    }
}

/// Why Claude retries an API call, for the `Retrying` row: "Anthropic's servers are
/// overloaded; next try in 5 s (10 tries at most)".
fn retry_reason(kind: &str, failure: Option<ApiFailure>, v: &Value) -> String {
    let mut reason = match failure {
        Some(ApiFailure::RateLimit) => "the usage limit is reached".to_string(),
        Some(ApiFailure::Overloaded) => "Anthropic's servers are overloaded".to_string(),
        Some(ApiFailure::ServerError) => "Anthropic's servers had an error".to_string(),
        _ if kind == "unknown" => "an API error".to_string(),
        // "invalid request", "max output tokens"
        _ => kind.replace('_', " "),
    };
    if let Some(ms) = v["retry_delay_ms"].as_i64() {
        reason.push_str(&format!("; next try in {} s", (ms.max(0) + 999) / 1000));
    }
    if let Some(max) = v["max_retries"].as_i64() {
        reason.push_str(&format!(" ({max} tries at most)"));
    }
    reason
}

/// Token counts of API calls, as the `usage` objects of the stream give them.
#[derive(Clone, Copy, Default)]
struct Tokens {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

impl Tokens {
    fn of(u: &Value) -> Self {
        let n = |k: &str| u[k].as_i64().unwrap_or(0);
        Tokens { input: n("input_tokens"), output: n("output_tokens"), cache_read: n("cache_read_input_tokens"), cache_write: n("cache_creation_input_tokens") }
    }

    fn plus(self, o: Tokens) -> Tokens {
        Tokens {
            input: self.input + o.input,
            output: self.output + o.output,
            cache_read: self.cache_read + o.cache_read,
            cache_write: self.cache_write + o.cache_write,
        }
    }
}

static TASK_CREATED_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"^Task #(\S+) created successfully").unwrap());
static EXIT_CODE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"(?m)^Exit code (\d+)").unwrap());

/// Turns Claude stream-json lines into `EventKind`s. No IO: the actor feeds it lines and
/// writes `outbox` replies; tests replay fixtures through it.
pub struct ClaudeNormalizer {
    agent_id: Id,
    run_id: Id,
    cwd: PathBuf,
    /// A worker: its `cwd` is a task worktree that Codly commits and later removes.
    worker: bool,
    session_id: String,
    model: Option<String>,
    /// "Claude Code is not signed in: run `…` in a terminal", for this run's account.
    sign_in: String,
    /// The run pays with the user's own API key or token command: signing in is no fix.
    own_key: bool,
    requested_tools: Vec<String>,
    status: Option<AgentStatus>,
    init_seen: bool,
    /// (api message id, block index) → block type, tool id, finalized.
    blocks: HashMap<(String, u64), (String, Option<String>, bool)>,
    /// Final text blocks waiting for the stop reason that tells their phase.
    pending_text: Vec<(String, String)>,
    tools: HashMap<String, ToolInfo>,
    tools_seen: HashSet<String>,
    /// The subagents of this run, by the id of the tool call that started each.
    helpers: HashMap<String, Helper>,
    /// The user's MCP servers that `system/init` gave as failed, until `mcp_status` says why.
    mcp_failed: Vec<String>,
    plan: Vec<PlanItem>,
    pending_creates: HashMap<String, String>,
    approvals: HashMap<Id, PendingApproval>,
    /// Tool calls denied in Codly: Claude's `permission_denied` line for them says nothing new.
    user_denied: HashSet<String>,
    /// Tool calls of this turn that Codly was asked about (a card), or whose denial a
    /// `permission_denied` line named: the result's `permission_denials` adds nothing on them.
    denials_known: HashSet<String>,
    /// The API failure Claude last retried, until an API call answers.
    api_failure: Option<ApiFailure>,
    /// The last error and the last message of this turn: a failed turn says its reason once.
    turn_error: Option<String>,
    turn_text: Option<String>,
    usage: Usage,
    /// This turn's tokens as the stream reports them: its finished API calls, and the call
    /// in progress. `usage` gets the turn's own numbers from its `result`.
    turn_live: Tokens,
    call_live: Tokens,
    last_context: Option<i64>,
    /// Set by the actor when Codly interrupted the turn (maps the next result to Interrupted).
    pub interrupt_requested: bool,
    /// Lines the actor must write: answers to the CLI's requests (e.g. errors for unsupported
    /// ones) and the normalizer's own `mcp_status` request.
    pub outbox: Vec<Value>,
}

impl ClaudeNormalizer {
    pub fn new(spec: &SpawnSpec, session_id: String) -> Self {
        ClaudeNormalizer {
            agent_id: spec.agent_id.clone(),
            run_id: spec.run_id.clone(),
            cwd: spec.cwd.clone(),
            worker: spec.role == Role::Worker,
            session_id,
            model: classify::split_model_effort(spec.model.as_deref()).0,
            sign_in: startup::not_signed_in(Provider::Claude, startup::account_home(spec), None),
            own_key: startup::uses_own_key(spec),
            requested_tools: tools_for(spec).iter().map(|t| t.to_string()).collect(),
            status: None,
            init_seen: false,
            blocks: HashMap::new(),
            pending_text: Vec::new(),
            tools: HashMap::new(),
            tools_seen: HashSet::new(),
            helpers: HashMap::new(),
            mcp_failed: Vec::new(),
            plan: Vec::new(),
            pending_creates: HashMap::new(),
            approvals: HashMap::new(),
            user_denied: HashSet::new(),
            denials_known: HashSet::new(),
            api_failure: None,
            turn_error: None,
            turn_text: None,
            usage: Usage::default(),
            turn_live: Tokens::default(),
            call_live: Tokens::default(),
            last_context: None,
            interrupt_requested: false,
            outbox: Vec::new(),
        }
    }

    fn set_status(&mut self, s: AgentStatus, out: &mut Vec<EventKind>) {
        if self.status != Some(s) {
            self.status = Some(s);
            out.push(EventKind::StatusChanged { status: s });
        }
    }

    /// An `Error` event, remembered as the last thing this turn reported.
    fn problem(&mut self, message: String, fatal: bool, out: &mut Vec<EventKind>) {
        self.turn_error = Some(message.clone());
        out.push(EventKind::Error { message, fatal });
    }

    /// One line for the user: what failed and what to do.
    fn explain(&self, failure: ApiFailure) -> String {
        match failure {
            ApiFailure::SignedOut if self.own_key => "Claude did not accept your API key or token: check it in Settings → API keys, then try again".into(),
            ApiFailure::SignedOut => format!("{}, then try again", self.sign_in),
            ApiFailure::OrgNotAllowed => "Claude does not let this account's organization use Claude Code: sign in with another account, then try again".into(),
            ApiFailure::Billing => "Claude reports a billing problem on this account: check your plan or your credits, then try again".into(),
            ApiFailure::OnHold => "This Claude account is on hold: check the account with Anthropic, then try again".into(),
            ApiFailure::ModelNotFound => match &self.model {
                Some(m) => format!("The model {m} is not available on this account: pick another model for this agent"),
                None => "This agent's model is not available on this account: pick another model for this agent".into(),
            },
            ApiFailure::CloudCredentials => "Claude could not use your cloud credentials (Bedrock or Vertex): renew them, then try again".into(),
            ApiFailure::RateLimit => "Claude's usage limit is reached: wait for it to reset, then try again".into(),
            ApiFailure::Overloaded => "Anthropic's servers are overloaded: wait a moment, then try again".into(),
            ApiFailure::ServerError => "Anthropic's servers had an error: wait a moment, then try again".into(),
        }
    }

    /// Why a failed turn failed, with what to do when Codly knows it: the API failure Claude
    /// was retrying, else the result's HTTP status, else the CLI's own words (`result`, or
    /// `errors` without its diagnostic lines).
    fn failure_reason(&self, v: &Value, retried: Option<ApiFailure>) -> Option<String> {
        // The spending limit of the process (`--max-budget-usd`, 2.1.311 §6): every later
        // turn of this run ends the same way, at once.
        if v["subtype"] == "error_max_budget_usd" {
            let said = cli_errors(v);
            let detail = if said.is_empty() { String::new() } else { format!(": {said}") };
            return Some(format!("Claude Code reached its spending limit, so it does no more work in this run{detail}"));
        }
        let raw = v["result"].as_str().map(str::trim).filter(|r| !r.is_empty());
        let failure = retried
            .or_else(|| ApiFailure::of("", v["api_error_status"].as_i64()))
            .or_else(|| raw.filter(|r| startup::says_signed_out(r)).map(|_| ApiFailure::SignedOut));
        if let Some(f) = failure {
            return Some(self.explain(f));
        }
        if let Some(r) = raw {
            return Some(r.to_string());
        }
        Some(cli_errors(v)).filter(|e| !e.is_empty())
    }

    /// The run's totals with this turn's API calls so far, as the stream reported them.
    fn live_usage(&self) -> Usage {
        let turn = self.turn_live.plus(self.call_live);
        Usage {
            input_tokens: self.usage.input_tokens + turn.input,
            output_tokens: self.usage.output_tokens + turn.output,
            cache_read_tokens: self.usage.cache_read_tokens + turn.cache_read,
            cache_write_tokens: self.usage.cache_write_tokens + turn.cache_write,
            context_used: self.last_context.or(self.usage.context_used),
            ..self.usage.clone()
        }
    }

    /// `SessionStarted` from the `initialize` control response (login, version, models; free).
    pub fn on_initialize(&mut self, resp: &Value) -> EventKind {
        // Signed in with a plan: {subscriptionType: "Claude Team", apiProvider: "firstParty", …};
        // a minimal env shows {tokenSource: "none", …}; an API key shows its tokenSource.
        let account = &resp["account"];
        let token_source = account["tokenSource"].as_str().or(account["apiKeySource"].as_str());
        let billing = if account["apiProvider"].as_str().is_some_and(|p| p != "firstParty") {
            Billing::ThirdParty
        } else if token_source.is_some_and(|t| t != "none") {
            Billing::ApiKey
        } else if account["subscriptionType"].is_string() || token_source == Some("none") {
            Billing::Subscription
        } else {
            Billing::Unknown
        };
        let alias = self.model.clone().unwrap_or_else(|| "default".into());
        let resolved = resp["models"]
            .as_array()
            .and_then(|ms| ms.iter().find(|m| m["value"] == alias.as_str()))
            .and_then(|m| m["resolvedModel"].as_str().map(str::to_owned));
        EventKind::SessionStarted {
            provider: Provider::Claude,
            model: resolved.or_else(|| self.model.clone()),
            cwd: self.cwd.to_string_lossy().into_owned(),
            cli_version: resp["claude_code_version"].as_str().map(str::to_owned),
            session_id: Some(self.session_id.clone()),
            billing,
            tools: Vec::new(),
        }
    }

    /// The actor sent a user message: a new turn starts.
    pub fn turn_started(&mut self, text: &str) -> Vec<EventKind> {
        let mut out = vec![EventKind::TurnStarted { input: Some(text.to_string()) }];
        self.interrupt_requested = false;
        self.api_failure = None;
        self.turn_error = None;
        self.turn_text = None;
        self.set_status(AgentStatus::Thinking, &mut out);
        out
    }

    pub fn on_message(&mut self, v: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or("") {
            "system" => self.on_system(v, &mut out),
            "stream_event" => self.on_stream_event(v, &mut out),
            "assistant" => self.on_assistant(v, &mut out),
            "user" => self.on_user(v, &mut out),
            "result" => self.on_result(v, &mut out),
            "rate_limit_event" => self.on_rate_limit(v, &mut out),
            "control_request" => self.on_control_request(v, &mut out),
            "control_cancel_request" => self.on_cancel(v, &mut out),
            // elapsed-time heartbeat only; Claude has no live command output (CRITIQUE G7)
            "tool_progress" => {}
            "control_response" => self.on_control_response(&v["response"], &mut out),
            other => out.push(EventKind::Unknown { raw_type: if other.is_empty() { "untyped".into() } else { other.into() } }),
        }
        out
    }

    fn on_system(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let subtype = v["subtype"].as_str().unwrap_or("");
        match subtype {
            "init" => self.on_init(v, out),
            "status" => match v["status"].as_str() {
                Some("requesting") => self.set_status(AgentStatus::Thinking, out),
                Some("compacting") => out.push(EventKind::Compacting { pre_tokens: None, post_tokens: None }),
                None => {} // permissionMode echo
                Some(_) => out.push(EventKind::Unknown { raw_type: "system/status".into() }),
            },
            "api_retry" => {
                let kind = v["error"].as_str().unwrap_or("unknown");
                let failure = ApiFailure::of(kind, v["error_status"].as_i64());
                self.api_failure = failure;
                match failure {
                    // Waiting cannot fix these: say what to do, once (Claude's own tries end
                    // in a failed turn).
                    Some(f) if f.needs_you() => {
                        let message = self.explain(f);
                        if self.turn_error.as_deref() != Some(message.as_str()) {
                            self.problem(message, true, out);
                        }
                    }
                    _ => {
                        if failure == Some(ApiFailure::RateLimit) {
                            self.set_status(AgentStatus::RateLimited, out);
                        }
                        out.push(EventKind::Retrying { attempt: v["attempt"].as_i64().unwrap_or(1), reason: retry_reason(kind, failure, v) });
                    }
                }
            }
            "permission_denied" => {
                // The denied call still ends as a failed tool result: this line says that
                // Claude Code refused it by itself (a rule, a hook, its permission mode). A
                // call denied in Codly needs no second line.
                let call = v["tool_use_id"].as_str();
                if !call.is_some_and(|id| self.user_denied.contains(id)) {
                    let tool = v["tool_name"].as_str().filter(|t| !t.is_empty()).unwrap_or("a tool");
                    let message = match v["message"].as_str().map(str::trim).filter(|m| !m.is_empty()) {
                        Some(m) => format!("Claude Code denied {tool} without asking: {m}"),
                        None => format!("Claude Code denied {tool} without asking"),
                    };
                    self.problem(message, false, out);
                }
                self.denials_known.extend(call.map(str::to_owned));
            }
            "model_refusal_fallback" => self.problem("The model refused this request: Claude Code goes on with its fallback model".into(), false, out),
            "model_refusal_no_fallback" => {
                self.problem("The model refused this request, and Claude Code has no fallback model to go on with".into(), false, out)
            }
            // Bookkeeping the timeline has no use for (the user's hooks, command lists, the
            // session title, the CLI's own summaries, a task's change of state that its
            // `task_notification` says again): known lines, kept in the raw log only.
            "hook_started" | "hook_progress" | "hook_response" | "commands_changed" | "session_title_changed" | "background_tasks_changed"
            | "files_persisted" | "post_turn_summary" | "task_updated" => {}
            "compact_boundary" => {
                let m = &v["compact_metadata"];
                out.push(EventKind::Compacting { pre_tokens: m["pre_tokens"].as_i64(), post_tokens: m["post_tokens"].as_i64() });
            }
            "task_started" | "task_progress" | "task_notification" => self.on_task(subtype, v, out),
            "thinking_tokens" => self.set_status(AgentStatus::Thinking, out),
            "session_state_changed" => match v["state"].as_str() {
                Some("requires_action") => self.set_status(AgentStatus::WaitingApproval, out),
                Some("running") if self.status == Some(AgentStatus::Idle) => self.set_status(AgentStatus::Thinking, out),
                _ => {}
            },
            _ => out.push(EventKind::Unknown { raw_type: format!("system/{subtype}") }),
        }
    }

    fn on_init(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        if self.init_seen {
            return;
        }
        self.init_seen = true;
        let tools: Vec<String> = v["tools"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let billing = match v["apiKeySource"].as_str() {
            Some("none") => Billing::Subscription,
            Some(_) => Billing::ApiKey,
            None => Billing::Unknown,
        };
        out.push(EventKind::SessionStarted {
            provider: Provider::Claude,
            model: v["model"].as_str().map(str::to_owned),
            cwd: v["cwd"].as_str().map(str::to_owned).unwrap_or_else(|| self.cwd.to_string_lossy().into_owned()),
            cli_version: v["claude_code_version"].as_str().map(str::to_owned),
            session_id: v["session_id"].as_str().map(str::to_owned).or(Some(self.session_id.clone())),
            billing,
            tools: tools.clone(),
        });
        // Unknown --tools names are dropped silently (CRITIQUE G5): say which ones.
        // `Agent` is listed under its alias `Task`
        let has = |t: &str| tools.iter().any(|x| x == t || (t == "Agent" && x == "Task"));
        let missing: Vec<&str> = self.requested_tools.iter().map(String::as_str).filter(|t| !has(t) && !OPTIONAL_TOOLS.contains(t)).collect();
        if !tools.is_empty() && !missing.is_empty() {
            let message = format!("Claude did not enable requested tools: {}", missing.join(", "));
            self.problem(message, false, out);
        }
        // The user's own servers (Connections, their Claude setup) that did not start: one
        // line for all of them, once `mcp_status` has said why (the init line gives no
        // reason). A server that still connects, or that waits for a sign-in the user never
        // made, is no failure of this run.
        let mut failed = Vec::new();
        for s in v["mcp_servers"].as_array().into_iter().flatten() {
            let status = s["status"].as_str().unwrap_or("unavailable");
            if s["name"] == "codly" {
                if status != "connected" {
                    self.problem(format!("codly MCP server is {status}"), false, out);
                }
            } else if status == "failed" {
                failed.push(s["name"].as_str().unwrap_or("unnamed").to_string());
            }
        }
        if !failed.is_empty() {
            self.mcp_failed = failed;
            self.outbox.push(json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } }));
        }
    }

    /// Is `resp` (the `response` of a `control_response`) the answer to a request of the
    /// normalizer's own? Every other control response is the actor's.
    pub fn asked(&self, resp: &Value) -> bool {
        resp["request_id"] == MCP_STATUS_REQUEST
    }

    /// The answer to `mcp_status`: one line for the user's servers that did not start, each
    /// with the CLI's reason ("Connection closed"). When the CLI cannot answer, the names are
    /// still said; a server that started since is left out.
    fn on_control_response(&mut self, resp: &Value, out: &mut Vec<EventKind>) {
        if !self.asked(resp) {
            return;
        }
        let failed = std::mem::take(&mut self.mcp_failed);
        let now = if resp["subtype"] == "success" { resp["response"]["mcpServers"].as_array() } else { None };
        let lines: Vec<String> = failed
            .into_iter()
            .filter_map(|name| {
                let Some(server) = now.and_then(|servers| servers.iter().find(|s| s["name"] == name.as_str())) else { return Some(name) };
                if server["status"] != "failed" {
                    return None;
                }
                Some(match server["error"].as_str().map(str::trim).filter(|e| !e.is_empty()) {
                    Some(error) => format!("{name} ({error})"),
                    None => name,
                })
            })
            .collect();
        if !lines.is_empty() {
            self.problem(format!("These MCP servers did not start, so the agent works without their tools in this run: {}", lines.join(", ")), false, out);
        }
    }

    /// `system/task_started`, `task_progress` and `task_notification`: the life of something
    /// that runs beside the agent's own calls. A subagent (`task_type: "local_agent"`) gets
    /// its events; a long or background command (`local_bash`) already is a tool call.
    fn on_task(&mut self, subtype: &str, v: &Value, out: &mut Vec<EventKind>) {
        let Some(call_id) = v["tool_use_id"].as_str() else { return };
        if subtype == "task_started" && v["task_type"] == "local_agent" && !self.helpers.contains_key(call_id) {
            // (its `Agent` call was in no line Codly saw: the subagent starts here)
            self.helper_started(call_id, v["description"].as_str().unwrap_or(""), out);
        }
        self.helper_named(call_id, v["subagent_type"].as_str(), out);
        let Some(helper) = self.helpers.get_mut(call_id) else { return };
        let (tool_uses, tokens) = (v["usage"]["tool_uses"].as_i64(), v["usage"]["total_tokens"].as_i64());
        let text = |key: &str| v[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
        match subtype {
            "task_started" => helper.background = v["is_backgrounded"] == true,
            "task_progress" => {
                let step = text("description");
                out.push(EventKind::SubAgentProgress { call_id: call_id.into(), name: None, step, tool_call_id: None, tool_uses, tokens, report: None });
            }
            // task_notification: it ended, and `summary` is what it reported
            _ => {
                helper.done = true;
                let report = text("summary");
                out.push(EventKind::SubAgentProgress { call_id: call_id.into(), name: None, step: None, tool_call_id: None, tool_uses, tokens, report });
                let status = v["status"].as_str().unwrap_or("completed").to_string();
                out.push(EventKind::SubAgent { call_id: call_id.into(), description: helper.description.clone(), status });
            }
        }
    }

    fn helper_started(&mut self, call_id: &str, description: &str, out: &mut Vec<EventKind>) {
        self.helpers.insert(call_id.to_string(), Helper { description: description.to_string(), name: None, background: false, done: false });
        out.push(EventKind::SubAgent { call_id: call_id.to_string(), description: description.to_string(), status: "running".into() });
    }

    /// The subagent's kind, said once.
    fn helper_named(&mut self, call_id: &str, name: Option<&str>, out: &mut Vec<EventKind>) {
        let (Some(helper), Some(name)) = (self.helpers.get_mut(call_id), name.filter(|n| !n.is_empty())) else { return };
        if helper.name.as_deref() != Some(name) {
            helper.name = Some(name.to_string());
            let name = helper.name.clone();
            out.push(EventKind::SubAgentProgress { call_id: call_id.into(), name, step: None, tool_call_id: None, tool_uses: None, tokens: None, report: None });
        }
    }

    fn on_stream_event(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        if !v["parent_tool_use_id"].is_null() {
            return; // subagent partials would interleave with the agent's own text
        }
        let ev = &v["event"];
        let msg_id = v["api_message_id"].as_str().or(ev["message"]["id"].as_str()).unwrap_or("").to_string();
        let index = ev["index"].as_u64().unwrap_or(0);
        match ev["type"].as_str().unwrap_or("") {
            "message_start" => {
                // An API call answers: the failure Claude retried before it is over.
                self.api_failure = None;
                self.turn_live = self.turn_live.plus(std::mem::take(&mut self.call_live));
                self.flush_text(MessagePhase::Unknown, out)
            }
            "content_block_start" => {
                let cb = &ev["content_block"];
                let ty = cb["type"].as_str().unwrap_or("").to_string();
                self.blocks.insert((msg_id, index), (ty, cb["id"].as_str().map(str::to_owned), false));
            }
            "content_block_delta" => {
                let d = &ev["delta"];
                let item_id = format!("{msg_id}:{index}");
                // 2.1.309 hides thinking text (empty deltas plus `system/thinking_tokens` estimates)
                match (d["type"].as_str(), d["text"].as_str().or(d["thinking"].as_str())) {
                    (Some("text_delta"), Some(t)) if !t.is_empty() => out.push(EventKind::TextDelta { item_id, text: t.to_string() }),
                    (Some("thinking_delta"), Some(t)) if !t.is_empty() => out.push(EventKind::ReasoningDelta { item_id, text: t.to_string() }),
                    _ => {} // input_json_delta, signature_delta, citations: the assistant line is authoritative
                }
            }
            "message_delta" => {
                let u = &ev["usage"];
                if u.is_object() {
                    self.last_context = Some(context_tokens(u));
                    // Usage as the turn goes, one update per API call: the context meter and
                    // the circuit breaker see a long turn while it runs.
                    self.call_live = Tokens::of(u);
                    out.push(EventKind::UsageUpdated { usage: self.live_usage() });
                }
                let phase = match ev["delta"]["stop_reason"].as_str() {
                    Some("end_turn") => MessagePhase::Final,
                    Some("tool_use") => MessagePhase::Commentary,
                    _ => MessagePhase::Unknown,
                };
                self.flush_text(phase, out);
            }
            "message_stop" => self.flush_text(MessagePhase::Unknown, out),
            "content_block_stop" | "ping" => {}
            other => out.push(EventKind::Unknown { raw_type: format!("stream_event/{other}") }),
        }
    }

    /// Index of a block of the full `assistant` message in the stream (`api_message_id:index`
    /// is the item id deltas used). The assistant line can arrive before `content_block_stop`.
    fn block_index(&mut self, msg_id: &str, ty: &str, tool_id: Option<&str>, pos: usize) -> u64 {
        let mut hit: Option<u64> = None;
        for ((m, i), (t, id, done)) in &self.blocks {
            if m == msg_id && t == ty && !*done && (tool_id.is_none() || id.as_deref() == tool_id) && hit.is_none_or(|h| *i < h) {
                hit = Some(*i);
            }
        }
        match hit {
            Some(i) => {
                if let Some(b) = self.blocks.get_mut(&(msg_id.to_string(), i)) {
                    b.2 = true;
                }
                i
            }
            None => pos as u64,
        }
    }

    fn flush_text(&mut self, phase: MessagePhase, out: &mut Vec<EventKind>) {
        for (item_id, text) in self.pending_text.drain(..) {
            self.turn_text = Some(text.trim().to_string());
            out.push(EventKind::Message { item_id, text, phase });
        }
    }

    fn on_assistant(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let msg = &v["message"];
        let msg_id = msg["id"].as_str().unwrap_or("").to_string();
        // A subagent's line: its tool calls are shown under it; its text reaches the agent as
        // the result of the `Agent` call (and the user as the report of `task_notification`).
        let subagent = !v["parent_tool_use_id"].is_null();
        let helper = v["parent_tool_use_id"].as_str().filter(|id| self.helpers.contains_key(*id)).map(str::to_owned);
        for (pos, block) in msg["content"].as_array().into_iter().flatten().enumerate() {
            match block["type"].as_str().unwrap_or("") {
                "text" if !subagent => {
                    let i = self.block_index(&msg_id, "text", None, pos);
                    self.pending_text.push((format!("{msg_id}:{i}"), block["text"].as_str().unwrap_or("").to_string()));
                }
                "thinking" if !subagent => {
                    let i = self.block_index(&msg_id, "thinking", None, pos);
                    let text = block["thinking"].as_str().unwrap_or("").to_string();
                    if !text.is_empty() {
                        out.push(EventKind::Reasoning { item_id: format!("{msg_id}:{i}"), text });
                    }
                }
                "tool_use" | "server_tool_use" => {
                    let _ = self.block_index(&msg_id, block["type"].as_str().unwrap_or(""), block["id"].as_str(), pos);
                    // text before a tool call in the same message is progress narration (a
                    // subagent's call says nothing about the agent's own text)
                    if !subagent {
                        self.flush_text(MessagePhase::Commentary, out);
                    }
                    self.tool_started(block, helper.as_deref(), out);
                }
                "text" | "thinking" | "redacted_thinking" => {}
                other => out.push(EventKind::Unknown { raw_type: format!("assistant/{other}") }),
            }
        }
    }

    /// `helper`: the subagent that makes this call (the id of its `Agent` call).
    fn tool_started(&mut self, block: &Value, helper: Option<&str>, out: &mut Vec<EventKind>) {
        let Some(id) = block["id"].as_str() else { return };
        if !self.tools_seen.insert(id.to_string()) {
            return; // the same tool_use repeated in a later assistant line
        }
        let name = block["name"].as_str().unwrap_or("").to_string();
        let input = block["input"].clone();
        let (activity, target, confidence) = classify::classify_tool(Provider::Claude, &name, &input);
        if name == "TaskCreate" {
            let subject = input["subject"].as_str().or(input["description"].as_str()).unwrap_or("task").to_string();
            self.pending_creates.insert(id.to_string(), subject);
        }
        if matches!(name.as_str(), "Agent" | "Task") {
            // (not a second time when its `task_started` line came first)
            if !self.helpers.contains_key(id) {
                self.helper_started(id, input["description"].as_str().unwrap_or(""), out);
            }
            self.helper_named(id, input["subagent_type"].as_str(), out);
        }
        out.push(EventKind::ToolStarted {
            call_id: id.to_string(),
            name: name.clone(),
            activity,
            target,
            input: input.clone(),
            confidence: confidence.into(),
        });
        if let Some(helper) = helper {
            let tool_call_id = Some(id.to_string());
            out.push(EventKind::SubAgentProgress { call_id: helper.into(), name: None, step: None, tool_call_id, tool_uses: None, tokens: None, report: None });
        }
        self.tools.insert(id.to_string(), ToolInfo { name, input, activity, started: Instant::now(), helper: helper.map(str::to_owned) });
        self.set_status(AgentStatus::Working, out);
    }

    fn on_user(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let mut any = false;
        for block in v["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_result" {
                any = true;
                self.tool_completed(block, &v["tool_use_result"], out);
            }
        }
        if any {
            if !self.tools.values().any(|t| t.activity != ActivityKind::Delegate) {
                self.set_status(AgentStatus::Thinking, out);
            }
        } else if v["parent_tool_use_id"].is_null() && !result_text(&v["message"]["content"]).starts_with("[Request interrupted") {
            // (the CLI's own interruption notice is known and says nothing new; so is the
            // prompt a subagent starts with, which its `Agent` call already shows)
            out.push(EventKind::Unknown { raw_type: "user".into() });
        }
    }

    fn tool_completed(&mut self, block: &Value, structured: &Value, out: &mut Vec<EventKind>) {
        let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
        let ok = block["is_error"].as_bool() != Some(true);
        let text = result_text(&block["content"]);
        let info = self.tools.remove(&id);
        let (name, activity) = info.as_ref().map(|t| (t.name.as_str(), t.activity)).unwrap_or(("", ActivityKind::Execute));
        let tests = if activity == ActivityKind::Test { classify::parse_test_counts(&text) } else { None };
        let exit_code = if name == "Bash" {
            EXIT_CODE_RE.captures(&text).and_then(|c| c[1].parse().ok()).or(ok.then_some(0))
        } else {
            None
        };
        out.push(EventKind::ToolCompleted {
            call_id: id.clone(),
            ok,
            output: Some(classify::truncate_output(&text, OUTPUT_MAX)),
            exit_code,
            duration_ms: info.as_ref().map(|t| t.started.elapsed().as_millis() as i64),
            tests_passed: tests.map(|t| t.0),
            tests_failed: tests.map(|t| t.1),
        });
        let Some(info) = info else { return };
        match info.name.as_str() {
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" if ok => {
                if let Some(p) = info.input["file_path"].as_str().or(info.input["notebook_path"].as_str()) {
                    out.push(EventKind::FilesChanged { call_id: Some(id), paths: vec![p.to_string()], source: "tool_input".into() });
                }
            }
            "Agent" | "Task" => {
                let Some(helper) = self.helpers.get_mut(&id) else { return };
                if structured["status"] == "async_launched" {
                    // The call only started it: `task_notification` says when it is done.
                    helper.background = true;
                } else if !helper.done {
                    helper.done = true;
                    let status = if ok { "completed" } else { "failed" };
                    out.push(EventKind::SubAgent { call_id: id, description: helper.description.clone(), status: status.into() });
                }
            }
            "TaskCreate" | "TaskUpdate" | "TaskList" | "TodoWrite" if self.apply_plan(&info.name, &id, &info.input, ok, &text, structured) => {
                out.push(self.plan_event());
            }
            _ => {}
        }
    }

    /// Mirror of Claude Code's own task bookkeeping (2.1.309 binary): creates get their id
    /// from the result, updates apply by `taskId`, TaskList resyncs the whole list.
    fn apply_plan(&mut self, tool: &str, call_id: &str, input: &Value, ok: bool, text: &str, structured: &Value) -> bool {
        let subject = self.pending_creates.remove(call_id);
        if !ok {
            return false;
        }
        match tool {
            "TaskCreate" => {
                let id = structured["task"]["id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| TASK_CREATED_RE.captures(text).map(|c| c[1].to_string()))
                    .unwrap_or_else(|| (self.plan.len() + 1).to_string());
                let text = structured["task"]["subject"].as_str().map(str::to_owned).or(subject).unwrap_or_else(|| format!("Task {id}"));
                self.plan.push(PlanItem { id, text, status: "pending".into() });
                true
            }
            "TaskUpdate" => {
                if structured["success"] == false {
                    return false;
                }
                let Some(id) = ["taskId", "id", "task_id"].iter().find_map(|k| input[*k].as_str().map(str::to_owned).or_else(|| input[*k].as_i64().map(|n| n.to_string())))
                else {
                    return false;
                };
                let status = input["status"].as_str();
                if status == Some("deleted") {
                    self.plan.retain(|p| p.id != id);
                    return true;
                }
                let pos = match self.plan.iter().position(|p| p.id == id) {
                    Some(i) => i,
                    None => {
                        self.plan.push(PlanItem { id: id.clone(), text: format!("Task {id}"), status: "pending".into() });
                        self.plan.len() - 1
                    }
                };
                let item = &mut self.plan[pos];
                if let Some(s) = status {
                    item.status = s.to_string();
                }
                if let Some(s) = input["subject"].as_str() {
                    item.text = s.to_string();
                }
                true
            }
            "TaskList" => {
                let Some(tasks) = structured["tasks"].as_array() else { return false };
                self.plan = tasks
                    .iter()
                    .map(|t| PlanItem {
                        id: t["id"].as_str().map(str::to_owned).unwrap_or_else(|| t["id"].to_string()),
                        text: t["subject"].as_str().unwrap_or("").to_string(),
                        status: t["status"].as_str().unwrap_or("pending").to_string(),
                    })
                    .collect();
                true
            }
            "TodoWrite" => {
                let Some(todos) = input["todos"].as_array() else { return false };
                self.plan = todos
                    .iter()
                    .enumerate()
                    .map(|(i, t)| PlanItem {
                        id: i.to_string(),
                        text: t["content"].as_str().unwrap_or("").to_string(),
                        status: t["status"].as_str().unwrap_or("pending").to_string(),
                    })
                    .collect();
                true
            }
            _ => false,
        }
    }

    fn plan_event(&self) -> EventKind {
        EventKind::PlanUpdated { steps: self.plan.iter().map(|p| PlanStep { text: p.text.clone(), status: p.status.clone() }).collect() }
    }

    fn on_result(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        // A session that could not start (2.1.311 §7: `--resume` of a conversation the CLI no
        // longer has): this one line, with no `system/init` before it and no turn, then the
        // process exits. It ends no turn of Codly's: it only says why.
        if !self.init_seen && v["is_error"] == true && v["num_turns"] == 0 {
            let said = cli_errors(v);
            let message = match startup::session_trouble(&said) {
                Some(why) => format!("{why} ({said})"),
                None if said.is_empty() => "Claude Code could not start this session".to_string(),
                None => format!("Claude Code could not start this session: {said}"),
            };
            self.problem(message, false, out);
            return;
        }
        self.flush_text(MessagePhase::Final, out);
        // Per-turn delta: `model_usage_since_last_result` (modelUsage and total_cost_usd are
        // session-cumulative, including earlier runs after --resume).
        let (mut input, mut output, mut cache_r, mut cache_w, mut cost, mut window) = (0, 0, 0, 0, 0.0, None::<i64>);
        let per_model = v["model_usage_since_last_result"].as_object().or(v["modelUsage"].as_object());
        if let Some(models) = per_model {
            for m in models.values() {
                input += m["inputTokens"].as_i64().unwrap_or(0);
                output += m["outputTokens"].as_i64().unwrap_or(0);
                cache_r += m["cacheReadInputTokens"].as_i64().unwrap_or(0);
                cache_w += m["cacheCreationInputTokens"].as_i64().unwrap_or(0);
                cost += m["costUSD"].as_f64().unwrap_or(0.0);
            }
        } else {
            let u = &v["usage"];
            input = u["input_tokens"].as_i64().unwrap_or(0);
            output = u["output_tokens"].as_i64().unwrap_or(0);
            cache_r = u["cache_read_input_tokens"].as_i64().unwrap_or(0);
            cache_w = u["cache_creation_input_tokens"].as_i64().unwrap_or(0);
            cost = v["total_cost_usd"].as_f64().unwrap_or(0.0) - self.usage.cost_usd.unwrap_or(0.0);
        }
        for m in v["modelUsage"].as_object().into_iter().flat_map(|m| m.values()) {
            if let Some(w) = m["contextWindow"].as_i64() {
                window = Some(window.map_or(w, |x: i64| x.max(w)));
            }
        }
        // The stream already reported this turn's own API calls; the result has the last word
        // (it adds the subagents' calls, which the circuit breaker keeps out of its velocity:
        // they are work of the whole turn). A streamed call that the result leaves out must
        // not make the totals go back: the circuit breaker reads a falling count as a new run.
        let live = self.live_usage();
        self.usage.input_tokens = (self.usage.input_tokens + input).max(live.input_tokens);
        self.usage.output_tokens = (self.usage.output_tokens + output).max(live.output_tokens);
        self.usage.cache_read_tokens = (self.usage.cache_read_tokens + cache_r).max(live.cache_read_tokens);
        self.usage.cache_write_tokens = (self.usage.cache_write_tokens + cache_w).max(live.cache_write_tokens);
        (self.turn_live, self.call_live) = (Tokens::default(), Tokens::default());
        self.usage.cost_usd = Some(self.usage.cost_usd.unwrap_or(0.0) + cost.max(0.0));
        let last_iter = v["usage"]["iterations"].as_array().and_then(|a| a.last()).map(context_tokens);
        self.usage.context_used = last_iter.or(self.last_context).or(self.usage.context_used);
        self.usage.context_window = window.or(self.usage.context_window);
        out.push(EventKind::UsageUpdated { usage: self.usage.clone() });

        let subtype = v["subtype"].as_str().unwrap_or("");
        let terminal = v["terminal_reason"].as_str().unwrap_or("");
        let status = if self.interrupt_requested || terminal.contains("interrupt") || terminal.contains("abort") {
            TurnStatus::Interrupted
        } else if subtype == "error_max_turns" {
            TurnStatus::MaxTurns
        } else if subtype == "success" && v["is_error"] != true {
            TurnStatus::Completed
        } else {
            TurnStatus::Failed
        };
        // Calls that Claude Code denied by itself (a rule, a hook, its permission mode), as the
        // result lists them: a call Codly was asked about has its card, and one that a
        // `permission_denied` line named is said already.
        let mut denied: Vec<&str> = Vec::new();
        for d in v["permission_denials"].as_array().into_iter().flatten() {
            let tool = d["tool_name"].as_str().filter(|t| !t.is_empty()).unwrap_or("a tool");
            if !d["tool_use_id"].as_str().is_some_and(|id| self.denials_known.contains(id)) && !denied.contains(&tool) {
                denied.push(tool);
            }
        }
        if !denied.is_empty() {
            // (not kept as the turn's last error: a failed turn still says its own reason)
            out.push(EventKind::Error { message: format!("Claude Code denied {} without asking", denied.join(", ")), fatal: false });
        }
        // A failed turn says why, once: not again when its last error or its last message
        // already is that line.
        let retried = self.api_failure.take();
        let reason = if status == TurnStatus::Failed { self.failure_reason(v, retried) } else { None };
        if let Some(reason) = &reason {
            let said = match &self.turn_error {
                Some(error) => error == reason,
                None => self.turn_text.as_deref() == Some(reason.as_str()),
            };
            if !said {
                self.problem(reason.clone(), false, out);
            }
        }
        out.push(EventKind::TurnCompleted {
            status,
            result_text: reason.or_else(|| v["result"].as_str().map(str::to_owned)),
            duration_ms: v["duration_ms"].as_i64(),
        });
        self.interrupt_requested = false;
        self.blocks.clear();
        // A background subagent works on past the end of the turn: it and its open calls stay.
        // One whose `Agent` call still waited for it is over with the turn.
        self.helpers.retain(|id, h| {
            if !h.done && !h.background {
                out.push(EventKind::SubAgent { call_id: id.clone(), description: h.description.clone(), status: "stopped".into() });
            }
            h.background && !h.done
        });
        let helpers = &self.helpers;
        self.tools.retain(|_, t| t.helper.as_ref().is_some_and(|h| helpers.contains_key(h)));
        self.user_denied.clear();
        self.denials_known.clear();
        self.set_status(AgentStatus::Idle, out);
    }

    fn on_rate_limit(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let info = &v["rate_limit_info"];
        let status = info["status"].as_str().map(str::to_owned);
        let kind = info["rateLimitType"].as_str().unwrap_or("");
        match info["unifiedWindows"].as_object() {
            Some(windows) => {
                for (name, w) in windows {
                    out.push(EventKind::RateLimit {
                        window: name.clone(),
                        used_fraction: w["utilization"].as_f64(),
                        resets_at: w["resetsAt"].as_i64().map(|s| s * 1000),
                        status: if name == kind { status.clone() } else { None },
                    });
                }
            }
            None => out.push(EventKind::RateLimit {
                window: if kind.is_empty() { "unknown".into() } else { kind.into() },
                used_fraction: info["utilization"].as_f64(),
                resets_at: info["resetsAt"].as_i64().map(|s| s * 1000),
                status: status.clone(),
            }),
        }
        if status.as_deref() == Some("rejected") {
            self.set_status(AgentStatus::RateLimited, out);
        }
    }

    fn on_control_request(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let request_id = v["request_id"].as_str().unwrap_or("").to_string();
        let req = &v["request"];
        match req["subtype"].as_str().unwrap_or("") {
            "can_use_tool" => self.approval_requested(request_id, req, out),
            other => {
                // hook_callback / mcp_message only happen for SDK features Codly does not register
                out.push(EventKind::Unknown { raw_type: format!("control_request/{other}") });
                self.outbox.push(json!({ "type": "control_response", "response": {
                    "subtype": "error", "request_id": request_id, "error": format!("Codly does not support {other}")
                }}));
            }
        }
    }

    fn approval_requested(&mut self, request_id: String, req: &Value, out: &mut Vec<EventKind>) {
        let tool = req["tool_name"].as_str().unwrap_or("").to_string();
        let input = req["input"].clone();
        let suggestions = req["permission_suggestions"].as_array().cloned().unwrap_or_default();
        let cwd = self.cwd.clone();
        let mut risks: Vec<String> = Vec::new();
        let subject = match tool.as_str() {
            "Bash" | "Monitor" => {
                let command = input["command"].as_str().unwrap_or("").to_string();
                risks = classify::risk_hints(&command, Some(&cwd), &cwd);
                ApprovalSubject::Command { command, cwd: Some(cwd.to_string_lossy().into_owned()) }
            }
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                let paths: Vec<String> = input["file_path"].as_str().or(input["notebook_path"].as_str()).map(|p| vec![p.to_string()]).unwrap_or_default();
                if paths.iter().any(|p| classify::outside(p, &cwd, &cwd)) {
                    risks.push("outside worktree".into());
                }
                ApprovalSubject::FileChange { paths, diff: edit_preview(&tool, &input) }
            }
            "AskUserQuestion" => ApprovalSubject::Question { questions: input["questions"].clone() },
            _ => ApprovalSubject::Tool { name: tool.clone(), input: input.clone() },
        };
        // `blocked_path` is the path the call touches (set even inside the worktree, live 2.1.309)
        if req["blocked_path"].as_str().is_some_and(|p| classify::outside(p, &cwd, &cwd)) && !risks.iter().any(|r| r == "outside worktree") {
            risks.push("outside worktree".into());
        }
        let choices = if tool == "AskUserQuestion" {
            vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]
        } else {
            // A session or permanent grant is a permission rule the CLI suggested: without
            // one, "for this session" could only be sent as a one-time allow, so it is not offered.
            let mut c = vec![ApprovalChoice::AllowOnce];
            if !suggestions.is_empty() {
                c.push(ApprovalChoice::AllowSession);
                // The CLI saves "always" in its settings under `cwd`. In a worker's task
                // worktree that file can be committed with the work and is lost with the
                // worktree: there, "always" is Codly's own rule for the repo.
                if !self.worker {
                    c.push(ApprovalChoice::AllowAlways);
                }
            }
            c.extend([ApprovalChoice::Deny, ApprovalChoice::Cancel]);
            c
        };
        let reason = ["decision_reason", "title", "description"].iter().find_map(|k| match &req[*k] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Null => None,
            Value::String(_) => None,
            other => Some(other.to_string()),
        });
        let approval = Approval {
            id: new_id(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            task_id: None,
            call_id: req["tool_use_id"].as_str().map(str::to_owned),
            subject,
            reason,
            choices,
            risks,
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        let tool_use_id = approval.call_id.clone();
        self.denials_known.extend(tool_use_id.clone());
        self.approvals.insert(approval.id.clone(), PendingApproval { request_id, tool_use_id, tool_name: tool, input, suggestions });
        out.push(EventKind::ApprovalRequested { approval });
        self.set_status(AgentStatus::WaitingApproval, out);
    }

    /// The `control_response` for a decision, plus status events. None if the approval is
    /// unknown or was already cancelled by the CLI.
    pub fn answer(&mut self, approval_id: &str, d: &ApprovalDecision) -> Option<(Value, Vec<EventKind>)> {
        let p = self.approvals.remove(approval_id)?;
        let mut out = Vec::new();
        if matches!(d.choice, ApprovalChoice::Deny | ApprovalChoice::Cancel)
            && let Some(id) = &p.tool_use_id
        {
            self.user_denied.insert(id.clone());
        }
        let response = match d.choice {
            ApprovalChoice::AllowOnce | ApprovalChoice::AllowSession | ApprovalChoice::AllowAlways => {
                let mut input = p.input.clone();
                if p.tool_name == "AskUserQuestion" {
                    input["answers"] = d.answers.clone().unwrap_or_else(|| json!({}));
                }
                let mut r = json!({ "behavior": "allow", "updatedInput": input });
                match d.choice {
                    ApprovalChoice::AllowAlways if !p.suggestions.is_empty() => r["updatedPermissions"] = Value::Array(p.suggestions),
                    // Claude has no session scope of its own; its suggestions accept one.
                    ApprovalChoice::AllowSession if !p.suggestions.is_empty() => {
                        let scoped: Vec<Value> = p
                            .suggestions
                            .into_iter()
                            .map(|mut s| {
                                s["destination"] = json!("session");
                                s
                            })
                            .collect();
                        r["updatedPermissions"] = Value::Array(scoped);
                    }
                    _ => {}
                }
                self.set_status(AgentStatus::Working, &mut out);
                r
            }
            ApprovalChoice::Deny => {
                self.set_status(AgentStatus::Thinking, &mut out);
                json!({ "behavior": "deny", "message": d.message.clone().unwrap_or_else(|| "The user denied this action.".into()), "interrupt": false })
            }
            ApprovalChoice::Cancel => {
                self.interrupt_requested = true;
                self.set_status(AgentStatus::Thinking, &mut out);
                json!({ "behavior": "deny", "message": d.message.clone().unwrap_or_else(|| "The user stopped this turn.".into()), "interrupt": true })
            }
        };
        let reply = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": p.request_id, "response": response } });
        Some((reply, out))
    }

    fn on_cancel(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let rid = v["request_id"].as_str().unwrap_or("");
        let Some(id) = self.approvals.iter().find(|(_, p)| p.request_id == rid).map(|(id, _)| id.clone()) else { return };
        self.approvals.remove(&id);
        out.push(EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() });
        if self.approvals.is_empty() {
            self.set_status(AgentStatus::Thinking, out);
        }
    }

    /// The process is gone: every open approval card closes.
    pub fn cancel_all_approvals(&mut self) -> Vec<EventKind> {
        self.approvals
            .drain()
            .map(|(id, _)| EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() })
            .collect()
    }
}

/// A result's `errors`, without the CLI's diagnostic lines.
fn cli_errors(result: &Value) -> String {
    let errors: Vec<&str> =
        result["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|e| !e.starts_with("[ede_diagnostic]")).collect();
    errors.join("; ")
}

/// Tokens the model saw on one API call (the context meter, never a sum of turns).
fn context_tokens(u: &Value) -> i64 {
    ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"].iter().map(|k| u[*k].as_i64().unwrap_or(0)).sum()
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A small `-`/`+` preview of an edit for the approval card.
fn edit_preview(tool: &str, input: &Value) -> Option<String> {
    let lines = |prefix: char, s: &str| s.lines().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n");
    let text = match tool {
        "Edit" => format!("{}\n{}", lines('-', input["old_string"].as_str()?), lines('+', input["new_string"].as_str().unwrap_or(""))),
        "MultiEdit" => input["edits"]
            .as_array()?
            .iter()
            .map(|e| format!("{}\n{}", lines('-', e["old_string"].as_str().unwrap_or("")), lines('+', e["new_string"].as_str().unwrap_or(""))))
            .collect::<Vec<_>>()
            .join("\n…\n"),
        "Write" => lines('+', input["content"].as_str()?),
        "NotebookEdit" => lines('+', input["new_source"].as_str()?),
        _ => return None,
    };
    Some(classify::truncate_output(&text, OUTPUT_MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::McpLaunch;

    fn spec(role: Role, read_only: bool) -> SpawnSpec {
        SpawnSpec {
            agent_id: "0192aaaa-bbbb-7ccc-8ddd-eeeeffff0001".into(),
            run_id: "run-1".into(),
            provider: Provider::Claude,
            role,
            model: Some("haiku:low".into()),
            cwd: PathBuf::from("/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/fixture-repo"),
            bin: PathBuf::from("claude"),
            env: HashMap::new(),
            session: SessionMode::New { session_id: None },
            session_cost_usd: 0.0,
            system_prompt: "You are a Codly worker.".into(),
            read_only,
            mcp: Some(McpLaunch { command: "/Applications/Codly.app/Contents/MacOS/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), "t".into())] }),
            use_user_setup: false,
            raw_log: PathBuf::from("/tmp/x/raw.ndjson"),
            byok_command: None,
        }
    }

    fn fixture(name: &str) -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures").join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn replay(norm: &mut ClaudeNormalizer, lines: &[Value]) -> Vec<EventKind> {
        lines.iter().flat_map(|v| norm.on_message(v)).collect()
    }

    #[test]
    fn byok_token_command_becomes_an_api_key_helper() {
        let s = spec(Role::Worker, false);
        assert!(!build_args(&s, "s", None).contains(&"--settings".to_string()), "no BYOK, no --settings");
        let s = SpawnSpec { byok_command: Some("~/bin/wif-token \"anthropic\"".into()), ..s };
        let a = build_args(&s, "s", None);
        let i = a.iter().position(|x| x == "--settings").unwrap();
        let v: Value = serde_json::from_str(&a[i + 1]).unwrap();
        assert_eq!(v, serde_json::json!({ "apiKeyHelper": "~/bin/wif-token \"anthropic\"" }));
    }

    #[test]
    fn args_for_a_worker() {
        let s = spec(Role::Worker, false);
        let a = build_args(&s, "11111111-2222-4333-8444-555555555555", Some(Path::new("/tmp/x/claude-mcp.json")));
        let joined = a.join(" ");
        for want in [
            "-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages",
            "--permission-prompt-tool stdio --permission-mode acceptEdits",
            "--session-id 11111111-2222-4333-8444-555555555555",
            "--tools Bash,Read,Edit,Write,Grep,Glob,TaskCreate,TaskUpdate,TaskList,TaskGet,WebFetch,WebSearch,Agent",
            "--strict-mcp-config --mcp-config /tmp/x/claude-mcp.json --allowedTools mcp__codly__*",
            "--setting-sources user,local",
            "--model haiku --effort low",
            "-n codly:worker-ffff0001",
        ] {
            assert!(joined.contains(want), "missing `{want}` in {joined}");
        }
        assert!(!joined.contains("ToolSearch"));
        let i = a.iter().position(|x| x == "--append-system-prompt").unwrap();
        assert_eq!(a[i + 1], "You are a Codly worker.");
        let cfg = mcp_config(&s).unwrap();
        assert_eq!(cfg["mcpServers"]["codly"]["alwaysLoad"], true);
        assert_eq!(cfg["mcpServers"]["codly"]["env"]["CODLY_MCP_TOKEN"], "t");
    }

    /// The file with a run's keys is the user's alone and does not outlive the run; what an
    /// earlier Codly left behind goes at the next start.
    #[test]
    fn the_mcp_config_file_is_private_and_removed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(runs.join("r1")).unwrap();
        let path = runs.join("r1").join(MCP_CONFIG_FILE);
        let file = PrivateFile::write(&path, &mcp_config(&spec(Role::Worker, false)).unwrap().to_string()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_to_string(&path).unwrap().contains("CODLY_MCP_TOKEN"));
        drop(file); // the run is over
        assert!(!path.exists());

        std::fs::create_dir_all(runs.join("r2")).unwrap();
        std::fs::write(runs.join("r1").join(MCP_CONFIG_FILE), "{}").unwrap();
        std::fs::write(runs.join("r2").join("raw.ndjson"), "{}\n").unwrap();
        std::fs::write(runs.join("not-a-run"), "").unwrap();
        remove_stale_mcp_configs(&runs);
        assert!(!runs.join("r1").join(MCP_CONFIG_FILE).exists());
        assert!(runs.join("r2").join("raw.ndjson").exists(), "nothing else is touched");
        remove_stale_mcp_configs(&dir.path().join("missing")); // no runs yet: nothing to do
    }

    #[test]
    fn args_for_read_only_and_resume() {
        let mut s = spec(Role::Lead, true);
        s.session = SessionMode::Resume { session_id: "abc".into() };
        s.mcp = None;
        let joined = build_args(&s, "abc", None).join(" ");
        assert!(joined.contains("--permission-mode manual"));
        assert!(joined.contains("--resume abc") && !joined.contains("--session-id"));
        assert!(joined.contains("--tools Read,Grep,Glob,WebFetch,WebSearch,Skill,LSP "));
        assert!(!joined.contains("--mcp-config") && !joined.contains("mcp__codly"));
        assert!(joined.contains("--strict-mcp-config"));
        assert_eq!(permission_mode(&spec(Role::Solo, false)), "manual");
    }

    #[test]
    fn replays_the_read_tool_fixture() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "e3bef2cd-ec8e-4a6a-9ec8-6cd8d19ec45f".into());
        let mut events = norm.turn_started("Use the Read tool to read README.md then reply DONE");
        events.extend(replay(&mut norm, &fixture("claude-tool.jsonl")));
        let names: Vec<&str> = events.iter().map(EventKind::type_name).collect();

        let started = events.iter().find_map(|e| match e {
            EventKind::SessionStarted { model, cli_version, session_id, billing, tools, .. } => Some((model, cli_version, session_id, billing, tools)),
            _ => None,
        });
        let (model, version, session, billing, tools) = started.expect("SessionStarted from system/init");
        assert_eq!(model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(version.as_deref(), Some("2.1.309"));
        assert_eq!(session.as_deref(), Some("e3bef2cd-ec8e-4a6a-9ec8-6cd8d19ec45f"));
        assert_eq!(*billing, Billing::Subscription);
        assert!(tools.contains(&"Read".to_string()));
        // the fixture ran with the default tool set: Grep/Glob/Task* missing gets reported
        assert!(events.iter().any(|e| matches!(e, EventKind::Error { message, fatal: false } if message.contains("Grep"))));

        let tool_started: Vec<_> = events.iter().filter(|e| matches!(e, EventKind::ToolStarted { .. })).collect();
        assert_eq!(tool_started.len(), 1, "tool_use deduped across stream_event + assistant");
        match tool_started[0] {
            EventKind::ToolStarted { call_id, name, activity, target, confidence, .. } => {
                assert_eq!(call_id, "toolu_016oYsYtLaWJhxioExpjXYGf");
                assert_eq!(name, "Read");
                assert_eq!(*activity, ActivityKind::Read);
                assert!(target.path.as_deref().unwrap().ends_with("fixture-repo/README.md"));
                assert_eq!(confidence, "observed");
            }
            _ => unreachable!(),
        }
        let completed = events.iter().find_map(|e| match e {
            EventKind::ToolCompleted { call_id, ok, output, .. } => Some((call_id, ok, output)),
            _ => None,
        });
        let (id, ok, output) = completed.expect("ToolCompleted");
        assert_eq!((id.as_str(), *ok), ("toolu_016oYsYtLaWJhxioExpjXYGf", true));
        assert!(output.as_deref().unwrap().contains("Fixture Repo"));

        // streamed text, then the final message with the same item id
        let delta = events.iter().find_map(|e| match e {
            EventKind::TextDelta { item_id, text } => Some((item_id.clone(), text.clone())),
            _ => None,
        });
        assert_eq!(delta, Some(("msg_011CgPgZ5DraWXssRR4rfPzQ:0".into(), "DONE".into())));
        let msg = events.iter().find_map(|e| match e {
            EventKind::Message { item_id, text, phase } => Some((item_id.clone(), text.clone(), *phase)),
            _ => None,
        });
        assert_eq!(msg, Some(("msg_011CgPgZ5DraWXssRR4rfPzQ:0".into(), "DONE".into(), MessagePhase::Final)));

        assert!(events.iter().any(|e| matches!(e, EventKind::RateLimit { window, .. } if window == "five_hour")));
        // Usage after each of the two API calls, then the result's own numbers: the same totals.
        let usages: Vec<Usage> = events.iter().filter_map(|e| if let EventKind::UsageUpdated { usage } = e { Some(usage.clone()) } else { None }).collect();
        assert_eq!(usages.len(), 3);
        assert_eq!((usages[0].input_tokens, usages[0].output_tokens, usages[0].cache_read_tokens, usages[0].cache_write_tokens), (2, 122, 12037, 15990));
        assert_eq!(usages[0].context_used, Some(2 + 122 + 12037 + 15990));
        assert_eq!((usages[1].input_tokens, usages[1].output_tokens, usages[1].cache_read_tokens, usages[1].cache_write_tokens), (4, 127, 40064, 16221));
        let usage = usages[2].clone();
        assert_eq!((usage.input_tokens, usage.output_tokens, usage.cache_read_tokens, usage.cache_write_tokens), (4, 127, 40064, 16221));
        assert_eq!(usage.context_used, Some(2 + 28027 + 231 + 5));
        assert_eq!(usage.context_window, Some(1_000_000));
        assert!((usage.cost_usd.unwrap() - 0.0037087).abs() < 1e-6);
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some(t), .. } if t == "DONE")));
        assert_eq!(events.last().map(EventKind::type_name), Some("status_changed"));
        assert!(matches!(events.last(), Some(EventKind::StatusChanged { status: AgentStatus::Idle })));

        // order: tool started before completed before turn completed
        let pos = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(pos("tool_started") < pos("tool_completed") && pos("tool_completed") < pos("message") && pos("message") < pos("turn_completed"));
        // The user's hooks are bookkeeping Codly knows: they stay in the raw log and make no
        // "unknown" rows (the one exception to "an unknown line is never dropped").
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type.starts_with("system/hook_"))));
    }

    /// Live 2.1.309 capture (adapter_smoke --interrupt-at-approval): TaskCreate ×2, a Bash
    /// `can_use_tool` (with `blocked_path` inside the worktree), Codly's interrupt, the CLI's
    /// `control_cancel_request`, and an `aborted_tools` result.
    #[test]
    fn replays_the_live_interrupt_capture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/testdata/claude-interrupt-live.jsonl");
        let lines: Vec<Value> = std::fs::read_to_string(path).unwrap().lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
        let mut s = spec(Role::Solo, false);
        s.cwd = PathBuf::from(lines.iter().find(|l| l["subtype"] == "init").unwrap()["cwd"].as_str().unwrap());
        let mut norm = ClaudeNormalizer::new(&s, "4a6796bf-c299-4975-8a0e-6bc9eaf3b8e2".into());
        let init = lines.iter().find(|l| l["type"] == "control_response").unwrap();
        assert!(matches!(norm.on_initialize(&init["response"]["response"]), EventKind::SessionStarted { billing: Billing::Subscription, .. }));
        let mut events = norm.turn_started("go");
        events.extend(replay(&mut norm, &lines));

        let a = approval_of(&events);
        assert!(matches!(&a.subject, ApprovalSubject::Command { command, .. } if command == "echo codly > hello.txt"));
        assert!(a.choices.contains(&ApprovalChoice::AllowAlways), "the CLI sent permission_suggestions");
        assert!(a.risks.is_empty(), "blocked_path inside the worktree is not a risk: {:?}", a.risks);
        assert!(events.iter().any(|e| matches!(e, EventKind::ApprovalResolved { approval_id, status: ApprovalStatus::Cancelled, by } if *approval_id == a.id && by == "provider")));
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::PlanUpdated { .. })).count(), 2);
        let plan = events.iter().rev().find_map(|e| if let EventKind::PlanUpdated { steps } = e { Some(steps.clone()) } else { None }).unwrap();
        assert_eq!(plan.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Write hello.txt", "Reply DONE"]);
        // the rejected Bash call completes as failed, the turn as interrupted (terminal_reason aborted_tools)
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: false, .. })));
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Interrupted, .. })));
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type == "user")));
        // (an interrupted turn is no failure: its diagnostic `errors` line is not shown)
        assert!(!events.iter().any(|e| matches!(e, EventKind::Error { .. })), "all requested tools were enabled");
        // Usage arrives during the turn, after each API call; the result confirms the totals.
        let end = events.iter().position(|e| matches!(e, EventKind::TurnCompleted { .. })).unwrap();
        let usages: Vec<Usage> = events[..end].iter().filter_map(|e| if let EventKind::UsageUpdated { usage } = e { Some(usage.clone()) } else { None }).collect();
        let tokens = |u: &Usage| (u.input_tokens, u.output_tokens, u.cache_read_tokens, u.cache_write_tokens);
        assert_eq!(usages.iter().map(tokens).collect::<Vec<_>>(), [(2, 299, 13025, 3685), (4, 385, 29735, 4141), (4, 385, 29735, 4141)]);
        assert_eq!(usages[0].context_used, Some(2 + 299 + 13025 + 3685));
    }

    #[test]
    fn replays_the_pong_fixture_without_partials() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "c84b6b3f".into());
        let events = replay(&mut norm, &fixture("claude-pong.jsonl"));
        let msg = events.iter().find_map(|e| match e {
            EventKind::Message { text, phase, .. } => Some((text.clone(), *phase)),
            _ => None,
        });
        assert_eq!(msg, Some(("pong".into(), MessagePhase::Final)));
        // post_turn_summary, commands_changed and the hooks are known bookkeeping: no rows.
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type.starts_with("system/"))), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })));
    }

    #[test]
    fn initialize_response_gives_session_info_for_free() {
        let lines = fixture("claude-control-protocol.jsonl");
        let init = lines.iter().find(|l| l["dir"] == "<-" && l["msg"]["response"]["request_id"] == "req_1").unwrap();
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        match norm.on_initialize(&init["msg"]["response"]["response"]) {
            EventKind::SessionStarted { cli_version, session_id, model, tools, .. } => {
                assert_eq!(cli_version.as_deref(), Some("2.1.309"));
                assert_eq!(session_id.as_deref(), Some("sess-1"));
                assert_eq!(model.as_deref(), Some("claude-haiku-5-5"), "alias resolved from initialize.models");
                assert!(tools.is_empty());
            }
            other => panic!("{other:?}"),
        }
        // the free control responses themselves are not events
        for l in lines.iter().filter(|l| l["dir"] == "<-") {
            let ev = norm.on_message(&l["msg"]);
            assert!(ev.iter().all(|e| !matches!(e, EventKind::Error { .. })), "{ev:?}");
        }
    }

    fn can_use_tool(tool: &str, input: Value, suggestions: Value) -> Value {
        json!({ "type": "control_request", "request_id": "cli-7", "request": {
            "subtype": "can_use_tool", "tool_name": tool, "input": input, "tool_use_id": "toolu_1",
            "permission_suggestions": suggestions, "decision_reason": "Bash commands need approval"
        }})
    }

    fn approval_of(events: &[EventKind]) -> Approval {
        events
            .iter()
            .find_map(|e| match e {
                EventKind::ApprovalRequested { approval } => Some(approval.clone()),
                _ => None,
            })
            .expect("ApprovalRequested")
    }

    #[test]
    fn bash_approval_round_trip() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let suggestion = json!([{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "echo:*" }], "behavior": "allow", "destination": "localSettings" }]);
        let events = norm.on_message(&can_use_tool("Bash", json!({ "command": "echo codly > hello.txt", "description": "write" }), suggestion.clone()));
        let a = approval_of(&events);
        assert!(matches!(&a.subject, ApprovalSubject::Command { command, .. } if command == "echo codly > hello.txt"));
        // A worker is not offered the CLI's "always": it would be saved in its task worktree.
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        assert_eq!(a.call_id.as_deref(), Some("toolu_1"));
        assert_eq!(a.reason.as_deref(), Some("Bash commands need approval"));
        assert!(events.iter().any(|e| matches!(e, EventKind::StatusChanged { status: AgentStatus::WaitingApproval })));

        let (reply, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
        assert_eq!(reply["type"], "control_response");
        assert_eq!(reply["response"]["request_id"], "cli-7");
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(reply["response"]["response"]["updatedInput"]["command"], "echo codly > hello.txt");
        assert!(reply["response"]["response"].get("updatedPermissions").is_none());
        assert!(norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).is_none(), "answered once");

        // session → the suggested rule, scoped to the session
        let a = approval_of(&norm.on_message(&can_use_tool("Bash", json!({"command": "ls"}), suggestion.clone())));
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowSession, message: None, answers: None }).unwrap();
        assert_eq!(r["response"]["response"]["updatedPermissions"][0]["destination"], "session");
        // always (a 1:1 agent, whose directory stays) → echo the suggestion
        let mut solo = ClaudeNormalizer::new(&spec(Role::Solo, false), "s".into());
        let a = approval_of(&solo.on_message(&can_use_tool("Bash", json!({"command": "ls"}), suggestion.clone())));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::AllowAlways, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        let (r, _) = solo.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowAlways, message: None, answers: None }).unwrap();
        assert_eq!(r["response"]["response"]["updatedPermissions"], suggestion);

        // No suggested rule: nothing would hold for the session, so only "once" is offered.
        // deny with feedback, cancel interrupts
        let a = approval_of(&norm.on_message(&can_use_tool("Bash", json!({"command": "rm -rf build"}), json!([]))));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        assert_eq!(a.risks, vec!["deletes files".to_string()]);
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Deny, message: Some("use pnpm".into()), answers: None }).unwrap();
        assert_eq!(r["response"]["response"], json!({ "behavior": "deny", "message": "use pnpm", "interrupt": false }));
        let a = approval_of(&norm.on_message(&can_use_tool("Bash", json!({"command": "ls"}), json!([]))));
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Cancel, message: None, answers: None }).unwrap();
        assert_eq!(r["response"]["response"]["interrupt"], true);
        assert!(norm.interrupt_requested);
    }

    #[test]
    fn questions_edits_cancel_and_unsupported_requests() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let qs = json!([{ "question": "Which DB?", "header": "DB", "options": [{"label": "sqlite"}, {"label": "pg"}], "multiSelect": false }]);
        let a = approval_of(&norm.on_message(&can_use_tool("AskUserQuestion", json!({ "questions": qs }), json!([]))));
        assert!(matches!(&a.subject, ApprovalSubject::Question { questions } if questions == &qs));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        let answers = json!({ "Which DB?": "sqlite" });
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: Some(answers.clone()) }).unwrap();
        assert_eq!(r["response"]["response"]["updatedInput"]["answers"], answers);
        assert_eq!(r["response"]["response"]["updatedInput"]["questions"], qs);

        let a = approval_of(&norm.on_message(&can_use_tool("Write", json!({ "file_path": "/etc/hosts", "content": "x\ny" }), json!([]))));
        match &a.subject {
            ApprovalSubject::FileChange { paths, diff } => {
                assert_eq!(paths, &vec!["/etc/hosts".to_string()]);
                assert_eq!(diff.as_deref(), Some("+x\n+y"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(a.risks, vec!["outside worktree".to_string()]);

        let a = approval_of(&norm.on_message(&can_use_tool("WebFetch", json!({ "url": "https://x" }), json!([]))));
        assert!(matches!(&a.subject, ApprovalSubject::Tool { name, .. } if name == "WebFetch"));
        let ev = norm.on_message(&json!({ "type": "control_cancel_request", "request_id": "cli-7" }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ApprovalResolved { status: ApprovalStatus::Cancelled, by, .. } if by == "provider")));

        let ev = norm.on_message(&json!({ "type": "control_request", "request_id": "h1", "request": { "subtype": "hook_callback" } }));
        assert!(matches!(&ev[0], EventKind::Unknown { raw_type } if raw_type == "control_request/hook_callback"));
        assert_eq!(norm.outbox[0]["response"]["subtype"], "error");
        assert_eq!(norm.outbox[0]["response"]["request_id"], "h1");
    }

    fn tool_use(id: &str, name: &str, input: Value) -> Value {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": format!("msg_{id}"), "content": [
            { "type": "tool_use", "id": id, "name": name, "input": input }
        ]}})
    }

    fn tool_result(id: &str, text: &str, structured: Value, is_error: bool) -> Value {
        json!({ "type": "user", "parent_tool_use_id": null, "tool_use_result": structured, "message": { "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": id, "content": text, "is_error": is_error }
        ]}})
    }

    fn last_plan(events: &[EventKind]) -> Vec<(String, String)> {
        events
            .iter()
            .rev()
            .find_map(|e| match e {
                EventKind::PlanUpdated { steps } => Some(steps.iter().map(|s| (s.text.clone(), s.status.clone())).collect()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn task_tools_maintain_the_agents_todo_list() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let mut ev = Vec::new();
        ev.extend(norm.on_message(&tool_use("t1", "TaskCreate", json!({ "subject": "Write the parser", "description": "x" }))));
        ev.extend(norm.on_message(&tool_result("t1", "Task #1 created successfully: Write the parser", json!({ "task": { "id": "1", "subject": "Write the parser" } }), false)));
        ev.extend(norm.on_message(&tool_use("t2", "TaskCreate", json!({ "subject": "Add tests", "description": "y" }))));
        ev.extend(norm.on_message(&tool_result("t2", "Task #2 created successfully: Add tests", Value::Null, false)));
        assert_eq!(last_plan(&ev), vec![("Write the parser".into(), "pending".into()), ("Add tests".into(), "pending".into())]);
        ev.extend(norm.on_message(&tool_use("t3", "TaskUpdate", json!({ "taskId": "1", "status": "in_progress" }))));
        ev.extend(norm.on_message(&tool_result("t3", "Updated task #1 status", json!({ "success": true, "taskId": "1" }), false)));
        assert_eq!(last_plan(&ev)[0], ("Write the parser".into(), "in_progress".into()));
        ev.extend(norm.on_message(&tool_use("t4", "TaskUpdate", json!({ "id": "2", "status": "deleted" }))));
        ev.extend(norm.on_message(&tool_result("t4", "ok", json!({ "success": true }), false)));
        assert_eq!(last_plan(&ev).len(), 1);
        let list = json!({ "tasks": [{ "id": "1", "subject": "Write the parser", "status": "completed", "blockedBy": [] }, { "id": "3", "subject": "Ship", "status": "pending", "blockedBy": [] }] });
        ev.extend(norm.on_message(&tool_use("t5", "TaskList", json!({}))));
        ev.extend(norm.on_message(&tool_result("t5", "#1 [completed] Write the parser", list, false)));
        assert_eq!(last_plan(&ev), vec![("Write the parser".into(), "completed".into()), ("Ship".into(), "pending".into())]);
        // legacy TodoWrite still works on older CLIs
        ev.extend(norm.on_message(&tool_use("t6", "TodoWrite", json!({ "todos": [{ "content": "a", "status": "completed", "activeForm": "A" }] }))));
        ev.extend(norm.on_message(&tool_result("t6", "ok", Value::Null, false)));
        assert_eq!(last_plan(&ev), vec![("a".into(), "completed".into())]);
        // a failed create changes nothing
        let before = ev.iter().filter(|e| matches!(e, EventKind::PlanUpdated { .. })).count();
        let mut more = norm.on_message(&tool_use("t7", "TaskCreate", json!({ "subject": "z" })));
        more.extend(norm.on_message(&tool_result("t7", "error", Value::Null, true)));
        assert_eq!(more.iter().filter(|e| matches!(e, EventKind::PlanUpdated { .. })).count(), 0);
        assert!(before > 0);
    }

    #[test]
    fn edits_report_files_and_tests_report_counts() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let mut ev = norm.on_message(&tool_use("e1", "Edit", json!({ "file_path": "/w/src/a.rs", "old_string": "a", "new_string": "b" })));
        ev.extend(norm.on_message(&tool_result("e1", "The file /w/src/a.rs has been updated.", Value::Null, false)));
        assert!(ev.iter().any(|e| matches!(e, EventKind::FilesChanged { paths, source, .. } if paths == &vec!["/w/src/a.rs".to_string()] && source == "tool_input")));
        let mut ev = norm.on_message(&tool_use("b1", "Bash", json!({ "command": "cargo test" })));
        ev.extend(norm.on_message(&tool_result("b1", "Exit code 101\ntest result: FAILED. 3 passed; 1 failed; 0 ignored", Value::Null, true)));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ToolStarted { activity: ActivityKind::Test, .. })));
        assert!(ev.iter().any(|e| matches!(e,
            EventKind::ToolCompleted { ok: false, exit_code: Some(101), tests_passed: Some(3), tests_failed: Some(1), .. })));
        // a denied edit changes nothing
        let mut ev = norm.on_message(&tool_use("e2", "Write", json!({ "file_path": "/w/b.rs", "content": "x" })));
        ev.extend(norm.on_message(&tool_result("e2", "The user doesn't want to proceed", Value::Null, true)));
        assert!(!ev.iter().any(|e| matches!(e, EventKind::FilesChanged { .. })));
    }

    #[test]
    fn tools_and_user_setup() {
        let mut sp = spec(Role::Worker, false);
        let joined = |s: &SpawnSpec| build_args(s, "sid", None).join(" ");
        let a = joined(&sp);
        assert!(a.contains("Monitor,TaskStop,Skill,LSP") && a.contains("--strict-mcp-config") && a.contains("user,local"));
        assert!(!a.contains("ToolSearch") && !a.contains("Cron") && !a.contains("ScheduleWakeup") && !a.contains("RemoteTrigger"));
        sp.use_user_setup = true;
        let a = joined(&sp);
        assert!(!a.contains("--strict-mcp-config") && a.contains("user,project,local") && a.contains("ToolSearch"));
        // the Lead stays read-only either way
        let lead = joined(&SpawnSpec { use_user_setup: true, ..spec(Role::Lead, true) });
        assert!(lead.contains("Skill,LSP") && !lead.contains("Bash") && !lead.contains("Monitor"));
    }

    #[test]
    fn interrupted_and_failed_results() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        norm.turn_started("x");
        norm.interrupt_requested = true;
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Interrupted, .. })));
        norm.turn_started("y");
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "error_max_turns", "is_error": true }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::MaxTurns, .. })));
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "API Error: 500" }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Failed, .. })));
        let ev = norm.on_message(&json!({ "type": "system", "subtype": "api_retry", "attempt": 2, "error": "overloaded" }));
        assert!(matches!(&ev[0], EventKind::Retrying { attempt: 2, reason } if reason == "Anthropic's servers are overloaded"));
        let ev = norm.on_message(&json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "error": "authentication_failed" }));
        assert!(matches!(&ev[0], EventKind::Error { fatal: true, .. }));
        let ev = norm.on_message(&json!({ "type": "brand_new_type" }));
        assert!(matches!(&ev[0], EventKind::Unknown { raw_type } if raw_type == "brand_new_type"));
    }

    fn errors(events: &[EventKind]) -> Vec<(&str, bool)> {
        events.iter().filter_map(|e| if let EventKind::Error { message, fatal } = e { Some((message.as_str(), *fatal)) } else { None }).collect()
    }

    fn failed_with(events: &[EventKind]) -> Option<&str> {
        events.iter().find_map(|e| match e {
            EventKind::TurnCompleted { status: TurnStatus::Failed, result_text, .. } => Some(result_text.as_deref().unwrap_or("")),
            _ => None,
        })
    }

    const SIGN_IN: &str = "Claude Code is not signed in: run `claude auth login` in a terminal, then try again";

    #[test]
    fn a_failed_turn_says_why_once() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let failed = |result: Value| {
            let mut v = json!({ "type": "result", "subtype": "error_during_execution", "is_error": true });
            v.as_object_mut().unwrap().extend(result.as_object().unwrap().clone());
            v
        };
        // No `result` text: the CLI's `errors`, without its diagnostic lines.
        norm.turn_started("a");
        let ev = norm.on_message(&failed(json!({ "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use", "the request was aborted"] })));
        assert_eq!(errors(&ev), [("the request was aborted", false)]);
        assert_eq!(failed_with(&ev), Some("the request was aborted"));
        let end = ev.iter().position(|e| matches!(e, EventKind::TurnCompleted { .. })).unwrap();
        assert!(matches!(ev[end - 1], EventKind::Error { .. }), "the reason comes before the turn's end: {ev:?}");
        // Only a diagnostic line: nothing to say.
        norm.turn_started("b");
        let ev = norm.on_message(&failed(json!({ "errors": ["[ede_diagnostic] result_type=user"] })));
        assert!(errors(&ev).is_empty());
        assert_eq!(failed_with(&ev), Some(""));

        // The CLI's own words stay when Codly knows nothing better.
        norm.turn_started("c");
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "API Error: 400 prompt is too long" }));
        assert_eq!(errors(&ev), [("API Error: 400 prompt is too long", false)]);
        assert_eq!(failed_with(&ev), Some("API Error: 400 prompt is too long"));
        // Said by the agent's last message already: not said again as an error.
        norm.turn_started("d");
        norm.on_message(&json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "m1", "content": [{ "type": "text", "text": "API Error: 400 prompt is too long" }] } }));
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "API Error: 400 prompt is too long" }));
        assert!(errors(&ev).is_empty(), "{ev:?}");
        assert_eq!(failed_with(&ev), Some("API Error: 400 prompt is too long"));

        // The HTTP status of the result, in plain words with what to do.
        norm.turn_started("e");
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "api_error_status": 529, "result": "API Error: 529 overloaded_error" }));
        assert_eq!(errors(&ev), [("Anthropic's servers are overloaded: wait a moment, then try again", false)]);
        assert_eq!(failed_with(&ev), Some("Anthropic's servers are overloaded: wait a moment, then try again"));

        // "Please run /login" is no command of Codly: the sign-in command instead.
        norm.turn_started("f");
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "Invalid API key · Please run /login" }));
        assert_eq!(errors(&ev), [(SIGN_IN, false)]);
        assert_eq!(failed_with(&ev), Some(SIGN_IN));
        // With the user's own API key, signing in is not the fix.
        let mut keyed = spec(Role::Worker, false);
        keyed.env.insert("ANTHROPIC_API_KEY".into(), "sk-ant-test".into());
        let mut own = ClaudeNormalizer::new(&keyed, "s".into());
        own.turn_started("f");
        let ev = own.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "api_error_status": 401, "result": "Invalid API key" }));
        assert_eq!(errors(&ev), [("Claude did not accept your API key or token: check it in Settings → API keys, then try again", false)]);
        // The account's own home goes into the command.
        let mut work = spec(Role::Worker, false);
        work.env.insert("CLAUDE_CONFIG_DIR".into(), "/Users/x/.claude-work".into());
        let mut work = ClaudeNormalizer::new(&work, "s".into());
        work.turn_started("f");
        let ev = work.on_message(&json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "error": "authentication_failed" }));
        assert_eq!(errors(&ev), [("Claude Code is not signed in: run `CLAUDE_CONFIG_DIR=/Users/x/.claude-work claude auth login` in a terminal, then try again", true)]);
    }

    #[test]
    fn api_retries_say_what_happens_and_what_to_do() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let retry = |fields: Value| {
            let mut v = json!({ "type": "system", "subtype": "api_retry" });
            v.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
            v
        };
        // Signed out: one line with the command, however often Claude retries, and the failed
        // turn does not say it again.
        norm.turn_started("a");
        let ev = norm.on_message(&retry(json!({ "attempt": 1, "error": "authentication_failed", "error_status": 401 })));
        assert_eq!(errors(&ev), [(SIGN_IN, true)]);
        assert!(norm.on_message(&retry(json!({ "attempt": 2, "error": "authentication_failed" }))).is_empty());
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "Invalid API key · Please run /login" }));
        assert!(errors(&ev).is_empty(), "{ev:?}");
        assert_eq!(failed_with(&ev), Some(SIGN_IN));

        // What only you can fix stops the wait, with what to do.
        norm.turn_started("b");
        for (kind, says) in [
            ("billing_error", "Claude reports a billing problem on this account: check your plan or your credits, then try again"),
            ("account_on_hold", "This Claude account is on hold: check the account with Anthropic, then try again"),
            ("oauth_org_not_allowed", "Claude does not let this account's organization use Claude Code: sign in with another account, then try again"),
            ("model_not_found", "The model haiku is not available on this account: pick another model for this agent"),
            ("cloud_credential_error", "Claude could not use your cloud credentials (Bedrock or Vertex): renew them, then try again"),
        ] {
            let ev = norm.on_message(&retry(json!({ "attempt": 1, "error": kind })));
            assert_eq!(errors(&ev), [(says, true)], "{kind}");
        }

        // What Claude retries by itself: why, when the next try is, and how many there are.
        let ev = norm.on_message(&retry(json!({ "attempt": 2, "max_retries": 10, "retry_delay_ms": 4200, "error": "overloaded", "error_status": 529 })));
        assert!(matches!(&ev[..], [EventKind::Retrying { attempt: 2, reason }] if reason == "Anthropic's servers are overloaded; next try in 5 s (10 tries at most)"), "{ev:?}");
        let ev = norm.on_message(&retry(json!({ "attempt": 1, "error": "unknown", "error_status": 503 })));
        assert!(matches!(&ev[..], [EventKind::Retrying { reason, .. }] if reason == "Anthropic's servers had an error"), "{ev:?}");
        let ev = norm.on_message(&retry(json!({ "attempt": 1, "error": "invalid_request" })));
        assert!(matches!(&ev[..], [EventKind::Retrying { reason, .. }] if reason == "invalid request"), "{ev:?}");
        let ev = norm.on_message(&retry(json!({ "attempt": 1, "error": "unknown" })));
        assert!(matches!(&ev[..], [EventKind::Retrying { reason, .. }] if reason == "an API error"), "{ev:?}");
        let ev = norm.on_message(&retry(json!({ "attempt": 3, "error": "rate_limit" })));
        assert!(matches!(&ev[..], [EventKind::StatusChanged { status: AgentStatus::RateLimited }, EventKind::Retrying { attempt: 3, reason }] if reason == "the usage limit is reached"), "{ev:?}");
        // Claude gives up on the limit: the failed turn says what to do.
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "API Error: 429" }));
        assert_eq!(failed_with(&ev), Some("Claude's usage limit is reached: wait for it to reset, then try again"));

        // A retry that worked explains nothing about a later failure.
        norm.turn_started("c");
        norm.on_message(&retry(json!({ "attempt": 1, "error": "overloaded" })));
        norm.on_message(&json!({ "type": "stream_event", "parent_tool_use_id": null, "event": { "type": "message_start", "message": { "id": "m2" } } }));
        let ev = norm.on_message(&json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "errors": ["the request was aborted"] }));
        assert_eq!(failed_with(&ev), Some("the request was aborted"));
    }

    #[test]
    fn known_system_lines_are_shown_as_what_they_are() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        norm.turn_started("go");
        let system = |subtype: &str, fields: Value| {
            let mut v = json!({ "type": "system", "subtype": subtype });
            v.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
            v
        };
        // Claude Code refused a call by itself (a rule, a hook, its mode).
        let ev = norm.on_message(&system("permission_denied", json!({ "tool_name": "Bash", "tool_use_id": "toolu_9", "decision_reason_type": "rule", "message": "Permission to use Bash has been denied." })));
        assert_eq!(errors(&ev), [("Claude Code denied Bash without asking: Permission to use Bash has been denied.", false)]);
        let ev = norm.on_message(&system("permission_denied", json!({ "tool_use_id": "toolu_8" })));
        assert_eq!(errors(&ev), [("Claude Code denied a tool without asking", false)]);
        // A call you denied in Codly: its card already says so.
        let a = approval_of(&norm.on_message(&can_use_tool("Bash", json!({ "command": "ls" }), json!([]))));
        norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }).unwrap();
        let ev = norm.on_message(&system("permission_denied", json!({ "tool_name": "Bash", "tool_use_id": "toolu_1", "message": "The user denied this action." })));
        assert!(ev.is_empty(), "{ev:?}");

        let ev = norm.on_message(&system("model_refusal_fallback", json!({})));
        assert_eq!(errors(&ev), [("The model refused this request: Claude Code goes on with its fallback model", false)]);
        let ev = norm.on_message(&system("model_refusal_no_fallback", json!({})));
        assert_eq!(errors(&ev), [("The model refused this request, and Claude Code has no fallback model to go on with", false)]);

        // Bookkeeping lines make no rows; a subtype Codly does not know still does.
        for subtype in ["hook_started", "hook_progress", "hook_response", "commands_changed", "session_title_changed", "background_tasks_changed", "files_persisted", "post_turn_summary", "task_updated"] {
            assert!(norm.on_message(&system(subtype, json!({}))).is_empty(), "{subtype}");
        }
        let ev = norm.on_message(&system("brand_new_subtype", json!({})));
        assert!(matches!(&ev[..], [EventKind::Unknown { raw_type }] if raw_type == "system/brand_new_subtype"));

        // The user's own MCP servers that did not start: Codly asks the CLI why, then says it
        // in one line. One that still connects, or that waits for a sign-in (a connector the
        // user never signed in to), is no failure.
        let ev = norm.on_message(&system("init", json!({ "tools": [], "mcp_servers": [
            { "name": "codly", "status": "connected" }, { "name": "context7", "status": "failed" }, { "name": "linear", "status": "needs-auth" },
            { "name": "slow", "status": "pending" }, { "name": "git", "status": "connected" }, { "name": "docs", "status": "failed" }
        ] })));
        assert!(errors(&ev).is_empty(), "{ev:?}");
        assert_eq!(norm.outbox.last(), Some(&json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } })));
        // A CLI that does not know the request (its recorded answer to an unknown subtype):
        // the names are still said.
        let refused = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": MCP_STATUS_REQUEST, "error": "Unsupported control request subtype: mcp_status" } });
        let ev = norm.on_message(&refused);
        assert_eq!(errors(&ev), [("These MCP servers did not start, so the agent works without their tools in this run: context7, docs", false)]);
    }

    /// The CLI's stdout lines of a 2.1.311 recording (docs/research/fixtures/claude-2.1.311),
    /// without the lines of the recording driver (`_note`, `_t`, `_sent`, `_stderr`, `_meta`).
    fn recorded(name: &str) -> Vec<Value> {
        fixture(&format!("claude-2.1.311/{name}.ndjson")).into_iter().filter(|l| l.get("type").is_some()).collect()
    }

    /// The subagent events, one short line each: only what the event says.
    fn subagent_lines(events: &[EventKind]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                EventKind::SubAgent { call_id, description, status } => Some(format!("{call_id} {status}: {description}")),
                EventKind::SubAgentProgress { call_id, name, step, tool_call_id, tool_uses, tokens, report } => {
                    let mut line = call_id.clone();
                    for (key, value) in [("name", name), ("step", step), ("call", tool_call_id), ("report", report)] {
                        if let Some(value) = value {
                            line.push_str(&format!(" {key}={value}"));
                        }
                    }
                    for (key, value) in [("uses", tool_uses), ("tokens", tokens)] {
                        if let Some(value) = value {
                            line.push_str(&format!(" {key}={value}"));
                        }
                    }
                    Some(line)
                }
                _ => None,
            })
            .collect()
    }

    fn tool_calls(events: &[EventKind]) -> Vec<(&str, &str)> {
        events.iter().filter_map(|e| if let EventKind::ToolStarted { call_id, name, .. } = e { Some((call_id.as_str(), name.as_str())) } else { None }).collect()
    }

    fn turn_ends(events: &[EventKind]) -> Vec<(TurnStatus, Option<&str>)> {
        events.iter().filter_map(|e| if let EventKind::TurnCompleted { status, result_text, .. } = e { Some((*status, result_text.as_deref())) } else { None }).collect()
    }

    /// Recording `subagent`: the Agent call returns at once ("async_launched"), the turn
    /// ends, the subagent works on, and the CLI starts a second turn when it is done.
    #[test]
    fn a_background_subagent_is_followed_past_the_turn_that_started_it() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("Use the Agent tool to have a subagent run `echo hi`");
        events.extend(replay(&mut norm, &recorded("subagent")));

        assert_eq!(
            subagent_lines(&events),
            [
                "toolu-1 running: Run echo hi via Bash",
                "toolu-1 name=general-purpose",
                // (the call returned and the turn ended: no "completed" yet)
                "toolu-1 call=toolu-2",
                "toolu-1 step=Running Echo hi to stdout uses=1 tokens=5932",
                "toolu-1 report=hi uses=1 tokens=6604",
                "toolu-1 completed: Run echo hi via Bash",
            ]
        );
        // Its own call is a tool call like any other, and the next event says whose it is.
        assert_eq!(tool_calls(&events), [("toolu-1", "Agent"), ("toolu-2", "Bash")]);
        let link = events.iter().position(|e| matches!(e, EventKind::SubAgentProgress { tool_call_id: Some(id), .. } if id == "toolu-2")).unwrap();
        assert!(matches!(&events[link - 1], EventKind::ToolStarted { call_id, .. } if call_id == "toolu-2"));
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { call_id, ok: true, exit_code: Some(0), .. } if call_id == "toolu-2")), "its call ends with its name known");
        // What the subagent said is its report, not a message of the agent.
        let said: Vec<&str> = events.iter().filter_map(|e| if let EventKind::Message { text, .. } = e { Some(text.as_str()) } else { None }).collect();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].starts_with("I've launched the subagent") && said[1].starts_with("The subagent ran `echo hi`"), "{said:?}");
        assert_eq!(turn_ends(&events).iter().map(|t| t.0).collect::<Vec<_>>(), [TurnStatus::Completed, TurnStatus::Completed]);
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { .. })), "{events:?}");
    }

    /// Recording `subagent-foreground`: the Agent call waits for the subagent.
    #[test]
    fn a_foreground_subagent_shows_its_kind_its_steps_and_its_report() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("Use the Agent tool to have a subagent run `echo hi`");
        events.extend(replay(&mut norm, &recorded("subagent-foreground")));

        assert_eq!(
            subagent_lines(&events),
            [
                "toolu-1 running: Run echo hi via Bash",
                "toolu-1 name=general-purpose",
                "toolu-1 step=Running Print hi via echo uses=1 tokens=5934",
                "toolu-1 call=toolu-2",
                "toolu-1 report=hi uses=1 tokens=6604",
                // (once: the Agent call's own result comes after it and adds nothing)
                "toolu-1 completed: Run echo hi via Bash",
            ]
        );
        assert_eq!(tool_calls(&events), [("toolu-1", "Agent"), ("toolu-2", "Bash")]);
        // The prompt the subagent starts with is a known line, not an "unknown" row.
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { .. })), "{events:?}");
        assert_eq!(turn_ends(&events).iter().map(|t| t.0).collect::<Vec<_>>(), [TurnStatus::Completed]);
    }

    /// The same recording, cut before the subagent reports: the turn ends (its recorded
    /// `result` line) while the Agent call still waits, as when the turn is stopped.
    #[test]
    fn a_foreground_subagent_is_over_with_its_turn() {
        let lines = recorded("subagent-foreground");
        let cut = lines.iter().position(|l| l["subtype"] == "task_updated").unwrap();
        let result = lines.iter().find(|l| l["type"] == "result").unwrap();
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("Use the Agent tool to have a subagent run `echo hi`");
        events.extend(replay(&mut norm, &lines[..cut]));
        assert_eq!(subagent_lines(&events).last().map(String::as_str), Some("toolu-1 call=toolu-2"), "still at work");
        let end = norm.on_message(result);
        assert_eq!(subagent_lines(&end), ["toolu-1 stopped: Run echo hi via Bash"]);
        assert!(matches!(end.last(), Some(EventKind::StatusChanged { status: AgentStatus::Idle })));
    }

    /// Recordings `mid-turn-user-message` and `background-wakeup`: `task_*` lines of a command
    /// (`task_type: "local_bash"`). The command is a tool call already.
    #[test]
    fn a_long_or_background_command_is_no_subagent() {
        for name in ["mid-turn-user-message", "background-wakeup"] {
            let lines = recorded(name);
            assert!(lines.iter().any(|l| l["subtype"] == "task_notification"), "{name} has task lines");
            let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
            let mut events = norm.turn_started("go");
            events.extend(replay(&mut norm, &lines));
            assert_eq!(subagent_lines(&events), Vec::<String>::new(), "{name}");
            assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { .. })), "{name}: {events:?}");
        }
    }

    /// Recording `broken-mcp`: two servers that cannot start. The init line names them; the
    /// answer to `mcp_status` has the reason of each.
    #[test]
    fn mcp_servers_that_did_not_start_are_named_with_the_reason() {
        let lines = recorded("broken-mcp");
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("Reply with the single word: pong");
        let init = lines.iter().find(|l| l["subtype"] == "init").unwrap();
        let ev = norm.on_message(init);
        assert!(!errors(&ev).iter().any(|(message, _)| message.contains("MCP")), "nothing is said before the reason is known: {ev:?}");
        assert_eq!(norm.outbox, [json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } })]);

        // An answer to someone else's request (the recording driver's `host-1`) is not Codly's.
        let other = lines.iter().find(|l| l["response"]["request_id"] == "host-1").unwrap();
        assert!(!norm.asked(&other["response"]));
        assert!(norm.on_message(other).is_empty());
        // The recorded answer to `mcp_status` after the turn (`host-2`), under the id Codly asks with.
        let mut answer = lines.iter().find(|l| l["response"]["request_id"] == "host-2").unwrap().clone();
        answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
        assert!(norm.asked(&answer["response"]));
        let ev = norm.on_message(&answer);
        assert_eq!(
            errors(&ev),
            [(
                "These MCP servers did not start, so the agent works without their tools in this run: \
                 exits-at-once (Connection closed), no-such-command (ENOENT: no such file or directory, posix_spawn 'stdio')",
                false
            )]
        );
        assert!(norm.on_message(&answer).is_empty(), "said once");

        // A server that is no longer failed when the CLI answers (`host-6`: one was switched
        // off meanwhile) is left out.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.on_message(init);
        let mut answer = lines.iter().find(|l| l["response"]["request_id"] == "host-6").unwrap().clone();
        answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
        let ev = norm.on_message(&answer);
        assert_eq!(errors(&ev), [("These MCP servers did not start, so the agent works without their tools in this run: exits-at-once (Connection closed)", false)]);
    }

    /// Recordings `limits-max-turns` and `limits-max-budget`.
    #[test]
    fn the_clis_own_limits_end_a_turn_with_their_reason() {
        let own_errors = |events: &[EventKind]| -> Vec<String> {
            errors(events).into_iter().map(|(message, _)| message.to_string()).filter(|m| !m.starts_with("Claude did not enable requested tools")).collect()
        };
        // The step limit ends that turn only: the next message is answered.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("Run `echo one`, then `echo two`");
        events.extend(replay(&mut norm, &recorded("limits-max-turns")));
        assert_eq!(turn_ends(&events), [(TurnStatus::MaxTurns, None), (TurnStatus::Completed, Some("alive"))]);
        assert_eq!(own_errors(&events), Vec::<String>::new(), "the turn's end says it: no second line");

        // The spending limit ends every turn of the run from then on.
        let budget = "Claude Code reached its spending limit, so it does no more work in this run: Reached maximum budget ($0.0001)";
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("Run `echo one`, then `echo two`");
        events.extend(replay(&mut norm, &recorded("limits-max-budget")));
        assert_eq!(turn_ends(&events), [(TurnStatus::Failed, Some(budget)), (TurnStatus::Failed, Some(budget))]);
        assert_eq!(own_errors(&events), [budget], "said once, before the first turn's end");
    }

    /// Recording `misc-events`: the result's `permission_denials` lists calls Codly itself was
    /// asked about. The same entry for a call nobody asked about is Claude Code's own denial.
    #[test]
    fn a_denial_codly_was_not_asked_about_is_said() {
        let lines = recorded("misc-events");
        let silent = |events: &[EventKind]| !errors(events).iter().any(|(message, _)| message.contains("without asking"));
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("go");
        events.extend(replay(&mut norm, &lines));
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::ApprovalRequested { .. })).count(), 2);
        assert!(silent(&events), "both denied calls had a card: {:?}", errors(&events));

        let result = lines.iter().find(|l| l["type"] == "result" && l["permission_denials"].as_array().is_some_and(|d| !d.is_empty())).unwrap();
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("go");
        let ev = norm.on_message(result);
        assert_eq!(errors(&ev), [("Claude Code denied Bash without asking", false)]);
        // A call that a `permission_denied` line named is not said twice.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("go");
        let call = result["permission_denials"][0]["tool_use_id"].clone();
        norm.on_message(&json!({ "type": "system", "subtype": "permission_denied", "tool_name": "Bash", "tool_use_id": call }));
        assert!(silent(&norm.on_message(result)));
    }

    /// Recording `resume-missing`: all the CLI writes for `--resume` of a conversation it no
    /// longer has is one `result` line (then it exits with code 1).
    #[test]
    fn a_conversation_that_is_gone_is_no_failed_turn() {
        let lines = recorded("resume-missing");
        assert_eq!(lines.len(), 1);
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let events = replay(&mut norm, &lines);
        assert!(
            matches!(&events[..], [EventKind::Error { message, fatal: false }]
                if message == "Claude Code no longer has this conversation (No conversation found with session ID: sess-1)"),
            "{events:?}"
        );
    }

    #[test]
    fn usage_follows_the_turn_and_never_goes_back() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into());
        let stream = |parent: Value, event: Value| json!({ "type": "stream_event", "parent_tool_use_id": parent, "event": event });
        let start = |norm: &mut ClaudeNormalizer| norm.on_message(&stream(Value::Null, json!({ "type": "message_start", "message": { "id": "m" } })));
        let delta = |norm: &mut ClaudeNormalizer, parent: Value, output: i64| {
            let usage = json!({ "input_tokens": 10, "output_tokens": output, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 5 });
            norm.on_message(&stream(parent, json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": usage })))
        };
        let tokens = |ev: &[EventKind]| {
            ev.iter().find_map(|e| if let EventKind::UsageUpdated { usage } = e { Some((usage.input_tokens, usage.output_tokens, usage.cache_read_tokens, usage.cache_write_tokens)) } else { None })
        };
        let result = |norm: &mut ClaudeNormalizer, input: i64, output: i64| {
            let model = json!({ "inputTokens": input, "outputTokens": output, "cacheReadInputTokens": 100, "cacheCreationInputTokens": 5, "costUSD": 0.01 });
            norm.on_message(&json!({ "type": "result", "subtype": "success", "result": "ok", "model_usage_since_last_result": { "m": model } }))
        };

        norm.turn_started("go");
        start(&mut norm);
        // One API call counts once, however often it reports its (growing) usage.
        assert_eq!(tokens(&delta(&mut norm, Value::Null, 25)), Some((10, 25, 100, 5)));
        assert_eq!(tokens(&delta(&mut norm, Value::Null, 40)), Some((10, 40, 100, 5)));
        start(&mut norm);
        assert_eq!(tokens(&delta(&mut norm, Value::Null, 60)), Some((20, 100, 200, 10)));
        // A subagent's stream is not the agent's own: its calls arrive with the result.
        assert_eq!(tokens(&delta(&mut norm, json!("toolu_agent"), 999)), None);
        // The result counts less than the stream did (a call it leaves out): the totals stay,
        // so the circuit breaker does not see a new run.
        assert_eq!(tokens(&result(&mut norm, 10, 40)), Some((20, 100, 200, 10)));

        // The next turn starts from there; a result that counts more (subagents) wins.
        norm.turn_started("again");
        start(&mut norm);
        assert_eq!(tokens(&delta(&mut norm, Value::Null, 7)), Some((30, 107, 300, 15)));
        assert_eq!(tokens(&result(&mut norm, 30, 50)), Some((50, 150, 300, 15)));
    }

    /// An actor on a child that reads its stdin and never answers: what Codly writes is in the
    /// raw log of `dir`, and no `result` ever comes.
    fn silent_actor(dir: &Path) -> (Actor, mpsc::UnboundedReceiver<AdapterEvent>, tokio::process::Child) {
        let env = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        let mut child = proc::command(Path::new("/bin/sh"), &["-c".into(), "cat > /dev/null".into()], dir, &env).spawn().unwrap();
        let (events, events_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            agent_id: "a".into(),
            run_id: "r".into(),
            events,
            stdin: child.stdin.take(),
            raw: Arc::new(RawLog::open(&dir.join("raw.ndjson")).unwrap()),
            pgid: 0, // nothing to signal: the child ends with the test
            stderr: StderrTail::default(),
            norm: ClaudeNormalizer::new(&spec(Role::Worker, false), "s".into()),
            queue: VecDeque::new(),
            turn_active: false,
            next_req: 0,
            stage: Stage::Running,
            deadline: None,
            start_failed: false,
            interrupt_failed: false,
            _mcp_file: None,
        };
        (actor, events_rx, child)
    }

    /// The interrupt requests Codly wrote to the CLI so far.
    fn interrupts_sent(dir: &Path) -> usize {
        let log = std::fs::read_to_string(dir.join("raw.ndjson")).unwrap();
        log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|l| l["codly_sent"]["request"]["subtype"] == "interrupt").count()
    }

    fn emitted(events: &mut mpsc::UnboundedReceiver<AdapterEvent>) -> Vec<EventKind> {
        std::iter::from_fn(|| events.try_recv().ok()).map(|e| e.kind).collect()
    }

    /// An attached image goes out in the user message as an image block before the text; one
    /// over the size limit stays a path, and the user is told before the turn starts.
    #[tokio::test]
    async fn an_attached_image_goes_out_as_an_image_block() {
        use crate::adapters::attachments::testing::{attach, message, png_of};
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let shot = attach(dir.path(), "shot.png", &png_of(300));
        let big = attach(dir.path(), "big.png", &png_of(4_000_000));
        let text = message("Make it look like this.", &[&shot, &big]);
        actor.on_cmd(AgentCmd::Prompt(text.clone())).await;

        let log = std::fs::read_to_string(dir.path().join("raw.ndjson")).unwrap();
        let sent: Vec<Value> = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["codly_sent"].clone()).filter(|l| l["type"] == "user").collect();
        // The raw log keeps the block without its bytes (`proc::without_image_data`); the
        // bytes themselves are compared with the recording in attachments.rs.
        let image = json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "<400 base64 characters left out>" } });
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["message"]["content"], json!([image, { "type": "text", "text": text }]));
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[0], EventKind::Error { message, fatal: false } if message.starts_with("big.png is over 3.5 MB")), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(k, EventKind::TurnStarted { input: Some(input) } if *input == text)), "the turn shows the text: {kinds:?}");
    }

    /// Stop always reaches the CLI, and a turn that does not end after it is not waited for
    /// without end. (The deadline is fired by hand: tokio's paused clock is not among the
    /// workspace's features.)
    #[tokio::test]
    async fn an_interrupt_that_gets_no_result_ends_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());

        // no turn runs as far as the adapter knows: the request still goes out, with no deadline
        actor.on_cmd(AgentCmd::Interrupt).await;
        assert_eq!(interrupts_sent(dir.path()), 1);
        assert!(actor.deadline.is_none() && !actor.norm.interrupt_requested);

        // a turn runs, and a message waits behind it
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("then test it".into())).await;
        assert!(actor.turn_active);
        actor.on_cmd(AgentCmd::Interrupt).await;
        assert_eq!(interrupts_sent(dir.path()), 2);
        assert!(actor.deadline.is_some() && actor.stage == Stage::Running && actor.norm.interrupt_requested);
        emitted(&mut events);

        // no `result` within the limit: Codly says so and ends the process
        actor.on_deadline().await;
        assert_eq!(actor.stage, Stage::Closing);
        assert!(actor.stdin.is_none() && actor.queue.is_empty());
        let kinds = emitted(&mut events);
        assert!(
            matches!(&kinds[..], [EventKind::Error { message, fatal: false }, EventKind::Decision { decision_type, .. }]
                if message.contains("did not stop within 8 s")
                    && message.contains("1 message that waited for this turn was not sent")
                    && decision_type == crate::adapters::GAVE_UP),
            "the exit that follows is marked as Codly's doing: {kinds:?}"
        );
        // The turn it leaves open ends as interrupted: a failed turn would be asked to continue.
        assert_eq!(actor.unfinished_turn(), (TurnStatus::Interrupted, None));
    }

    #[tokio::test]
    async fn an_interrupted_turn_that_ends_in_time_keeps_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("then test it".into())).await;
        actor.on_cmd(AgentCmd::Interrupt).await;
        assert!(actor.deadline.is_some());
        emitted(&mut events);

        let result = json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 });
        actor.on_json(result, "codly-0", &mut None).await;
        assert!(actor.deadline.is_none() && actor.stage == Stage::Running && actor.stdin.is_some());
        let kinds = emitted(&mut events);
        let ended = kinds.iter().position(|k| matches!(k, EventKind::TurnCompleted { status: TurnStatus::Interrupted, .. }));
        let next = kinds.iter().position(|k| matches!(k, EventKind::TurnStarted { input: Some(text) } if text == "then test it"));
        assert!(ended.is_some() && ended < next, "the message that waited is the next turn: {kinds:?}");
        assert!(actor.turn_active && actor.queue.is_empty());
        // No interrupt is pending any more: an exit now is a failed turn.
        assert_eq!(actor.unfinished_turn().0, TurnStatus::Failed);
    }
}
