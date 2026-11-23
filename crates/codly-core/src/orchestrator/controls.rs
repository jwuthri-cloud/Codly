//! Per-agent live controls (the agent panel): block tools, stop after this step, steer notes,
//! kill. Enforced here, in the orchestrator, not in a prompt:
//!
//! - **Block tools** denies every approval request except questions and Codly's own tools
//!   (the agent can think and answer, not act). The CLI may still run tools it never asks
//!   about (reads), so a `ToolStarted` backstop interrupts the turn once and tells the agent.
//! - **Stop after this step** lets the current turn (a "step") finish, then holds every prompt
//!   Codly would send (steer notes, chat, gate fix rounds) and keeps a worker out of the gate
//!   until you resume. Idle agents pause at once.
//! - **Steer** is an ordinary prompt tagged as a steer note: ACP queues it behind the running
//!   turn, Claude and Codex steer it into the turn. Nothing is typed into a terminal.
//! - **Kill** is the existing `stop_agent` (shutdown ladder), which also clears the controls.
//!
//! For a Claude agent, three more go to the live session as the control requests its CLI
//! takes without a restart (`SessionRequest`; docs/research/cli-claude-stream-json-2.1.311.md):
//!
//! - **Mode** (a 1:1 agent): ask for edits, accept edits, or plan first. It is kept on the
//!   agent and set again on every run. It changes what the CLI asks about, never what Codly
//!   answers: every request the CLI sends still goes through the approval broker, and a plan
//!   is always yours to accept. Two rules keep a mode under your permissions:
//!   - In "accept edits" the CLI does not ask about edits inside the worktree, nor about shell
//!     commands that write files there (recorded: `echo hi > inside.txt` ran with no request,
//!     a `touch` outside the worktree asked; deleting or moving a file with a command was not
//!     recorded in this mode, and Codly counts it as not asked either). The broker never sees
//!     those. So the mode is refused where your permissions are "ask first" (strict: every
//!     change asks you), and it ends when you choose them (`permissions_changed`).
//!   - In "plan first" the broker approves only reads, in every approval mode, until you accept
//!     the plan (ingest.rs `broker`).
//! - **Compact now** (a 1:1 agent): `/compact` as a turn of its own, between turns.
//! - **Context** reads how full the context is. The model switch (`set_agent_model`, runs.rs)
//!   uses the same path.
//!
//! `AgentControl` is the pure state machine (unit-tested below); the `impl Inner`/`impl Core`
//! blocks are the glue the ingest pipeline and `send_prompt` call.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::approvals::is_codly_tool;
use super::interrupts::Asker;
use super::{Inner, lock, row};
use crate::adapters::{AgentCmd, SessionRequest};
use crate::model::*;

/// Held steer notes / prompts per agent; the oldest drop first.
const MAX_HELD: usize = 32;

/// How long a CLI has to answer a session request. Claude answers at once, also during a turn
/// (a reconnect of an MCP server has its own limit: live_session.rs).
const SESSION_ANSWER_LIMIT: Duration = Duration::from_secs(10);

/// Claude Code's own command that summarizes the conversation to free context. Sent as a user
/// message with nothing around it: it is a turn of its own.
const COMPACT: &str = "/compact";

/// The CLI's answer to `get_context_usage`, as the agent panel shows it. None when the
/// answer has no token counts.
fn context_usage(answer: &Value) -> Option<ContextUsage> {
    let parts = answer["categories"].as_array().into_iter().flatten();
    Some(ContextUsage {
        used_tokens: answer["totalTokens"].as_i64()?,
        max_tokens: answer["maxTokens"].as_i64()?,
        percent: answer["percentage"].as_f64()?,
        auto_compact_at: answer["autoCompactThreshold"].as_i64().filter(|_| answer["isAutoCompactEnabled"] == true),
        // The other kinds are room that is not taken (free space, the reserve for compacting).
        parts: parts
            .filter(|c| c["kind"] == "used")
            .filter_map(|c| Some(ContextPart { name: c["name"].as_str()?.to_string(), tokens: c["tokens"].as_i64()? }))
            .collect(),
    })
}

/// What to do with a `ToolStarted` while tools may be blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolVerdict {
    Allow,
    /// Interrupt the turn (once) and tell the agent.
    Interrupt,
    /// Already interrupted this turn; the CLI is still winding down.
    Ignore,
}

