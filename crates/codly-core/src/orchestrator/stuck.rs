//! Plans that cannot go on by themselves (`scheduler::stuck_plans`): Codly says so once in the
//! plan's thread, with the ways out. Two of them are here: keep what is merged (the plan
//! finishes with its merged tasks), and cancel a task together with the tasks that wait for it.

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::json;

use super::scheduler::{self, Stuck};
use super::{Inner, lock, prompts};
use crate::model::*;
use crate::store;

/// Where the stop Codly last told you about is kept (the stopped tasks' ids), so a restart of
/// Codly does not say it again.
fn told_key(plan_id: &str) -> String {
    format!("stuck:{plan_id}")
}

/// "t1", "t1 and t2", "t1, t2 and t3".
fn list(keys: &[&str]) -> String {
    match keys {
        [] => String::new(),
        [one] => one.to_string(),
        [most @ .., last] => format!("{} and {last}", most.join(", ")),
    }
}

/// What Codly says in the thread of a plan that cannot go on by itself.
pub(super) fn stuck_text(plan: &Plan, tasks: &[Task], stuck: &Stuck) -> String {
    let of = |ids: &[Id]| -> Vec<&Task> { ids.iter().filter_map(|id| tasks.iter().find(|t| &t.id == id)).collect() };
    let (stopped, waiting) = (of(&stuck.stopped), of(&stuck.waiting));
    let total = tasks.iter().filter(|t| t.plan_id.as_deref() == Some(plan.id.as_str())).count();
    let mut text = format!("Plan \"{}\" cannot go on by itself.", plan.summary);
    for t in &stopped {
        let why = t.status_reason.as_deref().unwrap_or("no reason given");
        let why = prompts::clip(why.strip_prefix("blocked: ").or_else(|| why.strip_prefix("failed: ")).unwrap_or(why), 200);
        text.push_str(&match t.status {
            TaskStatus::Failed => format!(" {} \"{}\" failed: {why}.", t.key, t.title),
            TaskStatus::Cancelled => format!(" {} \"{}\" is cancelled.", t.key, t.title),
            _ => format!(" {} \"{}\" is blocked: {why}.", t.key, t.title),
        });
    }
    let keys: Vec<&str> = waiting.iter().map(|t| t.key.as_str()).collect();
    match (keys.len(), stopped.len()) {
        (0, _) => {}
        (n, 0) => text.push_str(&format!(" {} wait{} for a task that is not in the plan any more.", list(&keys), if n == 1 { "s" } else { "" })),
        (n, m) => text.push_str(&format!(" {} wait{} for {}.", list(&keys), if n == 1 { "s" } else { "" }, if m == 1 { "it" } else { "them" })),
    }
    let branch = plan.integration_branch.as_deref().unwrap_or("the plan's branch");
    text.push_str(&format!(" Merged into {branch}: {} of {total} task{}.", stuck.merged, if total == 1 { "" } else { "s" }));
    text.push_str(" What you can do: start a stopped task again from its card (Retry, or Reopen if you cancelled it); cancel it together with the tasks that wait for it;");
    text.push_str(if stuck.merged > 0 {
        " or choose Keep what is merged on the plan card: the plan then finishes with the merged tasks, and the rest is cancelled."
    } else {
        " or cancel the plan."
    });
    text
}

