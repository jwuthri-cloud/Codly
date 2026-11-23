//! Agent adapters: one long-lived actor per agent run that owns the CLI process,
//! translates Codly commands into the provider protocol, and normalizes everything
//! the provider emits into `EventKind`s.
//!
//! Contract (implemented by claude.rs, codex.rs, acp.rs):
//! - `spawn` starts the process (or a thread on the shared Codex app-server), performs the
//!   handshake, emits `SessionStarted`, and returns a handle. It does NOT send a first
//!   prompt; the orchestrator sends `AgentCmd::Prompt`.
//! - Every raw stdout line is appended to `spec.raw_log` BEFORE parsing.
//! - Unknown provider messages become `EventKind::Unknown`; nothing is dropped silently.
//!   Bookkeeping lines an adapter knows by name (Claude's hooks, command lists, summaries)
//!   make no event: the raw log keeps them.
//! - Approvals: on a provider permission request the adapter generates a Codly approval id,
//!   remembers the provider request id, and emits `ApprovalRequested`. The orchestrator
//!   replies with `AgentCmd::Approval`. If the provider cancels, emit `ApprovalResolved`
//!   with status `Cancelled` and by = "provider"; if the reply cannot be written to the CLI,
//!   emit it with status `Expired` and by = "provider".
//! - Status: emit `StatusChanged` on every coarse transition (Thinking when a request
//!   starts, Working while a tool runs, WaitingApproval, Idle when a turn ends).
//! - When the process exits, emit `Exited` and stop.
//! - `AgentCmd::Prompt` is text. The images it names as attached (the composer's `- <path>`
//!   lines) go to the CLI as images where it takes them, and the text keeps their paths
//!   (attachments.rs).
//! - `Interrupt` has a deadline. A turn that does not end in time loses its run: the adapter
//!   says so (`Error`, not fatal), emits `gave_up`, closes the turn as `Interrupted` and exits.
//!
//! Verified behaviour the orchestrator relies on (live, 2026-10-25; CRITIQUE.md §10):
//! - Claude emits `SessionStarted` twice: at spawn (no tool list yet) and after the first
//!   prompt (with the real tools). Treat it as an upsert.
//! - Adapters emit `ApprovalResolved` only for a request whose answer never reached the CLI
//!   (it cancelled, or the write failed); the orchestrator emits it for user/policy
//!   decisions. The store lets by = "provider" replace an earlier decision, and nothing else.
//! - Codex assigns its own thread ids (a pre-assigned session id is ignored); ACP agents queue
//!   a prompt sent mid-turn (one prompt = one turn), Codex and Claude steer it into the
//!   running turn (`PromptSteered`, no separate turn events) or, when that turn ends first,
//!   run it as a turn of its own.
//! - Claude and Codex can start a turn by themselves: `TurnStarted` with no input.
//! - Reasoning effort rides on the model string as a suffix (`"haiku:high"`, `"gpt-6-luna:low"`).
//! - The shared Codex app-server must be stopped at exit: `codex::shutdown_server()`.

pub mod acp;
mod attachments;
pub mod claude;
pub mod classify;
pub mod codex;
mod startup;

pub use startup::SessionGone;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;

use crate::model::{ApprovalDecision, EventKind, Id, PermissionMode, Provider, Role, new_id};