/// The per-agent state machine.
#[derive(Debug, Default)]
pub struct AgentControl {
    block_tools: bool,
    stop_requested: bool,
    paused: bool,
    /// The block backstop already interrupted the running turn.
    interrupted: bool,
    held: Vec<String>,
}

impl AgentControl {
    pub fn state(&self) -> ControlState {
        ControlState { block_tools: self.block_tools, stop_requested: self.stop_requested, paused: self.paused, held: self.held.len() as u32 }
    }

    pub fn set_block_tools(&mut self, on: bool) {
        self.block_tools = on;
        if !on {
            self.interrupted = false;
        }
    }

    /// "Stop after this step". A running turn finishes first; an idle agent pauses now.
    pub fn request_stop(&mut self, turn_active: bool) {
        if self.paused {
            return;
        }
        if turn_active {
            self.stop_requested = true;
        } else {
            self.paused = true;
        }
    }

    /// A turn ended: a pending stop takes effect.
    pub fn turn_completed(&mut self) {
        self.interrupted = false;
        if self.stop_requested {
            self.stop_requested = false;
            self.paused = true;
        }
    }

    /// Resume (or cancel a pending stop). Returns the held prompts, oldest first, to send now.
    pub fn resume(&mut self) -> Vec<String> {
        self.stop_requested = false;
        self.paused = false;
        std::mem::take(&mut self.held)
    }

    /// Whether prompts are held back right now.
    pub fn holding(&self) -> bool {
        self.stop_requested || self.paused
    }

    /// Route a prompt: `Some` = send it now, `None` = held until the resume.
    pub fn route_prompt(&mut self, text: String) -> Option<String> {
        if !self.holding() {
            return Some(text);
        }
        if self.held.len() >= MAX_HELD {
            self.held.remove(0);
        }
        self.held.push(text);
        None
    }

    pub fn on_tool_started(&mut self, codly_tool: bool) -> ToolVerdict {
        if !self.block_tools || codly_tool {
            ToolVerdict::Allow
        } else if self.interrupted {
            ToolVerdict::Ignore
        } else {
            self.interrupted = true;
            ToolVerdict::Interrupt
        }
    }

    /// Does an approval request get an automatic deny? Questions to you stay yours to answer.
    pub fn blocks(&self, subject: &ApprovalSubject) -> bool {
        self.block_tools
            && match subject {
                ApprovalSubject::Question { .. } => false,
                ApprovalSubject::Tool { name, .. } => !is_codly_tool(name),
                _ => true,
            }
    }
}

/// The text a steer note carries into the agent's next turn.
pub fn steer_prompt(text: &str) -> String {
    format!("[Steer note from the user — context for your next step, not a new task]\n{}", text.trim())
}

const BLOCKED_NOTE: &str = "[Codly] The user has blocked tools for you: do not call any tool. Answer from what you already know, and say what you would have done.";

// ───────────────────────────── glue ─────────────────────────────

impl Inner {
    fn control_state(&self, agent_id: &str) -> ControlState {
        lock(&self.controls).get(agent_id).map(AgentControl::state).unwrap_or_default()
    }

    /// Announce the agent's controls (persisted: the UI rebuilds them on reconnect).
    fn emit_control(&self, agent_id: &str) {
        let (ws, task, run) = match lock(&self.live).get(agent_id) {
            Some(l) => (l.workspace_id.clone(), l.task_id.clone(), Some(l.run_id.clone())),
            None => (self.store.agent_workspace(agent_id).ok().flatten(), None, None),
        };
        let control = self.control_state(agent_id);
        let _ = self.emit(ws.as_deref(), row(Some(agent_id), task.as_deref(), run.as_deref(), EventKind::ControlChanged { control }));
    }

    /// Record why Codly did something to an agent (the History shows it).
    pub(super) fn control_decision(&self, agent_id: &str, summary: String) {
        let (ws, task) = match lock(&self.live).get(agent_id) {
            Some(l) => (l.workspace_id.clone(), l.task_id.clone()),
            None => (self.store.agent_workspace(agent_id).ok().flatten(), None),
        };
        let kind = EventKind::Decision {
            id: new_id(),
            decision_type: "agent_control".into(),
            method: "human".into(),
            summary,
            detail: json!({ "agent_id": agent_id }),
        };
        let _ = self.emit(ws.as_deref(), row(Some(agent_id), task.as_deref(), None, kind));
    }

