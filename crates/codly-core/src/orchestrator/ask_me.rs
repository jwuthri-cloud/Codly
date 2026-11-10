//! "Ask me": an agent raises a card for the human with `codly_ask_user`; the human replies
//! ("respond & unblock") and Codly resumes whoever was waiting.
//!
//! - **Heads-up** cards need no action: the agent carries on, you can read and dismiss them.
//! - **Blocking** cards pause the agent's task (`Blocked`, with a "waiting for you" reason the
//!   gate and the board both show) until you reply. The reply reaches the worker as a `[User]`
//!   prompt in its own session, or the Lead as a message in its request thread. If the worker's
//!   process is gone, the task goes back to `Ready` with your answer appended to its goal.
//! - A card still open after `REMIND_AFTER_DAYS` is flagged (`reminded_at`) so the UI can nag.
//!
//! State is event-sourced (`crate::blockers`); this file is the orchestration around it.

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{Inner, lead, prompts, row};
use crate::blockers::{self, Blocker, BlockerSeverity, BlockerStatus};
use crate::mcp::ToolCtx;
use crate::model::*;

/// Prefix of the `status_reason` of a task that waits for an answer.
const WAITING: &str = "waiting for you:";
/// Open cards one agent may have at once; an agent that floods the inbox is told to stop.
const MAX_OPEN_PER_AGENT: usize = 5;
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 2000;
const MAX_ANSWER: usize = 4000;

/// Blocker changes are read-modify-write over the event log: one at a time.
static CHANGES: Mutex<()> = Mutex::new(());

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

