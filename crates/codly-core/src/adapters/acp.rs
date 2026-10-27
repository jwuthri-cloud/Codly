//! Generic ACP client (Agent Client Protocol v1: JSON-RPC 2.0, one JSON object per line on
//! stdio), one process per agent: Gemini `gemini --acp`, OpenCode `opencode acp`, Cursor
//! `cursor-agent acp`, Copilot `copilot --acp` (cli-others-acp §1, §11).
//!
//! Own tokio transport and plain serde_json instead of the SDK or schema crates: the SDK
//! broke its API three times in four months and has a documented handler deadlock; Codly
//! needs the raw line tap anyway (CRITIQUE C4).
//!
//! - fs and terminal client capabilities are off: agents work on their worktree directly
//!   (Gemini's fs delegation failed live with ENOENT, cli-others-acp §0.4). Live command
//!   output comes from the `_meta.terminal_output_delta` extension where the agent has it.
//! - The static system prompt is prepended to the first prompt of a new session.
//! - Prompts are queued Codly-side while a turn runs (ACP v1 cannot steer): one
//!   `AgentCmd::Prompt` = one turn.
//! - OpenCode allows edits and shell without asking by default; Codly forces "ask" through
//!   `OPENCODE_CONFIG_CONTENT`. Gemini gets a policy file that keeps workers out of its own
//!   memory/settings files (memory-knowledge §5.6).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use super::classify::{self, OUTPUT_MAX};
use super::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use crate::model::{
    ActivityKind, AgentStatus, Approval, ApprovalChoice, ApprovalDecision, ApprovalStatus, ApprovalSubject, EventKind, Id,
    MessagePhase, PlanStep, Provider, TurnStatus, Usage, new_id, now_ms,
};
use crate::proc::{self, Inbound, RawLog, StderrTail};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Keeps Gemini workers out of its own memory and settings (`~/.gemini/GEMINI.md`, the
/// private MEMORY.md index); 0.62 has no `save_memory` tool any more, older versions do.
pub const GEMINI_POLICY: &str = r#"# Written by Codly for one agent run.
[[rule]]
toolName = "save_memory"
decision = "deny"
priority = 900
denyMessage = "Codly agents record memories with the codly_memory_write tool."

[[rule]]
toolName = "write_file"
argsPattern = '\.gemini/'
decision = "deny"
priority = 900
denyMessage = "Codly agents must not edit Gemini's own settings or memory files."

[[rule]]
toolName = "replace"
argsPattern = '\.gemini/'
decision = "deny"
priority = 900
denyMessage = "Codly agents must not edit Gemini's own settings or memory files."
"#;

/// argv for the agent (after the binary), plus extra env.
pub fn launch(spec: &SpawnSpec, policy_path: Option<&Path>) -> (Vec<String>, Vec<(String, String)>) {
    let (model, _) = classify::split_model_effort(spec.model.as_deref());
    match spec.provider {
        Provider::Gemini => {
            let mut a = vec!["--acp".to_string(), "--skip-trust".to_string()];
            if let Some(m) = model {
                a.extend(["-m".into(), m]);
            }
            if let Some(p) = policy_path {
                a.extend(["--policy".into(), p.to_string_lossy().into_owned()]);
            }
            (a, Vec::new())
        }
        Provider::Opencode => {
            let permission = if spec.read_only {
                json!({ "edit": "deny", "bash": "deny", "webfetch": "ask" })
            } else {
                json!({ "edit": "ask", "bash": "ask", "webfetch": "ask" })
            };
            (vec!["acp".into()], vec![("OPENCODE_CONFIG_CONTENT".into(), json!({ "permission": permission }).to_string())])
        }
        Provider::Cursor => (vec!["acp".into()], Vec::new()),
        Provider::Copilot => (vec!["--acp".into()], Vec::new()),
        Provider::Claude | Provider::Codex => (Vec::new(), Vec::new()),
    }
}

/// The codly stdio MCP server in ACP's shape (env as `[{name, value}]`).
pub fn mcp_servers(spec: &SpawnSpec) -> Value {
    match &spec.mcp {
        Some(m) => {
            let mut servers = vec![json!({
                "name": "codly", "command": m.command, "args": m.args,
                "env": m.env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>(),
            })];
            // The user's default servers (Settings → Connections), as enabled when this run started.
            for x in crate::housekeeping::connections::load_ready(&spec.raw_log) {
                servers.push(json!({
                    "name": x.name, "command": x.command, "args": x.args,
                    "env": x.env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>(),
                }));
            }
            Value::Array(servers)
        }
        None => json!([]),
    }
}

pub async fn spawn(spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> anyhow::Result<AgentHandle> {
    let raw = Arc::new(RawLog::open(&spec.raw_log).with_context(|| format!("cannot open raw log {}", spec.raw_log.display()))?);
    let policy = if spec.provider == Provider::Gemini {
        let path = spec.raw_log.with_file_name("gemini-policy.toml");
        std::fs::write(&path, GEMINI_POLICY).with_context(|| format!("cannot write {}", path.display()))?;
        Some(path)
    } else {
        None
    };
    let (args, extra_env) = launch(&spec, policy.as_deref());
    let mut env = spec.env.clone();
    env.extend(extra_env);
    let mut child = proc::command(&spec.bin, &args, &spec.cwd, &env)
        .spawn()
        .with_context(|| format!("failed to start {}", spec.bin.display()))?;
    let pgid = child.id().map(|p| p as i32).unwrap_or(0);
    let stdin = child.stdin.take().context("no stdin")?;
    let lines = proc::spawn_json_reader(child.stdout.take().context("no stdout")?, raw.clone());
    let stderr = StderrTail::spawn(child.stderr.take().context("no stderr")?, 64 << 10);
    let (exit_tx, exit_rx) = oneshot::channel();
    tokio::spawn(async move {
        let code = child.wait().await.ok().and_then(proc::exit_code);
        let _ = exit_tx.send(code);
    });

    let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
    let (tx, cmds) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let actor = Actor {
        norm: AcpNormalizer::new(spec.provider, &spec.agent_id, &spec.run_id, spec.cwd.clone()),
        // Kept for a resume too: dropped once the session is resumed, sent if it falls back to new.
        system_prompt: Some(spec.system_prompt.clone()).filter(|s| !s.trim().is_empty()),
        spec,
        events,
        stdin: Some(stdin),
        raw,
        pgid,
        stderr,
        next_id: 0,
        session_id: None,
        pending: HashMap::new(),
        prompt_in_flight: false,
        queue: VecDeque::new(),
        replaying: false,
        stage: Stage::Running,
        deadline: None,
    };
    tokio::spawn(actor.run(cmds, lines, exit_rx, ready_tx));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT + Duration::from_secs(5), ready_rx).await {
        Ok(Ok(Ok(()))) => Ok(AgentHandle { agent_id, run_id, tx }),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) => Err(anyhow!("ACP adapter stopped during startup")),
        Err(_) => {
            let _ = tx.send(AgentCmd::Shutdown);
            Err(anyhow!("the agent did not finish the ACP handshake"))
        }
    }
}

