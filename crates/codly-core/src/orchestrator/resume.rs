//! What a restart of Codly asks, says and keeps.
//!
//! - **The question at launch** (`Settings::resume_on_launch`). Tasks that were in work when
//!   Codly last closed go back to Ready, and a running plan would start its next tasks. With
//!   "ask" (the default) and "never", none of that starts by itself: the scheduler starts no
//!   task (`Inner::launch_hold`), no plan runs its integration checks, and a task that was in
//!   its checks, review or merge is not checked again, until `Core::resume_work`. The app
//!   shows one question for "ask" and only the Resume work button for "never". With "always"
//!   the work goes on at once. Nothing is held when nothing would start: no task was in
//!   work, and every plan waits for you, for its paused office or for a budget.
//!   The hold is in the store too (`HOLD`): when Codly closes before your answer, the next
//!   start waits for the same tasks, whatever they say by then, and a task that only had its
//!   checks left still gets no worker turn. If you chose "always" meanwhile, they go on at
//!   that start.
//! - **"Resumed after restart"**: each task that goes on gets that line in its history.
//! - **Round counters**: the fix and conflict rounds a task has used are in the store, so a
//!   restart does not give it new ones (`count_rounds`, `forget_rounds`).
//! - **The files of a task that waits for your answer**: its worker's process is gone after a
//!   restart, but its question is still open, so the scheduler keeps its write scope
//!   (`Inner::tasks_asking`).
//! - **Notifications**: `Core::standing` tells the shell's notifier what stood before it
//!   started, so it does not tell you again about a task that was blocked before the quit.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::{Core, Inner, ask_me, gate, lock};
use crate::blockers::{self, BlockerSeverity, BlockerStatus};
use crate::model::*;
use crate::store::{self, Store};

/// Why no task starts while the work of the last run waits for your Resume. Each waiting
/// task shows it.
const HELD: &str = "paused: Codly restarted. Nothing starts until you press Resume";
/// Why a task whose worker goes on was not running (the worker's first prompt says it).
const RESUMING: &str = "resuming after Codly restarted";
const CHECKING: &str = "checking the work again after Codly restarted";

/// The work of the last run, while it waits for your Resume.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Hold {
    /// The tasks that were in work when Codly closed.
    tasks: Vec<Id>,
    /// Those of them whose worker had ended its turn: their gate runs again, with no worker turn.
    regate: Vec<Id>,
}

/// Where the store keeps the hold, from the start that sets it until it ends.
const HOLD: &str = "resume_hold";

/// What waits for your Resume since Codly opened (`Core::resume_offer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResumeOffer {
    /// Tasks that were in work when Codly last closed. 0: work was only about to start.
    pub tasks: u32,
}

/// What stands now and may have been told before, for a notifier that starts (`Core::standing`).
pub struct Standing {
    /// The tasks that are blocked or failed, in every office.
    pub tasks: Vec<Task>,
    /// The plans that wait for your Run, are finished or failed, in every office.
    pub plans: Vec<Plan>,
}

const ROUNDS: &str = "task_rounds:";

fn rounds_key(task_id: &str) -> String {
    format!("{ROUNDS}{task_id}")
}

/// The rounds each task had used when Codly last closed.
pub(super) fn stored_rounds(store: &Store) -> HashMap<Id, gate::Rounds> {
    let rows = store.kv_prefixed(ROUNDS).unwrap_or_else(|e| {
        tracing::warn!("the tasks' round counts cannot be read, so they start from zero: {e}");
        vec![]
    });
    let read = |(key, value): (String, String)| match serde_json::from_str::<gate::Rounds>(&value) {
        Ok(rounds) => Some((key[ROUNDS.len()..].to_string(), rounds)),
        Err(e) => {
            tracing::warn!("the round count in {key} cannot be read, so it starts from zero: {e}");
            None
        }
    };
    rows.into_iter().filter_map(read).collect()
}

/// Does a running plan with these tasks go on by itself? It does when one of its tasks can
/// start (`may_start`: its office is not paused and no budget is used up), and when every
/// task is done or cancelled, so its integration checks run next.
fn goes_on(tasks: &[Task], may_start: bool) -> bool {
    let done = |id: &Id| tasks.iter().any(|t| &t.id == id && t.status == TaskStatus::Done);
    let starts = |t: &Task| match t.status {
        TaskStatus::Ready => true,
        TaskStatus::Pending => t.depends_on.iter().all(done),
        _ => false,
    };
    let over = |t: &Task| matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled);
    let finishes = tasks.iter().any(|t| t.status == TaskStatus::Done) && tasks.iter().all(over);
    finishes || (may_start && tasks.iter().any(starts))
}

