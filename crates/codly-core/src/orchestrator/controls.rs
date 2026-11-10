//! Per-agent live controls (the agent panel): block tools, stop after this step, steer notes,
//! kill. Enforced here, in the orchestrator, not in a prompt:
//!
//! - **Block tools** denies every approval request except questions and Codly's own tools
//!   (the agent can think and answer, not act). The CLI may still run tools it never asks
//!   about (reads), so a `ToolStarted` backstop interrupts the turn once and tells the agent.
//! - **Stop after this step** lets the current turn (a "step") finish, then holds every prompt
//!   Codly would send (steer notes, chat, gate fix rounds) and keeps a worker out of the gate
//!   until you resume. Idle agents pause at once.
//! - **Steer** is an ordinary prompt tagged as a steer note: Claude/ACP queue it behind the
//!   running turn, Codex steers it into the turn. Nothing is typed into a terminal.
//! - **Kill** is the existing `stop_agent` (shutdown ladder), which also clears the controls.
//!
//! `AgentControl` is the pure state machine (unit-tested below); the `impl Inner`/`impl Core`
//! blocks are the glue the ingest pipeline and `send_prompt` call.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde_json::json;

use super::approvals::is_codly_tool;
use super::{Inner, lock, row};
use crate::adapters::AgentCmd;
use crate::model::*;

/// Held steer notes / prompts per agent; the oldest drop first.
const MAX_HELD: usize = 32;

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
    fn control_decision(&self, agent_id: &str, summary: String) {
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

    fn live_agent(&self, agent_id: &str) -> Result<Agent> {
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
        let sent = lock(&self.live).get(agent_id).is_some_and(|l| l.handle.send(AgentCmd::Interrupt));
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
        // role: a 1:1 agent on a task works as its worker.)
        let works = lock(&self.live).get(agent_id).is_some_and(|l| l.role == Role::Worker);
        if none_held && works && self.is_quiet(agent_id) {
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

    #[test]
    fn steer_prompt_is_tagged_and_trimmed() {
        let p = steer_prompt("  use the v2 client \n");
        assert!(p.starts_with("[Steer note from the user"));
        assert!(p.ends_with("use the v2 client"));
    }
}