    pub(super) fn live_agent(&self, agent_id: &str) -> Result<Agent> {
        let agent = self.agent(agent_id)?;
        if !self.is_live(agent_id) {
            bail!("{} is not running", agent.name);
        }
        Ok(agent)
    }

    /// `send_prompt`'s gate: Some = deliver now, None = held by a pending stop or a pause.
    pub(super) fn route_prompt(&self, agent_id: &str, text: String) -> Option<String> {
        let routed = lock(&self.controls).get_mut(agent_id).map_or(Some(text.clone()), |c| c.route_prompt(text));
        if routed.is_none() {
            self.emit_control(agent_id);
        }
        routed
    }

    /// Is the agent paused or about to be? Workers in that state stay out of the gate.
    pub(super) fn control_holding(&self, agent_id: &str) -> bool {
        lock(&self.controls).get(agent_id).is_some_and(AgentControl::holding)
    }

    /// A turn ended (called by the ingest pipeline before its reactions).
    pub(super) fn control_turn_done(&self, agent_id: &str) {
        let paused_now = lock(&self.controls).get_mut(agent_id).is_some_and(|c| {
            let was = c.stop_requested;
            c.turn_completed();
            was && c.paused
        });
        if paused_now {
            self.control_decision(agent_id, "paused after its step; resume it when you are ready".into());
            self.emit_control(agent_id);
        }
    }

    /// The backstop for tools the CLI ran without asking (reads). Called on `ToolStarted`.
    pub(super) fn control_tool_started(&self, agent_id: &str, name: &str) {
        let verdict = lock(&self.controls).get_mut(agent_id).map_or(ToolVerdict::Allow, |c| c.on_tool_started(is_codly_tool(name)));
        if verdict != ToolVerdict::Interrupt {
            return;
        }
        let sent = self.interrupt_run(agent_id, Asker::ToolBlock);
        if sent {
            self.control_decision(agent_id, format!("blocked tool {name}: interrupted the turn (tools are blocked for this agent)"));
            let _ = self.send_prompt(agent_id, BLOCKED_NOTE.into());
        }
    }

    /// Approval broker hook: deny at once when tools are blocked. True = handled.
    pub(super) fn control_deny(&self, ctx_run: &str, ctx_task: Option<&str>, ws: Option<&str>, agent_id: &str, approval: &Approval) -> bool {
        if !lock(&self.controls).get(agent_id).is_some_and(|c| c.blocks(&approval.subject)) {
            return false;
        }
        let decision = ApprovalDecision {
            choice: ApprovalChoice::Deny,
            message: Some("The user has blocked tools for you. Do not call tools; answer from what you know.".into()),
            answers: None,
        };
        let sent = lock(&self.live)
            .get(agent_id)
            .filter(|l| l.run_id == ctx_run)
            .is_some_and(|l| l.handle.send(AgentCmd::Approval { approval_id: approval.id.clone(), decision }));
        if sent {
            let kind = EventKind::ApprovalResolved { approval_id: approval.id.clone(), status: ApprovalStatus::Denied, by: "policy".into() };
            let _ = self.emit(ws, row(Some(agent_id), ctx_task, Some(ctx_run), kind));
            let what = match &approval.subject {
                ApprovalSubject::Command { command, .. } => format!("`{}`", command.chars().take(60).collect::<String>()),
                ApprovalSubject::FileChange { paths, .. } => paths.join(", "),
                ApprovalSubject::Tool { name, .. } => name.clone(),
                _ => "a permission".into(),
            };
            self.control_decision(agent_id, format!("denied {what}: tools are blocked for this agent"));
            let _ = self.update_agent(agent_id, |a| {
                if a.status == AgentStatus::WaitingApproval {
                    a.status = AgentStatus::Working;
                    a.activity = Some(ActivityKind::Thinking);
                }
            });
        }
        sent
    }

    /// The run ended (exit, kill): controls do not outlive the process.
    pub(super) fn control_clear(&self, agent_id: &str) {
        let had = lock(&self.controls).remove(agent_id).is_some_and(|c| !c.state().is_default());
        if had {
            self.emit_control(agent_id);
        }
    }