enum Pending {
    Prompt,
    /// best-effort config calls; errors become non-fatal Error events
    Config(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Running,
    Interrupting,
    Closing,
    Killing,
}

struct Actor {
    spec: SpawnSpec,
    events: mpsc::UnboundedSender<AdapterEvent>,
    stdin: Option<ChildStdin>,
    raw: Arc<RawLog>,
    pgid: i32,
    stderr: StderrTail,
    norm: AcpNormalizer,
    /// Prepended to the first prompt of a new session.
    system_prompt: Option<String>,
    next_id: i64,
    session_id: Option<String>,
    pending: HashMap<i64, Pending>,
    prompt_in_flight: bool,
    queue: VecDeque<String>,
    /// `session/load` replays history as updates: keep them out of the live stream.
    replaying: bool,
    stage: Stage,
    deadline: Option<tokio::time::Instant>,
}

type Ready = Option<oneshot::Sender<anyhow::Result<()>>>;

impl Actor {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.spec.agent_id.clone(), run_id: self.spec.run_id.clone(), kind });
    }

    fn emit_all(&self, kinds: Vec<EventKind>) {
        for k in kinds {
            self.emit(k);
        }
    }

    async fn write(&mut self, mut msg: Value) -> bool {
        msg["jsonrpc"] = json!("2.0");
        let Some(stdin) = self.stdin.as_mut() else { return false };
        match proc::write_json(stdin, &self.raw, &msg).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("acp stdin write failed: {e}");
                self.stdin = None;
                false
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({ "id": id, "method": method, "params": params })).await;
        id
    }

    /// Handshake-time request: send, then pump lines until its response arrives (other lines
    /// are handled normally). No `AgentCmd` can arrive yet: the handle is not returned.
    async fn call(&mut self, method: &str, params: Value, lines: &mut mpsc::UnboundedReceiver<Inbound>) -> anyhow::Result<Result<Value, Value>> {
        let id = self.request(method, params).await;
        let deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            let next = tokio::time::timeout_at(deadline, lines.recv()).await.map_err(|_| anyhow!("no answer to {method} within {} s", HANDSHAKE_TIMEOUT.as_secs()))?;
            match next {
                Some(Inbound::Json(v)) => {
                    if v.get("method").is_none() && v["id"].as_i64() == Some(id) {
                        return Ok(match v.get("error") {
                            Some(e) if !e.is_null() => Err(e.clone()),
                            _ => Ok(v["result"].clone()),
                        });
                    }
                    self.on_json(v).await;
                }
                Some(Inbound::Text(_)) | Some(Inbound::TooLong(_)) => {}
                Some(Inbound::Eof) | None => {
                    let tail = self.stderr.tail(1500);
                    return Err(anyhow!("the agent exited during {method}{}", if tail.is_empty() { String::new() } else { format!(": {tail}") }));
                }
            }
        }
    }

    async fn handshake(&mut self, lines: &mut mpsc::UnboundedReceiver<Inbound>) -> anyhow::Result<()> {
        let init = json!({
            "protocolVersion": 1,
            "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false, "_meta": { "terminal_output_delta": true } },
            "clientInfo": { "name": "codly", "title": "Codly", "version": crate::version() },
        });
        let init = self.call("initialize", init, lines).await?.map_err(|e| anyhow!("initialize failed: {}", err_msg(&e)))?;
        let caps = &init["agentCapabilities"];
        let cwd = self.spec.cwd.to_string_lossy().into_owned();
        let mcp = mcp_servers(&self.spec);
        let session = match self.spec.session.clone() {
            SessionMode::Resume { session_id } if !caps["sessionCapabilities"]["resume"].is_null() => {
                let r = self.call("session/resume", json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": mcp }), lines).await?;
                if r.is_ok() {
                    self.system_prompt = None; // the resumed conversation already has it
                }
                r.map(|mut v| {
                    v["sessionId"] = json!(session_id);
                    v
                })
            }
            SessionMode::Resume { session_id } if caps["loadSession"] == true => {
                self.replaying = true;
                let r = self.call("session/load", json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": mcp }), lines).await?;
                self.replaying = false;
                if r.is_ok() {
                    self.system_prompt = None;
                }
                r.map(|mut v| {
                    v["sessionId"] = json!(session_id);
                    v
                })
            }
            SessionMode::Resume { .. } => {
                self.emit(EventKind::Error { message: "this agent cannot resume sessions over ACP; starting a new one".into(), fatal: false });
                self.call("session/new", json!({ "cwd": cwd, "mcpServers": mcp }), lines).await?
            }
            SessionMode::New { .. } => self.call("session/new", json!({ "cwd": cwd, "mcpServers": mcp }), lines).await?,
        };
        let session = session.map_err(|e| {
            if e["code"] == -32000 {
                let methods: Vec<&str> = init["authMethods"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).collect();
                anyhow!("not signed in ({}): sign in with the CLI in a terminal first", methods.join(", "))
            } else {
                anyhow!("could not open a session: {}", err_msg(&e))
            }
        })?;
        let sid = session["sessionId"].as_str().context("the agent returned no sessionId")?.to_string();
        self.session_id = Some(sid.clone());

        let mut model = None;
        if let Some(want) = classify::split_model_effort(self.spec.model.as_deref()).0 {
            model = Some(want.clone());
            if self.spec.provider != Provider::Gemini {
                // Gemini takes `-m` at launch; others pick from their config options
                if let Some((config_id, value)) = pick_option(&session["configOptions"], "model", &want) {
                    let id = self.request("session/set_config_option", json!({ "sessionId": sid, "configId": config_id, "value": value })).await;
                    self.pending.insert(id, Pending::Config("model"));
                    model = Some(value);
                } else if session["models"].is_object() {
                    let id = self.request("session/set_model", json!({ "sessionId": sid, "modelId": want })).await;
                    self.pending.insert(id, Pending::Config("model"));
                } else {
                    self.emit(EventKind::Error { message: format!("model {want} is not offered by this agent"), fatal: false });
                }
            }
        }
        if self.spec.read_only {
            if let Some((config_id, value)) = pick_option(&session["configOptions"], "mode", "plan") {
                let id = self.request("session/set_config_option", json!({ "sessionId": sid, "configId": config_id, "value": value })).await;
                self.pending.insert(id, Pending::Config("mode"));
            } else if session["modes"]["availableModes"].as_array().is_some_and(|m| m.iter().any(|x| x["id"] == "plan")) {
                let id = self.request("session/set_mode", json!({ "sessionId": sid, "modeId": "plan" })).await;
                self.pending.insert(id, Pending::Config("mode"));
            }
        }
        let current = session["configOptions"]
            .as_array()
            .and_then(|o| o.iter().find(|x| x["category"] == "model" || x["id"] == "model"))
            .and_then(|o| o["currentValue"].as_str())
            .or(session["models"]["currentModelId"].as_str())
            .map(str::to_owned);
        self.emit(EventKind::SessionStarted {
            provider: self.spec.provider,
            model: model.or(current),
            cwd,
            cli_version: init["agentInfo"]["version"].as_str().map(str::to_owned),
            session_id: Some(sid),
            billing: crate::env::billing_hint(self.spec.provider, &self.spec.env),
            tools: Vec::new(),
        });
        Ok(())
    }

    async fn run(
        mut self,
        mut cmds: mpsc::UnboundedReceiver<AgentCmd>,
        mut lines: mpsc::UnboundedReceiver<Inbound>,
        mut exit_rx: oneshot::Receiver<Option<i64>>,
        ready: oneshot::Sender<anyhow::Result<()>>,
    ) {
        let mut ready: Ready = Some(ready);
        match self.handshake(&mut lines).await {
            Ok(()) => {
                if let Some(r) = ready.take() {
                    let _ = r.send(Ok(()));
                }
            }
            Err(e) => {
                if let Some(r) = ready.take() {
                    let _ = r.send(Err(e));
                }
                self.stdin = None;
                if tokio::time::timeout(Duration::from_secs(3), &mut exit_rx).await.is_err() {
                    proc::kill_group(self.pgid, Duration::from_secs(2)).await;
                }
                return;
            }
        }
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
                    Some(Inbound::Json(v)) => self.on_json(v).await,
                    Some(Inbound::Text(_)) => self.emit(EventKind::Unknown { raw_type: "non_json".into() }),
                    Some(Inbound::TooLong(n)) => self.emit(EventKind::Error { message: format!("dropped a {n}-byte line from the agent"), fatal: false }),
                    Some(Inbound::Eof) | None => break,
                },
                cmd = cmds.recv(), if cmds_open => match cmd {
                    Some(cmd) => self.on_cmd(cmd).await,
                    None => { cmds_open = false; self.begin_shutdown().await }
                },
                _ = timer => self.on_deadline().await,
            }
        }
        let code = match tokio::time::timeout(Duration::from_secs(5), &mut exit_rx).await {
            Ok(Ok(code)) => code,
            _ => {
                proc::kill_group(self.pgid, Duration::from_secs(1)).await;
                exit_rx.await.ok().flatten()
            }
        };
        let cancelled = self.norm.cancel_all_approvals();
        self.emit_all(cancelled);
        if self.prompt_in_flight {
            self.emit(EventKind::TurnCompleted { status: TurnStatus::Failed, result_text: Some("the agent exited before the turn finished".into()), duration_ms: None });
        }
        if self.stage == Stage::Running {
            let tail = self.stderr.tail(2000);
            let detail = if tail.is_empty() { String::new() } else { format!(": {tail}") };
            self.emit(EventKind::Error { message: format!("{} exited unexpectedly (code {code:?}){detail}", self.spec.provider.label()), fatal: true });
        }
        self.emit(EventKind::StatusChanged { status: AgentStatus::Offline });
        self.emit(EventKind::Exited { code });
    }

    async fn on_json(&mut self, v: Value) {
        let method = v.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = v.get("id").filter(|i| !i.is_null()).cloned();
        match (method, id) {
            // a response to one of our requests
            (None, Some(id)) => {
                let result = match v.get("error") {
                    Some(e) if !e.is_null() => Err(e.clone()),
                    _ => Ok(v["result"].clone()),
                };
                match id.as_i64().and_then(|i| self.pending.remove(&i)) {
                    Some(Pending::Prompt) => {
                        self.prompt_in_flight = false;
                        let events = match &result {
                            Ok(r) => self.norm.turn_ended(r),
                            Err(e) => self.norm.turn_failed(&err_msg(e)),
                        };
                        self.emit_all(events);
                        if self.stage == Stage::Interrupting {
                            self.close_stdin();
                        } else if self.stage == Stage::Running
                            && let Some(next) = self.queue.pop_front()
                        {
                            self.send_prompt(next).await;
                        }
                    }
                    Some(Pending::Config(what)) => {
                        if let Err(e) = result {
                            self.emit(EventKind::Error { message: format!("could not set the {what}: {}", err_msg(&e)), fatal: false });
                        }
                    }
                    None => {}
                }
            }
            // a request from the agent
            (Some(method), Some(id)) => {
                if method == "session/request_permission" {
                    let events = self.norm.on_permission(&id, &v["params"]);
                    self.emit_all(events);
                } else {
                    // fs/* and terminal/* were not advertised; vendor requests are not supported yet.
                    // ponytail: answer cursor/ask_question and cursor/create_plan once Cursor is installable here.
                    self.emit(EventKind::Unknown { raw_type: method.clone() });
                    self.write(json!({ "id": id, "error": { "code": -32601, "message": format!("Codly does not handle {method}") } })).await;
                }
            }
            (Some(method), None) => {
                if method == "session/update" {
                    if !self.replaying {
                        let events = self.norm.on_update(&v["params"]["update"]);
                        self.emit_all(events);
                    }
                } else {
                    self.emit(EventKind::Unknown { raw_type: method });
                }
            }
            (None, None) => self.emit(EventKind::Unknown { raw_type: "untyped".into() }),
        }
    }

    async fn on_cmd(&mut self, cmd: AgentCmd) {
        match cmd {
            AgentCmd::Prompt(text) => {
                if self.stage != Stage::Running {
                    return;
                }
                if self.prompt_in_flight {
                    self.queue.push_back(text);
                } else {
                    self.send_prompt(text).await;
                }
            }
            AgentCmd::Interrupt => self.cancel_turn().await,
            AgentCmd::Approval { approval_id, decision } => match self.norm.answer(&approval_id, &decision) {
                Some((reply, events, cancel_turn)) => {
                    self.write(reply).await;
                    self.emit_all(events);
                    if cancel_turn {
                        self.cancel_turn().await;
                    }
                }
                None => tracing::warn!("acp: no pending approval {approval_id}"),
            },
            AgentCmd::Shutdown => self.begin_shutdown().await,
        }
    }

    async fn send_prompt(&mut self, text: String) {
        let Some(sid) = self.session_id.clone() else { return };
        let full = match self.system_prompt.take() {
            Some(sys) => format!("<codly_instructions>\n{sys}\n</codly_instructions>\n\n{text}"),
            None => text.clone(),
        };
        let id = self.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": full }] })).await;
        self.pending.insert(id, Pending::Prompt);
        self.prompt_in_flight = true;
        let events = self.norm.turn_started(&text);
        self.emit_all(events);
    }

    /// `session/cancel` (a notification) and answer every open permission request with
    /// `cancelled`, as the spec requires; the prompt then returns `stopReason: cancelled`.
    async fn cancel_turn(&mut self) {
        if !self.prompt_in_flight {
            return;
        }
        self.norm.interrupt_requested = true;
        if let Some(sid) = self.session_id.clone() {
            self.write(json!({ "method": "session/cancel", "params": { "sessionId": sid } })).await;
        }
        for (reply, event) in self.norm.cancel_permissions() {
            self.write(reply).await;
            self.emit(event);
        }
    }

    async fn begin_shutdown(&mut self) {
        if self.stage != Stage::Running {
            return;
        }
        self.queue.clear();
        if self.prompt_in_flight {
            self.cancel_turn().await;
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
                tokio::spawn(proc::kill_group(self.pgid, Duration::from_secs(3)));
            }
            Stage::Running | Stage::Killing => self.deadline = None,
        }
    }
}

