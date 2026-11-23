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
//! Turns follow the CLI's own lines (recorded on 2.1.311: cli-claude-stream-json-2.1.311 §1, §5,
//! and its part 2, §1 to §8):
//! - Every user message carries a `uuid`. A prompt for an idle CLI starts a turn at once. One
//!   sent while a turn runs is written at once too: the CLI keeps it and takes it into the
//!   running turn at the next tool boundary (`PromptSteered`), or runs it as a turn of its own
//!   when the turn ends first (`TurnStarted`). Its `command_lifecycle` line (`started`) says
//!   which; until then the prompt waits and the orchestrator counts it as queued. Several
//!   prompts that wait are taken together: into the running turn, or as one new turn (the
//!   first opens it, the others join it).
//! - Codly holds a prompt back until the turn ends only when the CLI has no such lines, when a
//!   stop is on its way (the prompt after an interrupt is the next turn), and for a slash
//!   command (written mid-turn, the CLI itself runs it as a turn of its own after the running
//!   one: holding it keeps its place among the prompts sent after it).
//! - A stop can reach the CLI while a prompt waits in it. The answer to the interrupt lists the
//!   uuids of the prompts the CLI still holds (`still_queued`): it runs those as the next turn
//!   by itself, and a prompt the answer does not name is Codly's to send again.
//! - When the agent itself is stopped, the interrupt says `cancel_queued`: the CLI drops the
//!   prompts that wait in it and writes a `cancelled` line for each. The background commands
//!   it still tracks are ended with `stop_task`, and only then is stdin closed: with one of
//!   them open the CLI does not exit, it waits for the command and runs a turn after it. A
//!   prompt the CLI did not drop would run next: its process is ended before it can.
//! - The CLI starts a turn by itself when a background task ends while it is idle.
//!   `system/init` opens every turn, so one that comes while no turn is open is such a turn:
//!   `TurnStarted` with no input.
//!
//! `AgentCmd::Session` (permission mode, model, reasoning effort, context use, reconnect an
//! MCP server, stop a background task) is written at once as a control request: the CLI
//! answers those between turns and during one. Request and answer shapes, and what a
//! permission suggestion does when it is sent back, are recorded on 2.1.311
//! (docs/research/cli-claude-stream-json-2.1.311.md §2, §8 and §10; its part 2: §6, §18, and
//! the effort and the default model in scenario 13).

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
use super::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SessionReply, SessionRequest, SpawnSpec, attachments, startup};
use crate::housekeeping::connections::{self, ReadyMcp};
use crate::model::{
    ActivityKind, AgentStatus, Approval, ApprovalChoice, ApprovalDecision, ApprovalStatus, ApprovalSubject, BackgroundEnded, BackgroundTask, Billing,
    EventKind, Id, McpServerStatus, MessagePhase, PermissionMode, PlanStep, Provider, Role, TurnStatus, Usage, new_id, now_ms,
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
/// Tools the CLI may leave out of `init.tools` although they were asked for: never reported as
/// "not enabled". The CLI drops a name without a word (part 2 of the 2.1.311 recordings,
/// scenario 10), so only a tool the agent cannot work without is worth a line.
/// - Its version or the user's setup decides: LSP needs a language server, Skill needs skills.
/// - `WebFetch` and `LSP` were not in `init.tools` of a run with the 1:1 tool list (recorded,
///   same scenario; the recording does not say why).
/// - `ExitPlanMode` is only needed in plan mode (`ClaudeNormalizer::may_be_missing`): a CLI
///   that does not know the name runs every other mode as before.
const OPTIONAL_TOOLS: &[&str] = &["Monitor", "TaskStop", "Skill", "LSP", TOOL_SEARCH, "WebFetch", EXIT_PLAN_MODE];
/// Needed to reach the user's own MCP servers when there are many of them.
const TOOL_SEARCH: &str = "ToolSearch";
/// How Claude shows its plan in plan mode: the call arrives as a `can_use_tool` request with
/// the plan text (cli-claude-stream-json-2.1.311 §10). Left out of `--tools`, the CLI denies
/// the call by a rule of its own and a plan could never be accepted.
pub const EXIT_PLAN_MODE: &str = "ExitPlanMode";

/// The name `set_model` takes for the CLI's own default model (part 2 of the 2.1.311
/// recordings, scenario 13: the answer names the model it resolves to).
pub const DEFAULT_MODEL: &str = "default";

/// The reasoning-effort levels Claude Code lists for its models (`supportedEffortLevels` in the
/// `initialize` answer of the 2.1.311 recordings). `apply_flag_settings` answers "success" for
/// any other word and drops it (part 2, scenario 13), so no other word is sent. `low` and
/// `high` are the levels recorded with that request: whoever sets a level reads it back
/// (`SessionRequest::Settings`).
pub const EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The model an `assistant` line names when the CLI wrote its text itself (the output of a
/// local command such as `/compact`): no model ran.
const SYNTHETIC_MODEL: &str = "<synthetic>";

const INIT_TIMEOUT: Duration = Duration::from_secs(90);

/// How long a turn has to end after an interrupt request. Past it the CLI is stuck: its process
/// is ended (stdin closed, then SIGTERM, then SIGKILL) and the session resumes with the next run.
/// (Recorded: the `result` of a turn interrupted in a tool call came within 0.01 s, part 2 §8.)
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(8);

/// How long the CLI has, when an agent is stopped, to say that the background commands Codly
/// asked it to stop are gone (recorded: at once, part 2 §6). Past it the CLI gets SIGTERM, at
/// which it ends them itself. Short, so that both fit in the time the app waits at exit.
const STOP_TASKS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the interrupt that starts the stop of an agent has to do its work: end the running
/// turn (its `result`), or, with no turn running, end the background helpers (the list without
/// them). Recorded: 0.02 to 0.09 s for both (part 4 §1 and §2).
const STOP_INTERRUPT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn tools_for(spec: &SpawnSpec) -> Vec<&'static str> {
    let mut t = if spec.read_only { READ_ONLY_TOOLS } else { WORKER_TOOLS }.to_vec();
    if spec.use_user_setup {
        t.push(TOOL_SEARCH);
    }
    // Only a 1:1 agent can be put in plan mode (`PermissionMode::Plan`).
    if spec.role == Role::Solo && !spec.read_only {
        t.push(EXIT_PLAN_MODE);
    }
    t
}

/// Workers edit freely inside their own worktree. In `acceptEdits` the CLI asks about neither
/// an edit there nor a shell command that changes, moves or deletes files there (recorded:
/// `mv`, `rm` and `rm -rf sub && mkdir sub && mv …` ran with no request; part 2 of the 2.1.311
/// recordings, scenario 9). It still asks about an edit of the folder's own `.claude`
/// settings (same scenario), a write outside the folder, and the other shell commands (with
/// the shell sandbox too: `settings_json`). Solo agents work in the user's checkout and
/// read-only roles have nothing to accept, so both ask for everything.
pub fn permission_mode(spec: &SpawnSpec) -> &'static str {
    if spec.role == Role::Worker && !spec.read_only { "acceptEdits" } else { "manual" }
}

/// The `--mcp-config` document for the codly server (or None without MCP). `ready`: the
/// user's default servers with their keys (`connections::load_ready`), looked up once per
/// spawn so this document and `--allowedTools` name the same servers.
pub fn mcp_config(spec: &SpawnSpec, ready: &[ReadyMcp]) -> Option<Value> {
    let m = spec.mcp.as_ref()?;
    let env: serde_json::Map<String, Value> = m.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
    let mut doc = json!({ "mcpServers": { "codly": {
        "type": "stdio", "command": m.command, "args": m.args, "env": env,
        // keep codly tools loaded even if a future default defers MCP tools
        "alwaysLoad": true
    }}});
    // The user's default servers (Settings → Connections), as enabled when this run started.
    for x in ready {
        let env: serde_json::Map<String, Value> = x.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        doc["mcpServers"][&x.name] = json!({ "type": "stdio", "command": x.command, "args": x.args, "env": env });
    }
    Some(doc)
}

/// `--allowedTools` patterns for the default servers that only read; the others ask first.
fn default_mcp_allowed(ready: &[ReadyMcp]) -> Vec<String> {
    ready.iter().filter(|x| x.read_only).map(|x| format!("mcp__{}__*", x.name)).collect()
}

/// The risk on a request to run a command outside the shell sandbox. The rules send every
/// risk to the user, in every approval mode but Full access, and no saved rule covers it.
pub const UNSANDBOXED: &str = "runs outside the sandbox";

/// The tool name of the sandbox's own request: a sandboxed command wants to reach a host that
/// Claude Code's settings do not allow (`input.host`). Recorded on 2.1.311
/// (docs/research/cli-claude-stream-json-2.1.311-part2.md, scenario 14): allowed, the command
/// goes on; denied, it ends with an error that names the sandbox.
const SANDBOX_NETWORK: &str = "SandboxNetworkAccess";

/// The host that a `SANDBOX_NETWORK` request asks for ("" when it names none), for what is
/// shown to the user: the tool's own name says nothing to them. None: `tool` is another tool.
pub fn sandbox_host<'a>(tool: &str, input: &'a Value) -> Option<&'a str> {
    (tool == SANDBOX_NETWORK).then(|| input["host"].as_str().unwrap_or_default())
}

/// Does this run ask for Claude Code's shell sandbox? An agent with a shell and
/// `Settings.claude_sandbox` on.
pub(super) fn sandboxed(spec: &SpawnSpec) -> bool {
    spec.sandbox_shell && !spec.read_only
}

/// The run's `--settings` document: it goes above the user's own settings files and below
/// managed ones. None when Codly sets nothing.
/// - `apiKeyHelper`: the BYOK token command (byok.rs). Claude runs it for its API key and
///   re-runs it on expiry.
/// - `sandbox`: Claude Code's own OS sandbox for the shell (`Settings.claude_sandbox`), for
///   an agent that has a shell. Recorded on macOS with 2.1.311
///   (docs/research/cli-claude-stream-json-2.1.311.md §9 and -part2.md, scenarios 14 to 16):
///   with `enabled`, a write outside the working directory fails. By itself the sandbox would
///   also run every shell command with no permission request. `autoAllowBashIfSandboxed:
///   false` puts the shell back under the permission mode: a command asks as it does without
///   the sandbox, so Codly's approval rules still decide it, and the sandbox is the floor
///   under them. A command the model wants to run outside the sandbox asks, marked
///   `sandboxOverride` (`UNSANDBOXED`). Codly passes no list of folders or hosts: the CLI's
///   defaults and the user's own Claude Code settings decide them, and for a host they do
///   not allow the CLI asks Codly (`SANDBOX_NETWORK`), which asks the user.
///   `failIfUnavailable` is in the CLI's settings schema; what it does is from the CLI's
///   documentation, not recorded: where Claude Code cannot sandbox, it does not start. Codly
///   does not rely on it: it asks the CLI once the session is up (`sandbox_refusal`). A CLI
///   that does end there is said in the same words (`startup::explain`).
pub fn settings_json(spec: &SpawnSpec) -> Option<Value> {
    let mut doc = serde_json::Map::new();
    if let Some(cmd) = &spec.byok_command {
        doc.insert("apiKeyHelper".into(), json!(cmd));
    }
    if sandboxed(spec) {
        doc.insert("sandbox".into(), json!({ "enabled": true, "failIfUnavailable": true, "autoAllowBashIfSandboxed": false }));
    }
    (!doc.is_empty()).then(|| Value::Object(doc))
}

/// A run that asked for the shell sandbox and was not started, because the sandbox is not on
/// as Codly asked for it: the text says why, and what to do. Nothing is wrong with the
/// conversation, and a new one would not start either: a caller that resumes keeps the session.
#[derive(Debug)]
pub struct SandboxNotOn(pub String);

impl std::fmt::Display for SandboxNotOn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SandboxNotOn {}

impl SandboxNotOn {
    /// The way out that ends every text: the switch that asked for the sandbox.
    const SWITCH: &'static str = "turn off \"Sandbox Claude's shell\" in Settings → Safety";
    /// The words every text has: `retry::needs_you` knows by them a start that only you can
    /// fix (no wait turns a sandbox on).
    pub(crate) const NOT_STARTED: &'static str = "did not start this agent";
    /// The words of the one text that a later start can get past (`no_answer`): the CLI was
    /// slow, not the sandbox off. `retry::needs_you` lets the task try again by itself.
    pub(crate) const NO_ANSWER: &'static str = "did not answer Codly's check of its sandbox";

    /// Claude Code did not turn its sandbox on and gave no reason: the CLI said so
    /// (`sandbox_refusal`), or it ended at its start because of it (`startup::explain`).
    pub(super) fn off() -> Self {
        Self(format!("Claude Code did not turn its sandbox on, so Codly did not start this agent. Check Claude Code's sandbox on this computer, or {}", Self::SWITCH))
    }

    /// The CLI answered the `initialize` request and then not Codly's check of its sandbox
    /// (`get_sandbox_dialog`) before the time for a start was over.
    fn no_answer() -> Self {
        let waited = INIT_TIMEOUT.as_secs();
        Self(format!("Claude Code {} within {waited} s, so Codly did not start this agent. Try again, or {}", Self::NO_ANSWER, Self::SWITCH))
    }
}

/// How much of the CLI's own reasons a refusal shows.
const SANDBOX_ERRORS_MAX: usize = 300;

/// Why a run that asked for the shell sandbox must not go on, read from the CLI's answer to
/// `get_sandbox_dialog` (`resp`: the `response` of the control response). None: the sandbox
/// is on and shell commands ask. The answer is recorded on 2.1.311 with the sandbox off
/// (`enabled: false`), on as Codly asks for it (`mode: "regular"`), and on with the CLI's
/// default (`mode: "auto-allow"`: commands run with no request). Codly's own `--settings`
/// never give the last one; settings above them (managed ones) can.
///
/// The text says what the answer says. `supported` and `dependencies.errors` are in every
/// recorded answer, as `true` and an empty list: no machine at hand has a sandbox that cannot
/// run. So what an error is made of is not recorded: a text is shown as it is, anything else
/// as its JSON. They only choose the words: `enabled` and `mode` alone decide the refusal.
fn sandbox_refusal(resp: &Value) -> Option<SandboxNotOn> {
    const SWITCH: &str = SandboxNotOn::SWITCH;
    if resp["subtype"] != "success" {
        let error = resp["error"].as_str().map_or_else(|| resp["error"].to_string(), str::to_owned);
        // A CLI that does not know the request answers as it does to every request it does not
        // know (recorded: "Unsupported control request subtype: <name>"): it is too old to say.
        let fix = if error.starts_with("Unsupported control request subtype") {
            "Update Claude Code (run `claude update` in a terminal)"
        } else {
            "Check Claude Code's sandbox on this computer"
        };
        return Some(SandboxNotOn(format!("Codly could not check that Claude Code's sandbox is on ({error}), so it did not start this agent. {fix}, or {SWITCH}")));
    }
    let status = &resp["response"];
    if status["enabled"] != true {
        let errors: Vec<String> = status["dependencies"]["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| e.as_str().map_or_else(|| e.to_string(), |text| text.trim().trim_end_matches('.').to_owned()))
            .filter(|e| !e.is_empty())
            .collect();
        if !errors.is_empty() {
            let errors = classify::truncate_chars(&errors.join("; "), SANDBOX_ERRORS_MAX);
            return Some(SandboxNotOn(format!("Claude Code says its sandbox cannot run here: {errors}. Codly did not start this agent. Fix that, or {SWITCH}")));
        }
        if status["supported"] == false {
            return Some(SandboxNotOn(format!("Claude Code says its sandbox cannot run on this computer, so Codly did not start this agent. To work without the sandbox, {SWITCH}")));
        }
        return Some(SandboxNotOn::off());
    }
    if status["mode"] == "auto-allow" {
        return Some(SandboxNotOn(format!(
            "Claude Code's sandbox would run this agent's commands without asking, so Codly's approval rules would not see them (a setting of your organization or of this computer's Claude Code decides that: sandbox.autoAllowBashIfSandboxed). Codly did not start this agent. Change that setting, or {SWITCH}"
        )));
    }
    None
}

/// The settings files Claude loads: the user's own always; the project's with the user's
/// setup; the working directory's local file only where the orchestrator says it is safe
/// (`SpawnSpec::local_settings`).
fn setting_sources(spec: &SpawnSpec) -> &'static str {
    match (spec.use_user_setup, spec.local_settings) {
        (true, true) => "user,project,local",
        (true, false) => "user,project",
        (false, true) => "user,local",
        (false, false) => "user",
    }
}

/// The full `claude` argv. `mcp_config_path` is a 0600 file holding [`mcp_config`] (a file,
/// not an inline string, so the per-run MCP token never shows up in `ps`); `ready` is the
/// list that document was made from.
pub fn build_args(spec: &SpawnSpec, session_id: &str, mcp_config_path: Option<&Path>, ready: &[ReadyMcp]) -> Vec<String> {
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
        allowed.extend(default_mcp_allowed(ready));
        a.extend(["--allowedTools".into(), allowed.join(",")]);
    }
    a.extend(["--setting-sources".into(), setting_sources(spec).into()]);
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
    if let Some(settings) = settings_json(spec) {
        a.extend(["--settings".into(), settings.to_string()]);
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
    // The keys of the default servers come from the Keychain through a subprocess, which can
    // wait on a permission dialog: once per spawn, and not on a runtime thread.
    let ready = {
        let raw_log = spec.raw_log.clone();
        tokio::task::spawn_blocking(move || connections::load_ready(&raw_log)).await.context("reading the default MCP servers")?
    };
    let mcp_file = match mcp_config(&spec, &ready) {
        Some(doc) => Some(PrivateFile::write(&spec.raw_log.with_file_name(MCP_CONFIG_FILE), &doc.to_string())?),
        None => None,
    };
    let args = build_args(&spec, &session_id, mcp_file.as_ref().map(|f| f.0.as_path()), &ready);
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
    // Also ends what is left in the CLI's process group once it is gone. That is not what its
    // Bash calls started: each of those is a group of its own (`proc::command`).
    proc::reap(child, move |code| {
        let _ = exit_tx.send(code);
    });

    let (tx, cmds) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (sandbox_asked_tx, mut sandbox_asked) = oneshot::channel();
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
        waiting: Vec::new(),
        interrupts: HashMap::new(),
        left_in_cli: false,
        lifecycle: false,
        cancels_queued: false,
        turn_active: false,
        next_req: 0,
        session_replies: HashMap::new(),
        stage: Stage::Running,
        deadline: None,
        start_failed: false,
        interrupt_failed: false,
        _mcp_file: mcp_file,
        sandbox_check: sandboxed(&spec).then_some(SandboxCheck::Due { asked: sandbox_asked_tx }),
    };
    tokio::spawn(actor.run(cmds, lines, exit_rx, ready_tx));
    match tokio::time::timeout(INIT_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(()))) => Ok(AgentHandle { agent_id: spec.agent_id, run_id: spec.run_id, tx }),
        Ok(Ok(Err(e))) => Err(startup::explain(&spec, None, e)),
        Ok(Err(_)) => Err(anyhow!("claude adapter stopped during startup")),
        Err(_) => {
            let _ = tx.send(AgentCmd::Shutdown);
            Err(start_timed_out(sandbox_asked.try_recv().is_ok()))
        }
    }
}

/// The start got no answer within `INIT_TIMEOUT`. `sandbox_asked`: the CLI answered the
/// `initialize` request, and the answer that did not come is the one to Codly's check of its
/// sandbox (`SandboxCheck`). That is a `SandboxNotOn`, as every other outcome of that check
/// but "on": nothing is wrong with the conversation.
fn start_timed_out(sandbox_asked: bool) -> anyhow::Error {
    if sandbox_asked {
        return SandboxNotOn::no_answer().into();
    }
    anyhow!("claude did not answer the initialize request within {} s", INIT_TIMEOUT.as_secs())
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
    /// Shutdown asked while no turn ran and the CLI's last list named a helper: interrupt
    /// sent, waiting for the list without helpers.
    StoppingHelpers,
    /// Shutdown, the turn is over: `stop_task` sent for each background command, waiting for
    /// the CLI to say that none is left.
    StoppingTasks,
    /// stdin closed, waiting for the process to exit.
    Closing,
    /// SIGTERM → SIGKILL ladder running.
    Killing,
}