    pub(super) fn set_block_tools(&self, agent_id: &str, on: bool) -> Result<ControlState> {
        let agent = self.live_agent(agent_id)?;
        if agent.role == Role::Lead {
            bail!("the Lead needs its tools to plan; stop it instead");
        }
        lock(&self.controls).entry(agent_id.to_string()).or_default().set_block_tools(on);
        self.control_decision(agent_id, format!("{} for {}", if on { "tools blocked" } else { "tools allowed again" }, agent.name));
        self.emit_control(agent_id);
        Ok(self.control_state(agent_id))
    }

    pub(super) fn stop_after_step(&self, agent_id: &str) -> Result<ControlState> {
        let agent = self.live_agent(agent_id)?;
        let active = lock(&self.live).get(agent_id).is_some_and(|l| l.turn_active || l.queued > 0);
        lock(&self.controls).entry(agent_id.to_string()).or_default().request_stop(active);
        let state = self.control_state(agent_id);
        let what = if state.paused { "paused" } else { "will pause after its current step" };
        self.control_decision(agent_id, format!("{} {what}", agent.name));
        self.emit_control(agent_id);
        Ok(state)
    }

    pub(super) fn resume_agent(self: &Arc<Self>, agent_id: &str) -> Result<ControlState> {
        let agent = self.live_agent(agent_id)?;
        let held = {
            let mut controls = lock(&self.controls);
            let c = controls.entry(agent_id.to_string()).or_default();
            if !c.holding() {
                bail!("{} is not paused", agent.name);
            }
            c.resume()
        };
        self.control_decision(agent_id, format!("{} resumed ({} held message{})", agent.name, held.len(), if held.len() == 1 { "" } else { "s" }));
        self.emit_control(agent_id);
        let none_held = held.is_empty();
        for text in held {
            if let Err(e) = self.send_prompt(agent_id, text) {
                tracing::debug!("held prompt not delivered: {e}");
            }
        }
        // A worker whose last turn was its finish waits at the gate for this moment. (The run's
        // role: a 1:1 agent on a task works as its worker.) One whose turn failed has no finish
        // to check: it is asked to go on, now if that prompt is still owed, else after its wait.
        let (works, failed) = lock(&self.live).get(agent_id).map_or((false, false), |l| (l.role == Role::Worker, l.failed_turns > 0));
        if none_held && works && self.is_quiet(agent_id) && !self.continue_failed_turn(agent_id) && !failed {
            if let Some(task) = agent.task_id.clone() {
                let me = self.clone();
                tokio::spawn(async move { me.gate(task).await });
            }
        }
        Ok(self.control_state(agent_id))
    }

    pub(super) fn steer_agent(&self, agent_id: &str, text: &str) -> Result<ControlState> {
        let text = text.trim();
        if text.is_empty() {
            bail!("write the steer note first");
        }
        let agent = self.live_agent(agent_id)?;
        let ws = self.store.agent_workspace(agent_id)?;
        let thread = match agent.role {
            Role::Lead => super::lead::thread_of(&agent).to_string(),
            _ => format!("agent:{agent_id}"),
        };
        self.chat(ws.as_deref(), &thread, "user", &format!("Steer note: {text}"), None)?;
        self.control_decision(agent_id, format!("steer note for {}: {}", agent.name, text.chars().take(80).collect::<String>()));
        self.send_prompt(agent_id, steer_prompt(text))?;
        Ok(self.control_state(agent_id))
    }

    /// Send a request about its session to the agent's live Claude run. It is on its way when
    /// this returns (before anything sent after it); the receiver gets the CLI's answer.
    pub(super) fn ask_session(&self, agent_id: &str, request: SessionRequest) -> Result<mpsc::UnboundedReceiver<Result<Value, String>>> {
        let (reply, answer) = mpsc::unbounded_channel();
        let live = lock(&self.live);
        let l = live.get(agent_id).ok_or_else(|| anyhow!("that agent is not running"))?;
        if l.provider != Provider::Claude {
            bail!("only a Claude agent takes this without a restart");
        }
        if l.starting.is_some() {
            bail!("that agent is still starting; try again in a moment");
        }
        if l.stopping || !l.handle.send(AgentCmd::Session { request, reply }) {
            bail!("that agent is shutting down");
        }
        Ok(answer)
    }