fn err_msg(e: &Value) -> String {
    e["message"].as_str().map(str::to_owned).unwrap_or_else(|| e.to_string())
}

/// (configId, value) of the `category` select option matching `want` (exact, or as the
/// last path segment: "big-pickle" matches "opencode/big-pickle").
fn pick_option(options: &Value, category: &str, want: &str) -> Option<(String, String)> {
    let opt = options.as_array()?.iter().find(|o| o["category"] == category || o["id"] == category)?;
    let mut values = Vec::new();
    for o in opt["options"].as_array().into_iter().flatten() {
        // options may be grouped: {group, options: [...]}
        match o["options"].as_array() {
            Some(group) => values.extend(group.iter().filter_map(|g| g["value"].as_str())),
            None => values.extend(o["value"].as_str()),
        }
    }
    let value = values.iter().find(|v| **v == want).or_else(|| values.iter().find(|v| v.ends_with(&format!("/{want}"))))?;
    Some((opt["id"].as_str()?.to_string(), value.to_string()))
}

// ───────────────────────────── normalization (pure) ─────────────────────────────

struct ToolState {
    /// The merged ToolCall: `tool_call_update` fields replace (ACP §1.6).
    call: Value,
    /// The first title, before agents replace it with a human one (OpenCode: "bash").
    first_title: Option<String>,
    started: Option<(Instant, ActivityKind)>,
}

