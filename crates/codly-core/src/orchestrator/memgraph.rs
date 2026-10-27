//! Core entry points for the Knowledge room's Find, Files and Messages lenses. The logic is in
//! `memory::panel` (pure, unit-tested); this only loads the rows and finds the memory folder.

use anyhow::anyhow;

use super::*;
use crate::memory::panel::{self, ExactHit, MemoryFile, MessageGraph};

impl Core {
    /// Literal text search across tasks, blackboard notes and memories of the open workspace.
    pub async fn exact_search(&self, query: &str) -> anyhow::Result<Vec<ExactHit>> {
        let Some(ws) = self.inner.workspace() else { return Ok(Vec::new()) };
        let view = MemoryView { workspace_id: Some(&ws.id), agent_id: None, task_ids: &[] };
        let tasks = self.inner.store.tasks(&ws.id)?;
        let memories = self.inner.store.with(|db| guard("memory", || memory::list_memories(db, &view, None, 2000)))?;
        Ok(panel::exact_search(query, &tasks, &memories, 60))
    }

    /// Who messaged whom, from the chats and blackboard notes Codly already recorded.
    pub async fn message_graph(&self) -> anyhow::Result<MessageGraph> {
        let Some(ws) = self.inner.workspace() else { return Ok(MessageGraph::default()) };
        let view = MemoryView { workspace_id: Some(&ws.id), agent_id: None, task_ids: &[] };
        let agents = self.inner.store.agents(&ws.id)?;
        let tasks = self.inner.store.tasks(&ws.id)?;
        let chats = self.inner.store.recent_chats(Some(&ws.id), 500)?;
        let notes = self.inner.store.with(|db| guard("memory", || memory::list_memories(db, &view, None, 2000)))?;
        Ok(panel::build_message_graph(&agents, &chats, &notes, &tasks))
    }

    /// The agent's memory file (a starter template until it is first saved).
    pub async fn memory_file(&self, agent_id: &str) -> anyhow::Result<MemoryFile> {
        let agent = self.workspace_agent(agent_id)?;
        panel::read_memory_file(&self.inner.data("memory"), &agent.id, &agent.name)
    }

    pub async fn save_memory_file(&self, agent_id: &str, text: &str) -> anyhow::Result<MemoryFile> {
        let agent = self.workspace_agent(agent_id)?;
        panel::write_memory_file(&self.inner.data("memory"), &agent.id, &agent.name, text)
    }

    /// An agent of the open workspace: the memory folder is addressed by ids Codly knows only.
    fn workspace_agent(&self, agent_id: &str) -> anyhow::Result<Agent> {
        let ws = self.inner.require_ws()?;
        let agent = self.inner.store.agent(agent_id)?.ok_or_else(|| anyhow!("unknown agent"))?;
        if self.inner.store.agent_workspace(agent_id)?.as_deref() != Some(ws.id.as_str()) {
            return Err(anyhow!("that agent is not in this workspace"));
        }
        Ok(agent)
    }
}
