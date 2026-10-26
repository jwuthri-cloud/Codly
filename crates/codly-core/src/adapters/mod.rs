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
//! - Approvals: on a provider permission request the adapter generates a Codly approval id,
//!   remembers the provider request id, and emits `ApprovalRequested`. The orchestrator
//!   replies with `AgentCmd::Approval`. If the provider cancels, emit `ApprovalResolved`
//!   with status `Cancelled` and by = "provider".
//! - Status: emit `StatusChanged` on every coarse transition (Thinking when a request
//!   starts, Working while a tool runs, WaitingApproval, Idle when a turn ends).
//! - When the process exits, emit `Exited` and stop.
//!
//! Verified behaviour the orchestrator relies on (live, 2026-10-25; CRITIQUE.md §10):
//! - Claude emits `SessionStarted` twice: at spawn (no tool list yet) and after the first
//!   prompt (with the real tools). Treat it as an upsert.
//! - Adapters emit `ApprovalResolved` only when the CLI itself cancels a request; the
//!   orchestrator emits it for user/policy decisions.
//! - Codex assigns its own thread ids (a pre-assigned session id is ignored); Claude and ACP
//!   agents queue a prompt sent mid-turn (one prompt = one turn), Codex steers it into the
//!   running turn (no separate turn events).
//! - Reasoning effort rides on the model string as a suffix (`"haiku:high"`, `"gpt-6-luna:low"`).
//! - The shared Codex app-server must be stopped at exit: `codex::shutdown_server()`.

pub mod acp;
pub mod claude;
pub mod classify;
pub mod codex;

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::model::{ApprovalDecision, EventKind, Id, Provider, Role};

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
    /// Static role rules, appended to the CLI's own system prompt where supported
    /// (Claude `--append-system-prompt`, Codex `developerInstructions`); for ACP agents
    /// the adapter prepends it to the first prompt.
    pub system_prompt: String,
    /// Lead / reviewer / planner: no edits. Claude: `--permission-mode plan` or a read-only
    /// `--tools` list; Codex: sandbox read-only; ACP: deny edit permissions.
    pub read_only: bool,
    pub mcp: Option<McpLaunch>,
    /// `<data>/runs/<run_id>/raw.ndjson`, append-only.
    pub raw_log: PathBuf,
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
}

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
