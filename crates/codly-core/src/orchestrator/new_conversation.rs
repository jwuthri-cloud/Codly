//! "New conversation" (`/new` in a 1:1 chat): the same agent, folder and model go on with an
//! empty context, as `/clear` or `/new` do in the CLI itself.
//!
//! Nothing of the CLI is needed that is not recorded: a run with no session id starts a new
//! session (`new_session`), as every agent's first message does. So Codly stops the agent's
//! process through the normal stop, forgets its session id, and the next message starts the new
//! conversation by the usual path. The old conversation is not deleted: the CLI keeps its own
//! files, and the decision on record holds its session id, so it can be found again.
//!
//! Only a 1:1 agent, between turns, off every task, and not in your terminal. Its files, its
//! branch and its checkpoints stay as they are. A note about restored files that waits for its
//! next prompt goes: it speaks of turns the new conversation never had.

use anyhow::{Result, bail};
use serde_json::json;

use super::{Inner, row};
use crate::model::*;
use crate::store;

/// The line by Codly in the agent's 1:1 thread.
fn new_conversation_line(name: &str) -> String {
    format!("New conversation. {name} starts without what was said above. Files and checkpoints are as they are.")
}

impl Inner {
    async fn new_conversation(&self, ws: &Workspace, id: &str) -> Result<()> {
        let a = self.agent(id)?;
        if a.archived || self.store.agent_workspace(id)?.as_deref() != Some(ws.id.as_str()) {
            bail!("{} doesn't work in this office", a.name);
        }
        if a.role != Role::Solo {
            bail!("only a 1:1 agent can start a new conversation");
        }
        // No scheduling pass from the check below until its session is gone: while its idle chat
        // shuts down it looks free, and a pass would hand it a task in the middle of this.
        let _g = self.sched_lock.lock().await;
        // A task given to it that is not over (waiting for its turn, or stopped and retried
        // later) goes back to it in that task's worktree, where a note about restored files may
        // wait for that task. One that stopped waits for you, not for its end.
        let open = |t: &Task| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled);
        let held = self.store.tasks(&ws.id)?.into_iter().find(|t| a.task_id.as_ref() == Some(&t.id) || (t.assignee.as_ref() == Some(&a.id) && open(t)));
        if let Some(t) = held.as_ref().filter(|t| a.task_id.is_none() && matches!(t.status, TaskStatus::Failed | TaskStatus::Blocked)) {
            bail!("{} still has {} ({}): retry it or cancel it first", a.name, t.key, store::enum_str(&t.status));
        }
        if a.task_id.is_some() || held.is_some() {
            bail!("{} is on {}: start a new conversation when the task is done", a.name, held.map_or("a task".into(), |t| t.key));
        }
        self.not_in_terminal(&a)?;
        // Looked at and told to stop in one step: a prompt or a start that got in first keeps
        // its turn, and nothing gets in after the look.
        let mut working = false;
        self.stop_run_if(id, |l| {
            working = l.turn_active || l.queued > 0 || l.starting.is_some();
            !working
        });
        if working {
            bail!("{} is still working: stop its turn first", a.name);
        }
        if !self.stop_and_wait(id).await {
            bail!("{} did not stop in time; try again", a.name);
        }
        let mut old = None;
        self.update_agent(id, |a| {
            old = a.session_id.take();
            // The context meter: the new conversation holds nothing yet.
            a.usage.context_used = None;
        })?;
        self.forget_restore_notes(id);
        self.chat(Some(&ws.id), &format!("agent:{id}"), "codly", &new_conversation_line(&a.name), None)?;
        let kind = EventKind::Decision {
            id: new_id(),
            decision_type: "conversation".into(),
            method: "human".into(),
            summary: format!("you started a new conversation with {}", a.name),
            detail: json!({ "agent_id": id, "session_id": old }),
        };
        self.emit(Some(&ws.id), row(Some(id), None, None, kind))?;
        Ok(())
    }
}

impl super::Core {
    /// "New conversation" with a 1:1 agent: its next message starts a new session of its CLI,
    /// in the same worktree, with the same model. Refused for any other agent, mid-turn, on a
    /// task and in your terminal.
    pub async fn new_conversation(&self, agent_id: &str) -> Result<()> {
        let ws = self.inner.require_ws()?;
        self.inner.new_conversation(&ws, agent_id).await
    }
}