    /// Wait for the CLI's answer to `ask_session`.
    pub(super) async fn session_answer(answer: mpsc::UnboundedReceiver<Result<Value, String>>) -> Result<Value> {
        Self::session_answer_within(answer, SESSION_ANSWER_LIMIT).await
    }

    /// As `session_answer`, for a request the CLI answers only when work of its own is over.
    pub(super) async fn session_answer_within(mut answer: mpsc::UnboundedReceiver<Result<Value, String>>, limit: Duration) -> Result<Value> {
        match tokio::time::timeout(limit, answer.recv()).await {
            Ok(Some(Ok(v))) => Ok(v),
            Ok(Some(Err(e))) => bail!("Claude Code answered: {e}"),
            Ok(None) => bail!("the agent's process ended before it answered"),
            Err(_) => bail!("Claude Code did not answer within {} s", limit.as_secs()),
        }
    }

    /// Put a 1:1 Claude agent in another permission mode: its live session now, every later
    /// run when it starts (`restore_permission_mode`).
    pub(super) async fn set_permission_mode(&self, agent_id: &str, mode: PermissionMode) -> Result<Agent> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo || agent.provider != Provider::Claude {
            bail!("only a 1:1 Claude agent has a mode to switch");
        }
        // Its run of a task works as a worker, in the task's own worktree, where edits are
        // accepted: that run is not the conversation this switch is for.
        let chat = lock(&self.live).get(agent_id).map(|l| l.role == Role::Solo && !l.read_only);
        if agent.task_id.is_some() || chat == Some(false) {
            bail!("{} is on a task: its mode can change when the task is done", agent.name);
        }
        if mode == PermissionMode::AcceptEdits && self.asks_first(agent_id) {
            bail!(
                "{}'s permissions are \"Ask first\": every change asks you, so it cannot accept edits by itself. Give it other permissions first.",
                agent.name
            );
        }
        if agent.permission_mode.unwrap_or_default() == mode {
            return Ok(agent);
        }
        if chat == Some(true) {
            let answer = self.ask_session(agent_id, SessionRequest::PermissionMode(mode))?;
            Self::session_answer(answer).await?;
        }
        self.control_decision(agent_id, format!("{} now works in the mode \"{}\"", agent.name, mode.label()));
        self.update_agent(agent_id, |a| a.permission_mode = Some(mode))
    }

    /// A run started: a 1:1 Claude agent goes back to the mode you chose for it (its CLI
    /// starts in "ask for edits"). Called before the run's first prompt is sent, so the mode
    /// is on when the CLI reads that prompt.
    pub(super) fn restore_permission_mode(self: &Arc<Self>, agent: &Agent, role: Role, read_only: bool) {
        let Some(mode) = agent.permission_mode.filter(|m| *m != PermissionMode::Ask) else { return };
        if agent.provider != Provider::Claude || role != Role::Solo || read_only {
            return;
        }
        if mode == PermissionMode::AcceptEdits && self.asks_first(&agent.id) {
            return self.end_accept_edits(agent); // the CLI starts in "ask for edits"
        }
        let answer = match self.ask_session(&agent.id, SessionRequest::PermissionMode(mode)) {
            Ok(answer) => answer,
            // Stopped while it started: there is no session to set up.
            Err(e) => return tracing::debug!("{}'s mode not restored: {e}", agent.name),
        };
        let (me, id, name) = (self.clone(), agent.id.clone(), agent.name.clone());
        tokio::spawn(async move {
            let Err(e) = Self::session_answer(answer).await else { return };
            // The CLI stayed in "ask for edits", the careful one: the switch shows that, and why.
            me.mode_problem(&id, format!("{name} could not go back to the mode \"{}\" ({e}). It asks before every edit instead.", mode.label()));
            let _ = me.update_agent(&id, |a| a.permission_mode = None);
        });
    }

    /// A mode switch Codly made by itself did not happen: say so in the agent's timeline.
    fn mode_problem(&self, agent_id: &str, message: String) {
        let (ws, run) = match lock(&self.live).get(agent_id) {
            Some(l) => (l.workspace_id.clone(), Some(l.run_id.clone())),
            None => (self.store.agent_workspace(agent_id).ok().flatten(), None),
        };
        let _ = self.emit(ws.as_deref(), row(Some(agent_id), None, run.as_deref(), EventKind::Error { message, fatal: false }));
    }

    /// Are the permissions of the agent's own conversation "ask first" (strict: every change
    /// asks you)? The broker reads the same choice for a 1:1 chat.
    fn asks_first(&self, agent_id: &str) -> bool {
        let ws = self.store.agent_workspace(agent_id).ok().flatten();
        self.approval_mode_for(ws.as_deref(), Some(&format!("agent:{agent_id}"))) == ApprovalMode::Strict
    }

    /// "Accept edits" cannot stay where every change asks you (`asks_first`): the agent's switch
    /// goes back to "ask for edits", on record. The caller sees to a live session first.
    fn end_accept_edits(&self, agent: &Agent) {
        self.control_decision(&agent.id, format!("{} asks for edits again: its permissions are \"Ask first\", where every change asks you", agent.name));
        if let Err(e) = self.update_agent(&agent.id, |a| a.permission_mode = Some(PermissionMode::Ask)) {
            tracing::warn!("{}'s mode not recorded: {e}", agent.name);
        }
    }

    /// Your permissions changed, in Settings or for one conversation. Where they are now "ask
    /// first", a 1:1 agent in "accept edits" goes back to "ask for edits": its live
    /// conversation at once, before its switch says so.
    pub(super) async fn permissions_changed(&self) {
        let agents = match self.store.all_agents() {
            Ok(agents) => agents,
            Err(e) => return tracing::warn!("permission modes not checked after a change of permissions: {e}"),
        };
        for agent in agents.iter().filter(|a| a.permission_mode == Some(PermissionMode::AcceptEdits) && self.asks_first(&a.id)) {
            // Its run of a task is no conversation in this mode (`restore_permission_mode`).
            let chat = lock(&self.live).get(&agent.id).is_some_and(|l| l.role == Role::Solo && !l.read_only);
            if chat {
                // Starting or shutting down: no session takes the request. A run that is
                // starting sets no mode (`restore_permission_mode` asks the same question).
                let Ok(answer) = self.ask_session(&agent.id, SessionRequest::PermissionMode(PermissionMode::Ask)) else { continue };
                if let Err(e) = Self::session_answer(answer).await {
                    let message = format!(
                        "{} did not leave the mode \"accept edits\" ({e}), although its permissions are now \"Ask first\". Stop it: its next run asks for edits.",
                        agent.name
                    );
                    self.mode_problem(&agent.id, message);
                    continue;
                }
            }
            self.end_accept_edits(agent);
        }
    }

    /// The CLI changed its permission mode by itself (`adapters::permission_mode_changed`):
    /// Claude Code ends plan mode when you accept its plan. The agent's switch follows what is
    /// true, in its own conversation only (a task run of a 1:1 agent has no switch).
    pub(super) fn control_mode_changed(&self, agent_id: &str, role: Role, detail: &Value) {
        let Ok(mode) = serde_json::from_value::<PermissionMode>(detail["mode"].clone()) else { return };
        if role != Role::Solo {
            return;
        }
        if let Err(e) = self.update_agent(agent_id, |a| a.permission_mode = Some(mode)) {
            tracing::debug!("permission mode not recorded: {e}");
        }
    }

    /// "Compact now": Claude Code summarizes the conversation to free context. It is a turn of
    /// its own, so only between turns, and only for a 1:1 agent: for a worker, a reviewer or
    /// the Lead the end of a turn means its work is ready to be checked.
    pub(super) fn compact_claude(&self, agent_id: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        if self.control_holding(agent_id) {
            bail!("{} is paused: resume it first", agent.name);
        }
        {
            let mut live = lock(&self.live);
            let l = live.get_mut(agent_id).ok_or_else(|| anyhow!("{} is not running", agent.name))?;
            if l.provider != Provider::Claude {
                bail!("only a Claude or a Codex agent can be compacted from here");
            }
            if l.role != Role::Solo {
                if agent.role == Role::Solo {
                    bail!("{} is on a task: compact it when the task is done", agent.name);
                }
                bail!("only a 1:1 agent can be compacted by hand");
            }
            if l.starting.is_some() || l.stopping {
                bail!("{} is starting or shutting down; try again in a moment", agent.name);
            }
            if l.turn_active || l.queued > 0 {
                bail!("{} is still working: compact it when its turn ends", agent.name);
            }
            // The command alone (`deliver_prompt` would put the briefing around it): one turn.
            if !l.handle.send(AgentCmd::Prompt(COMPACT.into())) {
                bail!("that agent's process is gone");
            }
            l.queued += 1;
        }
        self.control_decision(agent_id, format!("asked {} to compact its conversation", agent.name));
        Ok(())
    }

    /// How full the agent's context is, asked of its live Claude session.
    pub(super) async fn agent_context_usage(&self, agent_id: &str) -> Result<ContextUsage> {
        self.live_agent(agent_id)?;
        let answer = self.ask_session(agent_id, SessionRequest::ContextUsage)?;
        let answer = Self::session_answer(answer).await?;
        context_usage(&answer).ok_or_else(|| anyhow!("Claude Code's answer had no token counts"))
    }
}