impl Inner {
    /// Why no task starts: the work of the last run waits for your Resume. The scheduler asks
    /// at every pass.
    pub(super) fn launch_hold(&self) -> Option<String> {
        lock(&self.held_work).is_some().then(|| HELD.to_string())
    }

    /// Recovery that asks first: a task that was in work waits, Ready, for your Resume.
    pub(super) fn hold_task(&self, t: &Task) -> Result<()> {
        self.update_task(&t.id, |t| {
            t.status = TaskStatus::Ready;
            t.status_reason = Some(HELD.into());
            Ok(())
        })?;
        Ok(())
    }

    /// A task that was in work goes on: its gate runs again (`gate_only`: it is Running, and
    /// the caller starts the gate), or the scheduler resumes its worker (Ready). Its history
    /// says so. A task that moved on since `was` was read (you cancelled it) is left alone.
    pub(super) fn resume_task(&self, was: &Task, gate_only: bool) -> Result<()> {
        self.update_task(&was.id, |t| {
            if t.status != was.status {
                bail!("it is {} now", store::enum_str(&t.status));
            }
            (t.status, t.status_reason) = match gate_only {
                true => (TaskStatus::Running, Some(CHECKING.into())),
                false => (TaskStatus::Ready, Some(RESUMING.into())),
            };
            Ok(())
        })?;
        let ws = self.store.task_workspace(&was.id)?;
        let how = if gate_only { "Codly checks its work again" } else { "its worker goes on where it stopped" };
        self.decide(ws.as_deref(), Some(&was.id), "retry", format!("Resumed after restart: {how}"), json!({ "plan_id": was.plan_id }))?;
        Ok(())
    }

    /// The end of a recovery that asks first. `in_work` tasks were in work (`regate`: those
    /// that only had their gate left). They, the tasks an earlier start held, and whatever
    /// else would start by itself now, wait for your Resume.
    pub(super) fn hold_at_launch(&self, in_work: Vec<Id>, regate: Vec<Id>) {
        let mut hold = Hold { tasks: in_work, regate };
        match self.still_held() {
            Ok(before) => {
                // What this start found about a task counts, not what an earlier one wrote.
                for id in before.tasks {
                    if !hold.tasks.contains(&id) {
                        if before.regate.contains(&id) {
                            hold.regate.push(id.clone());
                        }
                        hold.tasks.push(id);
                    }
                }
            }
            Err(e) => tracing::warn!("could not read the tasks an earlier start held: {e}"),
        }
        let waits = !hold.tasks.is_empty()
            || self.starts_by_itself().unwrap_or_else(|e| {
                // Not known: nothing starts before you say so.
                tracing::warn!("could not tell whether work would start by itself, so it waits for Resume: {e}");
                true
            });
        if let Err(e) = self.keep_hold(waits.then_some(&hold)) {
            tracing::warn!("the work that waits for your Resume is not in the store, so the next start may not ask for the same tasks: {e}");
        }
        if waits {
            *lock(&self.held_work) = Some(hold);
        }
    }

    /// Write the hold to the store, or take it out of it (None).
    fn keep_hold(&self, hold: Option<&Hold>) -> Result<()> {
        let row = hold.map(|hold| serde_json::to_string(hold)).transpose()?;
        self.store.kv_write(&[(HOLD.to_string(), row)])
    }

    /// The hold of a start that Codly closed on before your answer, as that start wrote it.
    /// None: no hold was left open.
    fn hold_left_open(&self) -> Result<Option<Hold>> {
        let Some(row) = self.store.kv_get(HOLD)? else { return Ok(None) };
        let hold = serde_json::from_str(&row).unwrap_or_else(|e| {
            tracing::warn!("the tasks an earlier start held cannot be read, so none is said to be resumed: {e}");
            Hold::default()
        });
        Ok(Some(hold))
    }

    /// The tasks of a hold that was left open which still wait (ready): you may have
    /// cancelled one meanwhile.
    fn still_held(&self) -> Result<Hold> {
        let Some(mut hold) = self.hold_left_open()? else { return Ok(Hold::default()) };
        let ready: HashSet<Id> = self.store.tasks_with_status(&[TaskStatus::Ready])?.into_iter().map(|t| t.id).collect();
        hold.tasks.retain(|id| ready.contains(id));
        Ok(hold)
    }

    /// Recovery with "always", after a start that asked and that Codly closed on before your
    /// answer: the tasks that start held go on now. Not `in_work`: those went on already.
    /// Returns the tasks whose gate runs again.
    pub(super) fn resume_left_open(&self, in_work: &[Id]) -> Vec<Id> {
        let resumed = (|| -> Result<Vec<Id>> {
            let Some(mut hold) = self.hold_left_open()? else { return Ok(vec![]) };
            hold.tasks.retain(|id| !in_work.contains(id));
            Ok(self.end_hold(&hold, &self.store.tasks_with_status(&[TaskStatus::Ready])?))
        })();
        resumed.unwrap_or_else(|e| {
            tracing::warn!("could not resume the tasks an earlier start held: {e}");
            vec![]
        })
    }