impl Inner {
    /// Say in each stuck plan's thread that it cannot go on, once per stop: again only when
    /// other tasks stopped it, or after it moved on and got stuck again. `plans` and `tasks`
    /// are the office's, as the scheduling pass read them.
    pub(super) fn tell_stuck(&self, ws: &Workspace, plans: &[Plan], tasks: &[Task], stuck: &[Stuck]) -> Result<()> {
        let moved_on: Vec<Id> = {
            let mut told = lock(&self.stuck_told);
            let ids: Vec<Id> =
                told.keys().filter(|id| plans.iter().any(|p| &p.id == *id) && !stuck.iter().any(|s| &s.plan_id == *id)).cloned().collect();
            for id in &ids {
                told.remove(id);
            }
            ids
        };
        if !moved_on.is_empty() {
            self.store.kv_write(&moved_on.iter().map(|id| (told_key(id), None)).collect::<Vec<_>>())?;
        }
        for s in stuck {
            let stop = s.stopped.join(",");
            if lock(&self.stuck_told).get(&s.plan_id) == Some(&stop) {
                continue;
            }
            // The first look since Codly started: what it said before it quit still stands.
            let on_record = self.store.kv_get(&told_key(&s.plan_id))?;
            lock(&self.stuck_told).insert(s.plan_id.clone(), stop.clone());
            if on_record.as_deref() == Some(stop.as_str()) {
                continue;
            }
            let Some(plan) = plans.iter().find(|p| p.id == s.plan_id) else { continue };
            self.store.kv_write(&[(told_key(&plan.id), Some(stop))])?;
            // No plan attachment: the thread shows a plan's card on the lines that carry one,
            // and this line is not the place for it.
            self.chat(Some(&ws.id), &plan.thread, "codly", &stuck_text(plan, tasks, s), None)?;
            let key_of = |id: &Id| tasks.iter().find(|t| &t.id == id).map(|t| t.key.clone());
            self.decide(
                Some(&ws.id),
                None,
                "schedule",
                format!("plan \"{}\" cannot go on by itself; told you the ways out", prompts::clip(&plan.summary, 60)),
                json!({
                    "plan_id": plan.id, "merged": s.merged,
                    "stopped": s.stopped.iter().filter_map(key_of).collect::<Vec<_>>(),
                    "waiting": s.waiting.iter().filter_map(key_of).collect::<Vec<_>>(),
                }),
            )?;
        }
        Ok(())
    }

    /// Keep what is merged: every task of a running plan that is not done is cancelled, and the
    /// plan finishes with the merged ones (its integration checks, then the card to review and
    /// merge it). With nothing merged the plan ends cancelled.
    pub(super) async fn finish_plan(self: &Arc<Self>, plan_id: &str) -> Result<Plan> {
        let plan = self.plan(plan_id)?;
        if plan.status != PlanStatus::Running {
            bail!("only a running plan can finish with what is merged; this one is {}", store::enum_str(&plan.status));
        }
        // Behind a merge in flight, like Cancel plan: its task is done when the others are
        // cancelled, so nothing lands on the integration branch for a task the board shows
        // cancelled. Never call this holding `merge_lock` or `sched_lock`.
        let queue = self.merge_lock.lock().await;
        let mut cancelled = vec![];
        for t in self.store.plan_tasks(plan_id)? {
            let mut mine = false;
            self.update_task_as(&t.id, true, |t| {
                mine = !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled);
                if mine {
                    t.status = TaskStatus::Cancelled;
                    t.status_reason = Some("cancelled: you kept what is merged".to_string());
                }
                Ok(())
            })?;
            if mine {
                self.release_task_agents(&t);
                cancelled.push(t.key);
            }
        }
        drop(queue);
        self.decide_by(
            Some(&plan.workspace_id),
            None,
            "schedule",
            "human",
            format!("you kept what is merged of plan \"{}\"; cancelled {}", prompts::clip(&plan.summary, 60), cancelled.join(", ")),
            json!({ "plan_id": plan.id, "cancelled": cancelled }),
        )?;
        // The next pass finishes the plan (or cancels it, with nothing merged).
        self.kick();
        self.plan(plan_id)
    }

    /// Cancel a task and every task that waits for it, directly or through others: those could
    /// never start. Returns them as they are now, the task first.
    pub(super) async fn cancel_task_tree(&self, task_id: &str) -> Result<Vec<Task>> {
        let root = self.task(task_id)?;
        let all = match root.plan_id.as_deref() {
            Some(p) => self.store.plan_tasks(p)?,
            None => vec![],
        };
        let mut out = vec![self.move_task(task_id, TaskStatus::Cancelled).await?];
        for t in scheduler::dependents(&all, task_id).into_iter().filter(|t| t.status != TaskStatus::Cancelled) {
            out.push(self.move_task(&t.id, TaskStatus::Cancelled).await?);
        }
        Ok(out)
    }
}

