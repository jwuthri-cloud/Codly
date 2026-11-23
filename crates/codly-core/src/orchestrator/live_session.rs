//! What you do from the agent sheet to the things that run beside a live Claude agent's own
//! calls: its MCP servers and the commands it left running in the background. Both go to the
//! live session as the control requests its CLI takes (`SessionRequest`, sent as the other
//! live controls are: controls.rs), and both are on record as your act.
//!
//! - **Reconnect** an MCP server (`mcp_reconnect`). The CLI's answer is yours to read: when
//!   the server still does not connect, its own reason is the error. The adapter then asks the
//!   CLI where its servers stand, so the agent's `McpServer` events follow.
//! - **Stop** a background command (`stop_task`). The CLI answers the same whether or not a
//!   command ended; the `BackgroundCommand` event of that command says that it was stopped.
//!
//! The sheet knows the servers and the commands from their events, and each event comes once:
//! a server's when its state changes, a command's when it starts. A snapshot has only the
//! newest events of the workspace, so the run keeps those rows (`SessionRows`) and a snapshot
//! carries the ones its events no longer hold: a run that lives for hours (a Lead's) still
//! shows its servers and its running commands after you come back to its office.

use std::time::Duration;

use anyhow::{Result, bail};

use super::{Inner, lock};
use crate::adapters::SessionRequest;
use crate::model::{EventKind, EventRow};

/// How long Claude Code has to answer a reconnect. It answers when its try to connect is over
/// (recorded: the answer carries the connect error), and it gives a server `MCP_TIMEOUT` to
/// start: 30 s unless you set another (docs/research/cli-claude-codex.md).
const RECONNECT_ANSWER_LIMIT: Duration = Duration::from_secs(60);

/// What runs beside a live run's own calls, as its events last said it: the last `McpServer`
/// row of each server, and the `BackgroundCommand` row of each command that still runs.
#[derive(Default)]
pub(super) struct SessionRows(Vec<EventRow>);

impl SessionRows {
    /// One more `McpServer` or `BackgroundCommand` row of the run: it takes the place of the
    /// row of the same server or command. A command that is over is forgotten.
    fn seen(&mut self, row: &EventRow) {
        let keep = match &row.kind {
            EventKind::McpServer { name, .. } => {
                self.0.retain(|r| !matches!(&r.kind, EventKind::McpServer { name: known, .. } if known == name));
                true
            }
            EventKind::BackgroundCommand { task_id, status, .. } => {
                self.0.retain(|r| !matches!(&r.kind, EventKind::BackgroundCommand { task_id: known, .. } if known == task_id));
                status == "running"
            }
            _ => false,
        };
        if keep {
            self.0.push(row.clone());
        }
    }
}

impl Inner {
    /// The ingest pipeline's hook: an `McpServer` or `BackgroundCommand` row of an agent's
    /// run. Only a row that is on record is kept: a snapshot gives the rows of the store.
    pub(super) fn session_row_seen(&self, agent_id: &str, row: &EventRow) {
        if row.id == 0 {
            return;
        }
        if let Some(l) = lock(&self.live).get_mut(agent_id).filter(|l| row.run_id.as_deref() == Some(l.run_id.as_str())) {
            l.session_rows.seen(row);
        }
    }

    /// For a snapshot of the workspace `ws` with these `events` (oldest first): the rows its
    /// live runs keep that are older than them, oldest first.
    pub(super) fn session_rows(&self, ws: Option<&str>, events: &[EventRow]) -> Vec<EventRow> {
        let Some(first) = events.first().map(|e| e.id) else { return vec![] };
        let live = lock(&self.live);
        let mut rows: Vec<EventRow> =
            live.values().filter(|l| l.workspace_id.as_deref() == ws).flat_map(|l| &l.session_rows.0).filter(|r| r.id < first).cloned().collect();
        rows.sort_by_key(|r| r.id);
        rows
    }

    async fn reconnect_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let server = server.trim();
        if server.is_empty() {
            bail!("which MCP server?");
        }
        let answer = self.ask_session(agent_id, SessionRequest::McpReconnect(server.to_string()))?;
        self.control_decision(agent_id, format!("asked {} to reconnect its MCP server {server}", agent.name));
        Self::session_answer_within(answer, RECONNECT_ANSWER_LIMIT).await?;
        Ok(())
    }

    async fn stop_background_task(&self, agent_id: &str, task_id: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let task_id = task_id.trim();
        if task_id.is_empty() {
            bail!("which background command?");
        }
        let answer = self.ask_session(agent_id, SessionRequest::StopTask(task_id.to_string()))?;
        self.control_decision(agent_id, format!("asked {} to stop a command it left running in the background", agent.name));
        Self::session_answer(answer).await?;
        Ok(())
    }
}

impl super::Core {
    /// Try again to connect one MCP server of a live Claude agent's session. Err = the server
    /// still does not connect, with Claude Code's own reason; the agent's `McpServer` events
    /// say where the server stands after the try. Claude Code answers when its try is over:
    /// that can take as long as the server takes to start.
    pub async fn reconnect_agent_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        self.inner.reconnect_mcp(agent_id, server).await
    }

    /// Stop a command that a live Claude agent left running in the background (`task_id` of
    /// its `BackgroundCommand` event). The command's next event says that it was stopped.
    pub async fn stop_agent_task(&self, agent_id: &str, task_id: &str) -> Result<()> {
        self.inner.stop_background_task(agent_id, task_id).await
    }
}
