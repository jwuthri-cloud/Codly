//! What a restart of Codly asks, says and keeps.
//!
//! - **The question at launch** (`Settings::resume_on_launch`). Tasks that were in work when
//!   Codly last closed go back to Ready, and a running plan would start its next tasks. With
//!   "ask" (the default) and "never", none of that starts by itself: the scheduler starts no
//!   task (`Inner::launch_hold`), no plan runs its integration checks, and a task that was in
//!   its checks, review or merge is not checked again, until `Core::resume_work`. The app
//!   shows one question for "ask" and only the Resume work button for "never". With "always"
//!   the work goes on at once. Nothing is held when nothing would start: no task was in
//!   work, and every plan waits for you, for its paused office, for a budget or for a repo
//!   folder that is not there. A hold that is on ends by itself only when nothing could
//!   start any more: you cancelled every task it held, no plan would go on and no task waits
//!   (`Inner::held`). A pause or a budget does not end it: they end too, and the work would
//!   then start with no Resume. The same holds for a hold that Codly closed on: the next
//!   start keeps it on while work could start.
//!   The hold is in the store too (`HOLD`): when Codly closes before your answer, the next
//!   start waits for the same tasks, whatever they say by then, and a task that only had its
//!   checks left still gets no worker turn. If you chose "always" meanwhile, they go on at
//!   that start. The end of a hold writes each task first and takes the hold out of the store
//!   last (`Inner::end_hold`): a crash or a failed write in between leaves the hold on, and
//!   the next Resume work ends it.
//! - **"Resumed after restart"**: each task that goes on gets that line in its history.
//! - **Round counters**: the fix and conflict rounds a task has used are in the store, so a
//!   restart does not give it new ones (`count_rounds`, `forget_rounds`).
//! - **Rows that outlive their owner**: what the store kept for an agent that is let go, and
//!   for the tasks of an office that is closed, goes with them (`forget_agent`,
//!   `forget_office_tasks`).
//! - **The files of a task that waits for your answer**: its worker's process is gone after a
//!   restart, but its question is still open, so the scheduler keeps its write scope
//!   (`Inner::tasks_asking`).
//! - **Notifications**: `Core::standing` tells the shell's notifier what stood before it
//!   started, so it does not tell you again about a task that was blocked before the quit.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::{Core, Inner, ask_me, gate, lock, required_checks};
use crate::blockers::{self, BlockerSeverity, BlockerStatus};
use crate::model::*;
use crate::store::{self, Store};

/// Why no task starts while the work of the last run waits for your Resume. Each waiting
/// task shows it. It names the button by its label and its place: the app has other Resume
/// controls (a paused agent's, a paused office's).
pub const HELD: &str = "paused: Codly restarted. Nothing starts until you press 'Resume work' at the top of the window";
/// Why a task whose worker goes on was not running (the worker's first prompt says it).
const RESUMING: &str = "resuming after Codly restarted";
const CHECKING: &str = "checking the work again after Codly restarted";