/// How the codly MCP server is launched by the agent CLI (stdio transport).
#[derive(Debug, Clone)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub enum SessionMode {
    /// Start a new session. Claude: pass the pre-assigned id via `--session-id`.
    New { session_id: Option<String> },
    /// Resume a provider session (Claude `--resume`, Codex `thread/resume`, ACP `session/load`).
    Resume { session_id: String },
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub agent_id: Id,
    /// The agent's name as the user knows it. Codex: the name a new thread gets in Codex's own
    /// lists. The other CLIs do not use it.
    pub agent_name: String,
    pub run_id: Id,
    pub provider: Provider,
    pub role: Role,
    pub model: Option<String>,
    /// Working directory: the task worktree, or the repo root for Lead/Solo.
    pub cwd: PathBuf,
    /// Absolute path of the CLI binary (from env::detect).
    pub bin: PathBuf,
    /// Complete child environment (captured login env, scrubbed). Use `env_clear().envs(..)`.
    pub env: HashMap<String, String>,
    pub session: SessionMode,
    /// What the earlier runs on the resumed session cost together, as Codly counted them (0 for
    /// a new session). For ACP agents: they report a session's cost as a running total, and the
    /// adapter takes this off, so a run's cost starts from zero as with every other CLI.
    pub session_cost_usd: f64,
    /// Static role rules, appended to the CLI's own system prompt where supported
    /// (Claude `--append-system-prompt`, Codex `developerInstructions`); for ACP agents
    /// the adapter prepends it to the first prompt.
    pub system_prompt: String,
    /// Lead / reviewer / planner: no edits. Claude: `--permission-mode plan` or a read-only
    /// `--tools` list; Codex: sandbox read-only; ACP: deny edit permissions.
    pub read_only: bool,
    pub mcp: Option<McpLaunch>,
    /// Load the user's own setup beside Codly's, instead of Codly's servers alone. Claude: their
    /// MCP servers, plugins, skills and project settings; off while the folder's own CLI
    /// configuration is not trusted (`orchestrator/trust.rs`). Codex: the MCP servers of their
    /// Codex config (off = each is switched off for this thread). ACP agents ignore it.
    pub use_user_setup: bool,
    /// Codex only: commands in the workspace-write sandbox may use the network without asking
    /// (`Settings.codex_network_access`).
    pub sandbox_network: bool,
    /// Claude only: load `.claude/settings.local.json` of the working directory. Only in the
    /// user's own checkout, where that file is theirs or part of what they trusted; never in a
    /// Codly worktree, where an agent can write it.
    pub local_settings: bool,
    /// `<data>/runs/<run_id>/raw.ndjson`, append-only.
    pub raw_log: PathBuf,
    /// BYOK token command (byok.rs) the CLI runs itself, re-running it when the token expires:
    /// Claude `apiKeyHelper`, Codex `[model_providers.<id>.auth]`. A saved key travels in `env`.
    pub byok_command: Option<String>,
    /// Claude only: the agent's shell commands run inside Claude Code's own OS sandbox
    /// (`Settings.claude_sandbox`, `claude::settings_json`). Codex always has its own sandbox,
    /// and the ACP agents have none.
    pub sandbox_shell: bool,
    /// Claude only: `--max-budget-usd`, what was left of your cost budget when the run started
    /// (`breaker::rest_usd`). Claude Code counts this process alone and stops a turn right
    /// after the model request that passes it: the tool call that request asked for does not
    /// run, and every later turn of the process fails at once (2.1.311 part 4, section 10), so
    /// Codly ends the run there. Not recorded: a limit reached while a helper works, or during
    /// a long tool call. Several Claude agents at once each hold the whole rest, so together
    /// they can pass the budget until their turns end (the check between turns stays). None:
    /// no limit of its own.
    pub max_budget_usd: Option<f64>,
}

/// Commands the orchestrator sends to a running agent.
#[derive(Debug, Clone)]
pub enum AgentCmd {
    /// Start a new user turn (or queue it if a turn is running).
    Prompt(String),
    /// Stop the current turn; the process stays alive for the next one.
    Interrupt,
    Approval { approval_id: Id, decision: ApprovalDecision },
    /// Graceful shutdown ladder (interrupt → close stdin → SIGTERM → SIGKILL).
    Shutdown,
    /// Change or read the live session itself, with no restart. Claude only: the other
    /// adapters answer with an error.
    Session { request: SessionRequest, reply: SessionReply },
}

