//! After a context compaction a worker gets its task again. The CLI replaces the conversation
//! with its own summary, and the brief's goal, write scope, checks and "do not commit" can be
//! lost in it. The task rides the next prompt the worker gets (the gate's feedback, your
//! answer, a message from you): a prompt of its own would be one more turn for the worker to
//! answer, and its end one more turn for the gate to check.
//!
//! Nothing is kept in memory: the task's events say when its worker's context was compacted
//! and which prompts carried the task (a brief's first line names it). Each time the task is
//! sent again is a Decision on the task.

use anyhow::Result;
use serde_json::json;

use super::{Inner, lock, prompts};
use crate::model::*;

/// The first line of a task's brief: a prompt with it carries the task.
fn marker(t: &Task) -> String {
    format!("[Codly] Task {}:", t.key)
}

/// The task as its brief has it (goal, done-when, write scope, checks, "do not commit"),
/// without the context packs.
fn rules(t: &Task) -> String {
    let brief = prompts::Brief { task: t, plan: None, deps: vec![], pointer_pack: None, memory_pack: None, role: None, history: vec![] };
    format!("[Codly] Your context was compacted since you last read your task. Here it is again:\n{}", prompts::task_brief(&brief))
}

impl Inner {
    /// A prompt for a worker's run of a task, with the task after it when the worker's context
    /// was compacted since a prompt last carried it.
    pub(super) fn with_task_rules(&self, agent_id: &str, text: String) -> String {
        let task_id = lock(&self.live).get(agent_id).filter(|l| l.role == Role::Worker).and_then(|l| l.task_id.clone());
        let Some(task_id) = task_id else { return text };
        match self.rules_lost(agent_id, &task_id) {
            Ok(Some(task)) if !text.contains(&marker(&task)) => {
                let ws = self.store.task_workspace(&task_id).ok().flatten();
                let summary = format!("{}: the worker's context was compacted; its next prompt carries the task again", task.key);
                if let Err(e) = self.decide(ws.as_deref(), Some(&task_id), "context", summary, json!({ "plan_id": task.plan_id })) {
                    tracing::debug!("decision not recorded: {e}");
                }
                format!("{text}\n\n{}", rules(&task))
            }
            Ok(_) => text,
            Err(e) => {
                tracing::warn!("could not tell if the worker of task {task_id} lost its task in a compaction: {e}");
                text
            }
        }
    }

    /// The task, if its worker's context was compacted after the last prompt that carried it.
    fn rules_lost(&self, agent_id: &str, task_id: &str) -> Result<Option<Task>> {
        let own = |e: &&EventRow| e.agent_id.as_deref() == Some(agent_id);
        let Some(compacted) = self.store.task_events(task_id, "compacting")?.iter().filter(own).map(|e| e.id).max() else {
            return Ok(None);
        };
        let task = self.task(task_id)?;
        let marker = marker(&task);
        let mut told = None;
        // A prompt is a turn of its own, or (Codex) steered into the running one.
        for kind in ["turn_started", "prompt_steered"] {
            for e in self.store.task_events(task_id, kind)?.iter().filter(own) {
                let input = match &e.kind {
                    EventKind::TurnStarted { input } => input.as_deref(),
                    EventKind::PromptSteered { input } => Some(input.as_str()),
                    _ => None,
                };
                if input.is_some_and(|i| i.contains(&marker)) {
                    told = told.max(Some(e.id));
                }
            }
        }
        Ok(told.is_none_or(|t| t < compacted).then_some(task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_name_the_task_as_its_brief_does() {
        let t: Task = serde_json::from_value(json!({
            "id": "t", "plan_id": null, "key": "t1", "title": "Do it", "goal": "Make it work", "acceptance": ["it works"],
            "write_scope": ["src/**"], "verify": ["cargo test"], "depends_on": [], "status": "running", "assignee": null,
            "agent_hint": null, "branch": null, "worktree": null, "attempt": 1, "status_reason": null, "summary": null,
            "created_at": 0, "updated_at": 0,
        }))
        .unwrap();
        let r = rules(&t);
        assert!(r.contains(&marker(&t)), "a prompt with the rules counts as one that carried the task: {r}");
        for part in ["Make it work", "it works", "src/**", "cargo test", "Do not commit", "codly_submit_report"] {
            assert!(r.contains(part), "{part} is missing: {r}");
        }
    }
}
