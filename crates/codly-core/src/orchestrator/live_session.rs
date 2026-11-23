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

use anyhow::{Result, bail};

use super::Inner;
use crate::adapters::SessionRequest;

impl Inner {
    async fn reconnect_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let server = server.trim();
        if server.is_empty() {
            bail!("which MCP server?");
        }
        let answer = self.ask_session(agent_id, SessionRequest::McpReconnect(server.to_string()))?;
        self.control_decision(agent_id, format!("asked {} to reconnect its MCP server {server}", agent.name));
        Self::session_answer(answer).await?;
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
    /// say where the server stands after the try.
    pub async fn reconnect_agent_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        self.inner.reconnect_mcp(agent_id, server).await
    }

    /// Stop a command that a live Claude agent left running in the background (`task_id` of
    /// its `BackgroundCommand` event). The command's next event says that it was stopped.
    pub async fn stop_agent_task(&self, agent_id: &str, task_id: &str) -> Result<()> {
        self.inner.stop_background_task(agent_id, task_id).await
    }
}