impl super::Core {
    /// Keep what is merged, from the card of a running plan: its tasks that are not done are
    /// cancelled and the plan finishes with the merged ones, ready to review and merge. With
    /// nothing merged the plan ends cancelled.
    pub async fn finish_plan(&self, plan_id: &str) -> anyhow::Result<Plan> {
        self.inner.finish_plan(plan_id).await
    }

    /// Cancel a task together with every task that waits for it (they could never start).
    /// Returns them as they are now, the task first.
    pub async fn cancel_task_tree(&self, task_id: &str) -> anyhow::Result<Vec<Task>> {
        self.inner.cancel_task_tree(task_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        serde_json::from_value(json!({
            "id": "p", "workspace_id": "ws", "thread": "lead", "request": "", "summary": "Add sign-in", "assumptions": [],
            "open_questions": [], "status": "running", "base_commit": null, "integration_branch": "codly/sign-in/integration",
            "integration_worktree": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    fn task(key: &str, status: &str, reason: Option<&str>) -> Task {
        serde_json::from_value(json!({
            "id": key, "plan_id": "p", "key": key, "title": format!("Do {key}"), "goal": "", "acceptance": [], "write_scope": [],
            "verify": [], "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "account_hint": null,
            "branch": null, "worktree": null, "attempt": 1, "status_reason": reason, "summary": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    #[test]
    fn the_notice_names_what_stopped_what_waits_and_the_ways_out() {
        let tasks = vec![
            task("t1", "done", None),
            task("t2", "blocked", Some("blocked: could not start the worker: Claude Code is not signed in")),
            task("t3", "pending", None),
            task("t4", "pending", None),
        ];
        let stuck = Stuck { plan_id: "p".into(), stopped: vec!["t2".into()], waiting: vec!["t3".into(), "t4".into()], merged: 1 };
        assert_eq!(
            stuck_text(&plan(), &tasks, &stuck),
            "Plan \"Add sign-in\" cannot go on by itself. t2 \"Do t2\" is blocked: could not start the worker: Claude Code is not signed in. \
             t3 and t4 wait for it. Merged into codly/sign-in/integration: 1 of 4 tasks. What you can do: start a stopped task again \
             from its card (Retry, or Reopen if you cancelled it); cancel it together with the tasks that wait for it; or choose Keep \
             what is merged on the plan card: the plan then finishes with the merged tasks, and the rest is cancelled."
        );
    }

    #[test]
    fn with_nothing_merged_the_way_out_is_to_cancel_the_plan() {
        let tasks = vec![task("t1", "cancelled", Some("cancelled by you")), task("t2", "failed", Some("failed: `npm test` failed (exit 1) after 3 fix rounds")), task("t3", "pending", None)];
        let stuck = Stuck { plan_id: "p".into(), stopped: vec!["t1".into(), "t2".into()], waiting: vec!["t3".into()], merged: 0 };
        let text = stuck_text(&plan(), &tasks, &stuck);
        assert!(text.contains("t1 \"Do t1\" is cancelled. t2 \"Do t2\" failed: `npm test` failed (exit 1) after 3 fix rounds. t3 waits for them."), "{text}");
        assert!(text.contains("Merged into codly/sign-in/integration: 0 of 3 tasks.") && text.ends_with("or cancel the plan."), "{text}");
        assert_eq!((list(&[]), list(&["a"]), list(&["a", "b", "c"])), (String::new(), "a".to_string(), "a, b and c".to_string()));
    }
}