struct PendingPermission {
    request_id: Value,
    options: Vec<(ApprovalChoice, String)>,
}

/// Turns ACP `session/update`s, permission requests and prompt results into `EventKind`s.
pub struct AcpNormalizer {
    provider: Provider,
    agent_id: Id,
    run_id: Id,
    cwd: PathBuf,
    status: Option<AgentStatus>,
    /// (item id, text so far) of the agent message / thought being streamed
    message: Option<(String, String)>,
    thought: Option<(String, String)>,
    seq: u64,
    last_text: Option<String>,
    tools: HashMap<String, ToolState>,
    approvals: HashMap<Id, PendingPermission>,
    usage: Usage,
    turn_started_at: Option<Instant>,
    pub interrupt_requested: bool,
}

impl AcpNormalizer {
    pub fn new(provider: Provider, agent_id: &str, run_id: &str, cwd: PathBuf) -> Self {
        AcpNormalizer {
            provider,
            agent_id: agent_id.into(),
            run_id: run_id.into(),
            cwd,
            status: None,
            message: None,
            thought: None,
            seq: 0,
            last_text: None,
            tools: HashMap::new(),
            approvals: HashMap::new(),
            usage: Usage::default(),
            turn_started_at: None,
            interrupt_requested: false,
        }
    }

    fn set_status(&mut self, s: AgentStatus, out: &mut Vec<EventKind>) {
        if self.status != Some(s) {
            self.status = Some(s);
            out.push(EventKind::StatusChanged { status: s });
        }
    }

    fn busy_status(&self) -> AgentStatus {
        if !self.approvals.is_empty() {
            AgentStatus::WaitingApproval
        } else if self.tools.values().any(|t| t.started.is_some()) {
            AgentStatus::Working
        } else {
            AgentStatus::Thinking
        }
    }