    /// Would work start by itself now? A running plan that goes on (`goes_on`; one that was
    /// in its integration checks runs them again), or a ready task of no plan. Nothing starts
    /// in a paused office or past a budget.
    fn starts_by_itself(&self) -> Result<bool> {
        let settings = self.settings();
        let over_budget = self.budget_hold().is_some();
        let may_start = |ws: &str| !over_budget && !settings.paused && !settings.paused_offices.iter().any(|id| id == ws);
        for plan in self.store.plans_with_status(&[PlanStatus::Running, PlanStatus::Integrating])? {
            if goes_on(&self.store.plan_tasks(&plan.id)?, may_start(&plan.workspace_id)) {
                return Ok(true);
            }
        }
        for t in self.store.tasks_with_status(&[TaskStatus::Ready])?.iter().filter(|t| t.plan_id.is_none()) {
            if self.store.task_workspace(&t.id)?.is_some_and(|ws| may_start(&ws)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Your Resume: the hold ends (`end_hold`), and the gates of the tasks that only had
    /// their checks left start.
    async fn resume_work(self: &Arc<Self>) -> Result<()> {
        // Under the scheduler's lock: no pass starts a task between the end of the hold and
        // the writes below.
        let sched = self.sched_lock.lock().await;
        let waiting = self.store.tasks_with_status(&[TaskStatus::Ready])?;
        let Some(hold) = lock(&self.held_work).take() else { return Ok(()) };
        let regate = self.end_hold(&hold, &waiting);
        drop(sched);
        for task_id in regate {
            tokio::spawn(self.clone().gate(task_id));
        }
        self.kick();
        Ok(())
    }

    /// The end of a hold. Each of its tasks that still waits (`waiting`: the ready tasks) goes
    /// on (`resume_task`), the other tasks lose the reason they waited for and start as the
    /// scheduler finds room for them, and the store forgets the hold. Returns the tasks whose
    /// gate the caller starts.
    fn end_hold(&self, hold: &Hold, waiting: &[Task]) -> Vec<Id> {
        let mut regate = vec![];
        for t in waiting {
            let was_in_work = hold.tasks.contains(&t.id);
            let gate_only = was_in_work && hold.regate.contains(&t.id);
            let res = if was_in_work {
                // Whatever it says by now (a paused office writes its own reason).
                self.resume_task(t, gate_only)
            } else if t.status_reason.as_deref() == Some(HELD) {
                // It was not in work: it only loses the reason it waited for.
                let cleared = self.update_task(&t.id, |now| {
                    if now.status != TaskStatus::Ready {
                        bail!("it is {} now", store::enum_str(&now.status));
                    }
                    now.status_reason = None;
                    Ok(())
                });
                cleared.map(drop)
            } else {
                continue;
            };
            match res {
                Ok(()) if gate_only => regate.push(t.id.clone()),
                Ok(()) => {}
                // One task that cannot be written does not keep the others waiting.
                Err(e) => tracing::warn!("resuming task {}: {e}", t.key),
            }
        }
        if let Err(e) = self.keep_hold(None) {
            tracing::warn!("the hold stays in the store, so the next start asks again about the tasks that still wait: {e}");
        }
        regate
    }

    /// The blocked tasks of an office that wait for your answer to a question that is still
    /// open (Ask me). `tasks` are the office's. The cards are read only when a task waits.
    pub(super) fn tasks_asking(&self, ws: &str, tasks: &[Task]) -> Result<HashSet<Id>> {
        let waits = |t: &&Task| t.status == TaskStatus::Blocked && t.status_reason.as_deref().is_some_and(|r| r.starts_with(ask_me::WAITING));
        if !tasks.iter().any(|t| waits(&t)) {
            return Ok(HashSet::new());
        }
        let open: HashSet<Id> = self
            .store
            .with(|db| blockers::list(db, Some(ws)))?
            .into_iter()
            .filter(|b| b.status == BlockerStatus::Open && b.severity == BlockerSeverity::Blocking)
            .filter_map(|b| b.task_id)
            .collect();
        Ok(tasks.iter().filter(waits).filter(|t| open.contains(&t.id)).map(|t| t.id.clone()).collect())
    }

    /// Change the rounds a task has used, and keep them in the store.
    pub(super) fn count_rounds<T>(&self, task_id: &str, change: impl FnOnce(&mut gate::Rounds) -> T) -> Result<T> {
        let mut all = lock(&self.fix_rounds);
        let rounds = all.entry(task_id.to_string()).or_default();
        let out = change(&mut *rounds);
        self.store.kv_set(&rounds_key(task_id), &serde_json::to_string(&*rounds)?)?;
        Ok(out)
    }

    /// The task is over, or starts over (a retry): the rounds it used are forgotten.
    pub(super) fn forget_rounds(&self, task_id: &str) {
        if lock(&self.fix_rounds).remove(task_id).is_some()
            && let Err(e) = self.store.kv_write(&[(rounds_key(task_id), None)])
        {
            tracing::warn!("the round count of task {task_id} stays in the store: {e}");
        }
    }
}

impl Core {
    /// The work of the last run that waits for your Resume. None: nothing waits.
    pub fn resume_offer(&self) -> Option<ResumeOffer> {
        lock(&self.inner.held_work).as_ref().map(|hold| ResumeOffer { tasks: hold.tasks.len() as u32 })
    }

    /// Your Resume: the work that was under way when Codly last closed goes on.
    pub async fn resume_work(&self) -> Result<()> {
        self.inner.resume_work().await
    }

    /// The stopped tasks and the plans a notification tells about, as they stand now.
    pub fn standing(&self) -> Result<Standing> {
        let store = &self.inner.store;
        Ok(Standing {
            tasks: store.tasks_with_status(&[TaskStatus::Blocked, TaskStatus::Failed])?,
            plans: store.plans_with_status(&[PlanStatus::AwaitingApproval, PlanStatus::Done, PlanStatus::Failed])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: &str, deps: &[&str]) -> Task {
        serde_json::from_value(json!({
            "id": id, "plan_id": "p", "key": id, "title": "Add sign-in", "goal": "", "acceptance": [], "write_scope": [],
            "verify": [], "depends_on": deps, "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
            "attempt": 1, "status_reason": null, "summary": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    #[test]
    fn a_running_plan_goes_on_when_a_task_can_start_or_only_its_checks_are_left() {
        // A task that is ready, or that waits only for work that is merged.
        assert!(goes_on(&[task("a", "done", &[]), task("b", "ready", &[])], true));
        assert!(goes_on(&[task("a", "done", &[]), task("b", "pending", &["a"])], true));
        assert!(goes_on(&[task("a", "pending", &[])], true));
        // No task starts in a paused office or past a budget.
        assert!(!goes_on(&[task("a", "done", &[]), task("b", "ready", &[])], false));
        // Every task done or cancelled: the plan's integration checks run next, paused or not.
        assert!(goes_on(&[task("a", "done", &[]), task("b", "cancelled", &[])], false));
        // What is left waits for you: nothing would start.
        let stopped = [task("a", "done", &[]), task("b", "blocked", &[]), task("c", "failed", &[]), task("d", "pending", &["b"])];
        assert!(!goes_on(&stopped, true));
        assert!(!goes_on(&[task("a", "cancelled", &[])], true), "you cancelled every task: the plan ends cancelled");
        assert!(!goes_on(&[], true));
    }

    /// A settings file written before the choice existed loads, and asks.
    #[test]
    fn settings_saved_before_the_choice_load_with_ask() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(old.as_object_mut().unwrap().remove("resume_on_launch"), Some(json!("ask")));
        let settings: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(settings.resume_on_launch, ResumeOnLaunch::Ask);
        for (text, choice) in [("always", ResumeOnLaunch::Always), ("never", ResumeOnLaunch::Never)] {
            assert_eq!(serde_json::from_value::<ResumeOnLaunch>(json!(text)).unwrap(), choice);
        }
    }

    /// The counts are stored by name, and a count written by an older version still loads.
    #[test]
    fn round_counts_are_read_back_from_the_store() {
        let store = Store::open_in_memory().unwrap();
        assert!(stored_rounds(&store).is_empty());
        store.kv_set(&rounds_key("t1"), r#"{"fix":2,"conflict":1,"left_out":true}"#).unwrap();
        store.kv_set(&rounds_key("t2"), r#"{"fix":1}"#).unwrap();
        store.kv_set(&rounds_key("t3"), "not json").unwrap();
        let read: HashMap<Id, serde_json::Value> = stored_rounds(&store).into_iter().map(|(id, r)| (id, serde_json::to_value(r).unwrap())).collect();
        assert_eq!(read.len(), 2, "a count that cannot be read starts from zero: {read:?}");
        assert_eq!(read["t1"], json!({ "fix": 2, "conflict": 1, "left_out": true }));
        assert_eq!(read["t2"], json!({ "fix": 1, "conflict": 0, "left_out": false }));
    }
}