impl super::Core {
    /// The agent's live controls (all off for an agent that is not running).
    pub async fn agent_control(&self, agent_id: &str) -> Result<ControlState> {
        self.inner.agent(agent_id).map_err(|_| anyhow!("unknown agent"))?;
        Ok(self.inner.control_state(agent_id))
    }

    /// Let the agent think and answer but not run tools (approvals are denied, tool starts
    /// interrupt the turn). Not for the Lead.
    pub async fn set_agent_block_tools(&self, agent_id: &str, on: bool) -> Result<ControlState> {
        self.inner.set_block_tools(agent_id, on)
    }

    /// Finish the current step, then pause; nothing new starts until `resume_agent`.
    pub async fn stop_agent_after_step(&self, agent_id: &str) -> Result<ControlState> {
        self.inner.stop_after_step(agent_id)
    }

    /// Resume a paused agent (or cancel a pending stop) and deliver what was held.
    pub async fn resume_agent(&self, agent_id: &str) -> Result<ControlState> {
        self.inner.resume_agent(agent_id)
    }

    /// A note that reaches the agent as context on its next turn; held while it is paused.
    pub async fn steer_agent(&self, agent_id: &str, text: &str) -> Result<ControlState> {
        self.inner.steer_agent(agent_id, text)
    }