    pub fn turn_started(&mut self, text: &str) -> Vec<EventKind> {
        let mut out = vec![EventKind::TurnStarted { input: Some(text.to_string()) }];
        self.turn_started_at = Some(Instant::now());
        self.interrupt_requested = false;
        self.last_text = None;
        self.set_status(AgentStatus::Thinking, &mut out);
        out
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}-{}", self.seq)
    }

    fn flush_message(&mut self, phase: MessagePhase, out: &mut Vec<EventKind>) {
        if let Some((item_id, text)) = self.message.take()
            && !text.trim().is_empty()
        {
            if phase != MessagePhase::Commentary {
                self.last_text = Some(text.trim().to_string());
            }
            out.push(EventKind::Message { item_id, text, phase });
        }
    }

    fn flush_thought(&mut self, out: &mut Vec<EventKind>) {
        if let Some((item_id, text)) = self.thought.take()
            && !text.trim().is_empty()
        {
            out.push(EventKind::Reasoning { item_id, text });
        }
    }

    pub fn on_update(&mut self, u: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        match u["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" => {
                let text = content_text(&u["content"]);
                let mid = u["messageId"].as_str();
                if self.message.as_ref().is_some_and(|(id, _)| mid.is_some_and(|m| m != id)) {
                    self.flush_message(MessagePhase::Commentary, &mut out);
                }
                self.flush_thought(&mut out);
                if self.message.is_none() {
                    let id = mid.map(str::to_owned).unwrap_or_else(|| self.next_id("msg"));
                    self.message = Some((id, String::new()));
                }
                let (id, buf) = self.message.as_mut().unwrap();
                buf.push_str(&text);
                let item_id = id.clone();
                out.push(EventKind::TextDelta { item_id, text });
                let st = self.busy_status();
                self.set_status(st, &mut out);
            }
            "agent_thought_chunk" => {
                let text = content_text(&u["content"]);
                if self.thought.is_none() {
                    let id = self.next_id("thought");
                    self.thought = Some((id, String::new()));
                }
                let (id, buf) = self.thought.as_mut().unwrap();
                buf.push_str(&text);
                let item_id = id.clone();
                out.push(EventKind::ReasoningDelta { item_id, text });
                let st = self.busy_status();
                self.set_status(st, &mut out);
            }
            "tool_call" | "tool_call_update" => self.tool_update(u, &mut out),
            "plan" => {
                let steps = u["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| PlanStep { text: e["content"].as_str().unwrap_or("").into(), status: e["status"].as_str().unwrap_or("pending").into() })
                    .collect();
                out.push(EventKind::PlanUpdated { steps });
            }
            "usage_update" => {
                self.usage.context_used = u["used"].as_i64().or(self.usage.context_used);
                self.usage.context_window = u["size"].as_i64().or(self.usage.context_window);
                if u["cost"]["currency"] == "USD" {
                    self.usage.cost_usd = u["cost"]["amount"].as_f64();
                }
                out.push(EventKind::UsageUpdated { usage: self.usage.clone() });
            }
            "compaction_update" => out.push(EventKind::Compacting { pre_tokens: None, post_tokens: None }),
            "user_message_chunk" | "available_commands_update" | "current_mode_update" | "config_option_update" | "session_info_update"
            | "compaction_summary_chunk" => {}
            other => out.push(EventKind::Unknown { raw_type: format!("session/update:{other}") }),
        }
        out
    }

    fn tool_update(&mut self, u: &Value, out: &mut Vec<EventKind>) {
        let Some(id) = u["toolCallId"].as_str().map(str::to_owned) else { return };
        let entry = self.tools.entry(id.clone()).or_insert_with(|| ToolState { call: json!({}), first_title: u["title"].as_str().map(str::to_owned), started: None });
        for key in ["title", "kind", "status", "locations", "rawInput", "rawOutput", "content", "name", "_meta"] {
            if let Some(v) = u.get(key).filter(|v| !v.is_null()) {
                entry.call[key] = v.clone();
            }
        }
        // live command output (Zed's `_meta` terminal extension, opted into at initialize)
        let delta = u["_meta"]["terminal_output_delta"]["data"].as_str().or(u["_meta"]["terminal_output"]["data"].as_str());
        if let Some(chunk) = delta {
            out.push(EventKind::ToolOutput { call_id: id.clone(), chunk: chunk.to_string() });
        }
        let status = entry.call["status"].as_str().unwrap_or("pending").to_string();
        // OpenCode first announces an empty pending call (a shell call only knows its cwd);
        // wait until the input says what it is.
        let has_input = if entry.call["kind"] == "execute" {
            entry.call["rawInput"]["command"].is_string()
        } else {
            ["rawInput", "locations", "content"].iter().any(|k| match &entry.call[*k] {
                Value::Object(o) => !o.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => false,
            })
        };
        if entry.started.is_none() && (status != "pending" || has_input) {
            self.start_tool(&id, out);
        }
        if matches!(status.as_str(), "completed" | "failed") {
            if self.tools.get(&id).is_some_and(|t| t.started.is_none()) {
                self.start_tool(&id, out);
            }
            self.complete_tool(&id, status == "completed", out);
        }
    }

    fn start_tool(&mut self, id: &str, out: &mut Vec<EventKind>) {
        self.flush_thought(out);
        self.flush_message(MessagePhase::Commentary, out);
        let Some(t) = self.tools.get_mut(id) else { return };
        let kind = t.call["kind"].as_str().unwrap_or("other").to_string();
        let mut view = t.call.clone();
        // MCP and other tools: classify by their programmatic name, not the display title
        if view["name"].is_null()
            && let Some(n) = tool_name(id, t.first_title.as_deref())
        {
            view["name"] = json!(n);
        }
        let (activity, target, confidence) = classify::classify_tool(self.provider, &kind, &view);
        t.started = Some((Instant::now(), activity));
        let name = view["name"].as_str().unwrap_or(&kind).to_string();
        let input = t.call.get("rawInput").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({ "title": t.call["title"] }));
        out.push(EventKind::ToolStarted { call_id: id.to_string(), name, activity, target, input, confidence: confidence.into() });
        let st = self.busy_status();
        self.set_status(st, out);
    }

    fn complete_tool(&mut self, id: &str, ok: bool, out: &mut Vec<EventKind>) {
        let Some(t) = self.tools.remove(id) else { return };
        let call = &t.call;
        let mut text: Vec<String> = call["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["type"] == "content")
            .map(|c| content_text(&c["content"]))
            .filter(|s| !s.is_empty())
            .collect();
        if text.is_empty()
            && let Some(o) = call["rawOutput"]["output"].as_str()
        {
            text.push(o.to_string());
        }
        let output = (!text.is_empty()).then(|| classify::truncate_output(&text.join("\n"), OUTPUT_MAX));
        let exit_code = call["rawOutput"]["metadata"]["exit"].as_i64().or(call["_meta"]["terminal_exit"]["exit_code"].as_i64());
        let (started, activity) = t.started.map_or((None, ActivityKind::Execute), |(s, a)| (Some(s), a));
        let tests = match &output {
            Some(o) if activity == ActivityKind::Test => classify::parse_test_counts(o),
            _ => None,
        };
        out.push(EventKind::ToolCompleted {
            call_id: id.to_string(),
            ok,
            output,
            exit_code,
            duration_ms: started.map(|s| s.elapsed().as_millis() as i64),
            tests_passed: tests.map(|t| t.0),
            tests_failed: tests.map(|t| t.1),
        });
        if ok {
            let kind = call["kind"].as_str().unwrap_or("");
            let mut paths = diff_paths(call);
            if paths.is_empty() && matches!(kind, "edit" | "delete" | "move") {
                paths = locations(call);
            }
            if !paths.is_empty() {
                out.push(EventKind::FilesChanged { call_id: Some(id.to_string()), paths, source: "provider".into() });
            }
        }
        let st = self.busy_status();
        self.set_status(st, out);
    }

    /// `session/request_permission` → an approval card; choices by option `kind`, never
    /// by `optionId` (ids are agent-specific: Gemini `proceed_once`, Cursor `allow-once`).
    pub fn on_permission(&mut self, request_id: &Value, p: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        let call = &p["toolCall"];
        let call_id = call["toolCallId"].as_str().unwrap_or("").to_string();
        if !call_id.is_empty() {
            self.tool_update(call, &mut out);
            if self.tools.get(&call_id).is_some_and(|t| t.started.is_none()) {
                self.start_tool(&call_id, &mut out); // asking about it means it is about to run
            }
        }
        let merged = self.tools.get(&call_id).map(|t| t.call.clone()).unwrap_or_else(|| call.clone());
        let kind = merged["kind"].as_str().unwrap_or("other");
        let title = merged["title"].as_str().unwrap_or("").to_string();
        let mut risks = Vec::new();
        let subject = match kind {
            "execute" => {
                let command = merged["rawInput"]["command"].as_str().map(str::to_owned).unwrap_or_else(|| title.clone());
                let cwd = merged["rawInput"]["cwd"].as_str().map(PathBuf::from).unwrap_or_else(|| self.cwd.clone());
                risks = classify::risk_hints(&command, Some(&cwd), &self.cwd);
                ApprovalSubject::Command { command, cwd: Some(cwd.to_string_lossy().into_owned()) }
            }
            "edit" | "delete" | "move" => {
                let mut paths = diff_paths(&merged);
                for l in locations(&merged) {
                    if !paths.contains(&l) {
                        paths.push(l);
                    }
                }
                if paths.iter().any(|x| classify::outside(x, &self.cwd, &self.cwd)) {
                    risks.push("outside worktree".into());
                }
                ApprovalSubject::FileChange { paths, diff: render_diffs(&merged) }
            }
            _ => ApprovalSubject::Tool { name: if title.is_empty() { kind.to_string() } else { title.clone() }, input: merged["rawInput"].clone() },
        };
        let mut options: Vec<(ApprovalChoice, String)> = Vec::new();
        for o in p["options"].as_array().into_iter().flatten() {
            let Some(oid) = o["optionId"].as_str() else { continue };
            let name = o["name"].as_str().unwrap_or("").to_lowercase();
            let choice = match o["kind"].as_str() {
                Some("allow_once") => ApprovalChoice::AllowOnce,
                // Gemini labels its allow_always "Allow for this session"
                Some("allow_always") if name.contains("session") => ApprovalChoice::AllowSession,
                Some("allow_always") => ApprovalChoice::AllowAlways,
                Some("reject_once") | Some("reject_always") => ApprovalChoice::Deny,
                _ => continue,
            };
            if !options.iter().any(|(c, _)| *c == choice) {
                options.push((choice, oid.to_string()));
            }
        }
        let mut choices: Vec<ApprovalChoice> = options.iter().map(|(c, _)| *c).collect();
        choices.push(ApprovalChoice::Cancel); // always possible: cancel the turn
        let approval = Approval {
            id: new_id(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            task_id: None,
            call_id: Some(call_id).filter(|c| !c.is_empty()),
            subject,
            reason: Some(title).filter(|t| !t.is_empty()),
            choices,
            risks,
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        self.approvals.insert(approval.id.clone(), PendingPermission { request_id: request_id.clone(), options });
        out.push(EventKind::ApprovalRequested { approval });
        self.set_status(AgentStatus::WaitingApproval, &mut out);
        out
    }

    /// (response, events, cancel the turn?) for a decision.
    pub fn answer(&mut self, approval_id: &str, d: &ApprovalDecision) -> Option<(Value, Vec<EventKind>, bool)> {
        let p = self.approvals.remove(approval_id)?;
        use ApprovalChoice::*;
        let order: &[ApprovalChoice] = match d.choice {
            AllowOnce => &[AllowOnce],
            AllowSession => &[AllowSession, AllowOnce],
            AllowAlways => &[AllowAlways, AllowSession, AllowOnce],
            Deny => &[Deny],
            Cancel => &[],
        };
        let option = order.iter().find_map(|c| p.options.iter().find(|(x, _)| x == c).map(|(_, id)| id.clone()));
        let outcome = match &option {
            Some(oid) => json!({ "outcome": "selected", "optionId": oid }),
            None => json!({ "outcome": "cancelled" }),
        };
        let mut out = Vec::new();
        let st = if option.is_none() { AgentStatus::Thinking } else { self.busy_status() };
        self.set_status(st, &mut out);
        Some((json!({ "id": p.request_id, "result": { "outcome": outcome } }), out, option.is_none()))
    }

    /// On `session/cancel`: answer every open permission request with `cancelled`.
    pub fn cancel_permissions(&mut self) -> Vec<(Value, EventKind)> {
        self.approvals
            .drain()
            .map(|(id, p)| {
                (
                    json!({ "id": p.request_id, "result": { "outcome": { "outcome": "cancelled" } } }),
                    EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() },
                )
            })
            .collect()
    }

    pub fn cancel_all_approvals(&mut self) -> Vec<EventKind> {
        self.cancel_permissions().into_iter().map(|(_, e)| e).collect()
    }

    /// The `session/prompt` response: the turn is over.
    pub fn turn_ended(&mut self, r: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        self.flush_thought(&mut out);
        self.flush_message(MessagePhase::Final, &mut out);
        // tokens: unstable `usage` (OpenCode) or Gemini's `_meta.quota.token_count`
        let (u, q) = (&r["usage"], &r["_meta"]["quota"]["token_count"]);
        if u.is_object() {
            self.usage.input_tokens += u["inputTokens"].as_i64().unwrap_or(0);
            self.usage.output_tokens += u["outputTokens"].as_i64().unwrap_or(0) + u["thoughtTokens"].as_i64().unwrap_or(0);
            self.usage.cache_read_tokens += u["cachedReadTokens"].as_i64().unwrap_or(0);
            self.usage.cache_write_tokens += u["cachedWriteTokens"].as_i64().unwrap_or(0);
            out.push(EventKind::UsageUpdated { usage: self.usage.clone() });
        } else if q.is_object() {
            self.usage.input_tokens += q["input_tokens"].as_i64().unwrap_or(0);
            self.usage.output_tokens += q["output_tokens"].as_i64().unwrap_or(0);
            out.push(EventKind::UsageUpdated { usage: self.usage.clone() });
        }
        let status = match r["stopReason"].as_str() {
            _ if self.interrupt_requested => TurnStatus::Interrupted,
            Some("end_turn") => TurnStatus::Completed,
            Some("cancelled") => TurnStatus::Interrupted,
            Some("max_turn_requests") => TurnStatus::MaxTurns,
            _ => TurnStatus::Failed,
        };
        self.close_turn(status, &mut out);
        out
    }

    pub fn turn_failed(&mut self, message: &str) -> Vec<EventKind> {
        let mut out = Vec::new();
        self.flush_thought(&mut out);
        self.flush_message(MessagePhase::Unknown, &mut out);
        out.push(EventKind::Error { message: message.to_string(), fatal: false });
        let status = if self.interrupt_requested { TurnStatus::Interrupted } else { TurnStatus::Failed };
        self.close_turn(status, &mut out);
        out
    }

    fn close_turn(&mut self, status: TurnStatus, out: &mut Vec<EventKind>) {
        out.extend(self.cancel_all_approvals());
        self.tools.clear();
        self.interrupt_requested = false;
        let duration_ms = self.turn_started_at.take().map(|s| s.elapsed().as_millis() as i64);
        out.push(EventKind::TurnCompleted { status, result_text: self.last_text.take(), duration_ms });
        self.set_status(AgentStatus::Idle, out);
    }
}

/// Programmatic tool name: Gemini ids are `read_file__read_file_1792…`, OpenCode's first
/// title is the tool ("bash", "write").
fn tool_name(id: &str, first_title: Option<&str>) -> Option<String> {
    if let Some((name, rest)) = id.split_once("__")
        && rest.starts_with(name)
    {
        return Some(name.to_string());
    }
    first_title.filter(|t| !t.contains(' ') && !t.contains('/') && !t.is_empty()).map(str::to_owned)
}

fn content_text(c: &Value) -> String {
    match c["type"].as_str() {
        Some("text") => c["text"].as_str().unwrap_or("").to_string(),
        Some("resource") => c["resource"]["text"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn locations(call: &Value) -> Vec<String> {
    call["locations"].as_array().into_iter().flatten().filter_map(|l| l["path"].as_str().map(str::to_owned)).collect()
}

fn diff_paths(call: &Value) -> Vec<String> {
    call["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "diff").filter_map(|c| c["path"].as_str().map(str::to_owned)).collect()
}

fn render_diffs(call: &Value) -> Option<String> {
    let parts: Vec<String> = call["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "diff")
        .map(|c| {
            let old = c["oldText"].as_str().unwrap_or("");
            let new = c["newText"].as_str().unwrap_or("");
            let mut s = format!("--- {}\n", c["path"].as_str().unwrap_or(""));
            for l in old.lines() {
                s.push_str(&format!("-{l}\n"));
            }
            for l in new.lines() {
                s.push_str(&format!("+{l}\n"));
            }
            s
        })
        .collect();
    (!parts.is_empty()).then(|| classify::truncate_output(&parts.join("\n"), OUTPUT_MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures").join(name);
        std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    /// Feed the agent→client side of an ACP probe capture through the normalizer, answering
    /// permissions with allow-once like the probe did. Returns events and the answers.
    fn replay(provider: Provider, name: &str) -> (Vec<EventKind>, Vec<Value>) {
        let mut norm = AcpNormalizer::new(provider, "a1", "r1", PathBuf::from("/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/fixture-repo2"));
        let mut events = norm.turn_started("prompt");
        let mut answers = Vec::new();
        let mut prompt_id = None;
        for line in fixture(name) {
            let msg = &line["msg"];
            match line["dir"].as_str() {
                Some("c2a") if msg["method"] == "session/prompt" => prompt_id = msg["id"].as_i64(),
                Some("a2c") => match (msg["method"].as_str(), msg.get("id")) {
                    (Some("session/update"), _) => events.extend(norm.on_update(&msg["params"]["update"])),
                    (Some("session/request_permission"), Some(id)) => {
                        let ev = norm.on_permission(id, &msg["params"]);
                        let a = ev.iter().find_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).unwrap();
                        events.extend(ev);
                        let (reply, ev, cancel) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
                        assert!(!cancel);
                        answers.push(reply);
                        events.extend(ev);
                    }
                    (None, Some(id)) if id.as_i64() == prompt_id => events.extend(norm.turn_ended(&msg["result"])),
                    _ => {}
                },
                _ => {}
            }
        }
        (events, answers)
    }

    fn tools(events: &[EventKind]) -> Vec<(String, String, ActivityKind)> {
        events.iter().filter_map(|e| if let EventKind::ToolStarted { call_id, name, activity, .. } = e { Some((call_id.clone(), name.clone(), *activity)) } else { None }).collect()
    }

    #[test]
    fn replays_gemini_read() {
        let (events, _) = replay(Provider::Gemini, "acp-gemini.jsonl");
        assert_eq!(tools(&events), vec![("read_file__read_file_1792964533840_0".into(), "read_file".into(), ActivityKind::Read)]);
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: true, .. })));
        assert!(events.iter().any(|e| matches!(e, EventKind::Reasoning { text, .. } if text.contains("README Analysis"))));
        assert!(events.iter().any(|e| matches!(e, EventKind::Message { text, phase: MessagePhase::Final, .. } if text == "DONE")));
        let usage = events.iter().find_map(|e| if let EventKind::UsageUpdated { usage } = e { Some(usage.clone()) } else { None }).unwrap();
        assert_eq!((usage.input_tokens, usage.output_tokens), (26988, 19));
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some(t), .. } if t == "DONE")));
        assert!(matches!(events.last(), Some(EventKind::StatusChanged { status: AgentStatus::Idle })));
    }

    #[test]
    fn replays_gemini_write_with_permissions() {
        let (events, answers) = replay(Provider::Gemini, "acp-gemini-write.jsonl");
        // two permission requests (touch, write_file), answered by option kind
        assert_eq!(answers.len(), 2);
        assert!(answers.iter().all(|a| a["result"]["outcome"] == json!({ "outcome": "selected", "optionId": "proceed_once" })));
        assert_eq!(answers[0]["id"], 4);
        let approvals: Vec<Approval> = events.iter().filter_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).collect();
        assert!(matches!(&approvals[0].subject, ApprovalSubject::Command { command, .. } if command == "touch gemini.txt"));
        assert_eq!(approvals[0].choices, vec![ApprovalChoice::AllowSession, ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        match &approvals[1].subject {
            ApprovalSubject::FileChange { paths, diff } => {
                assert!(paths[0].ends_with("fixture-repo2/gemini.txt"));
                assert!(diff.as_deref().unwrap().contains("+hi"));
            }
            other => panic!("{other:?}"),
        }
        let started = tools(&events);
        let kinds: Vec<ActivityKind> = started.iter().map(|t| t.2).collect();
        assert_eq!(kinds[0], ActivityKind::Thinking, "update_topic is Gemini's intent pseudo-tool");
        assert!(kinds.contains(&ActivityKind::Create) && kinds.contains(&ActivityKind::Read) && kinds.contains(&ActivityKind::Execute));
        assert!(started.iter().any(|t| t.1 == "run_shell_command"));
        // failed writes do not report changed files; the successful one does
        let changed: Vec<&EventKind> = events.iter().filter(|e| matches!(e, EventKind::FilesChanged { .. })).collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::ToolCompleted { ok: false, .. })).count(), 2);
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })));
    }

    #[test]
    fn replays_opencode_read_and_write() {
        let (events, answers) = replay(Provider::Opencode, "acp-opencode.jsonl");
        assert!(answers.is_empty());
        let t = tools(&events);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].1.as_str(), t[0].2), ("read", ActivityKind::Read));
        let started = events.iter().find_map(|e| if let EventKind::ToolStarted { target, input, .. } = e { Some((target.clone(), input.clone())) } else { None }).unwrap();
        assert!(started.0.path.as_deref().unwrap().ends_with("README.md"), "started once the input is known");
        assert!(started.1["filePath"].as_str().is_some());
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: true, output: Some(o), .. } if o.contains("Fixture Repo"))));
        let usage = events.iter().rev().find_map(|e| if let EventKind::UsageUpdated { usage } = e { Some(usage.clone()) } else { None }).unwrap();
        assert_eq!((usage.context_used, usage.context_window, usage.cost_usd), (Some(15040), Some(200000), Some(0.0)));
        assert_eq!((usage.input_tokens, usage.output_tokens, usage.cache_read_tokens), (14970, 10, 70));
        assert!(events.iter().any(|e| matches!(e, EventKind::Message { item_id, text, .. } if item_id == "msg_17503fc300017nnXj6DOEo626d" && text == "DONE")));

        let (events, _) = replay(Provider::Opencode, "acp-opencode-write.jsonl");
        let t = tools(&events);
        assert_eq!(t.iter().map(|x| (x.1.as_str(), x.2)).collect::<Vec<_>>(), vec![("write", ActivityKind::Edit), ("bash", ActivityKind::Read)]);
        assert!(events.iter().any(|e| matches!(e, EventKind::FilesChanged { paths, .. } if paths[0].ends_with("hello.txt"))));
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { exit_code: Some(0), output: Some(o), .. } if o.contains("hello.txt"))));
    }

    #[test]
    fn permissions_cancel_and_live_output() {
        let mut norm = AcpNormalizer::new(Provider::Cursor, "a", "r", PathBuf::from("/w"));
        norm.turn_started("go");
        let req = json!({ "sessionId": "s", "toolCall": { "toolCallId": "t1", "title": "rm -rf build", "kind": "execute", "status": "pending",
                          "rawInput": { "command": "rm -rf build" } },
                          "options": [{ "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                                      { "optionId": "allow-always", "name": "Always allow", "kind": "allow_always" },
                                      { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" }] });
        let ev = norm.on_permission(&json!(7), &req);
        let a = ev.iter().find_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).unwrap();
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowAlways, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        assert_eq!(a.risks, vec!["deletes files".to_string()]);
        assert!(ev.iter().any(|e| matches!(e, EventKind::ToolStarted { activity: ActivityKind::Execute, .. })));
        // AllowSession is not offered: falls back to allow once
        let (r, _, cancel) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowSession, message: None, answers: None }).unwrap();
        assert_eq!(r, json!({ "id": 7, "result": { "outcome": { "outcome": "selected", "optionId": "allow-once" } } }));
        assert!(!cancel);

        let ev = norm.on_update(&json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "in_progress",
                                          "_meta": { "terminal_output_delta": { "terminal_id": "x", "data": "removed\n" } } }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ToolOutput { chunk, .. } if chunk == "removed\n")));

        let ev = norm.on_permission(&json!(8), &req);
        let a = ev.iter().find_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).unwrap();
        let (r, _, cancel) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Cancel, message: None, answers: None }).unwrap();
        assert_eq!(r["result"]["outcome"], json!({ "outcome": "cancelled" }));
        assert!(cancel);

        norm.on_permission(&json!(9), &req);
        let cancelled = norm.cancel_permissions();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].0, json!({ "id": 9, "result": { "outcome": { "outcome": "cancelled" } } }));
        norm.interrupt_requested = true;
        let ev = norm.turn_ended(&json!({ "stopReason": "cancelled" }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Interrupted, .. })));

        let ev = norm.on_update(&json!({ "sessionUpdate": "plan", "entries": [{ "content": "a", "priority": "high", "status": "in_progress" }] }));
        assert!(matches!(&ev[0], EventKind::PlanUpdated { steps } if steps[0].status == "in_progress"));
        let ev = norm.on_update(&json!({ "sessionUpdate": "something_new" }));
        assert!(matches!(&ev[0], EventKind::Unknown { raw_type } if raw_type == "session/update:something_new"));
    }

    #[test]
    fn launch_lines() {
        use crate::adapters::McpLaunch;
        use crate::model::Role;
        let mut spec = SpawnSpec {
            agent_id: "a".into(),
            run_id: "r".into(),
            provider: Provider::Gemini,
            role: Role::Worker,
            model: Some("gemini-2.5-flash".into()),
            cwd: PathBuf::from("/w"),
            bin: PathBuf::from("gemini"),
            env: HashMap::new(),
            session: SessionMode::New { session_id: None },
            system_prompt: String::new(),
            read_only: false,
            mcp: Some(McpLaunch { command: "/x/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), "t".into())] }),
            use_user_setup: false,
            raw_log: PathBuf::from("/tmp/r/raw.ndjson"),
        };
        let (a, env) = launch(&spec, Some(Path::new("/tmp/r/gemini-policy.toml")));
        assert_eq!(a, ["--acp", "--skip-trust", "-m", "gemini-2.5-flash", "--policy", "/tmp/r/gemini-policy.toml"]);
        assert!(env.is_empty());
        assert_eq!(mcp_servers(&spec), json!([{ "name": "codly", "command": "/x/codly", "args": ["mcp"], "env": [{ "name": "CODLY_MCP_TOKEN", "value": "t" }] }]));
        spec.provider = Provider::Opencode;
        let (a, env) = launch(&spec, None);
        assert_eq!(a, ["acp"]);
        let cfg: Value = serde_json::from_str(&env[0].1).unwrap();
        assert_eq!(cfg["permission"], json!({ "edit": "ask", "bash": "ask", "webfetch": "ask" }));
        spec.read_only = true;
        let cfg: Value = serde_json::from_str(&launch(&spec, None).1[0].1).unwrap();
        assert_eq!(cfg["permission"]["edit"], "deny");
        let opts = json!([{ "id": "model", "category": "model", "options": [{ "value": "opencode/big-pickle" }, { "group": "x", "options": [{ "value": "openai/gpt-4.1" }] }] }]);
        assert_eq!(pick_option(&opts, "model", "big-pickle"), Some(("model".into(), "opencode/big-pickle".into())));
        assert_eq!(pick_option(&opts, "model", "openai/gpt-4.1"), Some(("model".into(), "openai/gpt-4.1".into())));
        assert_eq!(pick_option(&opts, "model", "nope"), None);
        assert_eq!(tool_name("read_file__read_file_1792964533840_0", Some("README.md")).as_deref(), Some("read_file"));
        assert_eq!(tool_name("ccd5e765-06b2", Some("read")).as_deref(), Some("read"));
        assert_eq!(tool_name("ccd5e765-06b2", Some("git status --short")), None);
    }
}