/// The check that the shell sandbox a run asked for is really on. The start is not over
/// before the CLI said so: a run that would work unsandboxed is not started.
enum SandboxCheck {
    /// To ask once the CLI answered the `initialize` request. `asked` tells whoever waits for
    /// the start that the check is what it waits for from then on (`start_timed_out`).
    Due { asked: oneshot::Sender<()> },
    /// Asked (`get_sandbox_dialog`, under this request id). The session's first event waits
    /// with it: a start that fails has no events.
    Asked { request_id: String, started: EventKind },
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
    /// Prompts Codly holds until the CLI can take them (`can_steer`), oldest first.
    queue: VecDeque<String>,
    /// Prompts written while a turn ran, by the uuid Codly put on them: the CLI has them, and
    /// no turn took them yet.
    waiting: Vec<(String, String)>,
    /// Interrupt requests with no answer yet, by request id: the uuids of the prompts that
    /// waited in the CLI when each was written.
    interrupts: HashMap<String, Vec<String>>,
    /// Codly gave up a prompt that still waited in the CLI (the agent is stopped and the CLI did
    /// not drop it, or its turn did not stop). The CLI would still run it, so its process is
    /// not left to exit by itself.
    left_in_cli: bool,
    /// The CLI reports what it does with each user message (`msg_lifecycle_v1` in `system/init`).
    lifecycle: bool,
    /// An interrupt can tell the CLI to drop the prompts that wait in it
    /// (`interrupt_cancel_queued_v1` in `system/init`).
    cancels_queued: bool,
    turn_active: bool,
    next_req: u64,
    /// Session requests the CLI has not answered yet, by control request id.
    session_replies: HashMap<String, (SessionRequest, SessionReply)>,
    stage: Stage,
    /// While running: when an interrupted turn must have ended. In a shutdown: the stage's limit.
    deadline: Option<tokio::time::Instant>,
    start_failed: bool,
    /// An interrupt got no result within `INTERRUPT_TIMEOUT`, so the process is being ended.
    interrupt_failed: bool,
    /// Held, never read: dropping the actor removes the file (`PrivateFile`).
    _mcp_file: Option<PrivateFile>,
    /// Set while a run that asked for the shell sandbox starts; None for every other run, and
    /// once the CLI said that the sandbox is on.
    sandbox_check: Option<SandboxCheck>,
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
                    // A second answer with the id of `initialize` (in no recording) while the
                    // sandbox check waits for its own answer: that one still decides the start.
                    if matches!(self.sandbox_check, Some(SandboxCheck::Asked { .. })) {
                        *ready = Some(r);
                        return;
                    }
                    let started = self.norm.on_initialize(&resp["response"]);
                    if let Some(SandboxCheck::Due { asked }) = self.sandbox_check.take() {
                        // The run asked for the shell sandbox: it has not started before the
                        // CLI says that the sandbox is on (`on_sandbox_status`).
                        let request_id = self.control("get_sandbox_dialog", json!({})).await;
                        let _ = asked.send(()); // (an error: nobody waits for this start any more)
                        self.sandbox_check = Some(SandboxCheck::Asked { request_id, started });
                        *ready = Some(r);
                        return;
                    }
                    self.emit(started);
                    let _ = r.send(Ok(()));
                } else {
                    let _ = r.send(Err(anyhow!("claude rejected the initialize request: {}", resp["error"])));
                    self.start_failed = true; // spawn() returned the error: no events for this run
                    self.begin_shutdown().await;
                }
            } else if let Some(SandboxCheck::Asked { request_id, .. }) = &self.sandbox_check
                && resp["request_id"] == request_id.as_str()
            {
                let Some(SandboxCheck::Asked { started, .. }) = self.sandbox_check.take() else { return };
                self.on_sandbox_status(started, resp, ready).await;
            } else if let Some((request, reply)) = resp["request_id"].as_str().and_then(|id| self.session_replies.remove(id)) {
                self.session_answered(request, reply, resp);
                // (what the answer makes Codly ask next: the MCP servers' state after a
                // reconnect or a switch)
                for line in std::mem::take(&mut self.norm.outbox) {
                    self.write(&line).await;
                }
            } else {
                let interrupt = self.interrupts.remove(resp["request_id"].as_str().unwrap_or_default());
                if resp["subtype"] == "error" {
                    self.emit(EventKind::Error { message: format!("claude control request failed: {}", resp["error"]), fatal: false });
                } else if let Some(waited) = interrupt {
                    self.on_interrupt_answer(&resp["response"]["still_queued"], &waited).await;
                }
            }
            return;
        }
        let listed = v["type"] == "system" && v["subtype"] == "background_tasks_changed";
        // The agent is being stopped and its turn is over. What the CLI does from here (a turn
        // for a prompt it still held, a turn of its own) is not shown as this agent's work.
        // Its list of background tasks is still followed: the stop waits for it (`wind_down`).
        // While the stop waits for its helpers to end, their task lines and the list are still
        // shown: the helper rows end as the CLI says.
        if self.stage != Stage::Running && !self.turn_active {
            let helpers_end = self.stage == Stage::StoppingHelpers
                && v["type"] == "system"
                && matches!(v["subtype"].as_str(), Some("task_updated" | "task_notification" | "background_tasks_changed"));
            if helpers_end {
                for e in self.norm.on_message(&v) {
                    self.emit(e);
                }
            } else if listed {
                self.norm.on_background_tasks(&v["tasks"], &mut Vec::new());
            }
            if listed {
                self.on_background_tasks().await;
            }
            return;
        }
        if v["type"] == "command_lifecycle" {
            return self.on_lifecycle(&v).await;
        }
        let init = v["type"] == "system" && v["subtype"] == "init";
        if init {
            let can = |name: &str| v["capabilities"].as_array().is_some_and(|c| c.iter().any(|x| x.as_str() == Some(name)));
            self.lifecycle = can("msg_lifecycle_v1");
            self.cancels_queued = can("interrupt_cancel_queued_v1");
        }
        // No prompt of Codly's opened this turn: the CLI started it by itself (a background task
        // that ended woke it). `system/init` opens every turn, and a `result` ends one: it never
        // comes for a turn that is not open.
        if !self.turn_active && (init || v["type"] == "result") {
            self.open_turn(None);
        }
        let events = self.norm.on_message(&v);
        let turn_done = events.iter().any(|e| matches!(e, EventKind::TurnCompleted { .. }));
        for e in events {
            self.emit(e);
        }
        for reply in std::mem::take(&mut self.norm.outbox) {
            self.write(&reply).await;
        }
        if listed {
            self.on_background_tasks().await;
        }
        if turn_done {
            self.turn_active = false;
            if self.stage == Stage::Interrupting {
                self.wind_down().await;
            } else if self.stage == Stage::Running {
                self.deadline = None; // an interrupted turn ended in time
                self.flush().await;
            }
        } else if init && self.stage == Stage::Running {
            self.flush().await; // what was held until the CLI said it reports on its messages
        }
    }

    /// The CLI's answer to `get_sandbox_dialog`, asked once at the start of a run with the
    /// shell sandbox (shapes: cli-claude-stream-json-2.1.311 §9 and -part2, scenario 16). The
    /// run starts when the sandbox is on. When it is not, the start fails with the reason
    /// (`sandbox_refusal`, a `SandboxNotOn` error) and the process is ended: no command of
    /// this run was run.
    async fn on_sandbox_status(&mut self, started: EventKind, resp: &Value, ready: &mut Option<oneshot::Sender<anyhow::Result<()>>>) {
        let Some(r) = ready.take() else { return };
        match sandbox_refusal(resp) {
            None => {
                self.emit(started);
                let _ = r.send(Ok(()));
            }
            Some(why) => {
                let _ = r.send(Err(why.into()));
                self.start_failed = true; // spawn() returned the error: no events for this run
                self.begin_shutdown().await;
            }
        }
    }

    /// `command_lifecycle`: what the CLI does with a user message, named by the uuid Codly put
    /// on it. Only the prompts written while a turn ran are followed here: the one that opened
    /// its turn was announced when it was written. (The stopped turn's own prompt gets a
    /// `cancelled` line too, and a prompt a turn took gets `completed`: neither waits here.)
    async fn on_lifecycle(&mut self, v: &Value) {
        let Some(at) = self.waiting.iter().position(|(uuid, _)| v["command_uuid"] == uuid.as_str()) else { return };
        match v["state"].as_str() {
            // A turn took it: the running one, or a new one that it opens. Prompts that start
            // together after a turn make one turn: the first opens it, the others join it.
            Some("started") => {
                let (_, text) = self.waiting.remove(at);
                if self.turn_active {
                    self.emit(EventKind::PromptSteered { input: text });
                } else {
                    self.open_turn(Some(&text));
                }
            }
            // It never reached the model (recorded for a prompt the host withdrew: part 2 §1,
            // §5). At a stop of the agent Codly withdrew it itself (`cancel_queued`) and gives
            // it up. In any other case the CLI gave it up: it is sent again, as a turn of its own.
            Some("cancelled") => {
                let (_, text) = self.waiting.remove(at);
                if self.stage == Stage::Running {
                    self.take_back(vec![text]).await;
                }
            }
            _ => {} // "queued": it waits. "completed": the turn's `result` says so.
        }
    }

    /// The answer to an interrupt lists the prompts the CLI still holds, by the uuid Codly put
    /// on each (`still_queued`, plain strings: part 2 §1). The CLI runs those as the next turn
    /// by itself, so they are not sent again. A prompt that waited in the CLI when the
    /// interrupt was written, and that the answer does not name, is gone from the CLI: it is
    /// sent again, as a turn of its own.
    async fn on_interrupt_answer(&mut self, still_queued: &Value, waited: &[String]) {
        // No list: only the `command_lifecycle` lines say where the prompts went.
        let Some(held) = still_queued.as_array() else { return };
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|(uuid, _)| waited.contains(uuid) && !held.iter().any(|h| h.as_str() == Some(uuid.as_str())));
        self.waiting = kept;
        self.take_back(gone.into_iter().map(|(_, text)| text).collect()).await;
    }

    /// The CLI said its list of background tasks again (`system/background_tasks_changed`,
    /// part 2 §6; the normalizer holds the list). A stop of the agent that waits for its
    /// helpers or its commands to end goes on when none is left.
    async fn on_background_tasks(&mut self) {
        match self.stage {
            Stage::StoppingHelpers if !self.norm.helpers_in_background() => self.wind_down().await,
            Stage::StoppingTasks if self.norm.commands_in_background().is_empty() => self.close_stdin(),
            _ => {}
        }
    }

    /// Prompts the CLI gave up before a turn took them: back to the front of what Codly holds,
    /// in their order, and sent again as soon as the CLI can take them.
    async fn take_back(&mut self, texts: Vec<String>) {
        for text in texts.into_iter().rev() {
            self.queue.push_front(text);
        }
        if !self.busy() && self.stage == Stage::Running {
            self.flush().await;
        }
    }

    /// A turn opens: with the prompt that started it, or with none when the CLI started it.
    fn open_turn(&mut self, input: Option<&str>) {
        self.turn_active = true;
        let events = match input {
            Some(text) => self.norm.turn_started(text),
            None => self.norm.turn_self_started(),
        };
        for e in events {
            self.emit(e);
        }
    }

    /// The CLI has work it knows of: a turn runs, or prompts wait in it for their turn.
    fn busy(&self) -> bool {
        self.turn_active || !self.waiting.is_empty()
    }

    /// May this prompt be written to a busy CLI? Only when the CLI says what it does with it
    /// (`command_lifecycle`), when no stop is on its way (the prompt behind an interrupt is the
    /// next turn, not a part of the stopped one), and when it is no slash command. The CLI
    /// takes one written mid-turn, never into the running turn: it runs it as a turn of its
    /// own right after (part 2 §3). Held here, it does the same and keeps its place among the
    /// prompts sent after it (a slash command and a message that wait together in the CLI are
    /// not recorded).
    fn can_steer(&self, text: &str) -> bool {
        self.lifecycle && !self.norm.interrupt_requested && !text.starts_with('/')
    }

    /// Send what Codly holds, oldest first, as far as the CLI can take it now.
    async fn flush(&mut self) {
        while let Some(next) = self.queue.front() {
            if self.busy() && !self.can_steer(next) {
                break;
            }
            let Some(text) = self.queue.pop_front() else { break };
            self.send_prompt(text).await;
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
                self.queue.push_back(text);
                self.flush().await;
            }
            AgentCmd::Interrupt => {
                if self.stage != Stage::Running {
                    return; // a shutdown is under way: it interrupts the turn itself
                }
                // Sent even when no turn seems to run: Claude answers an idle interrupt at once
                // (`still_queued: []`), so "stop" is never dropped on a flag that went stale.
                self.norm.interrupt_requested = self.turn_active;
                let request_id = self.control("interrupt", json!({})).await;
                if !self.waiting.is_empty() {
                    self.interrupts.insert(request_id, self.waiting.iter().map(|(uuid, _)| uuid.clone()).collect());
                }
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
            AgentCmd::Session { request, reply } => self.session_request(request, reply).await,
        }
    }

    /// Write a session request as its control request (shapes: cli-claude-stream-json-2.1.311
    /// §2). The CLI answers at once, during a turn too, so nothing is queued here.
    async fn session_request(&mut self, request: SessionRequest, reply: SessionReply) {
        if self.stage != Stage::Running {
            let _ = reply.send(Err("Claude Code is shutting down".into()));
            return;
        }
        let (subtype, body) = match &request {
            SessionRequest::PermissionMode(mode) => ("set_permission_mode", json!({ "mode": mode.cli() })),
            SessionRequest::Model(model) => ("set_model", json!({ "model": model })),
            SessionRequest::ContextUsage => ("get_context_usage", json!({})),
            SessionRequest::McpReconnect(server) => ("mcp_reconnect", json!({ "serverName": server })),
            SessionRequest::McpEnabled(server, on) => ("mcp_toggle", json!({ "serverName": server, "enabled": on })),
            SessionRequest::StopTask(id) => match self.norm.task_to_stop(id) {
                Some(task) => ("stop_task", json!({ "task_id": task })),
                None => {
                    let _ = reply.send(Err(format!("nothing with the id \"{id}\" runs beside this agent: no background task and no helper at work")));
                    return;
                }
            },
            SessionRequest::Effort(level) => {
                // The CLI would answer "success" and drop a word it does not know (recorded).
                if let Some(level) = level.as_deref().filter(|l| !EFFORT_LEVELS.contains(l)) {
                    let _ = reply.send(Err(format!("Claude Code has no effort level \"{level}\"")));
                    return;
                }
                // (None: `effortLevel: null` takes the level away; the CLI's default follows)
                ("apply_flag_settings", json!({ "settings": { "effortLevel": level } }))
            }
            SessionRequest::Settings => ("get_settings", json!({})),
        };
        let id = self.control(subtype, body).await;
        if self.stdin.is_none() {
            let _ = reply.send(Err("Claude Code no longer reads its input".into()));
            return;
        }
        // Known before the answer: the CLI's status line for this switch can come first, and it
        // is no change of the CLI's own.
        if let SessionRequest::PermissionMode(mode) = &request {
            self.norm.permission_mode = Some(*mode);
        }
        self.session_replies.insert(id, (request, reply));
    }

    /// The CLI answered a session request: the answer goes to whoever asked (an error too: it
    /// is theirs to show, not a line of the timeline).
    fn session_answered(&mut self, request: SessionRequest, reply: SessionReply, resp: &Value) {
        let mut answer = if resp["subtype"] == "success" {
            Ok(resp["response"].clone())
        } else {
            Err(resp["error"].as_str().map_or_else(|| resp["error"].to_string(), str::to_owned))
        };
        match (&request, &mut answer) {
            // A refused switch leaves the CLI in the mode it had, which this request overwrote
            // here: its next status line says which one it is.
            (SessionRequest::PermissionMode(_), Err(_)) => self.norm.permission_mode = None,
            (SessionRequest::Model(model), Ok(_)) => {
                self.norm.model = Some(model.clone()).filter(|m| m != DEFAULT_MODEL);
                self.norm.model_switched();
            }
            // Connected or not, switched or not, the server's state may be another now. The
            // answer does not say it (recorded, `r5-mcp-reconnect-toggle`: a success has no
            // body), so the CLI is asked, and its answer is reported like the one at the
            // start of the run.
            (SessionRequest::McpReconnect(_) | SessionRequest::McpEnabled(..), _) => self.norm.ask_mcp_status(),
            // A helper stopped by the id of its call: whoever asked gets its description with
            // the CLI's (empty) answer, for the record of the stop.
            (SessionRequest::StopTask(id), Ok(said)) => {
                if let (Some(helper), Some(said)) = (self.norm.helpers.get(id.as_str()), said.as_object_mut()) {
                    said.insert("helper".into(), json!(helper.description));
                }
            }
            _ => {}
        }
        // Nobody waits any more (the asker gave up): the request still took effect.
        let _ = reply.send(answer);
    }

    /// Write a prompt to the CLI. An idle CLI starts a turn with it at once. A busy one keeps
    /// it, and its `command_lifecycle` lines say when a turn takes it (`on_lifecycle`).
    async fn send_prompt(&mut self, text: String) {
        // Attached images go as image blocks before the text, which keeps their paths.
        let attached = attachments::images(&text).await;
        if let Some(note) = attached.note {
            self.emit(EventKind::Error { message: note, fatal: false });
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let msg = json!({
            "type": "user", "session_id": "", "parent_tool_use_id": null, "uuid": uuid,
            "message": { "role": "user", "content": attachments::claude_content(&text, &attached.images) },
        });
        if !self.write(&msg).await {
            self.emit(EventKind::Error { message: "could not send the prompt: claude's stdin is closed".into(), fatal: false });
            return;
        }
        if self.busy() {
            self.waiting.push((uuid, text));
        } else {
            self.open_turn(Some(&text));
        }
    }

    /// Shutdown ladder: interrupt (≤ 5 s for the result, or with no turn running for the end
    /// of the background helpers) → stop the background commands (≤ 2 s) → close stdin (≤ 3 s)
    /// → SIGTERM → SIGKILL. With a prompt that still waits in the CLI, or a background command
    /// it did not report as stopped, the CLI would not exit when stdin closes: there SIGTERM
    /// comes at once.
    ///
    /// The interrupt ends every helper, background ones too, with or without a turn running,
    /// and starts no turn (part 4 §1 and §2: `r6-shutdown-helper-sequence-*` with a turn,
    /// `-idle-interrupt-*` without, n=2 each; with a turn the helpers are gone before its
    /// `result`). So with no turn running it is sent only when the CLI's last list names a
    /// helper. A helper never gets `stop_task` here: after it the CLI starts a turn by itself,
    /// which runs to its end also when stdin is closed at once (`-idle-close-now-*`, n=2).
    async fn begin_shutdown(&mut self) {
        if self.stage != Stage::Running {
            return;
        }
        self.queue.clear();
        if !self.turn_active {
            // (A prompt that still waits in the CLI ends the process at once: `wind_down`.)
            if !self.norm.helpers_in_background() || !self.waiting.is_empty() {
                return self.wind_down().await;
            }
            self.control("interrupt", json!({})).await;
            if self.stdin.is_none() {
                return self.wind_down().await; // (it did not reach the CLI: no list will come)
            }
            self.stage = Stage::StoppingHelpers;
            self.deadline = Some(tokio::time::Instant::now() + STOP_INTERRUPT_TIMEOUT);
            return;
        }
        self.norm.interrupt_requested = true;
        // `cancel_queued`: the CLI drops the prompts that wait in it, and its `cancelled` line
        // for each takes it off `waiting` (part 2 §1). Without it the CLI runs them as its next
        // turn the moment this one ends.
        let request = if self.cancels_queued && !self.waiting.is_empty() { json!({ "cancel_queued": true }) } else { json!({}) };
        self.control("interrupt", request).await;
        self.stage = Stage::Interrupting;
        self.deadline = Some(tokio::time::Instant::now() + STOP_INTERRUPT_TIMEOUT);
    }

    /// The stopped agent's turn is over, or none ran, and its helpers are ended
    /// (`begin_shutdown`): end the background commands the CLI still tracks, then close stdin.
    /// With one of them open the CLI does not exit when stdin closes: it waits for the command
    /// and then runs a turn (recorded: 43 s, part 2 §7). An interrupt does not end such a
    /// command (part 3 §4); `stop_task` ends it at once and starts no turn (part 2 §6). The
    /// steps are recorded as one run (part 3 §4, `r5-stop-sequence`, n=1): the interrupt,
    /// `stop_task` right after the stopped turn's `result`, stdin closed once the list is
    /// empty, and the exit a second later with nothing left running. The commands are the ones
    /// of the CLI's last list. (A helper and a command at work at once at a stop is not
    /// recorded as one run.)
    async fn wind_down(&mut self) {
        self.drop_unsent();
        let commands = self.norm.commands_in_background();
        // (A CLI that still holds a prompt is not asked for anything more: its process is ended.)
        if self.left_in_cli || commands.is_empty() || self.stdin.is_none() {
            return self.close_stdin();
        }
        for task_id in commands {
            self.control("stop_task", json!({ "task_id": task_id })).await;
        }
        // A request that could not be written (`write` gave stdin up): the CLI did not get
        // it, so no list will say that its command is gone. With stdin closed the CLI would
        // wait for that command; at SIGTERM it ends it.
        if self.stdin.is_none() {
            return self.kill();
        }
        self.stage = Stage::StoppingTasks;
        self.deadline = Some(tokio::time::Instant::now() + STOP_TASKS_TIMEOUT);
    }

    /// Give up the prompts no turn took: the ones Codly holds and the ones that wait in the CLI.
    fn drop_unsent(&mut self) {
        self.queue.clear();
        self.left_in_cli |= !self.waiting.is_empty();
        self.waiting.clear();
    }

    /// Close stdin, so that the CLI exits by itself. A CLI that still holds a prompt would run
    /// it first (it starts that turn the moment the running one ends): its process is ended.
    /// One that still has background work (a command that did not stop, a subagent) stays
    /// past the limit: at the SIGTERM that follows it ends the commands it tracks (part 2 §7).
    fn close_stdin(&mut self) {
        self.stdin = None;
        if self.left_in_cli {
            return self.kill();
        }
        self.stage = Stage::Closing;
        self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(3));
    }

    /// SIGTERM, then SIGKILL 3 s later. At SIGTERM the CLI ends the background commands it
    /// tracks; after a SIGKILL they run on (part 2 §7).
    fn kill(&mut self) {
        self.stage = Stage::Killing;
        self.deadline = None;
        tokio::spawn(proc::kill_group(self.pgid, Duration::from_secs(3)));
    }

    async fn on_deadline(&mut self) {
        match self.stage {
            // The turn did not end: nothing more is asked of this CLI.
            Stage::Interrupting => {
                self.drop_unsent();
                self.close_stdin();
            }
            // A helper is still listed: the commands are stopped as in any stop, and the steps
            // after them end what is left (at SIGTERM the CLI ends what it tracks).
            Stage::StoppingHelpers => self.wind_down().await,
            // A command is still open as far as the CLI says. With stdin closed the CLI would
            // wait for it; at SIGTERM it ends it.
            Stage::StoppingTasks => {
                self.stdin = None;
                self.kill();
            }
            Stage::Closing => self.kill(),
            // The turn did not end after the interrupt: the CLI is stuck in a tool or a request
            // that ignores it. End the process; the session resumes with the next run.
            Stage::Running => {
                self.interrupt_failed = true;
                let unsent = super::unsent_note(self.queue.len() + self.waiting.len());
                self.drop_unsent();
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
    /// The CLI's id of it as a task, once its `task_started` line said it: the id that the
    /// list of background tasks names.
    task_id: Option<String>,
    /// Why it failed, once a `task_updated` line said it (`patch.error`).
    error: Option<String>,
    done: bool,
}

/// The id of the `mcp_status` requests of a run (the actor's own ids are `codly-<n>`). They
/// share it: which answer is whose is their order (`ClaudeNormalizer::mcp_asked`).
const MCP_STATUS_REQUEST: &str = "codly-mcp-status";

/// Where an MCP server stands, from the CLI's `status` word for it (`system/init` and the
/// answer to `mcp_status`: cli-claude-stream-json-2.1.311 §8, part 2 §18 and §19) and its
/// `error`. A server that wants a sign-in can show as "failed": the CLI tried to register
/// with it and was refused, and its error names the HTTP 401. None = a word Codly does not know.
fn mcp_state(status: &str, error: Option<&str>) -> Option<McpServerStatus> {
    Some(match status {
        "connected" => McpServerStatus::Connected,
        "pending" => McpServerStatus::Connecting,
        "needs-auth" => McpServerStatus::NeedsSignIn,
        "failed" if error.is_some_and(|e| e.contains("HTTP 401")) => McpServerStatus::NeedsSignIn,
        "failed" => McpServerStatus::Failed,
        "disabled" => McpServerStatus::Disabled,
        _ => return None,
    })
}

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
    /// The model id the CLI says the session is on, from the `system/init` line of every turn.
    /// None before the first one, and from a switch Codly asked for until the next one
    /// (`model_switched`).
    session_model: Option<String>,
    /// The other model the agent's own lines last named, once the timeline said so
    /// (`on_turn_model`). None: they name the session's model.
    turn_model: Option<String>,
    /// The permission mode the session is in, when known: from the handshake, from a switch
    /// Codly asked for (the actor sets it), from the CLI's status lines.
    permission_mode: Option<PermissionMode>,
    /// "Claude Code is not signed in: run `…` in a terminal", for this run's account; on the
    /// sign-in of the user's shell, what to look at instead (`startup::sign_in_advice`).
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
    /// The `mcp_status` requests with no answer yet, oldest first: for each, the user's MCP
    /// servers that `system/init` gave as failed when it was written, until its answer says
    /// why (none for a request after a reconnect, a switch or the end of a turn). The CLI answers
    /// its control requests in the order it reads them (as in every recording), so the next
    /// answer is the one to the oldest request.
    mcp_asked: VecDeque<Vec<String>>,
    /// The user's MCP servers as the CLI last gave them, by name: its `status` word and its
    /// reason. A server is reported when one of the two changes.
    mcp: HashMap<String, (String, Option<String>)>,
    /// What runs in the background, as the CLI last listed it (`background_tasks_changed`).
    background: Vec<BackgroundTask>,
    /// How a listed task ended, by its id, in the word of its `task_notification` line: kept
    /// until the list line that follows no longer names the task.
    background_ended: HashMap<String, String>,
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
            session_model: None,
            turn_model: None,
            permission_mode: None,
            sign_in: startup::sign_in_advice(spec),
            own_key: startup::uses_own_key(spec),
            requested_tools: tools_for(spec).iter().map(|t| t.to_string()).collect(),
            status: None,
            init_seen: false,
            blocks: HashMap::new(),
            pending_text: Vec::new(),
            tools: HashMap::new(),
            tools_seen: HashSet::new(),
            helpers: HashMap::new(),
            mcp_asked: VecDeque::new(),
            mcp: HashMap::new(),
            background: Vec::new(),
            background_ended: HashMap::new(),
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
        self.permission_mode = resp["current_permission_mode"].as_str().and_then(PermissionMode::from_cli);
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
        self.turn_opened(Some(text.to_string()))
    }

    /// The CLI started a turn by itself (a background task that ended woke it): no prompt.
    pub fn turn_self_started(&mut self) -> Vec<EventKind> {
        self.turn_opened(None)
    }

    fn turn_opened(&mut self, input: Option<String>) -> Vec<EventKind> {
        let mut out = vec![EventKind::TurnStarted { input }];
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
            "result" => {
                self.on_result(v, &mut out);
                self.ask_about_connecting_mcp();
            }
            "rate_limit_event" => self.on_rate_limit(v, &mut out),
            "control_request" => self.on_control_request(v, &mut out),
            "control_cancel_request" => self.on_cancel(v, &mut out),
            // A heartbeat every 30 s while a tool call runs, with the call's id in
            // `parent_tool_use_id` and no output: a life sign for the watchdog. Recorded for a
            // Bash call (part 2 §8), a tool of an MCP server and the `Agent` call that waits for
            // a subagent (part 3 §3, `r5-tool-progress-mcp` and `-subagent`, n=1 each). A Bash
            // call inside a subagent has none of its own: the `Agent` call's stands for it.
            // WebFetch and WebSearch are not recorded (`watchdog::has_heartbeat`). Every
            // recorded line names its call; one that names none says nothing.
            "tool_progress" => {
                if let Some(call_id) = v["parent_tool_use_id"].as_str() {
                    out.push(EventKind::ToolProgress { call_id: call_id.into() });
                }
            }
            "control_response" => self.on_control_response(&v["response"], &mut out),
            // what the CLI does with each user message: the actor follows it (`on_lifecycle`)
            "command_lifecycle" => {}
            other => out.push(EventKind::Unknown { raw_type: if other.is_empty() { "untyped".into() } else { other.into() } }),
        }
        out
    }

    fn on_system(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let subtype = v["subtype"].as_str().unwrap_or("");
        match subtype {
            "init" => {
                self.on_init(v, out);
                self.on_mcp_servers(&v["mcp_servers"], out);
            }
            "status" => match v["status"].as_str() {
                Some("requesting") => self.set_status(AgentStatus::Thinking, out),
                Some("compacting") => out.push(EventKind::Compacting { pre_tokens: None, post_tokens: None }),
                None => self.on_permission_mode(v, out),
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
            // session title, the CLI's own summaries, a commit that its tool call already
            // shows): known lines, kept in the raw log only.
            "hook_started" | "hook_progress" | "hook_response" | "commands_changed" | "session_title_changed" | "files_persisted"
            | "post_turn_summary" | "vcs_state_changed" => {}
            "task_updated" => self.on_task_updated(v),
            // The whole list of what the CLI still runs in the background, said at every change
            // (cli-claude-stream-json-2.1.311-part2 §6). No request reads it: Codly keeps the last one.
            "background_tasks_changed" => self.on_background_tasks(&v["tasks"], out),
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

    /// A `system/status` line without a status: it names the session's permission mode after a
    /// switch (or the outcome of a compaction, which `compact_boundary` reports). A switch
    /// Codly asked for is known already; one the CLI made itself is reported.
    fn on_permission_mode(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let Some(name) = v["permissionMode"].as_str() else { return };
        match PermissionMode::from_cli(name) {
            Some(mode) if self.permission_mode == Some(mode) => {}
            Some(mode) => {
                self.permission_mode = Some(mode);
                out.push(super::permission_mode_changed(mode));
            }
            // Codly never asks for these modes and never sends back a suggestion to enter one.
            None => {
                self.permission_mode = None;
                self.problem(format!("Claude Code changed its permission mode to {name}, which Codly does not offer"), false, out);
            }
        }
    }

    /// May the CLI leave this tool out without a line in the timeline (`OPTIONAL_TOOLS`)? Not
    /// the plan tool of a session that plans: without it the CLI denies the call that shows the
    /// plan, and no plan could be accepted.
    fn may_be_missing(&self, tool: &str) -> bool {
        OPTIONAL_TOOLS.contains(&tool) && !(tool == EXIT_PLAN_MODE && self.permission_mode == Some(PermissionMode::Plan))
    }

    /// The CLI named the session's model in the `system/init` line of a turn. After a change,
    /// a line on that model is no news (`on_turn_model`).
    fn session_model_is(&mut self, model: &str) {
        if self.session_model.as_deref() != Some(model) {
            self.session_model = Some(model.to_string());
            self.turn_model = None;
        }
    }

    /// The CLI took a model switch that Codly asked for, and whoever asked says so: the
    /// switch is no news of the CLI's. Recorded (part 3 §8, `r5-model-midturn`, n=1): a
    /// switch during a turn takes effect at the next model request of that same turn, so the
    /// turn's later lines name the new model while its `system/init` named the old one, and
    /// the next turn's `system/init` names the new one. A switch while an answer is being
    /// written leaves the rest of that answer and its line on the old model (part 4 §5,
    /// `r6-model-while-streaming-1` and `-2`, n=2). So nothing is said about the model of a
    /// line before the next turn's `system/init` names the session's model again: no "now
    /// runs on" for the model Codly asked for, and no "runs on … again" after it.
    fn model_switched(&mut self) {
        self.session_model = None;
        self.turn_model = None;
    }

    fn on_init(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        if let Some(model) = v["model"].as_str() {
            self.session_model_is(model);
        }
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
        let missing: Vec<&str> = self.requested_tools.iter().map(String::as_str).filter(|t| !has(t) && !self.may_be_missing(t)).collect();
        if !tools.is_empty() && !missing.is_empty() {
            let message = format!("Claude did not enable requested tools: {}", missing.join(", "));
            self.problem(message, false, out);
        }
        if let Some(codly) = v["mcp_servers"].as_array().into_iter().flatten().find(|s| s["name"] == "codly") {
            let status = codly["status"].as_str().unwrap_or("unavailable");
            if status != "connected" {
                self.problem(format!("codly MCP server is {status}"), false, out);
            }
        }
    }

    /// `mcp_servers` of a `system/init` line (every turn starts with one): the user's own
    /// servers (Connections, their Claude setup). Each is reported when its state changes
    /// (`mcp_server`). The line gives no reason for a failed one, so that one is reported
    /// when `mcp_status` has said why.
    fn on_mcp_servers(&mut self, servers: &Value, out: &mut Vec<EventKind>) {
        let mut failed = Vec::new();
        for s in servers.as_array().into_iter().flatten() {
            let (Some(name), Some(status)) = (s["name"].as_str(), s["status"].as_str()) else { continue };
            if name == "codly" {
                continue; // Codly's own server: `on_init` says when it is not there
            }
            if status != "failed" {
                self.mcp_server(name, status, None, out);
            } else if self.mcp.get(name).is_none_or(|(known, _)| known != "failed") && !self.mcp_reason_asked(name) {
                failed.push(name.to_string());
            }
        }
        if !failed.is_empty() {
            self.ask_why_mcp_failed(failed);
        }
    }

    /// Ask the CLI where its MCP servers stand. The actor writes the request (`outbox`) and
    /// gives the answer back (`asked`).
    pub fn ask_mcp_status(&mut self) {
        self.ask_why_mcp_failed(Vec::new());
    }

    /// `ask_mcp_status`, for servers that an init line gave as failed: the answer to this
    /// request, and no other, says why each did not start (`on_control_response`).
    fn ask_why_mcp_failed(&mut self, failed: Vec<String>) {
        self.mcp_asked.push_back(failed);
        self.outbox.push(json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } }));
    }

    /// Does a request with no answer yet wait to say why this server did not start?
    fn mcp_reason_asked(&self, name: &str) -> bool {
        self.mcp_asked.iter().flatten().any(|asked| asked == name)
    }

    /// A turn is over and a server was still connecting when the CLI last gave it: the CLI is
    /// asked where its servers stand now. No line of the turn says how that ended, and a run
    /// of one turn (a worker's task) has no next init line.
    fn ask_about_connecting_mcp(&mut self) {
        if self.mcp.values().any(|(status, _)| status == "pending") {
            self.ask_mcp_status();
        }
    }

    /// One MCP server as the CLI just gave it: an event when its state or its reason is new.
    fn mcp_server(&mut self, name: &str, status: &str, error: Option<&str>, out: &mut Vec<EventKind>) {
        let seen = (status.to_string(), error.map(str::to_owned));
        if self.mcp.get(name) == Some(&seen) {
            return;
        }
        match mcp_state(status, error) {
            Some(state) => out.push(EventKind::McpServer { name: name.to_string(), status: state, error: seen.1.clone() }),
            None => out.push(EventKind::Unknown { raw_type: format!("mcp_server/{status}") }),
        }
        self.mcp.insert(name.to_string(), seen);
    }

    /// Is `resp` (the `response` of a `control_response`) the answer to a request of the
    /// normalizer's own? Every other control response is the actor's.
    pub fn asked(&self, resp: &Value) -> bool {
        resp["request_id"] == MCP_STATUS_REQUEST
    }

    /// The answer to `mcp_status`: where each of the user's servers stands, and one line for
    /// the ones that did not start, each with the CLI's reason ("Connection closed"): the ones
    /// the init line gave as failed, and the ones that were still connecting when the CLI last
    /// gave them and are failed now. A server that started since is left out of that line. So
    /// is one that wants a sign-in: it is the lasting state of a server nobody signed in to,
    /// not a failure of this run, and its state says it. When the CLI cannot answer, the names
    /// the init line gave as failed are still said. Every request has its own answer: the
    /// failed servers of an init line wait for the answer to the request that line made
    /// (`mcp_asked`), not for the answer to an earlier request, which can be older than the
    /// line.
    fn on_control_response(&mut self, resp: &Value, out: &mut Vec<EventKind>) {
        if !self.asked(resp) {
            return;
        }
        let mut failed = self.mcp_asked.pop_front().unwrap_or_default();
        let servers = if resp["subtype"] == "success" { resp["response"]["mcpServers"].as_array() } else { None };
        let reason = |s: &Value| s["error"].as_str().map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned);
        for s in servers.into_iter().flatten() {
            if let (Some(name), Some(status)) = (s["name"].as_str(), s["status"].as_str())
                && name != "codly"
            {
                let was_connecting = self.mcp.get(name).is_some_and(|(known, _)| known == "pending");
                // (one that a later request waits for is said by that request's answer)
                if status == "failed" && was_connecting && !failed.iter().any(|f| f == name) && !self.mcp_reason_asked(name) {
                    failed.push(name.to_string());
                }
                self.mcp_server(name, status, reason(s).as_deref(), out);
            }
        }
        let mut lines = Vec::new();
        for name in failed {
            match servers.and_then(|servers| servers.iter().find(|s| s["name"] == name.as_str())) {
                Some(server) => match (server["status"].as_str(), reason(server)) {
                    (Some("failed"), Some(error)) if mcp_state("failed", Some(&error)) == Some(McpServerStatus::Failed) => lines.push(format!("{name} ({error})")),
                    (Some("failed"), None) => lines.push(name),
                    _ => {}
                },
                // The CLI did not say: the server is failed, as the init line gave it.
                None => {
                    self.mcp_server(&name, "failed", None, out);
                    lines.push(name);
                }
            }
        }
        if !lines.is_empty() {
            self.problem(format!("These MCP servers did not start, so the agent works without their tools in this run: {}", lines.join(", ")), false, out);
        }
    }

    /// `system/task_started`, `task_progress` and `task_notification`: the life of something
    /// that runs beside the agent's own calls. A subagent (`task_type: "local_agent"`) gets
    /// its events. A command (`local_bash`) already is a tool call; one that is left running
    /// in the background is in the CLI's list of background tasks (`on_background_tasks`),
    /// and the word of its `task_notification` says there how it ended. The recorded words:
    /// "completed"; "failed" for a command that ends with another exit code than 0 (part 3
    /// §2, `r5-background-fail`, n=1), for a command whose shell is killed by SIGTERM or
    /// SIGKILL (part 4 §3, `r6-fails-signal-term` and `-kill`, n=1 each: there is no word for
    /// a signal) and for a subagent that fails by itself (part 4 §3, `r6-fails-helper-*`, n=1
    /// each); "stopped" after `stop_task`, for a command (part 2 §6) and for a subagent (part
    /// 3 §1, part 4 §2), and after an interrupt for every subagent (part 4 §1). A command the
    /// agent waits for that has run a while is a task too (`local_bash`, `is_backgrounded`
    /// false): its `task_notification` (its own task id, an empty `output_file`) ends no
    /// subagent and no listed task (part 4 §1).
    fn on_task(&mut self, subtype: &str, v: &Value, out: &mut Vec<EventKind>) {
        if subtype == "task_notification"
            && let (Some(task_id), Some(word)) = (v["task_id"].as_str(), v["status"].as_str())
            && self.background.iter().any(|t| t.id == task_id)
        {
            self.background_ended.insert(task_id.to_string(), word.to_string());
        }
        let Some(call_id) = v["tool_use_id"].as_str() else { return };
        if subtype == "task_started" && v["task_type"] == "local_agent" && !self.helpers.contains_key(call_id) {
            // (its `Agent` call was in no line Codly saw: the subagent starts here)
            self.helper_started(call_id, v["description"].as_str().unwrap_or(""), out);
        }
        self.helper_named(call_id, v["subagent_type"].as_str(), out);
        // (a subagent's end is said once: its row was closed before, by its list)
        let Some(helper) = self.helpers.get_mut(call_id).filter(|h| !h.done) else { return };
        let (tool_uses, tokens) = (v["usage"]["tool_uses"].as_i64(), v["usage"]["total_tokens"].as_i64());
        let text = |key: &str| v[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
        match subtype {
            "task_started" => {
                helper.background = v["is_backgrounded"] == true;
                helper.task_id = v["task_id"].as_str().map(str::to_owned);
            }
            "task_progress" => {
                let step = text("description");
                out.push(EventKind::SubAgentProgress { call_id: call_id.into(), name: None, step, tool_call_id: None, tool_uses, tokens, report: None });
            }
            // task_notification: it ended, and `summary` is what it reported. The line of a
            // subagent that was stopped has the subagent's description there (recorded,
            // `r5-stop-subagent-1` and `-2`): it reported nothing. One that failed reports why:
            // the error of its `task_updated` line, which `summary` repeats (recorded).
            _ => {
                helper.done = true;
                let report = helper.error.take().or_else(|| text("summary").filter(|said| *said != helper.description));
                if report.is_some() || tool_uses.is_some() || tokens.is_some() {
                    out.push(EventKind::SubAgentProgress { call_id: call_id.into(), name: None, step: None, tool_call_id: None, tool_uses, tokens, report });
                }
                let status = v["status"].as_str().unwrap_or("completed").to_string();
                out.push(EventKind::SubAgent { call_id: call_id.into(), description: helper.description.clone(), status });
            }
        }
    }

    /// `system/task_updated`: a task's change of state, which its `task_notification` then
    /// says again. Only the reason of a subagent that failed is kept from it: `patch.error`,
    /// recorded only beside `patch.status` "failed" (part 4 §3, `r6-fails-helper-background`
    /// and `-foreground`, n=1 each). Its end row reports it (`on_task`).
    fn on_task_updated(&mut self, v: &Value) {
        let (Some(task_id), Some(error)) = (v["task_id"].as_str(), v["patch"]["error"].as_str().map(str::trim).filter(|e| !e.is_empty())) else { return };
        if let Some(helper) = self.helpers.values_mut().find(|h| !h.done && h.task_id.as_deref() == Some(task_id)) {
            helper.error = Some(error.to_string());
        }
    }

    /// `system/background_tasks_changed`: the CLI's whole list of what runs in the background
    /// (part 2 §6: a command left running with `run_in_background`, a subagent), said at
    /// every change. The event carries the list and what left it since the last one, each
    /// with the word of its `task_notification` line (recorded: that line comes before the
    /// list line, when a task ends by itself, at `stop_task` and at SIGTERM), or "ended" when
    /// none came. The list is the whole truth each time, so a line that was missed leaves
    /// nothing open. What is listed (part 3 §2, `r5-background-fail`, n=1): a background
    /// command, a background subagent, and a foreground command from the moment the CLI is
    /// told to move it to the background. A foreground subagent or command is in no list.
    /// A task starts with the first list that names it: it keeps that time in later lists.
    fn on_background_tasks(&mut self, tasks: &Value, out: &mut Vec<EventKind>) {
        let mut tasks = background_tasks(tasks);
        for t in &mut tasks {
            t.started_at = self.background.iter().find(|known| known.id == t.id).map_or(Some(now_ms()), |known| known.started_at);
        }
        let gone: Vec<BackgroundTask> = self.background.iter().filter(|t| !tasks.iter().any(|now| now.id == t.id)).cloned().collect();
        // A background subagent that left the list with no `task_notification` (not recorded:
        // at `stop_task` and at its own end that line comes first): its row is closed here,
        // or it would say "running" for good. (A subagent that never was in a list is left
        // as it is: a foreground one is in none.)
        for (call_id, helper) in &mut self.helpers {
            let listed = |t: &BackgroundTask| helper.task_id.as_deref() == Some(t.id.as_str());
            if helper.background && !helper.done && gone.iter().any(listed) {
                helper.done = true;
                out.push(EventKind::SubAgent { call_id: call_id.clone(), description: helper.description.clone(), status: "stopped".into() });
            }
        }
        let ended = gone
            .into_iter()
            .map(|t| {
                let status = self.background_ended.remove(&t.id).unwrap_or_else(|| "ended".into());
                BackgroundEnded { id: t.id, kind: t.kind, description: t.description, status }
            })
            .collect();
        self.background_ended.retain(|id, _| tasks.iter().any(|t| t.id == *id));
        self.background.clone_from(&tasks);
        out.push(EventKind::BackgroundTasks { tasks, ended });
    }

    /// The commands of the CLI's last list of background tasks, by the id that its
    /// `stop_task` request takes.
    fn commands_in_background(&self) -> Vec<String> {
        self.background.iter().filter(|t| t.kind == "command").map(|t| t.id.clone()).collect()
    }

    /// Does the CLI's last list of background tasks name a helper?
    fn helpers_in_background(&self) -> bool {
        self.background.iter().any(|t| t.kind == "helper")
    }

    /// The task id that `stop_task` takes for `id`: a task of the CLI's last list as it is, or
    /// the call id of a helper at work, whose task id its `task_started` line said. `stop_task`
    /// with that id ends a helper whose `Agent` call still waits: the call returns an error
    /// result and the turn goes on (part 4 §2, `r6-shutdown-helper-foreground-stop-*`, n=2).
    /// None: nothing by that id runs beside the agent.
    fn task_to_stop(&self, id: &str) -> Option<String> {
        if self.background.iter().any(|t| t.id == id) {
            return Some(id.to_string());
        }
        self.helpers.get(id).filter(|h| !h.done).and_then(|h| h.task_id.clone())
    }

    fn helper_started(&mut self, call_id: &str, description: &str, out: &mut Vec<EventKind>) {
        let helper = Helper { description: description.to_string(), name: None, background: false, task_id: None, error: None, done: false };
        self.helpers.insert(call_id.to_string(), helper);
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

    /// The model one of the agent's own lines ran on. `system/init` names the session's model,
    /// but a turn can run on another one: recorded in plan mode, where a session on haiku
    /// planned on sonnet while `init.model` said haiku all the time, and went back to haiku
    /// when the plan was accepted (part 2 of the 2.1.311 recordings, scenario 10). The timeline
    /// says so, once per change. The agent's own model, which its next run starts with, stays
    /// the one the user picked.
    fn on_turn_model(&mut self, model: Option<&str>, out: &mut Vec<EventKind>) {
        let (Some(model), Some(session)) = (model.filter(|m| *m != SYNTHETIC_MODEL), self.session_model.as_deref()) else { return };
        if same_model(model, self.turn_model.as_deref().unwrap_or(session)) {
            return;
        }
        if same_model(model, session) {
            self.turn_model = None;
            out.push(EventKind::Notice { text: format!("Claude Code runs on {session} again") });
        } else {
            out.push(EventKind::Notice { text: format!("Claude Code now runs on {model}, not on {session}") });
            self.turn_model = Some(model.to_string());
        }
    }

    fn on_assistant(&mut self, v: &Value, out: &mut Vec<EventKind>) {
        let msg = &v["message"];
        let msg_id = msg["id"].as_str().unwrap_or("").to_string();
        // A subagent's line: its tool calls are shown under it; its text reaches the agent as
        // the result of the `Agent` call (and the user as the report of `task_notification`).
        let subagent = !v["parent_tool_use_id"].is_null();
        let helper = v["parent_tool_use_id"].as_str().filter(|id| self.helpers.contains_key(*id)).map(str::to_owned);
        // (A subagent has a model of its own.)
        if !subagent {
            self.on_turn_model(msg["model"].as_str(), out);
        }
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
        } else if v["parent_tool_use_id"].is_null()
            && !result_text(&v["message"]["content"]).starts_with("[Request interrupted")
            && !written_by_the_cli(v)
        {
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
                } else if !helper.done && (ok || helper.task_id.is_none()) {
                    // (A call that failed does not end a subagent whose task lines Codly
                    // follows: an interrupt rejects the call BEFORE those lines say "killed" /
                    // "stopped": part 4 §1, `r6-interrupt-helper-foreground-*`, n=2. They end
                    // it, or the end of the turn does when none comes.)
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
        // A background subagent works on past the end of a turn that ends as completed: it and
        // its open calls stay. One whose `Agent` call still waited for it is over with the turn.
        // An interrupted turn takes every subagent with it, background ones too, and the stream
        // says so itself before the turn's `result` (`task_notification` "stopped", the list
        // without them: part 4 §1, 6 of 6 recordings, each with ONE subagent in the middle of a
        // Bash call), so none is left here. Not recorded: a turn that FAILS by itself (an API
        // error, a limit) while a background subagent works; such a subagent stays here, as
        // after a completed turn.
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
        let cwd = self.cwd.clone();
        // A host request of the sandbox has one answer, allow or deny. The CLI keeps an allowed
        // host for the rest of its session whatever the scope (read in its bundle, not
        // recorded), so "once" and "for this session" would be the same, and the rule it
        // suggests (a `WebFetch` domain rule, recorded) is not passed back.
        let suggestions = if tool == SANDBOX_NETWORK { vec![] } else { grantable(&tool, &req["permission_suggestions"], &cwd) };
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
            SANDBOX_NETWORK => {
                // The rules read the text of the command that wants the host, not what it
                // does once it runs: this request names the host it really reaches for. Its
                // `tool_use_id` is an id of its own, so it does not say which command that is.
                risks.push(match sandbox_host(&tool, &input) {
                    Some(host) if !host.is_empty() => format!("network ({host})"),
                    _ => "network".into(),
                });
                ApprovalSubject::Tool { name: tool.clone(), input: input.clone() }
            }
            _ => ApprovalSubject::Tool { name: tool.clone(), input: input.clone() },
        };
        // `blocked_path` is the path the call touches (set even inside the worktree, live 2.1.309)
        if req["blocked_path"].as_str().is_some_and(|p| classify::outside(p, &cwd, &cwd)) && !risks.iter().any(|r| r == "outside worktree") {
            risks.push("outside worktree".into());
        }
        // The model asks to run this command outside the shell sandbox: the flag in its input,
        // or the reason the CLI gives for the request (both in the 2.1.311 recordings). Each
        // one is enough: only one case is recorded, so the reason may not come with every flag.
        if input["dangerouslyDisableSandbox"].as_bool() == Some(true) || req["decision_reason_type"].as_str() == Some("sandboxOverride") {
            risks.push(UNSANDBOXED.into());
        }
        let choices = if tool == "AskUserQuestion" {
            vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]
        } else {
            // A session or permanent grant is a permission rule the CLI suggested (`grantable`):
            // without one, "for this session" could only be sent as a one-time allow, so it is
            // not offered.
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
                // A plan the user changed before accepting it (`answers.plan`). The CLI takes the
                // changed text as the plan, rewrites its plan file and tells the model that the
                // user edited it (part 2 of the 2.1.311 recordings, scenario 11).
                if p.tool_name == EXIT_PLAN_MODE
                    && let Some(plan) = d.answers.as_ref().and_then(|a| a["plan"].as_str())
                {
                    input["plan"] = json!(plan);
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

/// The `tasks` of a `system/background_tasks_changed` line (recorded: `task_id`, `task_type`
/// "local_bash" for a command and "local_agent" for a subagent, `description`).
fn background_tasks(tasks: &Value) -> Vec<BackgroundTask> {
    let task = |t: &Value| {
        let kind = match t["task_type"].as_str() {
            Some("local_bash") => "command",
            Some("local_agent") => "helper",
            _ => "task",
        };
        let description = t["description"].as_str().unwrap_or_default().to_string();
        Some(BackgroundTask { id: t["task_id"].as_str()?.to_string(), kind: kind.into(), description, started_at: None })
    };
    tasks.as_array().into_iter().flatten().filter_map(task).collect()
}

/// Do two model names mean one model? A name of the CLI's model list can end in a mark for its
/// context window that a model id does not have ("claude-fable-5-1[1m]" in the `initialize`
/// answer of testdata/claude-interrupt-live.jsonl). Whether `system/init` keeps that mark is
/// not recorded, so it never makes two names differ.
///
/// Nor does a name that is part of the other one. Every recording is of Anthropic's own API.
/// There `system/init` and the agent's lines spell a model the same way for an alias and for
/// a dated id, and differently for an undated name of a dated model: a session started with
/// `claude-haiku-4-5` says that name in `system/init` and `claude-haiku-4-5-20251001` in
/// every line of the agent (part 3 §8, `r5-model-start-undated`, n=1). Another provider is
/// not recorded, and there the two can differ for one model too: the id the session was set
/// up with (a region and vendor in front, a version behind) and the plain id of the answer.
/// A false "now runs on" in every run would be worse than a missed one for two models where
/// one's name starts the other's.
fn same_model(a: &str, b: &str) -> bool {
    fn id(m: &str) -> &str {
        m.split('[').next().unwrap_or(m)
    }
    let (a, b) = (id(a), id(b));
    a.contains(b) || b.contains(a)
}

/// A `user` line the CLI wrote itself, not something the user or a tool said (recorded,
/// cli-claude-stream-json-2.1.311 §2 and §11): the echo of a local command (`isReplay`:
/// "Set model to …" after a model switch, "Compacted" after a compaction) and the summary a
/// compaction puts in place of the conversation (`isSynthetic`). The events of the switch
/// and of the compaction already say what happened.
fn written_by_the_cli(v: &Value) -> bool {
    v["isSynthetic"] == true || (v["isReplay"] == true && result_text(&v["message"]["content"]).starts_with("<local-command-stdout>"))
}

/// The permission suggestions of a request that "for this session" and "always" may send
/// back: allow rules for the tool that asked, and directories inside the agent's own folder.
/// Never `setMode`: sent back, it puts the whole session in another permission mode
/// (recorded, cli-claude-stream-json-2.1.311 §10), which is more than the request you
/// approved. A directory outside `cwd` would let later edits there pass without a question.
fn grantable(tool: &str, suggestions: &Value, cwd: &Path) -> Vec<Value> {
    let keep = |s: &&Value| match s["type"].as_str() {
        Some("addRules") => s["behavior"] == "allow" && s["rules"].as_array().is_some_and(|r| !r.is_empty() && r.iter().all(|r| r["toolName"] == tool)),
        Some("addDirectories") => s["directories"]
            .as_array()
            .is_some_and(|d| !d.is_empty() && d.iter().all(|d| d.as_str().is_some_and(|d| !classify::outside(d, cwd, cwd)))),
        _ => false,
    };
    suggestions.as_array().into_iter().flatten().filter(keep).cloned().collect()
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
            agent_name: "Ada".into(),
            run_id: "run-1".into(),
            provider: Provider::Claude,
            role,
            model: Some("haiku:low".into()),
            cwd: PathBuf::from("/tmp/scratch/fixture-repo"),
            bin: PathBuf::from("claude"),
            env: HashMap::new(),
            session: SessionMode::New { session_id: None },
            session_cost_usd: 0.0,
            system_prompt: "You are a Codly worker.".into(),
            read_only,
            mcp: Some(McpLaunch { command: "/Applications/Codly.app/Contents/MacOS/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), "t".into())] }),
            use_user_setup: false,
            sandbox_network: false,
            local_settings: true,
            raw_log: PathBuf::from("/tmp/x/raw.ndjson"),
            byok_command: None,
            sandbox_shell: false,
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
        assert!(!build_args(&s, "s", None, &[]).contains(&"--settings".to_string()), "no BYOK, no --settings");
        let s = SpawnSpec { byok_command: Some("~/bin/get-token \"anthropic\"".into()), ..s };
        let a = build_args(&s, "s", None, &[]);
        let i = a.iter().position(|x| x == "--settings").unwrap();
        let v: Value = serde_json::from_str(&a[i + 1]).unwrap();
        assert_eq!(v, serde_json::json!({ "apiKeyHelper": "~/bin/get-token \"anthropic\"" }));
    }

    /// The `--settings` value of an argv, parsed. There is at most one.
    fn settings_arg(args: &[String]) -> Option<Value> {
        let at: Vec<usize> = args.iter().enumerate().filter(|(_, x)| *x == "--settings").map(|(i, _)| i).collect();
        assert!(at.len() <= 1, "one --settings document: {args:?}");
        at.first().map(|i| serde_json::from_str(&args[i + 1]).unwrap())
    }

    /// The sandbox goes to the agents that have a shell, as the document recorded in
    /// docs/research/fixtures/claude-2.1.311/r4-sandbox-manual-noauto.ndjson (`sandbox.enabled`
    /// with `autoAllowBashIfSandboxed: false`, so that commands still ask), in one `--settings`
    /// document with the BYOK token command.
    #[test]
    fn the_shell_sandbox_is_passed_to_agents_with_a_shell() {
        let sandbox = json!({ "enabled": true, "failIfUnavailable": true, "autoAllowBashIfSandboxed": false });
        for role in [Role::Worker, Role::Solo] {
            let off = spec(role, false);
            assert_eq!(settings_arg(&build_args(&off, "s", None, &[])), None, "off by default: {role:?}");
            let on = SpawnSpec { sandbox_shell: true, ..off };
            assert_eq!(settings_arg(&build_args(&on, "s", None, &[])), Some(json!({ "sandbox": sandbox })), "{role:?}");
        }
        // Lead and reviewers have no shell to sandbox.
        for role in [Role::Lead, Role::Reviewer] {
            let read_only = SpawnSpec { sandbox_shell: true, ..spec(role, true) };
            assert_eq!(settings_arg(&build_args(&read_only, "s", None, &[])), None, "{role:?}");
            assert!(!tools_for(&read_only).contains(&"Bash"));
        }
        let both = SpawnSpec { sandbox_shell: true, byok_command: Some("op read key".into()), ..spec(Role::Worker, false) };
        assert_eq!(settings_arg(&build_args(&both, "s", None, &[])), Some(json!({ "apiKeyHelper": "op read key", "sandbox": sandbox })));
        // The recorded run used exactly these two keys (`failIfUnavailable` is recorded apart:
        // r4-sandbox-required-available).
        let recorded = &fixture("claude-2.1.311/r4-sandbox-manual-noauto.ndjson")[0]["_argv"];
        let argv: Vec<String> = recorded.as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect();
        assert_eq!(settings_arg(&argv).unwrap()["sandbox"], json!({ "enabled": true, "autoAllowBashIfSandboxed": false }));
    }

    /// With the sandbox on, a shell command is still Codly's rules' to decide: the CLI sends
    /// the ordinary request for it (recorded with `autoAllowBashIfSandboxed: false` in mode
    /// `manual`, the mode of a 1:1 agent), and the answer Codly writes is the recorded one.
    #[test]
    fn a_sandboxed_command_still_asks_the_rules() {
        use crate::model::ApprovalMode;
        use crate::orchestrator::approvals::{PolicyCtx, Verdict, evaluate};
        let lines = fixture("claude-2.1.311/r4-sandbox-manual-noauto.ndjson");
        let mut solo = spec(Role::Solo, false);
        solo.cwd = PathBuf::from(lines[0]["_cwd"].as_str().unwrap());
        let mut norm = ClaudeNormalizer::new(&solo, "s".into());
        let at = lines.iter().position(|l| l["type"] == "control_request").expect("a permission request");
        let a = approval_of(&norm.on_message(&lines[at]));
        assert!(matches!(&a.subject, ApprovalSubject::Command { command, .. } if command == "echo hi > inside.txt"), "{:?}", a.subject);
        assert!(!a.risks.iter().any(|r| r == UNSANDBOXED), "{:?}", a.risks);
        let ctx = |mode| PolicyCtx { mode, worktree: Some(solo.cwd.as_path()), verify: &[], read_only: false, edits: true, checkout: None, own_mcp: &[], provider: Some(Provider::Claude) };
        for mode in [ApprovalMode::Balanced, ApprovalMode::Strict] {
            assert!(matches!(evaluate(&a, &ctx(mode)), Verdict::Ask { .. }), "{mode:?}");
        }
        let (reply, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
        assert_eq!(reply, lines[at + 1]["_sent"], "the recorded allow");
    }

    /// The answer to `get_sandbox_dialog` says whether a run with the sandbox may go on: only
    /// when the sandbox is on and commands ask. Answers as recorded on 2.1.311.
    #[test]
    fn the_sandbox_status_decides_whether_the_run_goes_on() {
        let status = |name: &str| recorded_control(&recording(name), &json!({ "subtype": "get_sandbox_dialog" })).1["response"].clone();
        let refusal = |resp: &Value| sandbox_refusal(resp).map(|why| why.to_string());
        // On, as Codly asks for it: commands ask (`mode: "regular"`).
        for name in ["r4-sandbox-noauto", "r4-sandbox-manual-noauto"] {
            assert_eq!(refusal(&status(name)), None, "{name}");
        }
        // Off: the run would work with no sandbox at all.
        let off = refusal(&status("sandbox-off")).expect("the sandbox is off");
        assert_eq!(off, SandboxNotOn::off().to_string());
        assert!(off.starts_with("Claude Code did not turn its sandbox on"), "{off}");
        // On, but commands would run with no request (`mode: "auto-allow"`).
        let unasked = refusal(&status("r4-sandbox-required-available")).expect("commands would not ask");
        assert!(unasked.contains("without asking"), "{unasked}");
        assert!(unasked.contains("a setting of your organization or of this computer's Claude Code decides that"), "{unasked}");
        // A CLI that does not know the request: nothing says that the sandbox is on, and the
        // way out is a newer Claude Code. (The recorded answer to a request the CLI does not
        // know, `control-requests` `host-35`: the same shape and words, with another name.)
        let unknown = recording("control-requests").into_iter().find(|l| l["response"]["request_id"] == "host-35").expect("the recorded refusal")["response"].clone();
        assert_eq!(unknown["error"], "Unsupported control request subtype: definitely_not_a_subtype");
        let refused = refusal(&unknown).expect("no answer to the check");
        assert_eq!(
            refused,
            "Codly could not check that Claude Code's sandbox is on (Unsupported control request subtype: definitely_not_a_subtype), so it did not start this agent. \
             Update Claude Code (run `claude update` in a terminal), or turn off \"Sandbox Claude's shell\" in Settings → Safety"
        );
        // Another error (made up: none is recorded for this request) is no reason to update.
        let failed = refusal(&json!({ "subtype": "error", "request_id": "codly-2", "error": "sandbox status is not ready" })).expect("no answer to the check");
        assert!(failed.starts_with("Codly could not check") && failed.contains("sandbox status is not ready") && !failed.contains("claude update"), "{failed}");

        // The CLI's own reasons are said. `supported` and `dependencies.errors` are in every
        // recorded answer (`true` and `[]`); the values here are written by hand into the
        // recorded answer of `sandbox-off`: no machine at hand has a sandbox that cannot run.
        let with = |supported: bool, errors: Value| {
            let mut answer = status("sandbox-off");
            assert_eq!((&answer["response"]["supported"], &answer["response"]["dependencies"]["errors"]), (&json!(true), &json!([])));
            answer["response"]["supported"] = json!(supported);
            answer["response"]["dependencies"]["errors"] = errors;
            refusal(&answer).expect("the sandbox is off")
        };
        let missing = with(false, json!(["ripgrep (rg) not found", "bubblewrap (bwrap) is not installed."]));
        assert_eq!(
            missing,
            "Claude Code says its sandbox cannot run here: ripgrep (rg) not found; bubblewrap (bwrap) is not installed. \
             Codly did not start this agent. Fix that, or turn off \"Sandbox Claude's shell\" in Settings → Safety"
        );
        // An error that is no text is shown as the CLI wrote it.
        let odd = with(true, json!([{ "dependency": "bwrap" }]));
        assert!(odd.starts_with("Claude Code says its sandbox cannot run here: {\"dependency\":\"bwrap\"}. Codly did not start this agent."), "{odd}");
        let unsupported = with(false, json!([]));
        assert_eq!(
            unsupported,
            "Claude Code says its sandbox cannot run on this computer, so Codly did not start this agent. \
             To work without the sandbox, turn off \"Sandbox Claude's shell\" in Settings → Safety"
        );
        // They only choose the words: a sandbox that is on as Codly asked for it starts the run.
        let mut on = status("r4-sandbox-noauto");
        on["response"]["dependencies"]["errors"] = json!(["a warning in the wrong list"]);
        assert_eq!(refusal(&on), None);

        // Every refusal has the words by which a task's retry knows it (`retry::needs_you`).
        for why in [off, unasked, refused, failed, missing, odd, unsupported, SandboxNotOn::no_answer().to_string()] {
            assert!(why.contains(SandboxNotOn::NOT_STARTED) && why.ends_with("in Settings → Safety"), "{why}");
        }
    }

    /// The CLI answers the `initialize` request and then not Codly's check of its sandbox.
    /// Before, the start then failed with "claude did not answer the initialize request",
    /// which was not true, and with no type: a 1:1 agent that resumed lost its conversation.
    #[tokio::test]
    async fn a_sandbox_check_with_no_answer_is_not_called_a_missing_initialize_answer() {
        let (_, init) = recorded_control(&recording("r4-sandbox-required-available"), &json!({ "subtype": "initialize", "hooks": null }));
        let init_id = init["response"]["request_id"].as_str().unwrap().to_owned();
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let (asked_tx, mut asked) = oneshot::channel();
        actor.sandbox_check = Some(SandboxCheck::Due { asked: asked_tx });
        let (tx, mut started) = oneshot::channel();
        let mut ready = Some(tx);

        // No answer at all: the `initialize` request is the one without an answer.
        let e = start_timed_out(asked.try_recv().is_ok());
        assert_eq!(e.to_string(), "claude did not answer the initialize request within 90 s");
        assert!(!e.is::<SandboxNotOn>());

        // The handshake is answered, the check is written, and its answer never comes.
        actor.on_json(init, &init_id, &mut ready).await;
        assert_eq!(last_sent(dir.path())["request"], json!({ "subtype": "get_sandbox_dialog" }));
        assert!(ready.is_some() && started.try_recv().is_err() && emitted(&mut events).is_empty(), "not started");
        let e = start_timed_out(asked.try_recv().is_ok());
        assert_eq!(
            e.to_string(),
            "Claude Code did not answer Codly's check of its sandbox within 90 s, so Codly did not start this agent. \
             Try again, or turn off \"Sandbox Claude's shell\" in Settings → Safety"
        );
        assert!(e.is::<SandboxNotOn>(), "the callers that resume keep the conversation: {e:?}");
    }

    /// A run with the sandbox does not start before the CLI says that its sandbox is on:
    /// Codly asks once, right after the handshake, and the session's first event waits for
    /// the answer. A run that must not go on fails its start (`SandboxNotOn`), with no events,
    /// and its process is ended. Request and answers as recorded on 2.1.311.
    #[tokio::test]
    async fn a_run_with_the_sandbox_starts_only_when_the_sandbox_is_on() {
        let asked = json!({ "subtype": "get_sandbox_dialog" });
        let (_, init) = recorded_control(&recording("r4-sandbox-required-available"), &json!({ "subtype": "initialize", "hooks": null }));
        let init_id = init["response"]["request_id"].as_str().unwrap().to_owned();
        // The recording whose sandbox status is the answer, and whether the run then starts.
        for (status_of, starts) in [("r4-sandbox-noauto", true), ("sandbox-off", false), ("r4-sandbox-required-available", false)] {
            let dir = tempfile::tempdir().unwrap();
            let (mut actor, mut events, _child) = silent_actor(dir.path());
            actor.sandbox_check = Some(SandboxCheck::Due { asked: oneshot::channel().0 });
            let (tx, mut started) = oneshot::channel();
            let mut ready = Some(tx);

            actor.on_json(init.clone(), &init_id, &mut ready).await;
            let wrote = last_sent(dir.path());
            let (sent, mut status) = recorded_control(&recording(status_of), &asked);
            assert_eq!((&wrote["type"], &wrote["request"]), (&sent["type"], &sent["request"]), "Codly writes the recorded request");
            assert!(ready.is_some() && started.try_recv().is_err(), "{status_of}: not started before the answer");
            assert!(emitted(&mut events).is_empty(), "{status_of}: no event before the answer");

            status["response"]["request_id"] = wrote["request_id"].clone();
            actor.on_json(status, &init_id, &mut ready).await;
            let outcome = started.try_recv().expect("the start is decided");
            let kinds = emitted(&mut events);
            if starts {
                assert!(outcome.is_ok(), "{status_of}: {outcome:?}");
                assert!(matches!(&kinds[..], [EventKind::SessionStarted { .. }]), "{status_of}: {kinds:?}");
                assert!(actor.stage == Stage::Running && actor.stdin.is_some(), "{status_of}");
            } else {
                let why = outcome.expect_err("the run must not start");
                assert!(why.is::<SandboxNotOn>(), "{status_of}: the refusal has its type, for the callers that resume: {why:?}");
                assert!(why.to_string().contains("did not start this agent"), "{status_of}: {why}");
                assert!(kinds.is_empty(), "{status_of}: a start that failed has no events: {kinds:?}");
                assert!(actor.start_failed && actor.stage == Stage::Closing && actor.stdin.is_none(), "{status_of}: the process is ended");
            }
            assert!(actor.sandbox_check.is_none(), "{status_of}: asked once");
        }

        // A run without the sandbox asks nothing: the handshake alone starts it.
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let (tx, mut started) = oneshot::channel();
        actor.on_json(init, &init_id, &mut Some(tx)).await;
        assert!(started.try_recv().expect("the start is decided").is_ok());
        assert!(matches!(&emitted(&mut events)[..], [EventKind::SessionStarted { .. }]));
        assert!(!std::fs::read_to_string(dir.path().join("raw.ndjson")).unwrap().contains("get_sandbox_dialog"));
    }

    /// A second success answer with the id of `initialize` while the sandbox check waits for
    /// its own answer (no recording has one: the recorded answer, played twice). Before, it
    /// took the waiting check away and started the run, and the CLI's real answer about its
    /// sandbox was then read by nobody. Now the run is not started, the check is asked once,
    /// and its own answer decides.
    #[tokio::test]
    async fn a_second_initialize_answer_does_not_start_a_run_before_the_sandbox_is_checked() {
        let asked = json!({ "subtype": "get_sandbox_dialog" });
        let (_, init) = recorded_control(&recording("r4-sandbox-required-available"), &json!({ "subtype": "initialize", "hooks": null }));
        let init_id = init["response"]["request_id"].as_str().unwrap().to_owned();
        for (status_of, starts) in [("r4-sandbox-noauto", true), ("sandbox-off", false)] {
            let dir = tempfile::tempdir().unwrap();
            let (mut actor, mut events, _child) = silent_actor(dir.path());
            actor.sandbox_check = Some(SandboxCheck::Due { asked: oneshot::channel().0 });
            let (tx, mut started) = oneshot::channel();
            let mut ready = Some(tx);

            actor.on_json(init.clone(), &init_id, &mut ready).await;
            let wrote = last_sent(dir.path());
            assert_eq!(wrote["request"], asked);
            actor.on_json(init.clone(), &init_id, &mut ready).await;
            assert!(ready.is_some() && started.try_recv().is_err(), "{status_of}: not started by the second answer");
            assert!(emitted(&mut events).is_empty(), "{status_of}: no event before the check's answer");
            assert!(matches!(actor.sandbox_check, Some(SandboxCheck::Asked { .. })), "{status_of}: the check still waits for its answer");
            assert_eq!(last_sent(dir.path()), wrote, "{status_of}: asked once");

            let (_, mut status) = recorded_control(&recording(status_of), &asked);
            status["response"]["request_id"] = wrote["request_id"].clone();
            actor.on_json(status, &init_id, &mut ready).await;
            let outcome = started.try_recv().expect("the start is decided");
            assert_eq!(outcome.is_ok(), starts, "{status_of}: {outcome:?}");
            let kinds = emitted(&mut events);
            assert_eq!(kinds.len(), usize::from(starts), "{status_of}: {kinds:?}");
            assert_eq!(actor.start_failed, !starts, "{status_of}");
        }
    }

    /// A settings file saved before the two Claude switches existed loads with both off.
    #[test]
    fn an_older_settings_file_has_the_claude_switches_off() {
        use crate::model::Settings;
        let mut saved = serde_json::to_value(Settings::default()).unwrap();
        for key in ["claude_sandbox", "claude_shell_auth"] {
            assert_eq!(saved.as_object_mut().unwrap().remove(key), Some(json!(false)), "{key} is off by default");
        }
        let old: Settings = serde_json::from_value(saved).unwrap();
        assert!(!old.claude_sandbox && !old.claude_shell_auth);
    }

    /// A command the model wants to run outside the sandbox is the user's to decide in every
    /// approval mode but Full access, and no saved rule covers it. Requests replayed from the
    /// 2.1.311 recordings (sandbox on: `sandboxOverride`; sandbox off: an ordinary request).
    #[test]
    fn a_command_outside_the_sandbox_always_asks() {
        use crate::model::ApprovalMode;
        use crate::orchestrator::approvals::{PolicyCtx, Verdict, evaluate, rule_offer};
        let request = |name: &str| fixture(name).into_iter().find(|l| l["type"] == "control_request").expect("a permission request");
        let worker = spec(Role::Worker, false);
        let mut norm = ClaudeNormalizer::new(&worker, "s".into());

        let recorded = request("claude-2.1.311/sandbox-allow.ndjson");
        assert_eq!(recorded["request"]["input"]["dangerouslyDisableSandbox"], json!(true));
        let a = approval_of(&norm.on_message(&recorded));
        assert!(a.risks.iter().any(|r| r == UNSANDBOXED), "{:?}", a.risks);
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);

        let plain = approval_of(&norm.on_message(&request("claude-2.1.311/sandbox-off.ndjson")));
        assert!(!plain.risks.iter().any(|r| r == UNSANDBOXED), "{:?}", plain.risks);

        // The same request for a command the rules allow by themselves: only the sandbox risk is left.
        let mut harmless = recorded.clone();
        harmless["request"]["input"]["command"] = json!("ls");
        let a = approval_of(&norm.on_message(&harmless));
        assert_eq!(a.risks, vec![UNSANDBOXED.to_string()]);
        let ctx = |mode| PolicyCtx { mode, worktree: Some(worker.cwd.as_path()), verify: &[], read_only: false, edits: true, checkout: None, own_mcp: &[], provider: Some(Provider::Claude) };
        for mode in [ApprovalMode::Balanced, ApprovalMode::Strict, ApprovalMode::Autonomous] {
            assert!(matches!(evaluate(&a, &ctx(mode)), Verdict::Ask { .. }), "{mode:?}");
            assert!(rule_offer(&a, &ctx(mode)).is_empty(), "{mode:?}: no rule to save");
        }
        assert!(matches!(evaluate(&a, &ctx(ApprovalMode::Full)), Verdict::Allow { .. }));

        // The flag alone, without the CLI's reason, and the reason alone: each one asks.
        let mut flag_only = harmless.clone();
        flag_only["request"].as_object_mut().unwrap().remove("decision_reason_type");
        let mut reason_only = harmless.clone();
        reason_only["request"]["input"].as_object_mut().unwrap().remove("dangerouslyDisableSandbox");
        for request in [flag_only, reason_only] {
            let a = approval_of(&norm.on_message(&request));
            assert_eq!(a.risks, vec![UNSANDBOXED.to_string()], "{request}");
            assert!(matches!(evaluate(&a, &ctx(ApprovalMode::Autonomous)), Verdict::Ask { .. }), "{request}");
        }
        // Neither: an ordinary request, which Autonomous allows.
        let mut ordinary = harmless.clone();
        ordinary["request"].as_object_mut().unwrap().remove("decision_reason_type");
        ordinary["request"]["input"].as_object_mut().unwrap().remove("dangerouslyDisableSandbox");
        let a = approval_of(&norm.on_message(&ordinary));
        assert!(a.risks.is_empty(), "{:?}", a.risks);
        assert!(matches!(evaluate(&a, &ctx(ApprovalMode::Autonomous)), Verdict::Allow { .. }));
    }

    /// The host a sandboxed command reaches for is the user's to decide: the sandbox's host
    /// request asks in every approval mode but Full access, with the host on the card and one
    /// answer (allow or deny). The request and both answers as recorded on 2.1.311 (`curl`
    /// in the sandbox: r4-sandbox-network-allow, r4-sandbox-network-deny).
    #[test]
    fn a_host_the_sandbox_does_not_allow_always_asks() {
        use crate::model::ApprovalMode;
        use crate::orchestrator::approvals::{PolicyCtx, Verdict, evaluate, rule_offer};
        let allowed = fixture("claude-2.1.311/r4-sandbox-network-allow.ndjson");
        let at = allowed.iter().position(|l| l["type"] == "control_request").expect("the host request");
        let request = allowed[at].clone();
        assert_eq!(request["request"]["tool_name"], json!("SandboxNetworkAccess"));
        let solo = spec(Role::Solo, false);
        let mut norm = ClaudeNormalizer::new(&solo, "s".into());
        let a = approval_of(&norm.on_message(&request));
        assert!(matches!(&a.subject, ApprovalSubject::Tool { name, input } if name == "SandboxNetworkAccess" && *input == json!({ "host": "example.com" })));
        assert_eq!(a.risks, vec!["network (example.com)".to_string()]);
        assert_eq!(a.reason.as_deref(), Some("Allow network connection to example.com?"));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel], "no session or permanent grant");
        let ctx = |mode| PolicyCtx { mode, worktree: Some(solo.cwd.as_path()), verify: &[], read_only: false, edits: true, checkout: None, own_mcp: &[], provider: Some(Provider::Claude) };
        for mode in [ApprovalMode::Balanced, ApprovalMode::Strict, ApprovalMode::Autonomous] {
            assert!(matches!(evaluate(&a, &ctx(mode)), Verdict::Ask { .. }), "{mode:?}");
            assert!(rule_offer(&a, &ctx(mode)).is_empty(), "{mode:?}: no rule to save");
        }
        assert!(matches!(evaluate(&a, &ctx(ApprovalMode::Full)), Verdict::Allow { .. }));
        // The answer carries no permission rule: the CLI's suggestion is not passed back. It
        // is the answer of the recording, after which the command went on.
        let (reply, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
        assert_eq!(reply["response"]["response"], json!({ "behavior": "allow", "updatedInput": { "host": "example.com" } }));
        assert_eq!(reply, allowed[at + 1]["_sent"]);
        // Deny, as recorded: the command then ends with an error of the sandbox.
        let denied = fixture("claude-2.1.311/r4-sandbox-network-deny.ndjson");
        let at = denied.iter().position(|l| l["type"] == "control_request").expect("the host request");
        let a = approval_of(&norm.on_message(&denied[at]));
        let (reply, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }).unwrap();
        assert_eq!(reply, denied[at + 1]["_sent"]);

        // A request that names no host still asks.
        let mut nameless = request.clone();
        nameless["request"]["input"] = json!({});
        let a = approval_of(&norm.on_message(&nameless));
        assert_eq!(a.risks, vec!["network".to_string()]);
        assert!(matches!(evaluate(&a, &ctx(ApprovalMode::Autonomous)), Verdict::Ask { .. }));
    }

    #[test]
    fn args_for_a_worker() {
        let s = spec(Role::Worker, false);
        let a = build_args(&s, "11111111-2222-4333-8444-555555555555", Some(Path::new("/tmp/x/claude-mcp.json")), &[]);
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
        let cfg = mcp_config(&s, &[]).unwrap();
        assert_eq!(cfg["mcpServers"]["codly"]["alwaysLoad"], true);
        assert_eq!(cfg["mcpServers"]["codly"]["env"]["CODLY_MCP_TOKEN"], "t");
    }

    /// The default servers are looked up once per spawn: the MCP config and `--allowedTools`
    /// are made from that one list, so a server is never in one and missing from the other.
    #[test]
    fn the_mcp_config_and_the_allowed_tools_name_the_same_default_servers() {
        let s = spec(Role::Worker, false);
        let server = |name: &str, read_only: bool| ReadyMcp {
            name: name.into(),
            command: "npx".into(),
            args: vec!["-y".into(), format!("@example/{name}")],
            env: vec![("API_KEY".into(), format!("key-of-{name}"))],
            read_only,
        };
        let ready = [server("docs", true), server("tracker", false)];
        let cfg = mcp_config(&s, &ready).unwrap();
        let names: Vec<&String> = cfg["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(names, ["codly", "docs", "tracker"]);
        assert_eq!(cfg["mcpServers"]["tracker"], json!({ "type": "stdio", "command": "npx", "args": ["-y", "@example/tracker"], "env": { "API_KEY": "key-of-tracker" } }));
        let a = build_args(&s, "sid", Some(Path::new("/tmp/x/claude-mcp.json")), &ready);
        let i = a.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(a[i + 1], "mcp__codly__*,mcp__docs__*", "a read-only server runs without asking; the other asks first");
        assert!(!a.join(" ").contains("key-of-"), "keys go in the 0600 file, never in argv");
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
        let file = PrivateFile::write(&path, &mcp_config(&spec(Role::Worker, false), &[]).unwrap().to_string()).unwrap();
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
        let joined = build_args(&s, "abc", None, &[]).join(" ");
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
        assert_eq!(delta, Some(("msg-2:0".into(), "DONE".into())));
        let msg = events.iter().find_map(|e| match e {
            EventKind::Message { item_id, text, phase } => Some((item_id.clone(), text.clone(), *phase)),
            _ => None,
        });
        assert_eq!(msg, Some(("msg-2:0".into(), "DONE".into(), MessagePhase::Final)));

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
        norm.requested_tools.retain(|t| t != EXIT_PLAN_MODE); // the capture is older than the plan tool: its run did not ask for it
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
        let joined = |s: &SpawnSpec| build_args(s, "sid", None, &[]).join(" ");
        let a = joined(&sp);
        assert!(a.contains("Monitor,TaskStop,Skill,LSP") && a.contains("--strict-mcp-config") && a.contains("user,local"));
        assert!(!a.contains("ToolSearch") && !a.contains("Cron") && !a.contains("ScheduleWakeup") && !a.contains("RemoteTrigger"));
        sp.use_user_setup = true;
        let a = joined(&sp);
        assert!(!a.contains("--strict-mcp-config") && a.contains("user,project,local") && a.contains("ToolSearch"));
        // In a Codly worktree the local settings file never loads: an agent can write it.
        sp.local_settings = false;
        assert!(joined(&sp).contains("--setting-sources user,project --"));
        // A folder whose own configuration is not trusted: the user's settings alone, and
        // only Codly's MCP servers.
        sp.use_user_setup = false;
        let a = joined(&sp);
        assert!(a.contains("--setting-sources user --") && a.contains("--strict-mcp-config") && !a.contains("ToolSearch"));
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
        // On the sign-in of the user's shell (Settings → Safety), the login command is not the fix.
        let mut token = spec(Role::Worker, false);
        token.env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), "test-token".into());
        let mut token = ClaudeNormalizer::new(&token, "s".into());
        token.turn_started("f");
        let ev = token.on_message(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "OAuth token has expired" }));
        assert_eq!(
            errors(&ev),
            [("Claude Code is not signed in: the sign-in your shell selects (CLAUDE_CODE_OAUTH_TOKEN) was not accepted. Run `claude auth status` in a terminal to see why, then try again", false)]
        );
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
        for subtype in ["hook_started", "hook_progress", "hook_response", "commands_changed", "session_title_changed", "files_persisted", "post_turn_summary", "task_updated"] {
            assert!(norm.on_message(&system(subtype, json!({}))).is_empty(), "{subtype}");
        }
        // The line the CLI writes after a commit, as recorded (the commit's tool call shows it).
        let commit = recorded("r4-sandbox-worktree-plain").into_iter().find(|l| l["subtype"] == "vcs_state_changed").expect("the recorded line");
        assert_eq!((&commit["kind"], &commit["branch"]), (&json!("commit"), &json!("main")));
        assert!(norm.on_message(&commit).is_empty());
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
        // Each of the others has its state at once; a failed one waits for its reason.
        use McpServerStatus::{Connected, Connecting, Failed, NeedsSignIn};
        assert_eq!(mcp_states(&ev), [("linear", NeedsSignIn, None), ("slow", Connecting, None), ("git", Connected, None)]);
        assert_eq!(norm.outbox.last(), Some(&json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } })));
        // A CLI that does not know the request (its recorded answer to an unknown subtype):
        // the names are still said.
        let refused = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": MCP_STATUS_REQUEST, "error": "Unsupported control request subtype: mcp_status" } });
        let ev = norm.on_message(&refused);
        assert_eq!(errors(&ev), [("These MCP servers did not start, so the agent works without their tools in this run: context7, docs", false)]);
        assert_eq!(mcp_states(&ev), [("context7", Failed, None), ("docs", Failed, None)]);

        // The next turn's init line: only what changed is said, and a server that is still
        // failed is not asked about again. A status word Codly does not know is not dropped.
        norm.outbox.clear();
        let ev = norm.on_message(&system("init", json!({ "tools": [], "mcp_servers": [
            { "name": "codly", "status": "connected" }, { "name": "context7", "status": "failed" }, { "name": "linear", "status": "needs-auth" },
            { "name": "slow", "status": "connected" }, { "name": "git", "status": "sleeping" }, { "name": "docs", "status": "failed" }
        ] })));
        assert_eq!(mcp_states(&ev), [("slow", Connected, None)]);
        assert!(matches!(&ev[..], [_, EventKind::Unknown { raw_type }] if raw_type == "mcp_server/sleeping"), "{ev:?}");
        assert!(norm.outbox.is_empty());
    }

    /// A server that still connects at a turn's init line: the CLI is asked about it once more
    /// when the turn is over (a run of one turn has no next init line), so the server does not
    /// stay "connecting" for the whole run, and one that failed meanwhile is said. The init
    /// line with `pending` was not recorded; the status word is (`r4-mcp-needs-auth`, `host-1`).
    #[test]
    fn a_server_that_was_still_connecting_is_asked_about_when_the_turn_is_over() {
        use McpServerStatus::{Connected, Connecting, Failed};
        let asked = json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } });
        let init = json!({ "type": "system", "subtype": "init", "tools": [], "mcp_servers": [
            { "name": "codly", "status": "connected" }, { "name": "slow", "status": "pending" }, { "name": "late", "status": "pending" }, { "name": "git", "status": "connected" }
        ] });
        let result = json!({ "type": "result", "subtype": "success", "is_error": false, "result": "done", "duration_ms": 5 });
        // The answer in the recorded shape (`mcpServers`: name, status, and `error` on a failed one).
        let answer = |servers: Value| json!({ "type": "control_response", "response": { "subtype": "success", "request_id": MCP_STATUS_REQUEST, "response": { "mcpServers": servers } } });

        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("go");
        let ev = norm.on_message(&init);
        assert_eq!(mcp_states(&ev), [("slow", Connecting, None), ("late", Connecting, None), ("git", Connected, None)]);
        assert!(norm.outbox.is_empty(), "nothing failed: nothing to ask during the turn");
        let ev = norm.on_message(&result);
        assert!(matches!(ev.iter().find(|e| matches!(e, EventKind::TurnCompleted { .. })), Some(EventKind::TurnCompleted { status: TurnStatus::Completed, .. })), "{ev:?}");
        assert_eq!(norm.outbox, [asked.clone()]);

        // One failed meanwhile, one connected: each has its state, and the failure is said.
        norm.outbox.clear();
        let ev = norm.on_message(&answer(json!([
            { "name": "slow", "status": "failed", "error": "Connection closed" }, { "name": "late", "status": "connected" }, { "name": "git", "status": "connected" }
        ])));
        assert_eq!(mcp_states(&ev), [("slow", Failed, Some("Connection closed")), ("late", Connected, None)]);
        assert_eq!(errors(&ev), [("These MCP servers did not start, so the agent works without their tools in this run: slow (Connection closed)", false)]);
        // No server connects any more: the next turn's end asks nothing.
        norm.turn_started("again");
        norm.on_message(&result);
        assert!(norm.outbox.is_empty());

        // A CLI that cannot answer: the server is not called failed, nothing is said, and the
        // next turn's end asks again.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("go");
        norm.on_message(&init);
        norm.on_message(&result);
        assert_eq!(norm.outbox, [asked.clone()]);
        let refused = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": MCP_STATUS_REQUEST, "error": "Unsupported control request subtype: mcp_status" } });
        assert!(norm.on_message(&refused).is_empty());
        norm.outbox.clear();
        norm.turn_started("again");
        norm.on_message(&result);
        assert_eq!(norm.outbox, [asked]);
    }

    /// Two `mcp_status` requests with no answer yet: one after a reconnect, then the one of a
    /// turn's init line that names servers as failed. Each answer is handled for its own
    /// request. Before, the two shared one list of failed servers and the first answer took
    /// it, so the reconnect's answer spoke for the init line: with these answers one server
    /// was said there and the other in a second line. Recording `broken-mcp`: the init line
    /// and the answers `host-1` (one server still connects) and `host-2` (both failed), in an
    /// order written by hand.
    #[test]
    fn each_mcp_status_request_has_its_own_answer() {
        use McpServerStatus::{Connecting, Failed};
        let lines = recorded("broken-mcp");
        let init = lines.iter().find(|l| l["subtype"] == "init").unwrap();
        let ours = |host: &str| {
            let mut answer = lines.iter().find(|l| l["response"]["request_id"] == host).unwrap_or_else(|| panic!("no answer {host}")).clone();
            answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
            answer
        };
        let enoent = "ENOENT: no such file or directory, posix_spawn 'stdio'";
        let both = "These MCP servers did not start, so the agent works without their tools in this run: \
                    exits-at-once (Connection closed), no-such-command (ENOENT: no such file or directory, posix_spawn 'stdio')";

        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.ask_mcp_status(); // as after the answer to a reconnect (`Actor::session_answered`)
        norm.turn_started("Reply with the single word: pong");
        norm.on_message(init);
        assert_eq!(norm.outbox.len(), 2, "two requests wait for their answers");
        // The answer to the first request: where the servers stand, and no word yet about
        // the ones the init line named.
        let ev = norm.on_message(&ours("host-1"));
        assert_eq!(mcp_states(&ev), [("exits-at-once", Connecting, None), ("no-such-command", Failed, Some(enoent))]);
        assert!(errors(&ev).is_empty(), "the init line's request has its own answer: {ev:?}");
        // The answer to the init line's request: both, in one line, each with its reason.
        let ev = norm.on_message(&ours("host-2"));
        assert_eq!(mcp_states(&ev), [("exits-at-once", Failed, Some("Connection closed"))]);
        assert_eq!(errors(&ev), [(both, false)]);
        assert!(norm.on_message(&ours("host-2")).is_empty(), "said once");

        // Two init lines before the first answer: the servers are asked about once, and said once.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.on_message(init);
        norm.on_message(init);
        assert_eq!(norm.outbox.len(), 1, "the first request still waits to say why");
        assert_eq!(errors(&norm.on_message(&ours("host-2"))), [(both, false)]);
        assert!(norm.mcp_asked.is_empty());
    }

    /// A server that still connected at the end of a turn is asked about (first request).
    /// Before that answer comes, the next turn's init line gives the server as failed, so a
    /// second request waits to say why. The first answer already gives it as failed: it
    /// reports the server's state and leaves the line to the request that carries the server.
    /// Without that, both answers said the line: two lines for one failure. (Lines written
    /// here in the recorded shapes: an init line with `pending` is not recorded.)
    #[test]
    fn a_failed_server_that_a_later_request_waits_for_is_said_by_that_requests_answer() {
        use McpServerStatus::{Connecting, Failed};
        let init = |status: &str| json!({ "type": "system", "subtype": "init", "tools": [], "mcp_servers": [{ "name": "codly", "status": "connected" }, { "name": "slow", "status": status }] });
        let result = json!({ "type": "result", "subtype": "success", "is_error": false, "result": "done", "duration_ms": 5 });
        let answer = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": MCP_STATUS_REQUEST, "response": {
            "mcpServers": [{ "name": "slow", "status": "failed", "error": "Connection closed" }]
        } } });

        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("go");
        assert_eq!(mcp_states(&norm.on_message(&init("pending"))), [("slow", Connecting, None)]);
        norm.on_message(&result);
        assert_eq!(norm.outbox.len(), 1, "asked at the turn's end: the server still connected");
        norm.turn_started("again");
        assert!(mcp_states(&norm.on_message(&init("failed"))).is_empty(), "an init line gives no reason: the answer reports the server");
        assert_eq!(norm.outbox.len(), 2, "a second request, for the reason");
        assert!(norm.mcp_reason_asked("slow"));

        let first = norm.on_message(&answer);
        assert_eq!(mcp_states(&first), [("slow", Failed, Some("Connection closed"))]);
        assert!(errors(&first).is_empty(), "the request that carries the server says the line: {first:?}");
        let second = norm.on_message(&answer);
        assert!(mcp_states(&second).is_empty(), "its state is known: {second:?}");
        assert_eq!(errors(&second), [("These MCP servers did not start, so the agent works without their tools in this run: slow (Connection closed)", false)]);
        assert!(norm.mcp_asked.is_empty());
    }

    /// The MCP server events, as (name, state, reason).
    fn mcp_states(events: &[EventKind]) -> Vec<(&str, McpServerStatus, Option<&str>)> {
        events.iter().filter_map(|e| if let EventKind::McpServer { name, status, error } = e { Some((name.as_str(), *status, error.as_deref())) } else { None }).collect()
    }

    /// The CLI's stdout lines of a 2.1.311 recording (docs/research/fixtures/claude-2.1.311),
    /// without the lines of the recording driver (`_note`, `_t`, `_sent`, `_stderr`, `_meta`).
    fn recorded(name: &str) -> Vec<Value> {
        fixture(&format!("claude-2.1.311/{name}.ndjson")).into_iter().filter(|l| l.get("type").is_some()).collect()
    }

    /// While a long command runs the CLI writes a heartbeat every 30 s (recorded: a loop of
    /// 96 s, three heartbeats). Each becomes a live event that names the call: a life sign for
    /// the watchdog, and no row of the timeline.
    #[test]
    fn a_long_command_gives_a_life_sign_every_30_s() {
        let lines = recorded("r4-tool-progress-output");
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
        norm.turn_started("go");
        let events = replay(&mut norm, &lines);
        let at = |wanted: fn(&EventKind) -> bool| events.iter().enumerate().filter(|(_, e)| wanted(e)).map(|(i, _)| i).collect::<Vec<_>>();
        let beats = at(|e| matches!(e, EventKind::ToolProgress { call_id } if call_id == "toolu-1"));
        let started = at(|e| matches!(e, EventKind::ToolStarted { call_id, .. } if call_id == "toolu-1"));
        let completed = at(|e| matches!(e, EventKind::ToolCompleted { call_id, .. } if call_id == "toolu-1"));
        assert_eq!(beats.len(), 3, "{events:?}");
        assert!(started[0] < beats[0] && beats[2] < completed[0], "while the call is open");
        assert!(beats.iter().all(|i| events[*i].is_ephemeral()), "live only: never stored");
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type == "tool_progress")));
        // A line with no call to name (not recorded) says nothing.
        assert!(norm.on_message(&json!({ "type": "tool_progress", "elapsed_time_seconds": 30 })).is_empty());
    }

    /// Recordings `r5-tool-progress-mcp` and `r5-tool-progress-subagent` (part 3 §3, n=1
    /// each): a tool of an MCP server that takes 100 s, and an `Agent` call that waits 100 s
    /// for its subagent. Each gets the heartbeat a long Bash call gets, with the call's id.
    /// The subagent's own Bash call gets none: the event after its start says whose call it
    /// is, which is how the watchdog knows not to wait for a heartbeat of it.
    #[test]
    fn a_long_mcp_call_and_a_call_that_waits_for_a_subagent_give_a_life_sign_every_30_s() {
        let played = |name: &str| {
            let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
            let mut events = norm.turn_started("go");
            events.extend(replay(&mut norm, &recorded(name)));
            events
        };
        let beats = |events: &[EventKind]| -> Vec<String> { events.iter().filter_map(|e| if let EventKind::ToolProgress { call_id } = e { Some(call_id.clone()) } else { None }).collect() };

        let mcp = played("r5-tool-progress-mcp");
        assert_eq!(tool_calls(&mcp), [("toolu-1", "mcp__ping-slow__ping")]);
        assert_eq!(beats(&mcp), ["toolu-1", "toolu-1", "toolu-1"]);

        let helper = played("r5-tool-progress-subagent");
        assert_eq!(tool_calls(&helper), [("toolu-1", "Agent"), ("toolu-2", "Bash")]);
        assert_eq!(beats(&helper), ["toolu-1", "toolu-1", "toolu-1"], "the heartbeats are the Agent call's");
        let own = helper.iter().position(|e| matches!(e, EventKind::ToolStarted { call_id, .. } if call_id == "toolu-2")).unwrap();
        assert!(matches!(&helper[own + 1], EventKind::SubAgentProgress { call_id, tool_call_id: Some(id), .. } if call_id == "toolu-1" && id == "toolu-2"), "{:?}", helper[own + 1]);
        let first_beat = helper.iter().position(|e| matches!(e, EventKind::ToolProgress { .. })).unwrap();
        let ended = helper.iter().position(|e| matches!(e, EventKind::ToolCompleted { call_id, .. } if call_id == "toolu-2")).unwrap();
        assert!(own < first_beat && first_beat < ended, "they came while the subagent's call ran");
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

    /// The `BackgroundTasks` events, one line each: "<what runs> | <what ended>", a task as
    /// "<id> <kind>: <description>", an ended one with the word of its end after its kind.
    fn background_lines(events: &[EventKind]) -> Vec<String> {
        let or_none = |said: Vec<String>| if said.is_empty() { "-".to_string() } else { said.join(", ") };
        events
            .iter()
            .filter_map(|e| match e {
                EventKind::BackgroundTasks { tasks, ended } => {
                    let runs = tasks.iter().map(|t| format!("{} {}: {}", t.id, t.kind, t.description)).collect();
                    let ended = ended.iter().map(|t| format!("{} {} {}: {}", t.id, t.kind, t.status, t.description)).collect();
                    Some(format!("{} | {}", or_none(runs), or_none(ended)))
                }
                _ => None,
            })
            .collect()
    }

    /// A `system/background_tasks_changed` line with these tasks. Written here, in the
    /// recorded shape: the tests that use it say what no recording has.
    fn list_line(tasks: Value) -> Value {
        json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": tasks })
    }

    /// Recordings `r4-background-life`, `subagent` and `r4-background-and-queue-*`: the CLI
    /// says its whole list of background tasks at every change, a command and a subagent
    /// alike, and an empty list when the last one ended. Each event says the list and what
    /// left it, in the CLI's own word: "completed" for one that ended by itself, "stopped"
    /// for one the host stopped with `stop_task`.
    #[test]
    fn the_list_of_background_tasks_is_told_at_every_change_with_what_ended() {
        let said = |name: &str| {
            let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
            let mut events = norm.turn_started("go");
            events.extend(replay(&mut norm, &recorded(name)));
            (background_lines(&events), subagent_lines(&events))
        };
        let (lines, helpers) = said("r4-background-life");
        assert_eq!(
            lines,
            [
                "tid-1 command: sleep 8; echo woke | -",
                "- | tid-1 command completed: sleep 8; echo woke",
                "tid-2 command: sleep 127; echo late | -",
                "- | tid-2 command stopped: sleep 127; echo late",
            ]
        );
        assert_eq!(helpers, Vec::<String>::new(), "a command is no subagent");
        assert_eq!(said("subagent").0, ["tid-1 helper: Run echo hi via Bash | -", "- | tid-1 helper completed: Run echo hi via Bash"]);
        // A command the agent waits for (`is_backgrounded: false`) runs meanwhile: it is in
        // no list, and its own `task_notification` ends nothing here.
        for name in ["r4-background-and-queue-fold", "r4-background-and-queue-later"] {
            assert!(recorded(name).iter().any(|l| l["subtype"] == "task_started" && l["is_backgrounded"] == false), "{name}");
            assert_eq!(said(name).0, ["tid-1 command: sleep 12; echo woke | -", "- | tid-1 command completed: sleep 12; echo woke"], "{name}");
        }
        // A command the agent waits for, alone (`r4-tool-progress`): no list at all.
        assert_eq!(said("r4-tool-progress").0, Vec::<String>::new());
    }

    /// Recording `r5-background-fail` (part 3 §2, n=1). A background command that ends with
    /// exit code 1 leaves the list with the CLI's word "failed". A subagent the agent waits
    /// for is in no list, and neither is a command the agent waits for, until the CLI is told
    /// to move it to the background (the driver's `background_tasks` request, which Codly
    /// does not send): from then on it is listed, and its end is said like any other.
    #[test]
    fn a_background_command_that_fails_leaves_the_list_as_failed() {
        let lines = recorded("r5-background-fail");
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("go");
        events.extend(replay(&mut norm, &lines));
        assert_eq!(
            background_lines(&events),
            [
                "tid-1 command: Run sleep 4 then exit 1 in background | -",
                "- | tid-1 command failed: Run sleep 4 then exit 1 in background",
                "tid-4 command: Echo a, sleep 20 seconds, echo b | -",
                "- | tid-4 command completed: Echo a, sleep 20 seconds, echo b",
            ]
        );
        // The foreground subagent of the third turn: its row, and no list line.
        assert!(lines.iter().any(|l| l["subtype"] == "task_started" && l["task_type"] == "local_agent" && l["is_backgrounded"] == false));
        assert_eq!(
            subagent_lines(&events),
            [
                "toolu-2 running: Run echo with sleep in subagent",
                "toolu-2 name=general-purpose",
                "toolu-2 step=Running Echo a, sleep 6 seconds, echo b uses=1 tokens=5187",
                "toolu-2 call=toolu-3",
                "toolu-2 report=```\na\nb\n``` uses=1 tokens=6042",
                "toolu-2 completed: Run echo with sleep in subagent",
            ]
        );
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { .. })), "{events:?}");
    }

    /// What no recording has: a kind of task Codly has no name for, an entry with no id, and
    /// a task that leaves the list with no `task_notification` before (a line that was
    /// missed). The list is the whole list each time, so nothing stays open.
    #[test]
    fn a_task_that_leaves_the_list_with_no_word_of_its_end_is_said_as_ended() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let command = json!({ "task_id": "tid-1", "task_type": "local_bash", "description": "npm run dev" });
        let odd = json!({ "task_id": "tid-9", "task_type": "remote", "description": "x" });
        let first = norm.on_message(&list_line(json!([command, odd, { "description": "no id" }])));
        assert_eq!(background_lines(&first), ["tid-1 command: npm run dev, tid-9 task: x | -"]);
        assert_eq!(norm.commands_in_background(), ["tid-1"], "what a stop of the agent ends with `stop_task`");
        // The word of an end is kept for a task of the list only, until the list drops it.
        let ended = |task: &str| json!({ "type": "system", "subtype": "task_notification", "task_id": task, "tool_use_id": "toolu-1", "status": "completed" });
        assert!(norm.on_message(&ended("tid-9")).is_empty() && norm.on_message(&ended("tid-7")).is_empty());
        let second = norm.on_message(&list_line(json!([])));
        assert_eq!(background_lines(&second), ["- | tid-1 command ended: npm run dev, tid-9 task completed: x"]);
        assert!(norm.commands_in_background().is_empty() && norm.background_ended.is_empty());
        // Nothing left the list: nothing ended.
        assert_eq!(background_lines(&norm.on_message(&list_line(json!([])))), ["- | -"]);
    }

    /// Recording `subagent`, cut after the subagent started, then a list line written here
    /// that no longer names it. (The order task line, list line is recorded; a subagent
    /// that leaves the list with no `task_notification` is not.) Its row is closed, so it
    /// does not say "running" for good. A subagent that never was in a list keeps its row.
    #[test]
    fn a_background_subagent_that_leaves_the_list_with_no_end_line_is_closed() {
        let lines = recorded("subagent");
        let cut = lines.iter().position(|l| l["subtype"] == "task_started").unwrap() + 1;
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("go");
        events.extend(replay(&mut norm, &lines[..cut]));
        assert_eq!(subagent_lines(&events), ["toolu-1 running: Run echo hi via Bash", "toolu-1 name=general-purpose"]);
        let gone = norm.on_message(&list_line(json!([])));
        assert_eq!(subagent_lines(&gone), ["toolu-1 stopped: Run echo hi via Bash"]);
        assert_eq!(background_lines(&gone), ["- | tid-1 helper ended: Run echo hi via Bash"]);
        assert!(subagent_lines(&norm.on_message(&list_line(json!([])))).is_empty(), "closed once");

        // As recorded, its `task_notification` comes before the list drops it: one end, in
        // the CLI's word (`a_background_subagent_is_followed_past_the_turn_that_started_it`).
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let all = subagent_lines(&replay(&mut norm, &lines));
        assert_eq!(all.iter().filter(|l| l.contains("stopped") || l.contains("completed")).collect::<Vec<_>>(), ["toolu-1 completed: Run echo hi via Bash"]);

        // The same lines without the CLI's lists: the subagent was in no list Codly saw, so
        // a list that does not name it says nothing about it.
        let unlisted: Vec<Value> = lines[..cut].iter().filter(|l| l["subtype"] != "background_tasks_changed").cloned().collect();
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        replay(&mut norm, &unlisted);
        assert!(subagent_lines(&norm.on_message(&list_line(json!([])))).is_empty());
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
        // Each server's state, with the CLI's reason.
        use McpServerStatus::{Disabled, Failed};
        let enoent = "ENOENT: no such file or directory, posix_spawn 'stdio'";
        assert_eq!(mcp_states(&ev), [("exits-at-once", Failed, Some("Connection closed")), ("no-such-command", Failed, Some(enoent))]);
        assert!(norm.on_message(&answer).is_empty(), "said once");

        // A server that is no longer failed when the CLI answers (`host-6`: one was switched
        // off meanwhile) is left out.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.on_message(init);
        let mut answer = lines.iter().find(|l| l["response"]["request_id"] == "host-6").unwrap().clone();
        answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
        let ev = norm.on_message(&answer);
        assert_eq!(errors(&ev), [("These MCP servers did not start, so the agent works without their tools in this run: exits-at-once (Connection closed)", false)]);
        assert_eq!(mcp_states(&ev), [("exits-at-once", Failed, Some("Connection closed")), ("no-such-command", Disabled, None)]);
    }

    /// Recording `r4-mcp-needs-auth`: a server that answers 401 to everything. The CLI gives
    /// it as failed, and the reason names the refused sign-in step: it needs a sign-in, which
    /// is no failure of the run. Recording `r4-project-mcp-project`: a server that connected.
    #[test]
    fn an_mcp_server_that_wants_a_sign_in_says_so() {
        use McpServerStatus::{Connected, Connecting, Disabled, NeedsSignIn};
        let lines = recorded("r4-mcp-needs-auth");
        let refused = "Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}";
        let ours = |host: &str| {
            let mut answer = lines.iter().find(|l| l["response"]["request_id"] == host).unwrap_or_else(|| panic!("no answer {host}")).clone();
            answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
            answer
        };
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        norm.turn_started("Reply with the single word: pong");
        let inits: Vec<&Value> = lines.iter().filter(|l| l["subtype"] == "init").collect();
        let ev = norm.on_message(inits[0]);
        assert!(mcp_states(&ev).is_empty(), "the init line has no reason: {ev:?}");
        assert_eq!(norm.outbox.len(), 1, "Codly asks why");

        // `host-2`: failed, with the reason. No line about a server that "did not start".
        let ev = norm.on_message(&ours("host-2"));
        assert_eq!(mcp_states(&ev), [("needs-login", NeedsSignIn, Some(refused))]);
        assert!(errors(&ev).is_empty(), "{ev:?}");
        // The next turn's init line gives it as failed again: nothing new, nothing asked.
        norm.outbox.clear();
        assert!(mcp_states(&norm.on_message(inits[1])).is_empty());
        assert!(norm.outbox.is_empty());
        // `host-6`: switched off. `host-8`: on again, and refused again.
        assert_eq!(mcp_states(&norm.on_message(&ours("host-6"))), [("needs-login", Disabled, None)]);
        let ev = norm.on_message(&ours("host-8"));
        assert_eq!(mcp_states(&ev), [("needs-login", NeedsSignIn, Some(refused))]);
        assert!(errors(&ev).is_empty(), "{ev:?}");

        // `host-1`, asked before the first turn: the CLI still tries.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        assert_eq!(mcp_states(&norm.on_message(&ours("host-1"))), [("needs-login", Connecting, None)]);

        // A project's server that started: connected, said once.
        let lines = recorded("r4-project-mcp-project");
        let init = lines.iter().find(|l| l["subtype"] == "init").unwrap();
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
        assert_eq!(mcp_states(&norm.on_message(init)), [("ping-project", Connected, None)]);
        assert!(norm.outbox.is_empty());
        assert!(mcp_states(&norm.on_message(init)).is_empty());
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
            waiting: Vec::new(),
            interrupts: HashMap::new(),
            left_in_cli: false,
            lifecycle: false,
            cancels_queued: false,
            turn_active: false,
            next_req: 0,
            session_replies: HashMap::new(),
            stage: Stage::Running,
            deadline: None,
            start_failed: false,
            interrupt_failed: false,
            _mcp_file: None,
            sandbox_check: None,
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

    /// The user messages Codly wrote to the CLI so far.
    fn prompts_sent(dir: &Path) -> Vec<Value> {
        let log = std::fs::read_to_string(dir.join("raw.ndjson")).unwrap();
        log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["codly_sent"].clone()).filter(|m| m["type"] == "user").collect()
    }

    /// The turn accounting in `kinds`, a word per event: "start:<the prompt's first word>"
    /// ("start" for a turn no prompt started), "steer:<first word>", "end:<status>".
    fn turns(kinds: &[EventKind]) -> Vec<String> {
        let word = |text: &str| text.split_whitespace().next().unwrap_or_default().to_string();
        kinds
            .iter()
            .filter_map(|k| match k {
                EventKind::TurnStarted { input: Some(text) } => Some(format!("start:{}", word(text))),
                EventKind::TurnStarted { input: None } => Some("start".into()),
                EventKind::PromptSteered { input } => Some(format!("steer:{}", word(input))),
                EventKind::TurnCompleted { status, .. } => Some(format!("end:{status:?}")),
                _ => None,
            })
            .collect()
    }

    /// A recording of the real CLI (docs/research/fixtures/claude-2.1.311) played to an actor:
    /// each user message its driver sent is a prompt for the actor, each stdout line is a line
    /// the actor reads. The recording's `user-msg-N` (the driver's uuid on its N-th message)
    /// stands for the uuid the actor put on the N-th message it wrote. A control request of
    /// the driver is not the actor's to send: the test gives the actor the command that makes
    /// it write that request (`in_place_of_the_drivers_request`).
    struct Replay {
        actor: Actor,
        events: mpsc::UnboundedReceiver<AdapterEvent>,
        lines: VecDeque<Value>,
        /// The control request of the driver that `play` passed last.
        drivers_request: Option<Value>,
        /// The driver's request ids that stand for a request of the actor: recorded id → the actor's.
        answers: HashMap<String, Value>,
        dir: tempfile::TempDir,
        _child: tokio::process::Child,
    }

    impl Replay {
        fn of(name: &str) -> Replay {
            let dir = tempfile::tempdir().unwrap();
            let (actor, events, child) = silent_actor(dir.path());
            let lines = fixture(&format!("claude-2.1.311/{name}.ndjson")).into();
            Replay { actor, events, lines, drivers_request: None, answers: HashMap::new(), dir, _child: child }
        }

        fn written(&self) -> Vec<Value> {
            prompts_sent(self.dir.path())
        }

        /// `user-msg-N` in a recorded line becomes the uuid of the N-th message the actor
        /// wrote. One the actor has not written (it holds that prompt back) stays as it is.
        fn own_uuid(&self, recorded: &mut Value) {
            let nth = recorded.as_str().and_then(|id| id.strip_prefix("user-msg-")).and_then(|n| n.parse::<usize>().ok());
            if let Some(uuid) = nth.and_then(|n| self.written().get(n - 1).map(|m| m["uuid"].clone())) {
                *recorded = uuid;
            }
        }

        /// Play the recording up to and with the first line `stop` accepts (to its end when
        /// none does). Returns what the actor emitted meanwhile.
        async fn play(&mut self, stop: impl Fn(&Value) -> bool) -> Vec<EventKind> {
            while let Some(mut line) = self.lines.pop_front() {
                let last = stop(&line);
                if line["_sent"]["type"] == "user" {
                    let text = line["_sent"]["message"]["content"].as_str().unwrap().to_string();
                    self.actor.on_cmd(AgentCmd::Prompt(text)).await;
                } else if line["_sent"]["type"] == "control_request" {
                    self.drivers_request = Some(line["_sent"].clone());
                } else if line.as_object().is_some_and(|o| !o.keys().any(|k| k.starts_with('_'))) {
                    if line["type"] == "command_lifecycle" {
                        self.own_uuid(&mut line["command_uuid"]);
                    } else if line["type"] == "control_response" {
                        let own_request = line["response"]["request_id"].as_str().and_then(|id| self.answers.get(id)).cloned();
                        if let Some(id) = own_request {
                            line["response"]["request_id"] = id;
                        }
                        // the uuids an interrupt's answer lists
                        for list in ["still_queued", "cancelled"] {
                            let mut uuids = line["response"]["response"][list].as_array().cloned().unwrap_or_default();
                            if !uuids.is_empty() {
                                uuids.iter_mut().for_each(|uuid| self.own_uuid(uuid));
                                line["response"]["response"][list] = uuids.into();
                            }
                        }
                    }
                    self.actor.on_json(line, "codly-0", &mut None).await;
                }
                if last {
                    break;
                }
            }
            emitted(&mut self.events)
        }

        /// Give the actor `cmd` at the point where the driver wrote the control request that
        /// `play` passed last. The actor must write that same request; the CLI's recorded
        /// answer to it then comes back as the answer to the actor's.
        async fn in_place_of_the_drivers_request(&mut self, cmd: AgentCmd) {
            let recorded = self.drivers_request.take().expect("play passed a control request of the driver");
            self.actor.on_cmd(cmd).await;
            let wrote = last_sent(self.dir.path());
            assert_eq!((&wrote["type"], &wrote["request"]), (&recorded["type"], &recorded["request"]), "Codly writes the recorded request");
            self.answers.insert(recorded["request_id"].as_str().unwrap().to_string(), wrote["request_id"].clone());
        }
    }

    const BANANA: &str = "also say the word banana at the end";

    /// A message sent while a turn runs is written at once, with a uuid. It waits in the CLI
    /// until the next tool boundary, where the running turn takes it: one turn, one result.
    #[tokio::test]
    async fn a_message_sent_mid_turn_goes_into_the_running_turn() {
        let mut cli = Replay::of("mid-turn-user-message-uuid");
        let kinds = cli.play(|l| l["_sent"]["uuid"] == "user-msg-2").await;
        assert_eq!(turns(&kinds), ["start:Run"]);
        let written = cli.written();
        assert_eq!(written.len(), 2, "not held until the turn ends");
        assert!(written.iter().all(|m| m["uuid"].as_str().is_some_and(|u| u.len() == 36) && m.get("priority").is_none()), "{written:?}");
        assert_ne!(written[0]["uuid"], written[1]["uuid"]);
        assert_eq!(cli.actor.waiting.len(), 1, "it waits until the CLI says a turn took it");

        // the first tool call ends: the CLI takes the message into the turn
        let kinds = cli.play(|l| l["type"] == "command_lifecycle" && l["state"] == "started").await;
        assert!(kinds.iter().any(|k| matches!(k, EventKind::ToolCompleted { .. })), "{kinds:?}");
        assert!(matches!(kinds.last(), Some(EventKind::PromptSteered { input }) if input == BANANA), "{kinds:?}");
        assert!(cli.actor.waiting.is_empty() && cli.actor.turn_active);

        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["end:Completed"], "one result ends the one turn");
        assert!(kinds.iter().any(|k| matches!(k, EventKind::TurnCompleted { result_text: Some(text), .. } if text == "done banana")), "{kinds:?}");
        assert!(!cli.actor.turn_active && cli.actor.queue.is_empty());
    }

    /// A message the running turn did not take is a turn of its own: the CLI starts it when
    /// the turn has ended, and says so. (This recording made the CLI wait with
    /// `priority: "later"`. Codly sends no priority; whatever makes the CLI wait, the actor
    /// follows its lines.)
    #[tokio::test]
    async fn a_message_the_turn_did_not_take_is_the_next_turn() {
        let mut cli = Replay::of("mid-turn-user-message-uuid-later");
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed"]);
        assert!(!cli.actor.turn_active && cli.actor.waiting.len() == 1, "between the two turns the message still waits");

        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["start:also", "end:Completed"]);
        assert!(kinds.iter().any(|k| matches!(k, EventKind::TurnStarted { input: Some(text) } if text == BANANA)), "{kinds:?}");
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty());
    }

    /// The CLI can end the running turn early for a message (`priority: "now"` in this
    /// recording): the stopped turn ends as interrupted, the `cancelled` line of the message
    /// that had started it sends nothing again, and the message that waited is the next turn.
    #[tokio::test]
    async fn a_turn_stopped_for_a_message_ends_before_that_messages_turn() {
        let mut cli = Replay::of("mid-turn-user-message-uuid-now");
        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Interrupted", "start:also", "end:Completed"]);
        assert_eq!(cli.written().len(), 2);
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty());
    }

    /// A background task that ends wakes the CLI: it starts a turn no prompt asked for. The
    /// agent is at work again, that turn has its own start and end, and its result is not
    /// taken for the answer to a prompt sent meanwhile.
    #[tokio::test]
    async fn a_turn_the_cli_starts_by_itself_has_its_own_start_and_end() {
        let mut cli = Replay::of("background-wakeup");
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed"]);
        assert!(matches!(kinds.last(), Some(EventKind::StatusChanged { status: AgentStatus::Idle })), "{kinds:?}");

        // the task ends 5.5 s later, and the CLI opens a turn (`system/init`)
        let kinds = cli.play(|l| l["subtype"] == "init").await;
        assert_eq!(turns(&kinds), ["start"]);
        assert!(matches!(kinds.last(), Some(EventKind::StatusChanged { status: AgentStatus::Thinking })), "the agent shows as working: {kinds:?}");
        assert!(cli.actor.turn_active);

        // a prompt sent now waits in the CLI for a turn to take it
        cli.actor.on_cmd(AgentCmd::Prompt("fix the test".into())).await;
        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["end:Completed"]);
        assert_eq!(cli.written().len(), 2);
        assert!(!cli.actor.turn_active && cli.actor.waiting.len() == 1, "the result closed the CLI's own turn, not the prompt's");
    }

    /// What Codly still holds until the turn ends, on a CLI that takes messages mid-turn: the
    /// prompt behind a stop (it is the next turn, not a part of the stopped one) and a slash
    /// command, with what was sent after it.
    #[tokio::test]
    async fn a_prompt_behind_a_stop_and_a_slash_command_wait_for_the_turns_end() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.lifecycle = true;
        let stopped = json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 });
        let done = json!({ "type": "result", "subtype": "success", "is_error": false, "result": "ok", "duration_ms": 5 });

        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Interrupt).await;
        actor.on_cmd(AgentCmd::Prompt("try something different".into())).await;
        assert_eq!((prompts_sent(dir.path()).len(), actor.queue.len(), actor.waiting.len()), (1, 1, 0));
        emitted(&mut events);
        actor.on_json(stopped, "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["end:Interrupted", "start:try"]);

        actor.on_cmd(AgentCmd::Prompt("/compact".into())).await;
        actor.on_cmd(AgentCmd::Prompt("then test it".into())).await;
        assert_eq!((prompts_sent(dir.path()).len(), actor.queue.len(), actor.waiting.len()), (2, 2, 0), "in order, behind the slash command");
        actor.on_json(done, "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["end:Completed", "start:/compact"]);
        assert_eq!((prompts_sent(dir.path()).len(), actor.queue.len(), actor.waiting.len()), (4, 0, 1));

        // A turn that does not stop: what the CLI had not taken yet is lost with its process.
        actor.on_cmd(AgentCmd::Interrupt).await;
        actor.on_deadline().await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[0], EventKind::Error { message, .. } if message.contains("1 message that waited for this turn was not sent")), "{kinds:?}");
        assert!(actor.waiting.is_empty());
    }

    /// A message the CLI gives up before a turn took it is sent again when the turn has ended:
    /// a `cancelled` message never reached the model. (The line is recorded for a waiting
    /// message that the host withdrew, `r4-cancel-async-message`. Codly withdraws none while
    /// the agent runs, so here the CLI gave it up by itself, which no recording shows.)
    #[tokio::test]
    async fn a_waiting_message_the_cli_drops_is_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.lifecycle = true;
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("also run the tests".into())).await;
        let uuid = actor.waiting[0].0.clone();
        actor.on_json(json!({ "type": "command_lifecycle", "command_uuid": uuid, "state": "cancelled" }), "codly-0", &mut None).await;
        assert_eq!((prompts_sent(dir.path()).len(), actor.queue.len(), actor.waiting.len()), (2, 1, 0));
        emitted(&mut events);

        actor.on_json(json!({ "type": "result", "subtype": "success", "is_error": false, "result": "ok" }), "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["end:Completed", "start:also"]);
        assert_eq!(prompts_sent(dir.path()).len(), 3);
    }

    /// Stop while a message waits in the CLI, as recorded: the answer to the interrupt names
    /// the message (`still_queued`), and the CLI starts its turn in the instant the stopped one
    /// ends. Codly follows that: it does not send the message again, and the agent does not
    /// look busy afterwards.
    #[tokio::test]
    async fn a_message_that_waits_in_the_cli_at_a_stop_runs_next_and_is_not_sent_again() {
        let mut cli = Replay::of("r4-queue-interrupt-plain");
        let kinds = cli.play(|l| l["_sent"]["type"] == "control_request").await;
        assert_eq!(turns(&kinds), ["start:Run"]);
        assert_eq!((cli.actor.waiting.len(), cli.actor.queue.len()), (1, 0), "written at once: it waits in the CLI");

        cli.in_place_of_the_drivers_request(AgentCmd::Interrupt).await;
        let kinds = cli.play(|l| l["type"] == "control_response").await;
        assert!(turns(&kinds).is_empty(), "{kinds:?}");
        assert_eq!((cli.actor.waiting.len(), cli.actor.queue.len()), (1, 0), "the CLI keeps it: nothing to send again");
        assert!(cli.actor.interrupts.is_empty());

        // the stopped turn's result, then `started` for the message, its turn and its result
        let kinds = cli.play(|l| l["_meta"]["results_seen"] == 2).await;
        assert_eq!(turns(&kinds), ["end:Interrupted", "start:Reply", "end:Completed"]);
        assert!(kinds.iter().any(|k| matches!(k, EventKind::TurnCompleted { result_text: Some(text), .. } if text == "banana")), "{kinds:?}");
        assert_eq!(cli.written().len(), 2, "each message was written once");
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty() && cli.actor.deadline.is_none());
    }

    /// Two messages sent while a turn runs. The CLI takes both at the next tool boundary, in
    /// the order sent, and the turn ends with one result.
    #[tokio::test]
    async fn two_messages_sent_mid_turn_go_into_the_running_turn_together() {
        let mut cli = Replay::of("r4-two-queued-tools");
        let kinds = cli.play(|l| l["_sent"]["uuid"] == "user-msg-3").await;
        assert_eq!(turns(&kinds), ["start:Run"]);
        assert_eq!((cli.written().len(), cli.actor.waiting.len()), (3, 2));

        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["steer:also", "steer:and", "end:Completed"]);
        assert!(kinds.iter().any(|k| matches!(k, EventKind::TurnCompleted { result_text: Some(text), .. } if text == "done banana cherry")), "{kinds:?}");
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty());
    }

    /// The same two messages behind a turn that has no tool call: nothing takes them until the
    /// turn ends. The CLI then runs both as ONE new turn: the first opens it, the second joins
    /// it, and one result ends it. Every prompt is accounted for once.
    #[tokio::test]
    async fn two_messages_the_turn_did_not_take_are_one_next_turn() {
        let mut cli = Replay::of("r4-two-queued-notool");
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["start:Write", "end:Completed"]);
        assert_eq!((cli.written().len(), cli.actor.waiting.len()), (3, 2), "both still wait");

        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["start:also", "steer:and", "end:Completed"]);
        assert_eq!(cli.written().len(), 3);
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty());
    }

    /// A slash command sent while a turn runs. The recorded CLI got it mid-turn and ran it as
    /// a turn of its own right after the running one. Codly holds it until the turn ends and
    /// writes it then: the same turns in the same order, each with its own result.
    #[tokio::test]
    async fn a_slash_command_sent_mid_turn_is_the_turn_after() {
        for (name, command) in [("r4-slash-mid-turn-context", "/context"), ("r4-slash-mid-turn-compact", "/compact")] {
            let mut cli = Replay::of(name);
            let kinds = cli.play(|l| l["_sent"]["uuid"] == "user-msg-2").await;
            assert_eq!(turns(&kinds), ["start:Run"], "{name}");
            assert_eq!((cli.written().len(), cli.actor.queue.len(), cli.actor.waiting.len()), (1, 1, 0), "{name}: held until the turn ends");

            let kinds = cli.play(|l| l["type"] == "result").await;
            assert_eq!(turns(&kinds), ["end:Completed", format!("start:{command}").as_str()], "{name}");
            assert_eq!((cli.written().len(), cli.actor.queue.len()), (2, 0), "{name}");

            let kinds = cli.play(|_| false).await;
            assert_eq!(turns(&kinds), ["end:Completed"], "{name}: the command's own result ends its turn");
            assert_eq!(kinds.iter().any(|k| matches!(k, EventKind::Compacting { .. })), command == "/compact", "{name}: {kinds:?}");
            assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty(), "{name}");
        }
    }

    /// A background command ends while a turn runs and a message waits. The running turn takes
    /// the message at its next tool boundary, and the CLI starts no turn of its own for the
    /// command that ended: nothing is left open.
    #[tokio::test]
    async fn a_background_command_that_ends_mid_turn_starts_no_turn_of_its_own() {
        let mut cli = Replay::of("r4-background-and-queue-fold");
        let kinds = cli.play(|l| l["_sent"]["uuid"] == "user-msg-3").await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed", "start:Run"]);
        assert_eq!(cli.actor.norm.commands_in_background(), ["tid-1"], "the command of the first turn still runs");
        assert_eq!(cli.actor.waiting.len(), 1);

        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["steer:Reply", "end:Completed"]);
        assert!(cli.actor.norm.commands_in_background().is_empty());
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty());

        // The CLI made the message wait for the turn's end (`priority: "later"`, which Codly
        // does not send): the turn that runs still uses up the command's end, then the
        // message is a turn of its own.
        let mut cli = Replay::of("r4-background-and-queue-later");
        let kinds = cli.play(|_| false).await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed", "start:Run", "end:Completed", "start:Reply", "end:Completed"]);
        assert!(!cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.norm.commands_in_background().is_empty());
    }

    /// Stop while the actor counts a message as waiting in the CLI, and the CLI's answer names
    /// none (`still_queued: []`: the recorded answer of a stop with nothing waiting, played to
    /// an actor that wrote a message the recorded CLI never got). The CLI does not hold it, so
    /// it is Codly's to send again, as the turn after the stopped one.
    #[tokio::test]
    async fn a_message_that_waited_in_the_cli_at_a_stop_is_the_next_turn() {
        let mut cli = Replay::of("control-requests");
        cli.play(|l| l["response"]["request_id"] == "host-41").await;
        assert!(cli.actor.turn_active && cli.actor.lifecycle, "the turn with `sleep 20` runs");
        cli.actor.on_cmd(AgentCmd::Prompt("also run the tests".into())).await;
        let written = cli.written().len();
        assert_eq!((cli.actor.waiting.len(), cli.actor.queue.len()), (1, 0), "written at once: it waits in the CLI");

        // the recording's interrupt and its answer, for the request the actor wrote
        let stop = cli.lines.pop_front().unwrap();
        assert_eq!(stop["_sent"]["request"]["subtype"], "interrupt");
        cli.actor.on_cmd(AgentCmd::Interrupt).await;
        let mut answer = cli.lines.pop_front().unwrap();
        assert_eq!(answer["response"]["response"], json!({ "still_queued": [] }));
        answer["response"]["request_id"] = json!(format!("codly-{}", cli.actor.next_req));
        cli.actor.on_json(answer, "codly-0", &mut None).await;
        assert_eq!((cli.actor.waiting.len(), cli.actor.queue.len()), (0, 1), "the CLI no longer has it: Codly holds it until the turn has ended");
        assert_eq!(cli.written().len(), written);

        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["end:Interrupted", "start:also"]);
        assert_eq!(cli.written().len(), written + 1, "sent again, as the next turn");
        assert!(cli.actor.turn_active && cli.actor.waiting.is_empty() && cli.actor.queue.is_empty() && cli.actor.interrupts.is_empty());
    }

    /// Two stops, one before the message and one after it (lines written here in the recorded
    /// shapes: `still_queued` lists plain uuid strings). An answer for an interrupt written
    /// before the message is not about it; the answer that names it leaves it with the CLI,
    /// whose `started` line opens its turn.
    #[tokio::test]
    async fn a_message_the_cli_keeps_through_a_stop_is_not_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.lifecycle = true;
        let answer = |n: u64, still_queued: Value| {
            json!({ "type": "control_response", "response": { "subtype": "success", "request_id": format!("codly-{n}"), "response": { "still_queued": still_queued } } })
        };
        actor.on_cmd(AgentCmd::Interrupt).await; // idle: nothing waits
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("also run the tests".into())).await;
        let uuid = actor.waiting[0].0.clone();
        actor.on_json(answer(1, json!([])), "codly-0", &mut None).await;
        assert_eq!((actor.waiting.len(), actor.queue.len()), (1, 0), "the first stop came before the message");

        actor.on_cmd(AgentCmd::Interrupt).await;
        actor.on_json(answer(2, json!([uuid])), "codly-0", &mut None).await;
        assert_eq!((actor.waiting.len(), actor.queue.len()), (1, 0));
        emitted(&mut events);
        actor.on_json(json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 }), "codly-0", &mut None).await;
        actor.on_json(json!({ "type": "command_lifecycle", "command_uuid": uuid, "state": "started" }), "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["end:Interrupted", "start:also"]);
        assert_eq!(prompts_sent(dir.path()).len(), 2);
    }

    /// Stop agent (or quit) while a message waits in the CLI, as recorded: the interrupt says
    /// `cancel_queued`, the CLI drops the message (`cancelled`) and only the stopped turn's
    /// result follows. Nothing is left for the CLI to run, so stdin is closed and it may exit
    /// by itself.
    #[tokio::test]
    async fn a_stopped_agent_has_the_cli_drop_the_message_that_waited_in_it() {
        let mut cli = Replay::of("r4-queue-interrupt-cancel-queued");
        cli.play(|l| l["_sent"]["type"] == "control_request").await;
        assert_eq!(cli.actor.waiting.len(), 1);

        cli.in_place_of_the_drivers_request(AgentCmd::Shutdown).await;
        assert!(cli.actor.stage == Stage::Interrupting && cli.actor.stdin.is_some());
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["end:Interrupted"]);
        assert!(cli.actor.waiting.is_empty() && !cli.actor.left_in_cli, "the CLI dropped it");
        assert!(cli.actor.stage == Stage::Closing && cli.actor.stdin.is_none() && cli.actor.deadline.is_some(), "{:?}", cli.actor.stage);
        assert_eq!(cli.written().len(), 2, "the dropped message is not sent again");

        // What the recording's driver did next (a third message) is not this agent's any more.
        let kinds = cli.play(|_| false).await;
        assert!(kinds.is_empty(), "{kinds:?}");
        assert_eq!(cli.written().len(), 2);
    }

    /// The same stop on a CLI that does not drop the message (this recording's CLI was not
    /// asked to, and starts the message's turn in the instant the running turn ends). A stopped
    /// agent must not do that work: the message is given up, the process is ended as soon as
    /// the turn is (no wait for an exit by itself), and the turn the CLI starts meanwhile is
    /// not shown, so the exit leaves no open turn to fail.
    #[tokio::test]
    async fn a_stopped_agent_does_not_run_the_message_that_waited_in_the_cli() {
        let mut cli = Replay::of("mid-turn-user-message-uuid-now");
        cli.play(|l| l["_sent"]["uuid"] == "user-msg-2").await;
        assert_eq!(cli.actor.waiting.len(), 1);

        cli.actor.on_cmd(AgentCmd::Shutdown).await;
        assert!(cli.actor.stage == Stage::Interrupting && cli.actor.waiting.len() == 1, "no `cancelled` line for it yet");
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["end:Interrupted"]);
        assert!(cli.actor.stage == Stage::Killing && cli.actor.stdin.is_none(), "{:?}", cli.actor.stage);

        // `started` for the message, `system/init`, two tool calls, the answer, a `result`
        let kinds = cli.play(|_| false).await;
        assert!(kinds.is_empty(), "{kinds:?}");
        assert!(!cli.actor.turn_active);
    }

    /// Without a message in the CLI the stop keeps its steps: stdin is closed and the CLI has
    /// 3 s to exit by itself.
    #[tokio::test]
    async fn a_stopped_agent_with_nothing_in_the_cli_may_exit_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("then test it".into())).await; // no lifecycle lines: Codly holds it
        actor.on_cmd(AgentCmd::Shutdown).await;
        assert!(actor.stage == Stage::Interrupting && actor.queue.is_empty());
        emitted(&mut events);
        actor.on_json(json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 }), "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["end:Interrupted"]);
        assert!(actor.stage == Stage::Closing && actor.deadline.is_some());
    }

    /// A stopped agent whose turn does not end within the limit: stdin is closed anyway, and a
    /// CLI that still holds a message gets no time to exit by itself.
    #[tokio::test]
    async fn a_stop_does_not_wait_for_a_turn_that_does_not_end() {
        for waits_in_the_cli in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (mut actor, _events, _child) = silent_actor(dir.path());
            actor.lifecycle = waits_in_the_cli;
            actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
            actor.on_cmd(AgentCmd::Prompt("then test it".into())).await;
            assert_eq!(actor.waiting.len(), usize::from(waits_in_the_cli));
            actor.on_cmd(AgentCmd::Shutdown).await;
            assert_eq!(actor.stage, Stage::Interrupting);
            actor.on_deadline().await;
            assert_eq!(actor.stage, if waits_in_the_cli { Stage::Killing } else { Stage::Closing });
            assert!(actor.stdin.is_none() && actor.waiting.is_empty() && actor.queue.is_empty());
        }
    }

    /// Stop agent while a background command still runs, as recorded. With stdin closed the
    /// CLI would wait for the command and then run a turn. So the command is stopped first
    /// (`stop_task`), and stdin is closed when the CLI says that nothing runs in the
    /// background any more.
    #[tokio::test]
    async fn a_stopped_agent_ends_its_background_command_before_stdin_is_closed() {
        let mut cli = Replay::of("r4-background-life");
        // Two turns that each start a background command, and the turn the CLI starts by
        // itself when the first command ends. The driver stops the second command 2 s later.
        let kinds = cli.play(|l| l["_sent"]["request"] == json!({ "subtype": "stop_task", "task_id": "tid-2" })).await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed", "start", "end:Completed", "start:Run", "end:Completed"]);
        assert_eq!(cli.actor.norm.commands_in_background(), ["tid-2"]);

        cli.in_place_of_the_drivers_request(AgentCmd::Shutdown).await;
        assert!(cli.actor.stage == Stage::StoppingTasks && cli.actor.stdin.is_some() && cli.actor.deadline.is_some(), "{:?}", cli.actor.stage);

        // `task_updated` (killed), `task_notification` (stopped), then the empty list
        let kinds = cli.play(|l| l["subtype"] == "background_tasks_changed").await;
        assert!(kinds.is_empty(), "not shown as this agent's work: {kinds:?}");
        assert!(cli.actor.norm.commands_in_background().is_empty());
        assert!(cli.actor.stage == Stage::Closing && cli.actor.stdin.is_none() && cli.actor.deadline.is_some(), "{:?}", cli.actor.stage);
    }

    /// Recording `r5-stop-sequence` (part 3 §4, n=1): the whole stop of an agent as one run of
    /// the CLI. A background command runs, a turn runs a command, a third message waits in the
    /// CLI. The driver sends what Codly sends, in Codly's order: the interrupt with
    /// `cancel_queued`, `stop_task` right after the stopped turn's `result`, then it closes
    /// stdin (the recorded CLI exits a second later, with neither command left running).
    #[tokio::test]
    async fn a_stop_is_an_interrupt_then_stop_task_then_stdin_closed_as_recorded() {
        let mut cli = Replay::of("r5-stop-sequence");
        let kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "interrupt").await;
        assert_eq!(turns(&kinds), ["start:Run", "end:Completed", "start:Run"]);
        assert_eq!(cli.actor.norm.commands_in_background(), ["tid-1"], "the command of the first turn runs on");
        assert_eq!(cli.actor.waiting.len(), 1, "the third message waits in the CLI");

        // 1. The interrupt: the CLI drops the message that waits and ends the turn.
        cli.in_place_of_the_drivers_request(AgentCmd::Shutdown).await;
        assert_eq!(cli.actor.stage, Stage::Interrupting);
        let kinds = cli.play(|l| l["type"] == "result").await;
        assert_eq!(turns(&kinds), ["end:Interrupted"]);
        assert!(cli.actor.waiting.is_empty() && !cli.actor.left_in_cli, "the CLI dropped the message");

        // 2. `stop_task` for the background command, written with no wait after that result:
        // the driver's next request is the same line.
        let recorded = cli.lines.iter().find(|l| l["_sent"]["type"] == "control_request").expect("the driver's stop_task")["_sent"].clone();
        let wrote = last_sent(cli.dir.path());
        assert_eq!((&wrote["type"], &wrote["request"]), (&recorded["type"], &recorded["request"]));
        assert_eq!(wrote["request"], json!({ "subtype": "stop_task", "task_id": "tid-1" }));
        assert!(cli.actor.stage == Stage::StoppingTasks && cli.actor.stdin.is_some(), "{:?}", cli.actor.stage);

        // 3. The CLI says that nothing runs in the background any more: stdin is closed.
        let kinds = cli.play(|_| false).await;
        assert!(kinds.is_empty(), "not shown as this agent's work: {kinds:?}");
        assert!(cli.actor.norm.commands_in_background().is_empty());
        assert!(cli.actor.stage == Stage::Closing && cli.actor.stdin.is_none() && cli.actor.deadline.is_some(), "{:?}", cli.actor.stage);
        assert_eq!(cli.written().len(), 3, "no message is sent again");
    }

    /// Recordings `r5-stop-subagent-1` and `-2` (part 3 §1, n=2; the first is played here):
    /// `stop_task` with the task id of a background subagent, as "Stop" sends it. The
    /// subagent's row ends as "stopped", with no report (the line's `summary` only repeats its
    /// description), its own call is closed, and the list is empty. Then the CLI starts a turn
    /// by itself: a turn with no prompt of yours, and its own end.
    #[tokio::test]
    async fn a_stopped_background_subagent_ends_as_stopped_and_the_cli_starts_a_turn_by_itself() {
        for name in ["r5-stop-subagent-1", "r5-stop-subagent-2"] {
            let lines = recorded(name);
            let order: Vec<&str> = lines.iter().skip_while(|l| l["subtype"] != "task_updated").filter_map(|l| l["subtype"].as_str().or(l["type"].as_str())).take(4).collect();
            assert_eq!(order, ["task_updated", "task_notification", "background_tasks_changed", "control_response"], "{name}");
        }

        let mut cli = Replay::of("r5-stop-subagent-1");
        // The turn that starts the subagent in the background, and the subagent's own call.
        let mut kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "get_task_output").await;
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed"]);
        // (The driver first asked for the subagent's output, which Codly does not do: the CLI
        // has no such output, and its answer is an error.)
        let no_output = cli.lines.pop_front().unwrap();
        assert_eq!(no_output["response"]["subtype"], "error");

        cli.play(|l| l["_sent"]["request"]["subtype"] == "stop_task").await;
        let (reply, mut answer) = mpsc::unbounded_channel();
        cli.in_place_of_the_drivers_request(AgentCmd::Session { request: SessionRequest::StopTask("tid-1".into()), reply }).await;
        kinds.extend(cli.play(|l| l["type"] == "result").await);
        assert_eq!(answer.try_recv().expect("the CLI's answer reaches whoever asked"), Ok(json!({})));

        assert_eq!(
            subagent_lines(&kinds),
            [
                "toolu-1 running: Run delayed echo command",
                "toolu-1 name=general-purpose",
                "toolu-1 call=toolu-2",
                "toolu-1 step=Running Print begin, wait 113 seconds, print end uses=1 tokens=5189",
                "toolu-1 stopped: Run delayed echo command",
            ]
        );
        assert_eq!(background_lines(&kinds), ["tid-1 helper: Run delayed echo command | -", "- | tid-1 helper stopped: Run delayed echo command"]);
        assert!(kinds.iter().any(|k| matches!(k, EventKind::ToolCompleted { call_id, ok: false, .. } if call_id == "toolu-2")), "the subagent's own call is closed");
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed", "start", "end:Completed"], "the turn after the stop has no prompt");
        assert!(!kinds.iter().any(|k| matches!(k, EventKind::Unknown { .. })), "{kinds:?}");
        assert!(!cli.actor.turn_active && cli.actor.session_replies.is_empty());
    }

    /// A background helper and a command that the CLI does not report as stopped are not
    /// waited for without end, and a CLI that still holds a message is asked for nothing more:
    /// its process is ended. (The lists are written here: no recording has a CLI that ignores
    /// the stop.)
    #[tokio::test]
    async fn a_background_command_that_does_not_stop_does_not_hold_the_stop() {
        let tasks = json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [
            { "task_id": "tid-1", "run_id": "run-1", "task_type": "local_bash", "description": "npm run dev" },
            { "task_id": "tid-2", "run_id": "run-2", "task_type": "local_agent", "subagent_type": "general-purpose", "description": "Look at the tests" },
        ] });
        // The task ids of the `stop_task` requests Codly wrote to the CLI.
        let stops_sent = |dir: &Path| {
            let log = std::fs::read_to_string(dir.join("raw.ndjson")).unwrap();
            let sent = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["codly_sent"]["request"].clone());
            let stopped: Vec<Value> = sent.filter(|r| r["subtype"] == "stop_task").map(|r| r["task_id"].clone()).collect();
            stopped
        };

        let dir = tempfile::tempdir().unwrap();
        let (mut actor, _events, _child) = silent_actor(dir.path());
        actor.on_json(tasks.clone(), "codly-0", &mut None).await;
        assert_eq!(actor.norm.commands_in_background(), ["tid-1"], "the command; the helper gets no `stop_task`");
        // The helper first: the interrupt. Its list does not come in time: the command is
        // stopped all the same.
        actor.on_cmd(AgentCmd::Shutdown).await;
        assert_eq!((interrupts_sent(dir.path()), actor.stage), (1, Stage::StoppingHelpers));
        assert!(stops_sent(dir.path()).is_empty());
        actor.on_deadline().await;
        assert_eq!(stops_sent(dir.path()), [json!("tid-1")]);
        assert_eq!(actor.stage, Stage::StoppingTasks);
        actor.on_deadline().await;
        assert!(actor.stage == Stage::Killing && actor.stdin.is_none(), "SIGTERM makes the CLI end it: {:?}", actor.stage);

        let dir = tempfile::tempdir().unwrap();
        let (mut actor, _events, _child) = silent_actor(dir.path());
        actor.lifecycle = true;
        actor.on_json(tasks, "codly-0", &mut None).await;
        actor.on_cmd(AgentCmd::Prompt("build it".into())).await;
        actor.on_cmd(AgentCmd::Prompt("also run the tests".into())).await;
        actor.on_cmd(AgentCmd::Shutdown).await;
        actor.on_json(json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "duration_ms": 5 }), "codly-0", &mut None).await;
        assert!(actor.stage == Stage::Killing && actor.stdin.is_none(), "{:?}", actor.stage);
        assert!(stops_sent(dir.path()).is_empty(), "SIGTERM makes the CLI end what it tracks");
    }

    /// A stop whose `stop_task` request cannot be written: the CLI did not get it, so no
    /// list will say that the command is gone. The stop does not wait for that list: it
    /// goes straight to SIGTERM, at which the CLI ends the command.
    #[tokio::test]
    async fn a_stop_that_cannot_reach_the_cli_does_not_wait_for_its_list() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, _events, _child) = silent_actor(dir.path());
        actor.on_json(list_line(json!([{ "task_id": "tid-1", "task_type": "local_bash", "description": "npm run dev" }])), "codly-0", &mut None).await;
        assert_eq!(actor.norm.commands_in_background(), ["tid-1"]);
        // The other end of the actor's pipe is closed: it was the stdin of a process that is gone.
        let env = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        let mut gone = proc::command(Path::new("/bin/sh"), &["-c".into(), "exit 0".into()], dir.path(), &env).spawn().unwrap();
        let mut stdin = gone.stdin.take().unwrap();
        gone.wait().await.unwrap();
        // A process that another test starts while this one is started gets a copy of that end
        // too, and keeps it until its exec: for that long a write still goes into the pipe.
        // The end is closed for good once a write fails.
        let closed = async {
            use tokio::io::AsyncWriteExt;
            while stdin.write_all(b"\n").await.is_ok() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), closed).await.expect("nothing reads the pipe of a process that is gone");
        actor.stdin = Some(stdin);

        actor.on_cmd(AgentCmd::Shutdown).await;
        assert!(actor.stage == Stage::Killing && actor.stdin.is_none() && actor.deadline.is_none(), "no wait for a list: {:?}", actor.stage);
    }

    /// A `result` never ends a turn that is not open: one that comes with no turn announced
    /// (a CLI that writes no `system/init` when it starts a turn by itself) gets its start.
    #[tokio::test]
    async fn a_result_with_no_open_turn_gets_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        actor.on_json(json!({ "type": "result", "subtype": "success", "is_error": false, "result": "ok" }), "codly-0", &mut None).await;
        assert_eq!(turns(&emitted(&mut events)), ["start", "end:Completed"]);
        assert!(!actor.turn_active);
    }

    // ───── the session's mode, model and context (recordings of Claude Code 2.1.311) ─────

    /// The lines of a 2.1.311 recording: the CLI's stdout lines, and the driver's own
    /// (`_sent` = a line it wrote to the CLI).
    fn recording(name: &str) -> Vec<Value> {
        fixture(&format!("claude-2.1.311/{name}.ndjson"))
    }

    /// The control request with this body that the recording's driver sent, and the CLI's answer.
    fn recorded_control(lines: &[Value], request: &Value) -> (Value, Value) {
        let sent = lines
            .iter()
            .map(|l| &l["_sent"])
            .find(|s| s["type"] == "control_request" && s["request"] == *request)
            .unwrap_or_else(|| panic!("no recorded request {request}"));
        let answer = lines
            .iter()
            .find(|l| l["type"] == "control_response" && l["response"]["request_id"] == sent["request_id"])
            .unwrap_or_else(|| panic!("no recorded answer to {request}"));
        (sent.clone(), answer.clone())
    }

    /// The last line Codly wrote to the CLI.
    fn last_sent(dir: &Path) -> Value {
        let log = std::fs::read_to_string(dir.join("raw.ndjson")).unwrap();
        log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter_map(|l| l.get("codly_sent").cloned()).last().expect("a line was written")
    }

    /// Send a session request through the actor, compare the line it wrote with the recorded
    /// request, and feed it the recorded answer (under the id the actor gave its request).
    async fn ask_as_recorded(actor: &mut Actor, dir: &Path, lines: &[Value], request: SessionRequest, recorded: Value) -> Result<Value, String> {
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request, reply }).await;
        let wrote = last_sent(dir);
        let (sent, mut response) = recorded_control(lines, &recorded);
        assert_eq!((&wrote["type"], &wrote["request"]), (&sent["type"], &sent["request"]), "Codly writes the recorded request");
        assert!(answer.try_recv().is_err(), "no answer before the CLI's own");
        response["response"]["request_id"] = wrote["request_id"].clone();
        actor.on_json(response, "codly-0", &mut None).await;
        answer.try_recv().expect("the CLI's answer reaches whoever asked")
    }

    /// As `ask_as_recorded`, for the request the recording's driver sent under the id `host`
    /// (a recording that sends one request more than once).
    async fn ask_in_place_of(actor: &mut Actor, dir: &Path, lines: &[Value], request: SessionRequest, host: &str) -> Result<Value, String> {
        let sent = lines.iter().map(|l| &l["_sent"]).find(|s| s["request_id"] == host).unwrap_or_else(|| panic!("no recorded request {host}"));
        let answered = |l: &&Value| l["type"] == "control_response" && l["response"]["request_id"] == host;
        let mut response = lines.iter().find(answered).unwrap_or_else(|| panic!("no recorded answer to {host}")).clone();
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request, reply }).await;
        let wrote = last_sent(dir);
        assert_eq!((&wrote["type"], &wrote["request"]), (&sent["type"], &sent["request"]), "Codly writes the recorded request {host}");
        response["response"]["request_id"] = wrote["request_id"].clone();
        actor.on_json(response, "codly-0", &mut None).await;
        answer.try_recv().expect("the CLI's answer reaches whoever asked")
    }

    #[tokio::test]
    async fn session_requests_go_out_and_come_back_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let lines = recording("control-requests");
        let dir = dir.path();

        // Plan mode, then accept edits. The CLI's status line for a switch Codly asked for is
        // no news: it makes no record.
        let request = json!({ "subtype": "set_permission_mode", "mode": "plan" });
        let plan = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::PermissionMode(PermissionMode::Plan), request).await;
        assert_eq!(plan, Ok(json!({ "mode": "plan" })));
        let status = lines.iter().find(|l| l["subtype"] == "status" && l["permissionMode"] == "plan").expect("the recorded status line").clone();
        actor.on_json(status, "codly-0", &mut None).await;
        let request = json!({ "subtype": "set_permission_mode", "mode": "acceptEdits" });
        let accept = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::PermissionMode(PermissionMode::AcceptEdits), request).await;
        assert_eq!(accept, Ok(json!({ "mode": "acceptEdits" })));
        assert_eq!(actor.norm.permission_mode, Some(PermissionMode::AcceptEdits));

        // Another model; and one the CLI does not know: its own words are the error.
        let request = json!({ "subtype": "set_model", "model": "sonnet" });
        let model = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::Model("sonnet".into()), request).await;
        assert_eq!(model, Ok(json!({ "model": "claude-sonnet-5-5" })));
        let request = json!({ "subtype": "set_model", "model": "no-such-model-xyz" });
        let unknown = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::Model("no-such-model-xyz".into()), request).await;
        assert_eq!(unknown, Err("Model 'no-such-model-xyz' not found".to_string()));
        assert_eq!(actor.norm.model.as_deref(), Some("sonnet"), "a refused model is not the session's model");

        // How full the context is.
        let usage = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::ContextUsage, json!({ "subtype": "get_context_usage" })).await.unwrap();
        assert_eq!((usage["totalTokens"].as_i64(), usage["maxTokens"].as_i64()), (Some(6035), Some(1_000_000)));

        // Answers and errors went to whoever asked: none of it is a line of the timeline.
        let kinds = emitted(&mut events);
        assert!(kinds.is_empty(), "{kinds:?}");
        assert!(actor.session_replies.is_empty());

        // A refused switch: the session's mode is not the one Codly asked for.
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request: SessionRequest::PermissionMode(PermissionMode::Plan), reply }).await;
        let id = last_sent(dir)["request_id"].clone();
        let refused = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": id, "error": "refused" } });
        actor.on_json(refused, "codly-0", &mut None).await;
        assert_eq!(answer.try_recv().unwrap(), Err("refused".to_string()));
        assert_eq!(actor.norm.permission_mode, None);

        // During a shutdown nothing is written any more, and the asker is told so.
        actor.on_cmd(AgentCmd::Shutdown).await;
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request: SessionRequest::ContextUsage, reply }).await;
        assert_eq!(answer.try_recv().unwrap(), Err("Claude Code is shutting down".to_string()));
    }

    /// The recorded list line of `lines` that names the background task `id`.
    fn list_naming(lines: &[Value], id: &str) -> Value {
        let names = |l: &&Value| l["subtype"] == "background_tasks_changed" && l["tasks"].as_array().is_some_and(|t| t.iter().any(|t| t["task_id"] == id));
        lines.iter().find(names).unwrap_or_else(|| panic!("no recorded list names {id}")).clone()
    }

    /// Recording `r4-background-life`: `stop_task` on a background command that runs. The
    /// answer is empty: the CLI's list of its background tasks says that the task is gone.
    #[tokio::test]
    async fn stop_task_goes_out_and_comes_back_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, _events, _child) = silent_actor(dir.path());
        let lines = recording("r4-background-life");
        actor.on_json(list_naming(&lines, "tid-2"), "codly-0", &mut None).await;
        let request = json!({ "subtype": "stop_task", "task_id": "tid-2" });
        let answer = ask_as_recorded(&mut actor, dir.path(), &lines, SessionRequest::StopTask("tid-2".into()), request).await;
        assert_eq!(answer, Ok(json!({})));
        assert!(actor.session_replies.is_empty());
    }

    /// Recording `r4-mcp-needs-auth`: "Reconnect" goes out as `mcp_reconnect`, the CLI's own
    /// reason is the answer, and Codly then asks where the servers stand. Recording
    /// `r4-background-life`: "Stop" of a background command goes out as `stop_task`.
    #[tokio::test]
    async fn a_reconnect_and_a_stop_of_a_background_command_go_out_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let dir = dir.path();

        let lines = recording("r4-mcp-needs-auth");
        let request = json!({ "subtype": "mcp_reconnect", "serverName": "needs-login" });
        let answer = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::McpReconnect("needs-login".into()), request).await;
        let refused = "Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}";
        assert_eq!(answer, Err(refused.to_string()));
        assert!(emitted(&mut events).is_empty(), "the answer is the asker's, not a line of the timeline");
        // Whatever the answer, the server's state is asked for: the recorded answer (`host-4`)
        // comes back under Codly's id and is reported.
        let asked = last_sent(dir);
        assert_eq!(asked, json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } }));
        let mut status = lines.iter().find(|l| l["response"]["request_id"] == "host-4").unwrap().clone();
        status["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
        actor.on_json(status, "codly-0", &mut None).await;
        let kinds = emitted(&mut events);
        assert!(
            matches!(&kinds[..], [EventKind::McpServer { name, status: McpServerStatus::NeedsSignIn, error: Some(error) }] if name == "needs-login" && error == refused),
            "{kinds:?}"
        );

        let lines = recording("r4-background-life");
        actor.on_json(list_naming(&lines, "tid-2"), "codly-0", &mut None).await;
        emitted(&mut events);
        let request = json!({ "subtype": "stop_task", "task_id": "tid-2" });
        let answer = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::StopTask("tid-2".into()), request).await;
        assert_eq!(answer, Ok(json!({})));
        assert_eq!(last_sent(dir)["request"]["subtype"], "stop_task", "nothing is asked after it: the task lines say what happened");
        assert!(emitted(&mut events).is_empty() && actor.session_replies.is_empty());
    }

    /// Recording `r5-mcp-reconnect-toggle` (part 3 §10, n=1): a server that exits at its
    /// start, a reconnect while it still fails, a reconnect that works, then "Turn off" and
    /// "Turn on" (`mcp_toggle`). Every request goes out as recorded. A success has no body, so
    /// the answer says nothing about the server: Codly asks where the servers stand after
    /// each one, and the recorded answer to that is the server's next state.
    #[tokio::test]
    async fn an_mcp_server_is_turned_off_and_on_as_recorded() {
        use McpServerStatus::{Connected, Disabled, Failed};
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let lines = recording("r5-mcp-reconnect-toggle");
        let dir = dir.path();
        let asked = json!({ "type": "control_request", "request_id": MCP_STATUS_REQUEST, "request": { "subtype": "mcp_status" } });
        // The CLI's recorded answer to the driver's `mcp_status` under this id, as the answer
        // to the request Codly wrote.
        let status = |host: &str| {
            let mut answer = lines.iter().find(|l| l["response"]["request_id"] == host).unwrap_or_else(|| panic!("no answer {host}")).clone();
            assert!(answer["response"]["response"]["mcpServers"].is_array(), "{host} answers `mcp_status`");
            answer["response"]["request_id"] = json!(MCP_STATUS_REQUEST);
            answer
        };
        let reconnect = || SessionRequest::McpReconnect("ping-flaky".into());
        let switch = |on: bool| SessionRequest::McpEnabled("ping-flaky".into(), on);
        // (the driver's request, its recorded answer, the driver's `mcp_status` after it, the server then)
        let steps = [
            (reconnect(), "host-2", Err("Connection closed".to_string()), "host-3", (Failed, Some("Connection closed"))),
            (reconnect(), "host-4", Ok(Value::Null), "host-5", (Connected, None)),
            (switch(false), "host-6", Ok(Value::Null), "host-7", (Disabled, None)),
            (switch(true), "host-8", Ok(Value::Null), "host-9", (Connected, None)),
        ];
        for (request, host, answer, then, (state, reason)) in steps {
            assert_eq!(ask_in_place_of(&mut actor, dir, &lines, request, host).await, answer, "{host}");
            assert!(emitted(&mut events).is_empty(), "{host}: the answer is the asker's, not a line of the timeline");
            assert_eq!(last_sent(dir), asked, "{host}: the servers' state is asked for after it");
            actor.on_json(status(then), "codly-0", &mut None).await;
            let said = emitted(&mut events);
            assert_eq!(mcp_states(&said), [("ping-flaky", state, reason)], "{then}");
        }
        assert_eq!(lines.iter().find(|l| l["_sent"]["request_id"] == "host-6").unwrap()["_sent"]["request"], json!({ "subtype": "mcp_toggle", "serverName": "ping-flaky", "enabled": false }));
        assert!(actor.session_replies.is_empty() && actor.norm.mcp_asked.is_empty());

        // The turn after it: its init line gives the server as connected, which is no news.
        let init = lines.iter().find(|l| l["subtype"] == "init").expect("the recorded init line");
        assert_eq!(init["mcp_servers"], json!([{ "name": "ping-flaky", "status": "connected", "source": "dynamic" }]));
        assert!(mcp_states(&actor.norm.on_message(init)).is_empty());
    }

    /// The plan-mode recording: `ExitPlanMode` arrives as a request with the plan in it. Sent
    /// back with a note, the session stays in plan mode and the plan comes again; accepted,
    /// the CLI leaves plan mode by itself and Codly records that.
    #[test]
    fn a_plan_is_a_request_and_accepting_it_ends_plan_mode() {
        let lines = recording("permission-plan-mode");
        let mut s = spec(Role::Solo, false);
        s.cwd = PathBuf::from("/tmp/work/plan");
        let mut norm = ClaudeNormalizer::new(&s, "sess-1".into());
        norm.permission_mode = Some(PermissionMode::Plan); // as after Codly's own switch
        // Codly's answers, as the recording's driver gave them: the plan sent back with a
        // note, the next plan accepted, the command that follows allowed once.
        let note = "Not yet: add a final step that reads the file back to check it. Then present the plan again.";
        let once = || ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None };
        let mut decisions = vec![ApprovalDecision { choice: ApprovalChoice::Deny, message: Some(note.into()), answers: None }, once(), once()].into_iter();
        let mut plans = Vec::new();
        let mut events = norm.turn_started("plan it");
        for (i, line) in lines.iter().enumerate().filter(|(_, l)| l["type"].is_string()) {
            let out = norm.on_message(line);
            if let Some(EventKind::ApprovalRequested { approval }) = out.iter().find(|e| matches!(e, EventKind::ApprovalRequested { .. })) {
                if let ApprovalSubject::Tool { name, input } = &approval.subject {
                    assert_eq!(name, EXIT_PLAN_MODE);
                    assert_eq!(approval.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
                    plans.push(input["plan"].as_str().expect("the plan text").to_string());
                }
                let (reply, _) = norm.answer(&approval.id, &decisions.next().expect("three requests")).unwrap();
                assert_eq!(reply, lines[i + 1]["_sent"], "Codly's answer is the recorded one");
            }
            events.extend(out);
        }
        assert!(decisions.next().is_none(), "three requests: two plans and a command");
        assert_eq!(plans.len(), 2, "the plan, and the plan again after it was sent back");
        assert!(plans.iter().all(|p| p.starts_with("# Add `world` to hello.txt")), "{plans:?}");
        // One change of mode, the CLI's own, after the accepted plan: it asks for edits again.
        let modes: Vec<Value> = events
            .iter()
            .filter_map(|e| match e {
                EventKind::Decision { decision_type, detail, .. } if decision_type == crate::adapters::MODE_CHANGED => Some(detail["mode"].clone()),
                _ => None,
            })
            .collect();
        assert_eq!(modes, [json!("ask")]);
        assert_eq!(norm.permission_mode, Some(PermissionMode::Ask));
    }

    /// The recorded requests of mode `default`. "For this session" and "always" send back the
    /// rule for the tool that asked and a folder of the agent's own, never a switch of the
    /// session's mode and never a folder outside.
    #[test]
    fn a_grant_never_changes_the_mode_of_the_session() {
        let lines = recording("permission-request-shapes");
        let mut s = spec(Role::Solo, false);
        s.cwd = PathBuf::from("/tmp/work/permissions");
        let mut norm = ClaudeNormalizer::new(&s, "sess-1".into());
        let request = |tool: &str, n: usize| {
            let mut asked = lines.iter().filter(|l| l["request"]["subtype"] == "can_use_tool" && l["request"]["tool_name"] == tool);
            asked.nth(n).unwrap_or_else(|| panic!("no recorded {tool} request {n}")).clone()
        };
        let suggested = |r: &Value| r["request"]["permission_suggestions"].as_array().unwrap().iter().map(|s| s["type"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let granted = |r: &Value| r["response"]["response"]["updatedPermissions"].clone();
        let decide = |choice| ApprovalDecision { choice, message: None, answers: None };

        // An edit: the CLI suggests only "accept edits for the session". That is no grant for
        // this edit, so "once" is the only way to allow it.
        let edit = request("Edit", 0);
        assert_eq!(suggested(&edit), ["setMode"]);
        let a = approval_of(&norm.on_message(&edit));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        let (r, _) = norm.answer(&a.id, &decide(ApprovalChoice::AllowOnce)).unwrap();
        assert!(granted(&r).is_null());

        // `rm note.txt` in the agent's folder: its rule and that folder, for the session.
        let rm = request("Bash", 3);
        assert_eq!(rm["request"]["input"]["command"], "rm note.txt");
        assert_eq!(suggested(&rm), ["addRules", "addDirectories", "setMode"]);
        let a = approval_of(&norm.on_message(&rm));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::AllowAlways, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        let (r, _) = norm.answer(&a.id, &decide(ApprovalChoice::AllowSession)).unwrap();
        assert_eq!(
            granted(&r),
            json!([
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "rm note.txt" }], "behavior": "allow", "destination": "session" },
                { "type": "addDirectories", "directories": ["/tmp/work/permissions"], "destination": "session" },
            ])
        );

        // `touch /tmp/…` writes outside: "always" keeps the rule for that command, not `/tmp`.
        let touch = request("Bash", 2);
        assert_eq!(suggested(&touch), ["addRules", "addDirectories", "setMode"]);
        let a = approval_of(&norm.on_message(&touch));
        let (r, _) = norm.answer(&a.id, &decide(ApprovalChoice::AllowAlways)).unwrap();
        assert_eq!(
            granted(&r),
            json!([{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "touch /tmp/perm-probe-outside.txt" }], "behavior": "allow", "destination": "localSettings" }])
        );

        // A rule for another tool than the one that asked, or one that denies, is not sent back.
        let odd = json!([
            { "type": "addRules", "rules": [{ "toolName": "Edit", "ruleContent": "**" }], "behavior": "allow", "destination": "session" },
            { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "ls" }], "behavior": "deny", "destination": "session" },
        ]);
        let a = approval_of(&norm.on_message(&can_use_tool("Bash", json!({ "command": "ls" }), odd)));
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
    }

    #[test]
    fn only_a_one_to_one_agent_gets_the_plan_tool() {
        assert!(tools_for(&spec(Role::Solo, false)).contains(&EXIT_PLAN_MODE));
        assert!(build_args(&spec(Role::Solo, false), "sid", None, &[]).join(" ").contains("Skill,LSP,ExitPlanMode "));
        assert!(!tools_for(&spec(Role::Worker, false)).contains(&EXIT_PLAN_MODE));
        assert!(!tools_for(&spec(Role::Solo, true)).contains(&EXIT_PLAN_MODE), "a read-only run has nothing to plan");
        assert!(!tools_for(&spec(Role::Lead, true)).contains(&EXIT_PLAN_MODE));
        // Asked for, the CLI lists it among its tools (recorded): no "not enabled" line follows.
        let init = recording("permission-plan-mode").into_iter().find(|l| l["subtype"] == "init").expect("the recorded init line");
        assert!(init["tools"].as_array().unwrap().contains(&json!(EXIT_PLAN_MODE)));
    }

    /// The recorded model and effort requests (part 2, scenario 13). Before, no request named
    /// the CLI's default model or set an effort, so both changes restarted the process.
    #[tokio::test]
    async fn the_default_model_and_the_effort_go_out_and_come_back_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let lines = recording("r4-model-effort-fallback");
        let dir = dir.path();
        // The CLI's answer to the request the recording's driver sent under this id.
        let answer_to = |id: &str| {
            lines
                .iter()
                .find(|l| l["type"] == "control_response" && l["response"]["request_id"] == id)
                .unwrap_or_else(|| panic!("no recorded answer to {id}"))
                .clone()
        };

        // The default model has a name, and the answer says which model it is.
        let request = json!({ "subtype": "set_model", "model": DEFAULT_MODEL });
        let model = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::Model(DEFAULT_MODEL.into()), request).await;
        assert_eq!(model, Ok(json!({ "model": "claude-opus-5-5" })));
        assert_eq!(actor.norm.model, None, "\"default\" is not a model name to show");
        assert_eq!(actor.norm.session_model, None, "the next turn's init line names the session's model");

        // An effort level: the answer is a bare success, and the settings say what the session
        // works with.
        let request = json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": "low" } });
        let set = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::Effort(Some("low".into())), request).await;
        assert_eq!(set, Ok(Value::Null));
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request: SessionRequest::Settings, reply }).await;
        let wrote = last_sent(dir);
        assert_eq!(wrote["request"], json!({ "subtype": "get_settings" }));
        let mut settings = answer_to("host-8"); // the driver's `get_settings` after its "low"
        settings["response"]["request_id"] = wrote["request_id"].clone();
        actor.on_json(settings, "codly-0", &mut None).await;
        assert_eq!(answer.try_recv().unwrap().unwrap()["applied"]["effort"], "low");

        // A word that is no level: the recorded CLI answered "success" and kept the effort it
        // had. Codly does not send such a word, and says so.
        let bad = lines.iter().find(|l| l["_sent"]["request_id"] == "host-9").expect("the driver's request");
        assert_eq!(bad["_sent"]["request"]["settings"]["effortLevel"], "not-a-level");
        assert_eq!(answer_to("host-9")["response"]["subtype"], "success");
        assert_eq!(answer_to("host-10")["response"]["response"]["applied"]["effort"], "low");
        let (reply, mut answer) = mpsc::unbounded_channel();
        actor.on_cmd(AgentCmd::Session { request: SessionRequest::Effort(Some("not-a-level".into())), reply }).await;
        assert_eq!(answer.try_recv().unwrap(), Err("Claude Code has no effort level \"not-a-level\"".to_string()));
        assert_eq!(last_sent(dir), wrote, "nothing was written");
        assert!(actor.session_replies.is_empty());
        let kinds = emitted(&mut events);
        assert!(kinds.is_empty(), "{kinds:?}");
    }

    /// A model switch of Codly's while a turn runs (recording `control-requests`: the turn of
    /// `sleep 20`, with `set_model` written while the command runs). Before, the lines the
    /// turn still wrote on the model it had made two false rows after Codly's own "now works
    /// with sonnet": "now runs on haiku, not on sonnet", then "runs on sonnet again". The
    /// lines after the switch are written by hand, from recorded lines of this session with
    /// another text or model name: the recorded turn was stopped right after the switch.
    #[tokio::test]
    async fn a_model_switch_of_codlys_makes_no_rows_about_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let lines = recording("control-requests");
        let dir = dir.path();
        let notices = |events: &mut mpsc::UnboundedReceiver<AdapterEvent>| -> Vec<String> {
            emitted(events).into_iter().filter_map(|e| if let EventKind::Notice { text } = e { Some(text) } else { None }).collect()
        };
        // An `assistant` line of this session, as the CLI would write it for this text on this model.
        let said = |from: &Value, id: &str, model: &str| {
            let mut line = from.clone();
            line["message"]["id"] = json!(id);
            line["message"]["model"] = json!(model);
            line["message"]["content"] = json!([{ "type": "text", "text": "ok" }]);
            line
        };

        // The turn as recorded, up to the command that runs: the session is on haiku.
        let prompt = lines.iter().position(|l| l["_sent"]["message"]["content"].as_str().is_some_and(|c| c.contains("sleep 20"))).expect("the recorded prompt");
        let switch = lines.iter().position(|l| l["_sent"]["request_id"] == "host-39").expect("the recorded switch");
        let turn: Vec<&Value> = lines[prompt..switch].iter().filter(|l| l["type"].is_string()).collect();
        let init = *turn.iter().find(|l| l["subtype"] == "init").expect("the turn's init line");
        let on_haiku = *turn.iter().find(|l| l["type"] == "assistant").expect("a line of the turn");
        assert_eq!((&init["model"], &on_haiku["message"]["model"]), (&json!("claude-haiku-5-5"), &json!("claude-haiku-5-5")));
        actor.on_cmd(AgentCmd::Prompt("Run the Bash command `sleep 20`".into())).await;
        for line in &turn {
            actor.on_json((*line).clone(), "codly-0", &mut None).await;
        }
        assert!(actor.turn_active && notices(&mut events).is_empty());

        // Codly's switch while the turn runs: the request and the answer as recorded then (`host-39`).
        let request = json!({ "subtype": "set_model", "model": "sonnet" });
        assert_eq!(lines[switch]["_sent"]["request"], request);
        let recorded = lines.iter().find(|l| l["response"]["request_id"] == "host-39").expect("the recorded answer");
        let model = ask_as_recorded(&mut actor, dir, &lines, SessionRequest::Model("sonnet".into()), request).await;
        assert_eq!(model.as_ref(), Ok(&recorded["response"]["response"]));
        assert_eq!(model, Ok(json!({ "model": "claude-sonnet-5-5" })));
        let echo = &lines[switch + 1]; // the CLI's own line about the switch, as recorded
        assert!(echo["isReplay"] == true && echo["message"]["content"].as_str().is_some_and(|c| c.contains("Set model to")));
        actor.on_json(echo.clone(), "codly-0", &mut None).await;

        // (by hand) The request that was on its way still answers on the model the session had.
        actor.on_json(said(on_haiku, "msg-after-switch", "claude-haiku-5-5"), "codly-0", &mut None).await;
        assert_eq!(notices(&mut events), Vec::<String>::new(), "the switch was Codly's own, and whoever asked for it says so");
        // The turn ends (the recorded result of the turn before), and the next one starts:
        // (by hand) its init line names the new model, and its lines are on it.
        let result = lines[..prompt].iter().rev().find(|l| l["type"] == "result").expect("a recorded result");
        actor.on_json(result.clone(), "codly-0", &mut None).await;
        assert!(!actor.turn_active);
        actor.on_cmd(AgentCmd::Prompt("again".into())).await;
        let mut next_init = init.clone();
        next_init["model"] = json!("claude-sonnet-5-5");
        actor.on_json(next_init, "codly-0", &mut None).await;
        actor.on_json(said(on_haiku, "msg-next-turn", "claude-sonnet-5-5"), "codly-0", &mut None).await;
        assert_eq!(notices(&mut events), Vec::<String>::new(), "\"again\" would be false: the session never ran on sonnet before");

        // From that init line on the notice works as before: another model than the session's is said.
        actor.on_json(said(on_haiku, "msg-other-model", "claude-haiku-5-5"), "codly-0", &mut None).await;
        assert_eq!(notices(&mut events), ["Claude Code now runs on claude-haiku-5-5, not on claude-sonnet-5-5"]);
    }

    /// The model each `assistant` line of the agent's own names, in order, without repeats.
    fn models_said(lines: &[Value]) -> Vec<&str> {
        let mut said: Vec<&str> = lines.iter().filter(|l| l["type"] == "assistant" && l["parent_tool_use_id"].is_null()).filter_map(|l| l["message"]["model"].as_str()).collect();
        said.dedup();
        said
    }

    /// Recording `r5-model-midturn` (part 3 §8, n=1): Codly's own switch to sonnet while a
    /// command of a turn on haiku runs, then to the CLI's default model and back to haiku
    /// between turns. The turn's line after the switch is on sonnet while its init line said
    /// haiku. Every change here is one Codly asked for, and whoever asked says so: no row
    /// says "now runs on", and none says "runs on … again".
    #[tokio::test]
    async fn a_model_switch_during_a_turn_makes_no_rows_about_the_model_as_recorded() {
        let mut cli = Replay::of("r5-model-midturn");
        let lines = recording("r5-model-midturn");
        assert_eq!(models_said(&lines), ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"]);
        let first_turn = lines.iter().position(|l| l["type"] == "result").unwrap();
        assert_eq!(lines[..first_turn].iter().find(|l| l["subtype"] == "init").unwrap()["model"], "claude-haiku-5-5");
        assert_eq!(models_said(&lines[..first_turn]), ["claude-haiku-5-5", "claude-sonnet-5-5"], "the switch shows inside the turn");

        let mut kinds = Vec::new();
        for (model, resolved) in [("sonnet", "claude-sonnet-5-5"), (DEFAULT_MODEL, "claude-opus-5-5"), ("haiku", "claude-haiku-5-5")] {
            kinds.extend(cli.play(|l| l["_sent"]["request"]["subtype"] == "set_model").await);
            let (reply, mut answer) = mpsc::unbounded_channel();
            cli.in_place_of_the_drivers_request(AgentCmd::Session { request: SessionRequest::Model(model.into()), reply }).await;
            // the CLI's own line about the switch, then its answer
            kinds.extend(cli.play(|l| l["type"] == "control_response").await);
            assert_eq!(answer.try_recv().expect("the CLI's answer reaches whoever asked"), Ok(json!({ "model": resolved })), "{model}");
        }
        kinds.extend(cli.play(|_| false).await);

        assert_eq!(turns(&kinds), ["start:Run", "end:Completed", "start:Reply", "end:Completed", "start:Reply", "end:Completed", "start:Reply", "end:Completed"]);
        let notices: Vec<&str> = kinds.iter().filter_map(|k| if let EventKind::Notice { text } = k { Some(text.as_str()) } else { None }).collect();
        assert_eq!(notices, Vec::<&str>::new());
        assert!(!kinds.iter().any(|k| matches!(k, EventKind::Unknown { .. })), "the CLI's own lines about a switch are known: {kinds:?}");
        assert_eq!(cli.actor.norm.session_model.as_deref(), Some("claude-haiku-5-5"));
    }

    /// Recording `r5-model-start-undated` (part 3 §8, n=1): a session started with the
    /// undated name of a dated model. Its init line says `claude-haiku-4-5`, every line of
    /// the agent `claude-haiku-4-5-20251001`. That is one model: no row says that the turn
    /// runs on another one. (The three other starts of that section spell the model the
    /// same way in both places.)
    #[test]
    fn an_undated_model_name_and_its_dated_id_are_one_model() {
        let lines = recorded("r5-model-start-undated");
        let init = lines.iter().find(|l| l["subtype"] == "init").expect("the recorded init line");
        assert_eq!(init["model"], "claude-haiku-4-5");
        assert_eq!(models_said(&lines), ["claude-haiku-4-5-20251001"]);
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
        let mut events = norm.turn_started("Reply with the single word: pong");
        events.extend(replay(&mut norm, &lines));
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, EventKind::Notice { .. })), "{events:?}");
        assert!(same_model("claude-haiku-4-5", "claude-haiku-4-5-20251001") && !same_model("claude-haiku-4-5-20251001", "claude-haiku-5-5"));
        for name in ["r5-model-start-alias", "r5-model-start-full", "r5-model-start-dated"] {
            let lines = recorded(name);
            let init = lines.iter().find(|l| l["subtype"] == "init").expect("the recorded init line");
            assert_eq!(models_said(&lines), [init["model"].as_str().unwrap()], "{name}");
        }
    }

    /// Plan mode on a session started as Codly starts a 1:1 agent (part 2, scenarios 10 and
    /// 11). Before, a plan could only be accepted as it was or sent back, and nothing said that
    /// the plan was made by another model than the session's.
    #[test]
    fn a_changed_plan_is_accepted_as_the_plan_and_the_timeline_names_the_model_that_planned() {
        let lines = recording("r4-plan-from-default");
        let mut s = spec(Role::Solo, false);
        s.cwd = PathBuf::from("/tmp/work/r4-plan-from-default");
        let mut norm = ClaudeNormalizer::new(&s, "sess-1".into());
        norm.permission_mode = Some(PermissionMode::Plan); // as after Codly's own switch
        // Codly's answers, as the recording's driver gave them: the plan sent back with a
        // note, the next plan accepted with another text, the edit that follows allowed once.
        let note = "Not yet: add a final step that reads the file back to check it. Then present the plan again.";
        let changed = "# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n";
        let once = |answers: Option<Value>| ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers };
        let sent_back = ApprovalDecision { choice: ApprovalChoice::Deny, message: Some(note.into()), answers: None };
        let mut decisions = vec![sent_back, once(Some(json!({ "plan": changed }))), once(None)].into_iter();
        let mut events = norm.turn_started("plan it");
        for (i, line) in lines.iter().enumerate().filter(|(_, l)| l["type"].is_string()) {
            let out = norm.on_message(line);
            if let Some(EventKind::ApprovalRequested { approval }) = out.iter().find(|e| matches!(e, EventKind::ApprovalRequested { .. })) {
                let (reply, _) = norm.answer(&approval.id, &decisions.next().expect("three requests")).unwrap();
                assert_eq!(reply, lines[i + 1]["_sent"], "Codly's answer is the recorded one");
            }
            events.extend(out);
        }
        assert!(decisions.next().is_none(), "three requests: two plans and an edit");
        // The CLI took the changed text as the plan (its tool result says who edited it).
        let approved = lines.iter().find(|l| l["tool_use_result"]["planWasEdited"] == true).expect("the recorded approval");
        assert_eq!(approved["tool_use_result"]["plan"], changed);

        // `init.model` said haiku all the time; the plan was made on sonnet.
        assert!(events.iter().any(|e| matches!(e, EventKind::SessionStarted { model: Some(m), .. } if m == "claude-haiku-5-5")));
        let notices: Vec<&str> = events.iter().filter_map(|e| if let EventKind::Notice { text } = e { Some(text.as_str()) } else { None }).collect();
        assert_eq!(notices, ["Claude Code now runs on claude-sonnet-5-5, not on claude-haiku-5-5", "Claude Code runs on claude-haiku-5-5 again"]);
        // A mark for the context window is no other model.
        assert!(same_model("claude-fable-5-1[1m]", "claude-fable-5-1") && !same_model("claude-sonnet-5-5", "claude-haiku-5-5"));
        // Nor is another spelling of one model: names in the style of a cloud provider's ids
        // (not recorded), and a short name with its dated id (recorded for Anthropic's own
        // API: `an_undated_model_name_and_its_dated_id_are_one_model`). No line says the
        // model changed.
        let (cloud, plain) = ("us.anthropic.claude-sonnet-5-5-20261001-v1:0", "claude-sonnet-5-5-20261001");
        assert!(same_model(cloud, plain) && same_model(plain, cloud) && same_model("claude-sonnet-5-5", plain));
        assert!(!same_model(cloud, "claude-haiku-5-5-20261001"));
        norm.session_model_is(cloud);
        let mut said = Vec::new();
        norm.on_turn_model(Some(plain), &mut said);
        assert!(said.is_empty(), "{said:?}");
        norm.on_turn_model(Some("claude-haiku-5-5-20261001"), &mut said);
        let other = format!("Claude Code now runs on claude-haiku-5-5-20261001, not on {cloud}");
        assert!(matches!(said.as_slice(), [EventKind::Notice { text }] if *text == other), "{said:?}");
    }

    /// `init.tools` is the only place where a dropped tool shows (recorded). Before, a 1:1 run
    /// said "did not enable requested tools: WebFetch" although the CLI leaves that tool out by
    /// itself. A tool the agent cannot work without is still reported, and the plan tool
    /// counts only for a session that plans.
    #[test]
    fn a_missing_tool_is_reported_only_when_the_agent_needs_it() {
        let recorded = recording("r4-plan-from-default").into_iter().find(|l| l["subtype"] == "init").expect("the recorded init line");
        let without = |gone: &[&str]| {
            let mut init = recorded.clone();
            init["tools"].as_array_mut().unwrap().retain(|t| !gone.contains(&t.as_str().unwrap()));
            init
        };
        let said = |mode: PermissionMode, init: &Value| -> Vec<String> {
            let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
            norm.permission_mode = Some(mode);
            errors(&norm.on_message(init)).into_iter().map(|(message, _)| message.to_string()).collect()
        };
        let nothing = Vec::<String>::new();

        // As recorded, with the tool list of a 1:1 agent: no WebFetch, no LSP, and no line.
        let listed = |tool: &str| recorded["tools"].as_array().unwrap().contains(&json!(tool));
        assert!(!listed("WebFetch") && !listed("LSP") && listed(EXIT_PLAN_MODE));
        assert_eq!(said(PermissionMode::Ask, &recorded), nothing);
        // A CLI without the plan tool: nothing to say for an agent that does not plan. One that
        // plans could never have its plan accepted.
        assert_eq!(said(PermissionMode::Ask, &without(&[EXIT_PLAN_MODE])), nothing);
        assert_eq!(said(PermissionMode::Plan, &without(&[EXIT_PLAN_MODE])), ["Claude did not enable requested tools: ExitPlanMode"]);
        // A tool the work needs is always reported.
        assert_eq!(said(PermissionMode::Ask, &without(&["Bash", "Edit"])), ["Claude did not enable requested tools: Bash, Edit"]);
    }

    /// `/compact` with nothing to compact (part 2, scenario 12): the CLI always writes a
    /// result, so the turn of "Compact now" ends, and the CLI's own words say why nothing
    /// happened. A compaction that does run is reported with its token counts.
    #[test]
    fn compact_with_nothing_to_compact_ends_its_turn() {
        let lines = recording("r4-compact-empty");
        let end = lines.iter().position(|l| l["type"] == "result").expect("the result of the first /compact");
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
        let mut events = norm.turn_started("/compact");
        events.extend(lines[..=end].iter().filter(|l| l["type"].is_string()).flat_map(|l| norm.on_message(l)));
        assert!(events.iter().any(|e| matches!(e, EventKind::Message { text, .. } if text == "Error: No messages to compact")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })), "{events:?}");
        // The CLI's own text names no model, and nothing was compacted.
        assert!(!events.iter().any(|e| matches!(e, EventKind::Notice { .. } | EventKind::Compacting { .. })), "{events:?}");

        let rest: Vec<EventKind> = lines[end + 1..].iter().filter(|l| l["type"].is_string()).flat_map(|l| norm.on_message(l)).collect();
        assert!(rest.iter().any(|e| matches!(e, EventKind::Compacting { pre_tokens: Some(5765), post_tokens: Some(934) })), "{rest:?}");
        assert_eq!(rest.iter().filter(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })).count(), 3);
    }

    /// What the CLI writes back after a model switch and around a compaction is known: no
    /// "unknown" rows, and the compaction is reported with its token counts.
    #[test]
    fn lines_the_cli_writes_itself_make_no_unknown_rows() {
        for name in ["control-requests", "compaction-and-slash"] {
            let lines = recording(name);
            assert!(lines.iter().any(|l| l["type"] == "user" && l["isReplay"] == true), "{name} has such lines");
            let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "sess-1".into());
            let events: Vec<EventKind> = lines.iter().filter(|l| l["type"].is_string()).flat_map(|l| norm.on_message(l)).collect();
            assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type == "user")), "{name}: {events:?}");
            if name == "compaction-and-slash" {
                let compactions: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        EventKind::Compacting { pre_tokens: Some(pre), post_tokens: Some(post) } => Some((*pre, *post)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(compactions, [(5955, 1197), (7892, 546)]);
            }
        }
        // What the user or a tool did not say stays unknown when the CLI does not mark it as its own.
        let mut norm = ClaudeNormalizer::new(&spec(Role::Solo, false), "s".into());
        let ev = norm.on_message(&json!({ "type": "user", "message": { "role": "user", "content": "<local-command-stdout>x</local-command-stdout>" } }));
        assert!(matches!(&ev[..], [EventKind::Unknown { raw_type }] if raw_type == "user"), "{ev:?}");
    }

    // ───── helpers and the stop, as part 4 of the 2.1.311 recordings has them (`r6-*`) ─────

    /// The subtypes of the control requests Codly wrote to the CLI so far, in order.
    fn requests_sent(dir: &Path) -> Vec<String> {
        let log = std::fs::read_to_string(dir.join("raw.ndjson")).unwrap();
        log.lines().filter_map(|l| serde_json::from_str::<Value>(l).unwrap()["codly_sent"]["request"]["subtype"].as_str().map(str::to_owned)).collect()
    }

    /// The ends of the subagent rows, one line each: "<call id> <status>: <description>".
    fn subagent_ends(events: &[EventKind]) -> Vec<String> {
        subagent_lines(events).into_iter().filter(|l| [" completed: ", " stopped: ", " failed: "].iter().any(|end| l.contains(end))).collect()
    }

    /// Recordings `r6-shutdown-helper-sequence-1` and `-2` (part 4 §2, n=2; the first is
    /// played here): a helper in the background and a turn that runs a command, then the
    /// stop. The interrupt ends both: the helper's row ends as "stopped" and its list empties
    /// before the turn's `result`, and stdin is closed right after that result. No `stop_task`
    /// is written (the recorded driver's one after the result answered `{}` and changed
    /// nothing; the process then exited with nothing left running).
    #[tokio::test]
    async fn a_stop_with_a_helper_and_a_turn_at_work_is_one_interrupt_as_recorded() {
        let mut cli = Replay::of("r6-shutdown-helper-sequence-1");
        let mut kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "interrupt").await;
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed", "start:Run"]);
        assert!(cli.actor.norm.helpers_in_background() && cli.actor.norm.commands_in_background().is_empty());

        cli.in_place_of_the_drivers_request(AgentCmd::Shutdown).await;
        assert_eq!(cli.actor.stage, Stage::Interrupting);
        kinds.extend(cli.play(|l| l["type"] == "result").await);
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed", "start:Run", "end:Interrupted"]);
        assert_eq!(subagent_ends(&kinds), ["toolu-1 stopped: Run delayed echo command"]);
        assert_eq!(background_lines(&kinds).last().map(String::as_str), Some("- | tid-1 helper stopped: Run delayed echo command"));
        assert!(cli.actor.stage == Stage::Closing && cli.actor.stdin.is_none(), "{:?}", cli.actor.stage);

        let rest = cli.play(|_| false).await;
        assert!(rest.is_empty(), "{rest:?}");
        assert_eq!(requests_sent(cli.dir.path()), ["interrupt"]);
    }

    /// Recordings `r6-shutdown-helper-idle-interrupt-1` and `-2` (part 4 §2, n=2; the first
    /// is played here): a helper in the background and NO turn running, then the stop. Before,
    /// the stop closed stdin at once and left the helper to the later steps. Now the interrupt
    /// comes first: the helper's row ends as "stopped", the list empties, no turn starts, and
    /// stdin is closed when the list has no helper (the recorded process exited 0.53 s later).
    #[tokio::test]
    async fn a_stop_with_a_helper_and_no_turn_interrupts_first_as_recorded() {
        let mut cli = Replay::of("r6-shutdown-helper-idle-interrupt-1");
        let kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "interrupt").await;
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed"]);
        assert!(!cli.actor.turn_active && cli.actor.norm.helpers_in_background());

        cli.in_place_of_the_drivers_request(AgentCmd::Shutdown).await;
        assert!(cli.actor.stage == Stage::StoppingHelpers && cli.actor.stdin.is_some() && cli.actor.deadline.is_some(), "{:?}", cli.actor.stage);
        let kinds = cli.play(|l| l["subtype"] == "background_tasks_changed").await;
        assert_eq!(subagent_ends(&kinds), ["toolu-1 stopped: Run echo begin/sleep/end"]);
        assert_eq!(background_lines(&kinds), ["- | tid-1 helper stopped: Run echo begin/sleep/end"]);
        assert!(turns(&kinds).is_empty(), "no turn: {kinds:?}");
        assert!(cli.actor.stage == Stage::Closing && cli.actor.stdin.is_none(), "{:?}", cli.actor.stage);

        let rest = cli.play(|_| false).await;
        assert!(rest.is_empty(), "{rest:?}");
        assert_eq!(requests_sent(cli.dir.path()), ["interrupt"], "the helper gets no `stop_task`");
    }

    /// Recordings `r6-interrupt-helper-*-1` (part 4 §1; with the `-2` runs n=2 each, and each
    /// has ONE helper in the middle of a Bash call). An interrupt ends every helper: one whose
    /// `Agent` call still waits (`foreground`), one in the background while another turn runs
    /// (`background`) and one in the background with no turn running (`idle`). Each helper row
    /// ends once, as "stopped", the list empties, and no turn starts by itself. The
    /// `task_notification` of a Bash call that had become a task (its own task id, an empty
    /// `output_file`) ends nothing. Before, the foreground helper ended twice: "failed" at the
    /// rejected `Agent` call, then "stopped".
    #[tokio::test]
    async fn an_interrupt_ends_every_helper_once_as_recorded() {
        let cases = [
            ("r6-interrupt-helper-foreground-1", "Run timed echo/sleep command", vec!["start:Use", "end:Interrupted", "start:Reply", "end:Completed"]),
            ("r6-interrupt-helper-background-1", "Run delayed echo command", vec!["start:Use", "end:Completed", "start:Run", "end:Interrupted", "start:Reply", "end:Completed"]),
            ("r6-interrupt-helper-idle-1", "Run sleep echo test", vec!["start:Use", "end:Completed", "start:Reply", "end:Completed"]),
        ];
        for (name, helper, turns_said) in cases {
            let lines = recorded(name);
            let bash_task = |l: &Value| l["subtype"] == "task_notification" && l["output_file"] == "" && l["task_id"] == "tid-2";
            assert!(lines.iter().any(bash_task), "{name}: the helper's own Bash call is a task, and its end is said");

            let mut cli = Replay::of(name);
            let mut kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "interrupt").await;
            cli.in_place_of_the_drivers_request(AgentCmd::Interrupt).await;
            kinds.extend(cli.play(|_| false).await);
            assert_eq!(turns(&kinds), turns_said, "{name}");
            assert_eq!(subagent_ends(&kinds), [format!("toolu-1 stopped: {helper}")], "{name}");
            let listed = background_lines(&kinds);
            if name.ends_with("foreground-1") {
                assert!(listed.is_empty(), "a helper the agent waits for is in no list: {listed:?}");
            } else {
                assert_eq!(listed, [format!("tid-1 helper: {helper} | -"), format!("- | tid-1 helper stopped: {helper}")], "{name}");
            }
            assert!(cli.actor.norm.background.is_empty() && cli.actor.norm.helpers.is_empty(), "{name}: nothing is left open");
            assert!(!kinds.iter().any(|k| matches!(k, EventKind::Unknown { .. })), "{name}: {kinds:?}");
            assert_eq!(cli.actor.stage, Stage::Running, "{name}");
        }
    }

    /// Recordings `r6-fails-helper-background` and `-foreground` (part 4 §3, n=1 each): a
    /// helper whose model request is refused. Its row ends as "failed", with the reason of its
    /// `task_updated` line as its report; the background one leaves the list as "failed" too.
    /// Recording `r6-fails-signal-term` (n=1): a background command whose shell got SIGTERM
    /// leaves the list as "failed" (there is no word for a signal).
    #[test]
    fn a_helper_that_fails_says_why_and_a_killed_command_ends_as_failed_as_recorded() {
        for (name, helper) in [("r6-fails-helper-background", "Run echo hi"), ("r6-fails-helper-foreground", "Run echo hi via no-model")] {
            let lines = recorded(name);
            let updated = lines.iter().find(|l| l["subtype"] == "task_updated").expect("the recorded task_updated line");
            assert_eq!(updated["patch"]["status"], "failed", "{name}");
            let error = updated["patch"]["error"].as_str().expect("its reason");
            assert!(error.starts_with("Agent terminated early due to an API error"), "{error}");

            let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
            let mut events = norm.turn_started("go");
            events.extend(replay(&mut norm, &lines));
            let reported: Vec<&str> = events.iter().filter_map(|e| if let EventKind::SubAgentProgress { report: Some(r), .. } = e { Some(r.as_str()) } else { None }).collect();
            assert_eq!(reported, [error], "{name}");
            assert_eq!(subagent_ends(&events), [format!("toolu-1 failed: {helper}")], "{name}: once, the tool result after it adds nothing");
            if name.ends_with("background") {
                assert_eq!(background_lines(&events), [format!("tid-1 helper: {helper} | -"), format!("- | tid-1 helper failed: {helper}")]);
            } else {
                assert!(background_lines(&events).is_empty());
            }
            assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { .. })), "{name}: {events:?}");
        }

        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let mut events = norm.turn_started("go");
        events.extend(replay(&mut norm, &recorded("r6-fails-signal-term")));
        let command = "Run delayed echo sequence in background";
        assert_eq!(background_lines(&events), [format!("tid-1 command: {command} | -"), format!("- | tid-1 command failed: {command}")]);
        assert!(subagent_lines(&events).is_empty(), "a command is no helper");
    }

    /// Recording `r6-shutdown-helper-foreground-stop-1` (part 4 §2, n=2 with `-2`): "Stop" on
    /// a helper whose `Agent` call still waits. Codly gets the helper by the id of its call
    /// (the id its row has) and sends its task id, as recorded. The helper's row ends as
    /// "stopped", whoever asked gets its description with the CLI's answer, and the agent's
    /// turn goes on to its own end. An id that is neither a listed task nor a helper at work
    /// is refused, and nothing reaches the CLI.
    #[tokio::test]
    async fn a_helper_at_work_is_stopped_by_the_id_of_its_call_as_recorded() {
        let mut cli = Replay::of("r6-shutdown-helper-foreground-stop-1");
        let mut kinds = cli.play(|l| l["_sent"]["request"]["subtype"] == "stop_task").await;
        assert_eq!(turns(&kinds), ["start:Use"]);
        assert!(background_lines(&kinds).is_empty(), "a helper the agent waits for is in no list");

        // An id that names nothing running: refused, and nothing is written.
        let before = requests_sent(cli.dir.path());
        for unknown in ["toolu-9", "tid-9", "toolu-2"] {
            let (reply, mut answer) = mpsc::unbounded_channel();
            cli.actor.on_cmd(AgentCmd::Session { request: SessionRequest::StopTask(unknown.into()), reply }).await;
            let refused = answer.try_recv().expect("an answer at once").unwrap_err();
            assert!(refused.contains(&format!("nothing with the id \"{unknown}\" runs beside this agent")), "{refused}");
        }
        assert_eq!(requests_sent(cli.dir.path()), before);

        let (reply, mut answer) = mpsc::unbounded_channel();
        cli.in_place_of_the_drivers_request(AgentCmd::Session { request: SessionRequest::StopTask("toolu-1".into()), reply }).await;
        kinds.extend(cli.play(|l| l["type"] == "result").await);
        assert_eq!(answer.try_recv().expect("the CLI's answer reaches whoever asked"), Ok(json!({ "helper": "Run sleep echo command" })));
        assert_eq!(subagent_ends(&kinds), ["toolu-1 stopped: Run sleep echo command"]);
        assert!(kinds.iter().any(|k| matches!(k, EventKind::ToolCompleted { call_id, ok: false, .. } if call_id == "toolu-1")), "the Agent call returns an error");
        assert_eq!(turns(&kinds), ["start:Use", "end:Completed"], "the turn went on to its own end");
        // Once it ended, its call id names nothing any more.
        let (reply, mut answer) = mpsc::unbounded_channel();
        cli.actor.on_cmd(AgentCmd::Session { request: SessionRequest::StopTask("toolu-1".into()), reply }).await;
        assert!(answer.try_recv().unwrap().is_err());
    }

    /// Recording `r6-effort-null-first-fresh` (part 4 §4, n=2 with `-after-turn`): a process
    /// started with `--effort high`, and `effortLevel: null` as the first such request. It goes
    /// out as recorded, and the settings then say "medium": the level is taken away with no
    /// restart.
    #[tokio::test]
    async fn an_effort_is_taken_away_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, mut events, _child) = silent_actor(dir.path());
        let lines = recording("r6-effort-null-first-fresh");
        let dir = dir.path();
        let set = ask_in_place_of(&mut actor, dir, &lines, SessionRequest::Effort(None), "host-3").await;
        assert_eq!(set, Ok(Value::Null), "a bare success");
        assert_eq!(last_sent(dir)["request"], json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": null } }));
        let settings = ask_in_place_of(&mut actor, dir, &lines, SessionRequest::Settings, "host-4").await.unwrap();
        assert_eq!(settings["applied"]["effort"], "medium");
        assert!(emitted(&mut events).is_empty() && actor.session_replies.is_empty());
    }

    /// Recording `r6-model-while-streaming-1` (part 4 §5, n=2 with `-2`): `set_model sonnet`
    /// while an answer on haiku is written, a turn on sonnet, then `set_model haiku` and a turn
    /// on it. The rest of the first answer stays on haiku. Each switch is Codly's own: no row
    /// says that Claude Code "runs on" another model.
    #[tokio::test]
    async fn a_model_switch_while_an_answer_is_written_makes_no_rows_about_the_model_as_recorded() {
        let mut cli = Replay::of("r6-model-while-streaming-1");
        let lines = recording("r6-model-while-streaming-1");
        assert_eq!(models_said(&lines), ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]);
        let mut kinds = Vec::new();
        for (model, resolved) in [("sonnet", "claude-sonnet-5-5"), ("haiku", "claude-haiku-5-5")] {
            kinds.extend(cli.play(|l| l["_sent"]["request"]["subtype"] == "set_model").await);
            let (reply, mut answer) = mpsc::unbounded_channel();
            cli.in_place_of_the_drivers_request(AgentCmd::Session { request: SessionRequest::Model(model.into()), reply }).await;
            kinds.extend(cli.play(|l| l["type"] == "control_response").await);
            assert_eq!(answer.try_recv().expect("the CLI's answer reaches whoever asked"), Ok(json!({ "model": resolved })), "{model}");
        }
        kinds.extend(cli.play(|_| false).await);
        assert_eq!(turns(&kinds), ["start:Write", "end:Completed", "start:Reply", "end:Completed", "start:Reply", "end:Completed"]);
        let notices: Vec<&str> = kinds.iter().filter_map(|k| if let EventKind::Notice { text } = k { Some(text.as_str()) } else { None }).collect();
        assert_eq!(notices, Vec::<&str>::new());
        assert!(!kinds.iter().any(|k| matches!(k, EventKind::Unknown { .. })), "{kinds:?}");
    }

    /// A background task starts with the first list that names it: every later list that
    /// names it carries that time, so a row read again from a later list still has its timer.
    /// (Lists written here, in the recorded shape.)
    #[test]
    fn a_background_task_keeps_the_time_of_the_first_list_that_named_it() {
        let mut norm = ClaudeNormalizer::new(&spec(Role::Worker, false), "sess-1".into());
        let task = |id: &str| json!({ "task_id": id, "task_type": "local_bash", "description": "npm run dev" });
        let starts = |events: Vec<EventKind>| -> Vec<(String, Option<i64>)> {
            let Some(EventKind::BackgroundTasks { tasks, .. }) = events.into_iter().find(|e| matches!(e, EventKind::BackgroundTasks { .. })) else { panic!("a list event") };
            tasks.into_iter().map(|t| (t.id, t.started_at)).collect()
        };
        let before = now_ms();
        let first = starts(norm.on_message(&list_line(json!([task("tid-1")]))));
        let started = first[0].1.expect("a start time");
        assert!(started >= before);
        std::thread::sleep(Duration::from_millis(5));
        let second = starts(norm.on_message(&list_line(json!([task("tid-1"), task("tid-2")]))));
        assert_eq!(second[0], ("tid-1".to_string(), Some(started)), "one start time for one task");
        assert!(second[1].1.is_some_and(|t| t > started), "{second:?}");
        // Gone, then named again: that is work that starts anew.
        starts(norm.on_message(&list_line(json!([]))));
        std::thread::sleep(Duration::from_millis(5));
        let again = starts(norm.on_message(&list_line(json!([task("tid-1")]))));
        assert!(again[0].1.is_some_and(|t| t > started), "{again:?}");
    }
}