impl Inner {
    /// The `codly_ask_user` tool: record the card, and park the task when it blocks.
    pub(super) fn tool_ask_user(&self, ctx: &ToolCtx, ws: Option<&str>, args: &Value) -> Result<Value> {
        let title = s(args, "title").ok_or_else(|| anyhow!("title is required: one line, e.g. \"Which payment provider?\""))?;
        let body = s(args, "question").ok_or_else(|| anyhow!("question is required: what you need and why, in plain language"))?;
        let severity = match s(args, "severity").unwrap_or("blocking") {
            "blocking" => BlockerSeverity::Blocking,
            "heads_up" => BlockerSeverity::HeadsUp,
            other => bail!("severity must be blocking or heads_up, not {other}"),
        };
        let agent = self.agent(&ctx.agent_id)?;
        let task = ctx.task_id.as_deref().and_then(|t| self.store.task(t).ok().flatten());
        let thread = match (&task, agent.role) {
            (_, Role::Lead) => lead::thread_of(&agent).to_string(),
            (Some(t), _) => self.task_thread(t),
            _ => default_lead_thread(),
        };
        let (title, body) = (prompts::clip(title, MAX_TITLE), prompts::clip(body, MAX_BODY));

        let _g = CHANGES.lock().unwrap_or_else(|e| e.into_inner());
        let all = self.store.with(|db| blockers::list(db, ws))?;
        let mine: Vec<&Blocker> = all.iter().filter(|b| b.agent_id == ctx.agent_id && b.status == BlockerStatus::Open).collect();
        // Asking the same thing twice (a retried tool call) is one card.
        if let Some(dup) = mine.iter().find(|b| b.title == title && b.body == body) {
            return Ok(reply(dup.severity));
        }
        if mine.len() >= MAX_OPEN_PER_AGENT {
            bail!("you already have {MAX_OPEN_PER_AGENT} questions waiting for the user; wait for their answers");
        }
        let now = now_ms();
        let blocker = Blocker {
            id: new_id(),
            workspace_id: ws.map(String::from),
            agent_id: ctx.agent_id.clone(),
            task_id: task.as_ref().map(|t| t.id.clone()),
            thread,
            title: title.clone(),
            body,
            severity,
            status: BlockerStatus::Open,
            answer: None,
            created_at: now,
            updated_at: now,
            answered_at: None,
            reminded_at: None,
        };
        self.save_blocker(ws, &blocker)?;
        if severity == BlockerSeverity::Blocking
            && let Some(t) = task.filter(|t| matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested))
        {
            self.update_task(&t.id, |t| {
                t.status = TaskStatus::Blocked;
                t.status_reason = Some(format!("{WAITING} {}", prompts::clip(&title, 100)));
                Ok(())
            })?;
        }
        Ok(reply(severity))
    }

    fn save_blocker(&self, ws: Option<&str>, b: &Blocker) -> Result<()> {
        self.emit(ws, row(Some(&b.agent_id), b.task_id.as_deref(), None, EventKind::BlockerChanged { blocker: b.clone() }))?;
        Ok(())
    }

    /// Flag the open cards that have waited long enough; returns every card of the workspace.
    pub(super) fn blockers_with_reminders(&self, ws: Option<&str>) -> Result<Vec<Blocker>> {
        let _g = CHANGES.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.store.with(|db| blockers::list(db, ws))?;
        let now = now_ms();
        for b in all.iter_mut().filter(|b| blockers::needs_reminder(b, now)) {
            b.reminded_at = Some(now);
            b.updated_at = now;
            self.save_blocker(ws, b)?;
        }
        Ok(all)
    }

    /// Close an open card (answered or dismissed) and return it.
    fn close_blocker(&self, id: &str, status: BlockerStatus, answer: Option<String>) -> Result<Blocker> {
        let _g = CHANGES.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = self.store.with(|db| blockers::get(db, id))?.ok_or_else(|| anyhow!("unknown question {id}"))?;
        if b.status != BlockerStatus::Open {
            bail!("that question is already {}", if b.status == BlockerStatus::Answered { "answered" } else { "dismissed" });
        }
        let now = now_ms();
        (b.status, b.answer, b.updated_at, b.answered_at) = (status, answer, now, Some(now));
        self.save_blocker(b.workspace_id.as_deref(), &b)?;
        Ok(b)
    }

    /// "Respond & unblock": record the answer, then resume whoever waits.
    pub(super) async fn answer_blocker(self: &Arc<Self>, id: &str, answer: &str) -> Result<Blocker> {
        let answer = answer.trim();
        if answer.is_empty() {
            bail!("write an answer first");
        }
        let answer = prompts::clip(answer, MAX_ANSWER);
        let b = self.close_blocker(id, BlockerStatus::Answered, Some(answer.clone()))?;
        let text = format!("Answer to your question \"{}\": {answer}", b.title);
        self.resume(&b, text).await?;
        Ok(b)
    }

    /// Dismiss a card without an answer. A blocked agent is told to use its best judgment.
    pub(super) async fn dismiss_blocker(self: &Arc<Self>, id: &str) -> Result<Blocker> {
        let b = self.close_blocker(id, BlockerStatus::Dismissed, None)?;
        if b.severity == BlockerSeverity::Blocking {
            let text = format!(
                "The user skipped your question \"{}\". Continue with your best judgment and note the assumption in your report.",
                b.title
            );
            self.resume(&b, text).await?;
        }
        Ok(b)
    }

    /// Get the asker going again. A heads-up has nothing to resume unless you wrote back.
    async fn resume(self: &Arc<Self>, b: &Blocker, text: String) -> Result<()> {
        let asker = self.agent(&b.agent_id)?;
        if asker.role == Role::Lead {
            // In the office the question came from, whichever one is open now.
            let ws = match b.workspace_id.as_deref() {
                Some(id) => self.store.workspace(id)?.ok_or_else(|| anyhow!("unknown workspace"))?,
                None => self.require_ws()?,
            };
            return self.chat_lead_in(&ws, &b.thread, &text).await;
        }
        let others_wait = |task_id: &str| {
            let all = self.store.with(|db| blockers::list(db, b.workspace_id.as_deref())).unwrap_or_default();
            all.iter().any(|o| o.id != b.id && o.task_id.as_deref() == Some(task_id) && o.status == BlockerStatus::Open && o.severity == BlockerSeverity::Blocking)
        };
        let waiting_task = b
            .task_id
            .as_deref()
            .and_then(|t| self.task(t).ok())
            .filter(|t| t.status == TaskStatus::Blocked && t.status_reason.as_deref().is_some_and(|r| r.starts_with(WAITING)));
        // The answer first: if it cannot be delivered, the task must not show as running. A run
        // that is shutting down takes no more turns, and the card is closed by now (it cannot
        // be answered again): for the answer, that run is already gone.
        let sent = self.is_live(&b.agent_id)
            && self.send_prompt(&b.agent_id, format!("[User] {text}")).inspect_err(|e| tracing::debug!("answer not sent as a prompt: {e}")).is_ok();
        if sent {
            if let Some(t) = &waiting_task
                && !others_wait(&t.id)
            {
                // Your answer is your move: only you take a task out of Blocked.
                self.update_task_as(&t.id, true, |t| {
                    t.status = TaskStatus::Running;
                    t.status_reason = None;
                    Ok(())
                })?;
            }
            return Ok(());
        }
        // The worker's process is gone (app restarted) or going (stopped): rerun the task with
        // the answer in its brief.
        match waiting_task {
            Some(t) => {
                self.update_task_as(&t.id, true, |t| {
                    t.goal = format!("{}\n\n{text}", t.goal);
                    t.status = TaskStatus::Ready;
                    t.status_reason = Some("you answered; restarting the worker".into());
                    Ok(())
                })?;
                self.update_agent(&b.agent_id, |a| {
                    if a.task_id.as_deref() == Some(t.id.as_str()) {
                        a.task_id = None;
                    }
                })?;
                self.kick();
                Ok(())
            }
            None => self.chat_agent(&b.agent_id, &text).await,
        }
    }
}