/// The work of the last run, while it waits for your Resume.
#[derive(Clone, Default, Serialize, Deserialize)]
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
    /// Tasks that were in work when Codly last closed and still wait (not the ones you
    /// cancelled since). 0: work was only about to start.
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
    /// Why no task starts: the work of the last run waits for your Resume. None when no hold
    /// is on, also when the one that was on ended at this look (`held`). The scheduler asks at
    /// every pass.
    pub(super) fn launch_hold(&self) -> Option<String> {
        self.held().map(|_| HELD.to_string())
    }

    /// The hold as it stands now: how many of its tasks still wait for your Resume (ready,
    /// whatever reason they show: a paused office writes its own). None: no hold is on.
    /// A hold that waits for nothing any more ends here, by itself: you cancelled every task
    /// it held (or put it back in the backlog) and nothing else could start, so the scheduler
    /// is free and work you start from here on needs no Resume. "Could start" counts the work
    /// of a paused office and the work past a budget (`starts_by_itself(true)`): a pause or a
    /// budget ends, and what starts then still waits for your Resume. None of the hold's
    /// tasks is resumed here, so this needs no scheduler lock.
    fn held(&self) -> Option<u32> {
        let hold = lock(&self.held_work).clone()?;
        let looked = (|| -> Result<Option<u32>> {
            let ready = self.store.tasks_with_status(&[TaskStatus::Ready])?;
            let tasks = ready.iter().filter(|t| hold.tasks.contains(&t.id)).count() as u32;
            if tasks > 0 || self.starts_by_itself(true)? {
                return Ok(Some(tasks));
            }
            // No task of the hold is among `ready`: the ones that only showed its reason
            // lose it, and the store forgets the hold.
            if let (_, Err(e)) = self.end_hold(&hold, &ready) {
                // Its end is not written: the hold stays on, and the next look tries again.
                tracing::warn!("the hold that waits for nothing any more could not be ended: {e}");
                return Ok(Some(0));
            }
            *lock(&self.held_work) = None;
            Ok(None)
        })();
        looked.unwrap_or_else(|e| {
            // Not known: the hold stays, for the tasks as the start found them.
            tracing::warn!("could not tell what still waits for your Resume: {e}");
            Some(hold.tasks.len() as u32)
        })
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
    /// Ok = the task goes on: once its status is written, a history line that cannot be
    /// written does not stop it. (An error here keeps the caller from starting the gate, and
    /// a task that is Running with no gate has nobody on it until the next restart.)
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
        let how = if gate_only { "Codly checks its work again" } else { "its worker goes on where it stopped" };
        let said = self.store.task_workspace(&was.id).and_then(|ws| {
            self.decide(ws.as_deref(), Some(&was.id), "retry", format!("Resumed after restart: {how}"), json!({ "plan_id": was.plan_id }))
        });
        if let Err(e) = said {
            tracing::warn!("task {} goes on after the restart, but its history does not say so: {e}", was.key);
        }
        Ok(())
    }

    /// The end of a recovery that asks first. `in_work` tasks were in work (`regate`: those
    /// that only had their gate left). They, the tasks an earlier start held, and whatever
    /// else would start by itself now, wait for your Resume. A hold that Codly closed on
    /// (`was_on`) is kept as `held` keeps one: also for work that a pause or a budget stops
    /// for now.
    pub(super) fn hold_at_launch(&self, in_work: Vec<Id>, regate: Vec<Id>) {
        let mut hold = Hold { tasks: in_work, regate };
        let mut was_on = false;
        match self.still_held() {
            Ok(Some(before)) => {
                was_on = true;
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
            Ok(None) => {}
            Err(e) => tracing::warn!("could not read the tasks an earlier start held: {e}"),
        }
        let waits = !hold.tasks.is_empty()
            || self.starts_by_itself(was_on).unwrap_or_else(|e| {
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
    /// cancelled one meanwhile. None: no hold was left open.
    fn still_held(&self) -> Result<Option<Hold>> {
        let Some(mut hold) = self.hold_left_open()? else { return Ok(None) };
        let ready: HashSet<Id> = self.store.tasks_with_status(&[TaskStatus::Ready])?.into_iter().map(|t| t.id).collect();
        hold.tasks.retain(|id| ready.contains(id));
        Ok(Some(hold))
    }

    /// Recovery with "always", after a start that asked and that Codly closed on before your
    /// answer: the tasks that start held go on now. Not `in_work`: those went on already.
    /// Returns the tasks whose gate runs again.
    pub(super) fn resume_left_open(&self, in_work: &[Id]) -> Vec<Id> {
        let resumed = (|| -> Result<Vec<Id>> {
            let Some(mut hold) = self.hold_left_open()? else { return Ok(vec![]) };
            hold.tasks.retain(|id| !in_work.contains(id));
            let (regate, ended) = self.end_hold(&hold, &self.store.tasks_with_status(&[TaskStatus::Ready])?);
            if let Err(e) = ended {
                tracing::warn!("the hold stays in the store, so the next start asks again about the tasks that still wait: {e}");
            }
            Ok(regate)
        })();
        resumed.unwrap_or_else(|e| {
            tracing::warn!("could not resume the tasks an earlier start held: {e}");
            vec![]
        })
    }

    /// Would work start by itself? A running plan that goes on (`goes_on`; one that was in
    /// its integration checks runs them again), or a ready task of no plan. As things stand
    /// now, nothing starts in a paused office or past a budget: the test of a launch with no
    /// hold before it. `once_free` counts as if nothing were paused and no budget were
    /// reached: the test of a hold that is on (or was, when Codly closed), which a pause or a
    /// budget must not end. Either way nothing at all starts in an office the scheduler
    /// passes over because its repo folder is not there (`scheduled`).
    fn starts_by_itself(&self, once_free: bool) -> Result<bool> {
        let settings = self.settings();
        let over_budget = !once_free && self.budget_hold().is_some();
        let may_start = |ws: &str| once_free || !(over_budget || settings.paused || settings.paused_offices.iter().any(|id| id == ws));
        let mut offices = HashMap::new();
        for plan in self.store.plans_with_status(&[PlanStatus::Running, PlanStatus::Integrating])? {
            if self.scheduled(&mut offices, &plan.workspace_id)? && goes_on(&self.store.plan_tasks(&plan.id)?, may_start(&plan.workspace_id)) {
                return Ok(true);
            }
        }
        for t in self.store.tasks_with_status(&[TaskStatus::Ready])?.iter().filter(|t| t.plan_id.is_none()) {
            if let Some(ws) = self.store.task_workspace(&t.id)?
                && may_start(&ws)
                && self.scheduled(&mut offices, &ws)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Does the scheduler work in this office? The test of `schedule_pass`: the open office
    /// always, another one only while its repo folder is there (not on a disk that is not
    /// mounted, say). `seen` keeps the answer for one look at every office.
    fn scheduled(&self, seen: &mut HashMap<Id, bool>, ws_id: &str) -> Result<bool> {
        if let Some(known) = seen.get(ws_id) {
            return Ok(*known);
        }
        let there = self.workspace().is_some_and(|open| open.id == ws_id)
            || self.store.workspace(ws_id)?.is_some_and(|ws| Path::new(&ws.repo_path).is_dir());
        seen.insert(ws_id.to_string(), there);
        Ok(there)
    }

    /// Your Resume: the hold ends (`end_hold`), and the gates of the tasks that only had
    /// their checks left start. Err: a write failed, so the hold is still on and nothing new
    /// starts; your next Resume work ends it.
    async fn resume_work(self: &Arc<Self>) -> Result<()> {
        // Under the scheduler's lock: no pass starts a task between the writes below and the
        // end of the hold.
        let sched = self.sched_lock.lock().await;
        let waiting = self.store.tasks_with_status(&[TaskStatus::Ready])?;
        let Some(hold) = lock(&self.held_work).clone() else { return Ok(()) };
        let (regate, ended) = self.end_hold(&hold, &waiting);
        if ended.is_ok() {
            *lock(&self.held_work) = None;
        }
        drop(sched);
        // Also when the hold stays on: a task that is written as "in its checks" has nobody
        // on it but its gate.
        for task_id in regate {
            tokio::spawn(self.clone().gate(task_id));
        }
        self.kick();
        ended.map_err(|e| anyhow!("not all of the work could be resumed ({e}); press 'Resume work' again"))
    }

    /// The end of a hold, in the order that a crash or a failed write leaves safe: each task
    /// first, the hold itself last. Each of its tasks that still waits (`waiting`: the ready
    /// tasks) goes on (`resume_task`), the other tasks lose the reason they waited for and
    /// start as the scheduler finds room for them, and then the store forgets the hold.
    /// Returns the tasks whose gate the caller starts, and whether the hold is over. Err: a
    /// write failed. The hold then stays in the store as it was and the caller keeps it on,
    /// so nothing starts behind it; the next end writes the tasks that still wait once more
    /// (the history of one that was written before says "Resumed" a second time).
    fn end_hold(&self, hold: &Hold, waiting: &[Task]) -> (Vec<Id>, Result<()>) {
        let mut regate = vec![];
        let mut failed = None;
        for t in waiting {
            let was_in_work = hold.tasks.contains(&t.id);
            // Only its checks were left when the hold began, and its worktree is still there.
            // One whose worktree is gone by now (a hold can be days old) has nothing to check:
            // its worker goes on, in a new worktree on the task's branch.
            let gate_only = was_in_work && hold.regate.contains(&t.id) && t.worktree.as_deref().is_some_and(|wt| Path::new(wt).is_dir());
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
                // It moved on since `waiting` was read (you cancelled it): it waits no more.
                Err(e) if self.task(&t.id).is_ok_and(|now| now.status != TaskStatus::Ready) => {
                    tracing::debug!("task {} is not resumed: {e}", t.key);
                }
                // The others are still written: a task that got its checks back has its gate.
                Err(e) => {
                    tracing::warn!("resuming task {}: {e}", t.key);
                    failed = failed.or(Some(e));
                }
            }
        }
        let ended = match failed {
            Some(e) => Err(e),
            None => self.keep_hold(None),
        };
        (regate, ended)
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

    /// An agent is let go: what the store kept about its work goes with it. The result of
    /// its last "Check the work" (`required_checks.rs`), what Codly's commits left out in its
    /// worktree (`left_out.rs`) and the notes about files you put back (`checkpoints.rs`, which
    /// drops them with `forget_restore_notes`) are all about a worktree the agent no longer has.
    /// A row that cannot be removed is a leftover, not a reason to keep the agent.
    pub(super) fn forget_agent(&self, agent_id: &str) {
        let forgotten = self.store.kv_write(&[(required_checks::checked_key(agent_id), None)]).and_then(|()| self.forget_left_out(agent_id));
        if let Err(e) = forgotten {
            tracing::warn!("rows of agent {agent_id}, which was let go, stay in the store: {e}");
        }
        self.forget_restore_notes(agent_id);
    }

    /// An office is closed: its tasks are all over (`close_office` refuses otherwise), so the
    /// rounds they used and what Codly's commits left out for them are not kept. A row that
    /// cannot be removed is a leftover, not a reason to keep the office.
    pub(super) fn forget_office_tasks(&self, ws_id: &str) {
        let forgotten = (|| -> Result<()> {
            let tasks = self.store.tasks(ws_id)?;
            // Each row goes from the store whatever this run knows of it: a count that could
            // not be read at the start is a row too.
            {
                let mut known = lock(&self.fix_rounds);
                tasks.iter().for_each(|t| drop(known.remove(&t.id)));
            }
            self.store.kv_write(&tasks.iter().map(|t| (rounds_key(&t.id), None)).collect::<Vec<_>>())?;
            tasks.iter().try_for_each(|t| self.forget_left_out(&t.id))
        })();
        if let Err(e) = forgotten {
            tracing::warn!("rows of the tasks of office {ws_id}, which was closed, stay in the store: {e}");
        }
    }
}

impl Core {
    /// The work of the last run that waits for your Resume, as it stands now: a task you
    /// cancelled since Codly opened is not counted. None: nothing waits (any more), and
    /// nothing is held (`Inner::held`).
    pub fn resume_offer(&self) -> Option<ResumeOffer> {
        self.inner.held().map(|tasks| ResumeOffer { tasks })
    }

    /// Your Resume: the work that was under way when Codly last closed goes on. Err: the
    /// store did not take every write. The hold is still on (`resume_offer` says so), and
    /// the same call ends it once the store writes again.
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
    use crate::orchestrator::{CoreConfig, StartOptions};

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

    fn sh(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A core with one open repo of one commit, and no CLI.
    async fn start(root: &Path) -> (Arc<Core>, Workspace) {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("app.txt"), "app\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        (core, ws)
    }

    /// A task of the open office that waits for your Resume, with its worktree at `worktree`.
    async fn held_task(core: &Core, title: &str, worktree: &Path) -> Task {
        let made = core.create_task(title, "").await.unwrap();
        let held = |t: &mut Task| {
            (t.status, t.status_reason) = (TaskStatus::Ready, Some(HELD.into()));
            t.worktree = Some(worktree.to_string_lossy().into_owned());
            Ok(())
        };
        core.inner.update_task(&made.id, held).unwrap()
    }

    /// The store takes no decision from here on: a history line cannot be written.
    const NO_DECISIONS: &str =
        "CREATE TRIGGER no_decisions BEFORE INSERT ON event WHEN NEW.type = 'decision' BEGIN SELECT RAISE(ABORT, 'no room for a decision'); END";

    /// A task that only had its checks left is resumed, and the line that says so cannot be
    /// written. Its status is written by then (Running), so the task still counts as resumed:
    /// the end of the hold hands it to the caller, which starts its gate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resumed_task_reaches_its_gate_when_its_history_line_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        let pass = inner.sched_lock.lock().await; // no scheduling pass writes to the task meanwhile
        let task = held_task(&core, "Add sign-in", Path::new(&ws.repo_path)).await;
        let hold = Hold { tasks: vec![task.id.clone()], regate: vec![task.id.clone()] };
        inner.store.with(|db| Ok(db.execute_batch(NO_DECISIONS)?)).unwrap();
        assert!(inner.decide(Some(&ws.id), Some(&task.id), "retry", "a line", json!({})).is_err(), "the store takes no decision");

        let (regate, ended) = inner.end_hold(&hold, &[task.clone()]);
        assert_eq!(regate, [task.id.clone()], "its gate starts");
        assert!(ended.is_ok(), "{ended:?}");
        let now = inner.task(&task.id).unwrap();
        assert_eq!((now.status, now.status_reason.as_deref()), (TaskStatus::Running, Some(CHECKING)));
        inner.store.with(|db| Ok(db.execute_batch("DROP TRIGGER no_decisions")?)).unwrap();
        drop(pass);
        core.shutdown().await;
    }

    /// A hold can be days old. A task that only had its checks left when it began, and whose
    /// worktree is gone by its end, is not sent to checks that have nothing to run on: its
    /// worker goes on. One whose worktree is there gets its gate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_whose_worktree_is_gone_goes_on_through_its_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        let pass = inner.sched_lock.lock().await; // no scheduling pass writes to the tasks meanwhile
        let there = held_task(&core, "Add sign-in", Path::new(&ws.repo_path)).await;
        let gone = held_task(&core, "Add sign-out", &dir.path().join("removed")).await;
        let hold = Hold { tasks: vec![there.id.clone(), gone.id.clone()], regate: vec![there.id.clone(), gone.id.clone()] };

        assert_eq!(inner.end_hold(&hold, &[there.clone(), gone.clone()]).0, [there.id.clone()]);
        let now = inner.task(&gone.id).unwrap();
        assert_eq!((now.status, now.status_reason.as_deref()), (TaskStatus::Ready, Some(RESUMING)));
        assert_eq!(inner.task(&there.id).unwrap().status, TaskStatus::Running);
        drop(pass);
        core.shutdown().await;
    }

    /// A hold that is on in this run and in the store, as a start that asked leaves it.
    fn hold_on(inner: &Inner, hold: Hold) {
        inner.keep_hold(Some(&hold)).unwrap();
        *lock(&inner.held_work) = Some(hold);
    }

    /// How many lines say that a task was resumed.
    async fn resumed_lines(core: &Core) -> usize {
        let said = |e: &&EventRow| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.starts_with("Resumed after restart"));
        core.events(None, None, 2000).await.unwrap().iter().filter(said).count()
    }

    /// The store does not take the last write of a hold's end: its own row stays.
    const HOLD_STAYS: &str =
        "CREATE TRIGGER hold_stays BEFORE DELETE ON kv WHEN OLD.key = 'resume_hold' BEGIN SELECT RAISE(ABORT, 'no room to end the hold'); END";

    /// Your Resume writes each task first and takes the hold out of the store last. That last
    /// write fails (as a crash before it would leave things): the hold is still on, here and
    /// in the store, and counts the task that still waits. The task that got its checks back
    /// is not left without its gate. A second Resume ends the hold.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hold_whose_end_is_not_written_stays_on_until_the_next_resume() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        core.update_settings(Settings { lead_followups: false, ..core.settings() }).await.unwrap();
        let checks = held_task(&core, "Add sign-in", Path::new(&ws.repo_path)).await;
        let worker = held_task(&core, "Add sign-out", &dir.path().join("removed")).await;
        hold_on(inner, Hold { tasks: vec![checks.id.clone(), worker.id.clone()], regate: vec![checks.id.clone()] });
        assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }));
        inner.store.with(|db| Ok(db.execute_batch(HOLD_STAYS)?)).unwrap();

        let refused = core.resume_work().await.unwrap_err().to_string();
        assert!(refused.starts_with("not all of the work could be resumed") && refused.ends_with("press 'Resume work' again"), "{refused}");
        assert_ne!(inner.task(&checks.id).unwrap().status, TaskStatus::Ready, "the task that only had its checks left went to them");
        assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 1 }), "the hold is on, for the task that still waits");
        assert!(inner.store.kv_get(HOLD).unwrap().is_some(), "the next start would find the hold");
        assert_eq!(inner.task(&worker.id).unwrap().status, TaskStatus::Ready, "nothing starts behind a hold that is on");

        inner.store.with(|db| Ok(db.execute_batch("DROP TRIGGER hold_stays")?)).unwrap();
        core.resume_work().await.unwrap();
        assert_eq!(core.resume_offer(), None, "the second Resume ended the hold");
        assert_eq!(inner.store.kv_get(HOLD).unwrap(), None);
        core.shutdown().await;
    }

    /// One task of the hold cannot be written at your Resume. The hold stays on, and still
    /// counts both tasks: nothing starts while it is on. The second Resume writes the task
    /// and ends the hold.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hold_stays_on_while_one_of_its_tasks_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let (core, _) = start(dir.path()).await;
        let inner = &core.inner;
        let first = held_task(&core, "Add sign-in", &dir.path().join("removed")).await;
        let second = held_task(&core, "Add sign-out", &dir.path().join("removed")).await;
        hold_on(inner, Hold { tasks: vec![first.id.clone(), second.id.clone()], regate: vec![] });
        let no_change = format!(
            "CREATE TRIGGER no_change BEFORE INSERT ON event WHEN NEW.type = 'task_changed' AND NEW.task_id = '{}' \
             BEGIN SELECT RAISE(ABORT, 'no room for this task'); END",
            second.id
        );
        inner.store.with(|db| Ok(db.execute_batch(&no_change)?)).unwrap();

        let refused = core.resume_work().await.unwrap_err().to_string();
        assert!(refused.starts_with("not all of the work could be resumed"), "{refused}");
        assert_eq!(core.resume_offer(), Some(ResumeOffer { tasks: 2 }), "the hold is on: both tasks still wait");
        let unwritten = inner.task(&second.id).unwrap();
        assert_eq!((unwritten.status, unwritten.status_reason.as_deref()), (TaskStatus::Ready, Some(HELD)));
        assert_eq!(resumed_lines(&core).await, 1, "the first task was written");

        inner.store.with(|db| Ok(db.execute_batch("DROP TRIGGER no_change")?)).unwrap();
        core.resume_work().await.unwrap();
        assert_eq!(core.resume_offer(), None);
        assert_eq!(inner.store.kv_get(HOLD).unwrap(), None);
        assert_eq!(resumed_lines(&core).await, 3, "both tasks are written at the second Resume");
        core.shutdown().await;
    }

    /// You cancel a task between the look at what waits and the write that resumes it. That
    /// is no failed write: the task waits no more, and the hold ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_you_cancelled_meanwhile_does_not_keep_the_hold_on() {
        let dir = tempfile::tempdir().unwrap();
        let (core, _) = start(dir.path()).await;
        let inner = &core.inner;
        let as_read = held_task(&core, "Add sign-in", &dir.path().join("removed")).await;
        let hold = Hold { tasks: vec![as_read.id.clone()], regate: vec![] };
        hold_on(inner, hold.clone());
        core.move_task(&as_read.id, TaskStatus::Cancelled).await.unwrap();

        let (regate, ended) = inner.end_hold(&hold, &[as_read.clone()]);
        assert!(regate.is_empty() && ended.is_ok(), "{regate:?} {ended:?}");
        assert_eq!(inner.store.kv_get(HOLD).unwrap(), None, "the store forgot the hold");
        assert_eq!(inner.task(&as_read.id).unwrap().status, TaskStatus::Cancelled);
        core.shutdown().await;
    }

    /// The keep-awake and quit checks look at the hold too (`Core::activity`), and their look
    /// ends a hold that waits for nothing, as the scheduler's does: no pass runs here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_activity_look_ends_a_hold_that_waits_for_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (core, _) = start(dir.path()).await;
        let inner = &core.inner;
        let pass = inner.sched_lock.lock().await; // the scheduler does not look meanwhile
        let task = held_task(&core, "Add sign-in", &dir.path().join("removed")).await;
        hold_on(inner, Hold { tasks: vec![task.id.clone()], regate: vec![] });
        assert!(!core.activity().unwrap().busy(), "work that waits for you is not work under way");
        assert!(inner.store.kv_get(HOLD).unwrap().is_some(), "a task still waits: the hold is on");

        // You cancel the task. Nothing else would start, so the next look ends the hold.
        let cancel = |t: &mut Task| {
            t.status = TaskStatus::Cancelled;
            Ok(())
        };
        inner.update_task_as(&task.id, true, cancel).unwrap();
        core.activity().unwrap();
        assert!(lock(&inner.held_work).is_none(), "the hold is over in this run");
        assert_eq!(inner.store.kv_get(HOLD).unwrap(), None, "and in the store");
        drop(pass);
        core.shutdown().await;
    }

    /// An agent you let go leaves nothing behind in the store: the result of its last check,
    /// what Codly's commits left out in its worktree, the note about files you put back.
    /// Another agent's rows stay.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_agent_that_is_let_go_leaves_no_rows_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let store = &core.inner.store;
        let hire = |name: &str| core.inner.create_agent(&ws.id, name, Provider::Claude, Role::Solo, 0, None).unwrap();
        let (ada, bo) = (hire("ada"), hire("bo"));
        // As `required_checks.rs`, `left_out.rs` and `checkpoints.rs` name them.
        let rows_of = |id: &str| [required_checks::checked_key(id), format!("left_out:{id}:/work/tree"), format!("checkpoint_note:{id}:/work/tree")];
        for key in rows_of(&ada.id).iter().chain(&rows_of(&bo.id)) {
            store.kv_set(key, "kept").unwrap();
        }

        core.let_go(&ada.id).await.unwrap();
        for key in rows_of(&ada.id) {
            assert_eq!(store.kv_get(&key).unwrap(), None, "{key}");
        }
        for key in rows_of(&bo.id) {
            assert_eq!(store.kv_get(&key).unwrap().as_deref(), Some("kept"), "{key}");
        }
        core.shutdown().await;
    }

    /// The stored result of an agent's last check cannot be read (written by another version,
    /// or damaged). It counts as no result and the row goes, so the Git tab's question is
    /// answered now and at every later look.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_result_that_cannot_be_read_counts_as_none_and_goes() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let ada = core.inner.create_agent(&ws.id, "ada", Provider::Claude, Role::Solo, 0, None).unwrap();
        let key = required_checks::checked_key(&ada.id);
        core.inner.store.kv_set(&key, "not a result").unwrap();

        let nothing = required_checks::AgentCheck { running: false, last: None };
        assert_eq!(core.agent_check_state(&ada.id).await.unwrap(), nothing);
        assert_eq!(core.inner.store.kv_get(&key).unwrap(), None, "the row is gone");
        assert_eq!(core.agent_check_state(&ada.id).await.unwrap(), nothing);
        core.shutdown().await;
    }

    /// An office you close leaves no rows of its tasks in the store: the rounds they used
    /// (also a count this run could not read) and what Codly's commits left out for them.
    /// The rows of a task of another office stay.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_office_leaves_no_rows_of_its_tasks_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        let mut over = vec![];
        for title in ["Add sign-in", "Add sign-out"] {
            let task = core.create_task(title, "").await.unwrap();
            core.move_task(&task.id, TaskStatus::Cancelled).await.unwrap();
            over.push(task.id);
        }
        inner.count_rounds(&over[0], |_| ()).unwrap();
        inner.store.kv_set(&rounds_key(&over[1]), "not json").unwrap();
        inner.store.kv_set(&format!("left_out:{}:/work/tree", over[0]), "[]").unwrap();
        let elsewhere = [rounds_key("elsewhere"), "left_out:elsewhere:/work/tree".to_string()];
        for key in &elsewhere {
            inner.store.kv_set(key, "{}").unwrap();
        }
        let rows = || -> Vec<String> { [ROUNDS, "left_out:"].iter().flat_map(|prefix| inner.store.kv_prefixed(prefix).unwrap()).map(|row| row.0).collect() };
        assert_eq!(rows().len(), 5);

        core.close_office(&ws.id).await.unwrap();
        assert_eq!(rows(), elsewhere);
        assert!(lock(&inner.fix_rounds).is_empty(), "this run forgot the counts too");
        core.shutdown().await;
    }
}