/// What Codly asks of a live Claude session: the control requests that
/// docs/research/cli-claude-stream-json-2.1.311.md (§2, §8) and its part 2 (§6, §18; the
/// effort, the settings and the default model: scenario 13) recorded. The CLI answers them at
/// once; the mode, the model and the context reading are recorded during a turn too.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionRequest {
    /// `set_permission_mode`.
    PermissionMode(PermissionMode),
    /// `set_model`, with a name of the CLI's model list ("sonnet"), a model id, or "default"
    /// for the CLI's own default model (`claude::DEFAULT_MODEL`). The answer names the model
    /// it resolves to, and the next request to the model uses it.
    Model(String),
    /// `get_context_usage`: how full the context is.
    ContextUsage,
    /// `mcp_reconnect`: try again to connect the MCP server of this name. Recorded: for a
    /// server that still cannot connect the answer is an error with the CLI's reason; for one
    /// that connects it is a success with no body (part 3 §10, n=1).
    McpReconnect(String),
    /// `mcp_toggle`: turn the MCP server of this name off (false) or on (true). Recorded
    /// (part 3 §10, n=1): a success with no body either way; the server is then `disabled`,
    /// or started again and `connected`. Where it stands is in `mcp_status`. Not recorded:
    /// whether the CLI saves the switch (no process was started after an "off"), and what it
    /// answers for a server that does not start or that it does not know.
    McpEnabled(String, bool),
    /// `stop_task`, with the id of a background task (`BackgroundTask::id`): the CLI ends it.
    /// The answer is empty also when nothing was stopped (recorded, part 2 §6): the CLI's next
    /// list of its background tasks says whether the task is gone.
    StopTask(String),
    /// `apply_flag_settings` with `effortLevel`: the reasoning effort of the session, one of
    /// `claude::EFFORT_LEVELS` (the adapter sends no other word). The CLI answers "success"
    /// also for a level it then drops, so `Settings` is asked after it.
    Effort(String),
    /// `get_settings`: what the session works with now (`applied.model`, `applied.effort`).
    Settings,
}

/// Where the answer to a `SessionRequest` goes: the `response` object of the CLI's control
/// response, or its error text. Exactly one answer is sent; a sender dropped without one
/// means the process ended first. (A channel that can be cloned, because `AgentCmd` is.)
pub type SessionReply = mpsc::UnboundedSender<Result<serde_json::Value, String>>;

#[derive(Debug, Clone)]
pub struct AdapterEvent {
    pub agent_id: Id,
    pub run_id: Id,
    pub kind: EventKind,
}

#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub agent_id: Id,
    pub run_id: Id,
    pub tx: mpsc::UnboundedSender<AgentCmd>,
}

impl AgentHandle {
    pub fn send(&self, cmd: AgentCmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

/// Start an agent run with the right adapter for its provider.
pub async fn spawn(spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> anyhow::Result<AgentHandle> {
    match spec.provider {
        Provider::Claude => claude::spawn(spec, events).await,
        Provider::Codex => codex::spawn(spec, events).await,
        Provider::Gemini | Provider::Opencode | Provider::Cursor | Provider::Copilot => acp::spawn(spec, events).await,
    }
}

/// `decision_type` of the record an adapter leaves when it ends its run because the turn did
/// not stop after an interrupt (`gave_up`).
pub const GAVE_UP: &str = "interrupt_timeout";

/// What an adapter emits when it ends its run because the turn did not stop within `limit` of
/// an interrupt, before it closes the turn and exits. The orchestrator reads it: the exit that
/// follows is Codly's doing and no crash, and who asked for the interrupt says what is next.
pub fn gave_up(cli: &str, limit: Duration) -> EventKind {
    EventKind::Decision {
        id: new_id(),
        decision_type: GAVE_UP.into(),
        method: "rule".into(),
        summary: format!("{cli} did not stop within {} s of the interrupt, so Codly ended its run", limit.as_secs()),
        detail: json!({ "limit_secs": limit.as_secs() }),
    }
}

/// `decision_type` of the record an adapter leaves when the CLI changed its permission mode by
/// itself (`permission_mode_changed`).
pub const MODE_CHANGED: &str = "permission_mode";

/// What an adapter emits when the CLI left the permission mode Codly put it in: Claude Code
/// ends plan mode when you accept its plan. The orchestrator reads it, and the agent's switch
/// follows. A switch Codly asked for makes no such record.
pub fn permission_mode_changed(mode: PermissionMode) -> EventKind {
    EventKind::Decision {
        id: new_id(),
        decision_type: MODE_CHANGED.into(),
        method: "rule".into(),
        summary: format!("Claude Code changed its mode to \"{}\"", mode.label()),
        detail: json!({ "mode": mode }),
    }
}

/// What an adapter adds when it ends a run whose turn would not stop: the prompts that waited
/// behind that turn are gone with it, and the user must know.
fn unsent_note(waiting: usize) -> String {
    match waiting {
        0 => String::new(),
        1 => " 1 message that waited for this turn was not sent: send it again.".into(),
        n => format!(" {n} messages that waited for this turn were not sent: send them again."),
    }
}