fn reply(severity: BlockerSeverity) -> Value {
    match severity {
        BlockerSeverity::Blocking => json!({
            "ok": true,
            "status": "waiting_for_user",
            "note": "Your question is on the user's Ask me list and your task is paused until they answer. Stop working on it now \
                     and end your turn; their answer arrives as a later message."
        }),
        BlockerSeverity::HeadsUp => json!({ "ok": true, "status": "noted", "note": "The user will see it; no answer is needed. Carry on." }),
    }
}

// ───────────── the Core facade ─────────────

impl super::Core {
    /// Every "ask me" card of the open workspace, oldest first. Cards that have waited too
    /// long are flagged for a reminder first (`Blocker::reminded_at`).
    pub async fn list_blockers(&self) -> Result<Vec<Blocker>> {
        let Some(ws) = self.inner.workspace().map(|w| w.id) else { return Ok(vec![]) };
        self.inner.blockers_with_reminders(Some(&ws))
    }

    /// Reply to a card and resume the agent that waits for it.
    pub async fn answer_blocker(&self, blocker_id: &str, answer: &str) -> Result<Blocker> {
        self.inner.answer_blocker(blocker_id, answer).await
    }

    /// Dismiss a card without an answer (a blocked agent carries on by its best judgment).
    pub async fn dismiss_blocker(&self, blocker_id: &str) -> Result<Blocker> {
        self.inner.dismiss_blocker(blocker_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::{self, tool_specs};

    #[test]
    fn lead_and_workers_may_ask_but_reviewers_and_solo_agents_may_not() {
        let has = |r| tool_specs(r).iter().any(|t| t["name"] == tools::ASK_USER);
        assert!(has(Role::Lead) && has(Role::Worker));
        assert!(!has(Role::Reviewer) && !has(Role::Solo));
    }

    #[test]
    fn replies_tell_a_blocked_agent_to_stop_and_a_heads_up_to_carry_on() {
        assert!(reply(BlockerSeverity::Blocking)["note"].as_str().unwrap().contains("end your turn"));
        assert_eq!(reply(BlockerSeverity::HeadsUp)["status"], "noted");
    }
}