    /// How a 1:1 Claude agent works with edits: asks first, accepts them, or plans first. A
    /// live session switches at once; the mode is kept for its next runs.
    pub async fn set_agent_permission_mode(&self, agent_id: &str, mode: PermissionMode) -> Result<Agent> {
        self.inner.set_permission_mode(agent_id, mode).await
    }

    /// Ask a live agent, between turns, to compact its conversation now: a Codex agent off a
    /// task (`thread/compact/start`), or a 1:1 Claude agent (Claude Code's own command).
    pub async fn compact_agent(&self, agent_id: &str) -> Result<()> {
        if self.inner.agent(agent_id).is_ok_and(|a| a.provider == Provider::Codex) {
            self.inner.compact_codex(agent_id).await
        } else {
            self.inner.compact_claude(agent_id)
        }
    }

    /// How full a live Claude agent's context is, as its CLI counts it right now.
    pub async fn agent_context_usage(&self, agent_id: &str) -> Result<ContextUsage> {
        self.inner.agent_context_usage(agent_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(c: &str) -> ApprovalSubject {
        ApprovalSubject::Command { command: c.into(), cwd: None }
    }

    #[test]
    fn running_agent_pauses_only_when_its_turn_ends() {
        let mut c = AgentControl::default();
        c.request_stop(true);
        assert_eq!(c.state(), ControlState { stop_requested: true, ..Default::default() });
        // A steer note arriving now must not start another turn behind the current one.
        assert_eq!(c.route_prompt("note".into()), None);
        assert_eq!(c.state().held, 1);
        c.turn_completed();
        assert_eq!(c.state(), ControlState { paused: true, held: 1, ..Default::default() });
        assert_eq!(c.resume(), vec!["note".to_string()]);
        assert!(c.state().is_default());
        assert_eq!(c.route_prompt("go".into()), Some("go".into()));
    }

    #[test]
    fn idle_agent_pauses_at_once_and_resume_cancels_a_pending_stop() {
        let mut c = AgentControl::default();
        c.request_stop(false);
        assert!(c.state().paused && !c.state().stop_requested);
        c.resume();
        c.request_stop(true);
        assert!(c.resume().is_empty()); // changed its mind before the step ended
        c.turn_completed();
        assert!(!c.state().paused);
        // Stopping twice is harmless.
        c.request_stop(false);
        c.request_stop(true);
        assert!(c.state().paused && !c.state().stop_requested);
    }

    #[test]
    fn held_prompts_keep_their_order_and_are_capped() {
        let mut c = AgentControl::default();
        c.request_stop(false);
        for i in 0..MAX_HELD + 3 {
            assert_eq!(c.route_prompt(format!("n{i}")), None);
        }
        let held = c.resume();
        assert_eq!(held.len(), MAX_HELD);
        assert_eq!(held[0], "n3");
        assert_eq!(held.last().unwrap(), &format!("n{}", MAX_HELD + 2));
    }

    #[test]
    fn blocking_tools_denies_actions_but_not_questions_or_codly_tools() {
        let mut c = AgentControl::default();
        assert!(!c.blocks(&cmd("ls")));
        c.set_block_tools(true);
        assert!(c.blocks(&cmd("cargo test")));
        assert!(c.blocks(&ApprovalSubject::FileChange { paths: vec!["a.rs".into()], diff: None }));
        assert!(c.blocks(&ApprovalSubject::Tool { name: "Read".into(), input: json!({}) }));
        assert!(!c.blocks(&ApprovalSubject::Tool { name: "mcp__codly__codly_board".into(), input: json!({}) }));
        assert!(!c.blocks(&ApprovalSubject::Question { questions: json!([]) }));
        c.set_block_tools(false);
        assert!(!c.blocks(&cmd("cargo test")));
    }

    #[test]
    fn tool_backstop_interrupts_once_per_turn() {
        let mut c = AgentControl::default();
        assert_eq!(c.on_tool_started(false), ToolVerdict::Allow);
        c.set_block_tools(true);
        assert_eq!(c.on_tool_started(true), ToolVerdict::Allow);
        assert_eq!(c.on_tool_started(false), ToolVerdict::Interrupt);
        assert_eq!(c.on_tool_started(false), ToolVerdict::Ignore);
        c.turn_completed();
        assert_eq!(c.on_tool_started(false), ToolVerdict::Interrupt);
    }

    #[test]
    fn blocked_and_paused_are_independent() {
        let mut c = AgentControl::default();
        c.set_block_tools(true);
        c.request_stop(false);
        c.resume();
        assert!(c.state().block_tools && !c.state().paused);
    }

    /// Claude Code 2.1.311's recorded answer to `get_context_usage`.
    #[test]
    fn context_usage_is_read_from_the_recorded_answer() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures/claude-2.1.311/control-requests.ndjson");
        let lines: Vec<Value> = std::fs::read_to_string(&path).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect();
        let asked = lines.iter().position(|l| l["_sent"]["request"]["subtype"] == "get_context_usage").expect("the recorded request");
        let id = lines[asked]["_sent"]["request_id"].clone();
        let answer = lines.iter().find(|l| l["type"] == "control_response" && l["response"]["request_id"] == id).expect("its answer");
        let usage = context_usage(&answer["response"]["response"]).expect("token counts");
        assert_eq!((usage.used_tokens, usage.max_tokens, usage.percent), (6035, 1_000_000, 1.0));
        assert_eq!(usage.auto_compact_at, Some(967_000));
        // Free space and the reserve for compacting are not parts of what is used.
        let parts: Vec<(&str, i64)> = usage.parts.iter().map(|p| (p.name.as_str(), p.tokens)).collect();
        assert_eq!(parts, [("System prompt", 2251), ("System tools", 2342), ("Messages", 1442)]);
        assert_eq!(usage.parts.iter().map(|p| p.tokens).sum::<i64>(), usage.used_tokens);

        // Auto-compaction off: no threshold is shown. An answer without counts is no reading.
        let mut off = answer["response"]["response"].clone();
        off["isAutoCompactEnabled"] = json!(false);
        assert_eq!(context_usage(&off).unwrap().auto_compact_at, None);
        assert_eq!(context_usage(&json!({ "categories": [] })), None);
    }

    #[test]
    fn steer_prompt_is_tagged_and_trimmed() {
        let p = steer_prompt("  use the v2 client \n");
        assert!(p.starts_with("[Steer note from the user"));
        assert!(p.ends_with("use the v2 client"));
    }
}
