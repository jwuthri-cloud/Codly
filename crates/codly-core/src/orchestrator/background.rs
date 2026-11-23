//! Work that a run left behind its turn. A Claude Code agent can end its turn while a command
//! it started in the background still runs, or while a helper (a subagent) still works. What
//! they write next would miss Codly's commit, and the task's checks would run under them. So
//! the gate (`gate.rs`) does not check, review or merge a task while its worker's run has such
//! work open: the pass waits (`hold_for_background`), and goes on
//! - at the end of the turn that the CLI starts by itself once its background task is done
//!   (recorded for Claude Code 2.1.311: cli-claude-stream-json-2.1.311-part2 §6), as after
//!   any other turn of the worker;
//! - when nothing is left open and no such turn came (`background_changed`);
//! - after `WAIT` at the latest: Codly stops what still runs, says in the task's record what
//!   it stopped, and checks the work as it is (`background_wait_over`).
//!
//! `WAIT` is one budget for a run of the worker, not a time per wait. A turn of the worker
//! ends the wait of a pass, and the pass at the end of that turn waits again: it gets what is
//! left of the budget. With nothing left Codly stops the open work at once. So a dev server
//! that never ends, beside a test run that ends after 4 minutes and wakes the CLI, is stopped
//! 5 minutes after the first wait began, not 9. A new run of the task (a retry) has a new
//! budget.
//!
//! What is open comes from the adapters. Claude Code says its whole list at every change
//! (`EventKind::BackgroundTasks`, the one record of that work: the agent sheet shows the same
//! events). A Codex subagent works on a thread of its own, and its turn
//! can go on after the agent's turn ended: the `SubAgent` rows of those threads say when one
//! is at work. That part is built from the types of codex-cli 0.166.0 only, not verified on
//! a live run.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::json;

use super::{Inner, gate, lock, prompts, read, write};
use crate::adapters::{AgentCmd, SessionRequest};
use crate::git::name_list;
use crate::model::*;

/// How long the gate waits for a worker's background work before Codly stops it: in all the
/// waits of one run together.
pub(super) const WAIT: Duration = Duration::from_secs(300);
/// How long Codly waits, once nothing is open any more, for the turn that Claude Code starts
/// by itself when a background task ends (recorded: its first line follows the task's last
/// one at once). That turn's end runs the gate; with no such turn, the gate runs after this.
const SETTLE: Duration = Duration::from_secs(1);
/// How long the CLI gets to say that what Codly stopped is gone.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// What a live run has at work beside its own turns, and the gate's wait for it
/// (`Live::background`).
#[derive(Debug, Default)]
pub(super) struct Background {
    /// What is open, as the CLI last said.
    open: Vec<BackgroundTask>,
    /// Open work that the gate does not wait for a second time: its wait ran out, Codly asked
    /// the CLI to stop it, and the CLI still lists it.
    let_go: HashSet<String>,
    /// Open work that Codly asked the CLI to stop, while it still looks at what the CLI says
    /// is left (`background_wait_over`): a later wait that runs out asks for none of it again.
    stopping: HashSet<String>,
    /// A pass of the gate waits for this work: the number of that wait, and when it began. A
    /// turn that starts ends the wait, because the turn's end runs the gate again.
    hold: Option<(u64, Instant)>,
    /// The waits that began in this run.
    holds: u64,
    /// How long the waits of this run that are over took together: used up of the run's
    /// budget (`WAIT`).
    waited: Duration,
}

impl Background {
    /// The CLI said its whole list of background tasks.
    pub(super) fn listed(&mut self, tasks: &[BackgroundTask]) {
        self.open = tasks.to_vec();
        self.forget_gone();
    }

    /// A subagent started to work on a thread of its own, or it stopped.
    pub(super) fn helper(&mut self, id: &str, description: &str, at_work: bool) {
        self.open.retain(|t| t.id != id);
        if at_work {
            self.open.push(BackgroundTask { id: id.into(), kind: "helper".into(), description: description.into() });
        }
        self.forget_gone();
    }

    fn forget_gone(&mut self) {
        let open = &self.open;
        self.let_go.retain(|id| open.iter().any(|t| t.id == *id));
        self.stopping.retain(|id| open.iter().any(|t| t.id == *id));
    }

    /// A turn of the run started.
    pub(super) fn turn_started(&mut self, now: Instant) {
        self.end_hold(now);
    }

    /// The wait of a pass is over (none: nothing changes). Its time is used up of the budget.
    fn end_hold(&mut self, now: Instant) {
        if let Some((_, since)) = self.hold.take() {
            self.waited += now.saturating_duration_since(since);
        }
    }

    /// Does a pass of the gate wait for this work? Then Codly is about to act on the task.
    pub(super) fn holds_gate(&self) -> bool {
        self.hold.is_some()
    }

    /// The number of the wait of the gate, when a pass waits.
    fn held(&self) -> Option<u64> {
        self.hold.map(|(hold, _)| hold)
    }

    /// The open work that the gate waits for.
    fn waited_for(&self) -> Vec<BackgroundTask> {
        self.open.iter().filter(|t| !self.let_go.contains(&t.id)).cloned().collect()
    }

    /// The wait of the gate, when nothing it waits for is left.
    fn settled(&self) -> Option<u64> {
        self.held().filter(|_| self.waited_for().is_empty())
    }

    /// A pass of the gate waits from now on, unless one does already. Some = the new wait.
    fn begin_hold(&mut self, now: Instant) -> Option<u64> {
        if self.hold.is_some() {
            return None;
        }
        self.holds += 1;
        self.hold = Some((self.holds, now));
        self.held()
    }

    /// What is left of the run's `budget` at `now`: the waits that are over and the one that
    /// runs are used up.
    fn left(&self, now: Instant, budget: Duration) -> Duration {
        let running = self.hold.map_or(Duration::ZERO, |(_, since)| now.saturating_duration_since(since));
        budget.saturating_sub(self.waited + running)
    }

    /// A wait ran out with `left` still open: what of it Codly asks the CLI to stop now. Work
    /// that an earlier wait asked the CLI to stop, and still looks at, is not asked for again.
    fn stop_now(&mut self, left: &[BackgroundTask]) -> Vec<BackgroundTask> {
        let asked: Vec<BackgroundTask> = left.iter().filter(|t| !self.stopping.contains(&t.id)).cloned().collect();
        self.stopping.extend(asked.iter().map(|t| t.id.clone()));
        asked
    }

    /// The wait `hold` ran out, Codly asked the CLI to stop `asked`, and `kept` of it is
    /// still listed: nobody waits for that a second time. True = a pass of the gate is due
    /// now: that wait is still the run's, or a later one has nothing left to wait for (a turn
    /// started and ended meanwhile, and the pass at its end waits for the same work).
    fn give_up(&mut self, hold: u64, asked: &[BackgroundTask], kept: &[BackgroundTask]) -> bool {
        self.stopping.retain(|id| !asked.iter().any(|t| t.id == *id));
        self.let_go.extend(kept.iter().map(|t| t.id.clone()));
        self.held() == Some(hold) || self.settled().is_some()
    }
}

/// A wait as the task's record says it: whole seconds, rounded up ("2 s" for 1.2 s left).
fn whole_secs(d: Duration) -> Duration {
    Duration::from_secs(d.as_secs() + u64::from(d.subsec_nanos() > 0))
}

/// "the background command `npm run dev`", "the helper `Run the tests`".
fn name(t: &BackgroundTask) -> String {
    let kind = match t.kind.as_str() {
        "command" => "background command",
        "helper" => "helper",
        _ => "background task",
    };
    let said = t.description.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.is_empty() { format!("a {kind}") } else { format!("the {kind} `{}`", prompts::clip(&said, 80)) }
}

fn names<'a>(tasks: impl IntoIterator<Item = &'a BackgroundTask>) -> String {
    name_list(&tasks.into_iter().map(name).collect::<Vec<_>>())
}

/// "it" for one piece of work, "them" for more.
fn it_or_them(tasks: &[BackgroundTask]) -> &'static str {
    if tasks.len() == 1 { "it" } else { "them" }
}

impl Inner {
    /// Before the gate commits a worker's files: does the worker's run still have work open
    /// in the background? True = this pass waits for it. The task says so, and Codly stops
    /// the work when the wait runs out: after what is left of the run's budget, at once when
    /// nothing is left. True also when a turn of the worker started since the pass began: the
    /// pass ends here, and the end of that turn runs the gate.
    pub(super) fn hold_for_background(self: &Arc<Self>, task: &Task, worker: &str, ws: Option<&str>) -> Result<bool> {
        let budget = *read(&self.background_wait);
        let now = Instant::now();
        let (open, begun, left, run_id) = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(worker).filter(|l| l.task_id.as_deref() == Some(task.id.as_str())) else { return Ok(false) };
            // A turn started since this pass began (`ingest` runs beside it), or a prompt is
            // on its way. No wait begins under a turn: the turn's end would find that wait
            // and begin none of its own, and nothing would end it once its time ran out.
            if l.turn_active || l.queued > 0 {
                return Ok(true);
            }
            let open = l.background.waited_for();
            if open.is_empty() {
                l.background.end_hold(now);
                return Ok(false);
            }
            let begun = l.background.begin_hold(now);
            (open, begun, l.background.left(now, budget), l.run_id.clone())
        };
        let reason = if left.is_zero() {
            format!("its worker left {} running; Codly stops {} now and checks the work", names(&open), it_or_them(&open))
        } else {
            format!(
                "its worker left {} running; Codly checks the work when that ends, and stops {} after {}",
                names(&open),
                it_or_them(&open),
                gate::span(whole_secs(left))
            )
        };
        self.update_task(&task.id, |t| {
            if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        })?;
        let Some(hold) = begun else { return Ok(true) };
        // (With nothing left of the budget the record of the stop follows at once.)
        if !left.is_zero() {
            let detail = json!({ "plan_id": task.plan_id, "background": open, "wait_secs": whole_secs(left).as_secs() });
            self.decide(ws, Some(&task.id), "gate", format!("{}: not checked yet, {reason}", task.key), detail)?;
        }
        tokio::spawn(self.clone().background_wait_over(task.id.clone(), worker.to_string(), run_id, hold, left));
        Ok(true)
    }

    /// A run's background work changed (`ingest`). When a pass of the gate waits for it and
    /// nothing is left, the pass goes on: at the end of the turn the CLI starts by itself, or
    /// here, a moment later, when no such turn came (Codly stopped the work, a Codex subagent
    /// ended its turn).
    pub(super) fn background_changed(self: &Arc<Self>, agent_id: &str, run_id: &str) {
        let held = lock(&self.live)
            .get(agent_id)
            .filter(|l| l.run_id == run_id)
            .and_then(|l| Some((l.background.settled()?, l.task_id.clone()?)));
        let Some((hold, task_id)) = held else { return };
        let (me, agent_id, run_id) = (self.clone(), agent_id.to_string(), run_id.to_string());
        tokio::spawn(async move {
            tokio::time::sleep(SETTLE).await;
            // Still that wait: no turn started since (its end runs the gate), and no prompt is
            // on its way. A worker you hold stays out of the gate until your Resume.
            let quiet = lock(&me.live).get(&agent_id).is_some_and(|l| {
                l.run_id == run_id && !l.stopping && !l.turn_active && l.queued == 0 && l.background.settled() == Some(hold)
            });
            if quiet && !me.control_holding(&agent_id) {
                me.gate(task_id).await;
            }
        });
    }

    /// The wait `hold` of a pass of the gate ran out, `wait` after it began (what was left of
    /// the run's budget: nothing, when the budget was used up before). Still that wait, with
    /// work still open: Codly asks the CLI to stop each piece, looks at what the CLI then says
    /// is left, writes both in the task's record, and runs the gate on the work as it is. The
    /// wait stands until the gate runs, so the watchdog does not take the worker for idle
    /// meanwhile.
    ///
    /// Nothing else bounds the wait. So every way out of here, while the run lives, ends the
    /// wait or runs the gate, unless the wait is not this one any more (a turn's start ended
    /// it, and the pass at that turn's end waits for what is left of the budget), or an
    /// earlier wait still looks at its own stop of all this work: that one runs the pass.
    async fn background_wait_over(self: Arc<Self>, task_id: Id, worker: Id, run_id: Id, hold: u64, wait: Duration) {
        tokio::time::sleep(wait).await;
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let budget = *read(&self.background_wait);
        let you_hold = self.control_holding(&worker);
        let (left, asked, provider) = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(&worker).filter(|l| l.run_id == run_id && !l.stopping && l.background.held() == Some(hold)) else { return };
            // A worker you hold stays out of the gate: your Resume runs it. A prompt on its
            // way: the end of its turn runs the gate. That pass waits for what the budget has left.
            if you_hold || l.turn_active || l.queued > 0 {
                l.background.end_hold(Instant::now());
                return;
            }
            let left = l.background.waited_for();
            let asked = l.background.stop_now(&left);
            // An idle Codex agent takes an interrupt as: stop the turns of my subagents
            // (codex.rs `interrupt`; from the types, not verified on a live run).
            if l.provider == Provider::Codex && !asked.is_empty() {
                l.handle.send(AgentCmd::Interrupt);
            }
            (left, asked, l.provider)
        };
        // Nothing is left, and no pass came for it (`background_changed` runs one only a
        // moment after a change, when the run is quiet then): the pass goes on now.
        if left.is_empty() {
            return self.pass_goes_on(&worker, &run_id, task_id).await;
        }
        // All of it is work that an earlier wait asked the CLI to stop a moment ago (a turn
        // of the worker started and ended meanwhile, and the budget is used up). That wait
        // runs the pass when it has looked at what is left (`give_up`).
        if asked.is_empty() {
            return;
        }
        if provider == Provider::Claude {
            // `stop_task` is recorded on a background command (part 2 §6), not on a subagent.
            // Its answer is the same whether anything was stopped or not, so nobody waits for
            // it: the CLI's next list of its background tasks is what Codly reads below.
            for t in &asked {
                if let Err(e) = self.ask_session(&worker, SessionRequest::StopTask(t.id.clone())) {
                    tracing::debug!("background task {} not stopped: {e}", t.id);
                }
            }
        }
        let until = Instant::now() + STOP_WAIT;
        let kept = loop {
            let kept: Option<Vec<BackgroundTask>> = lock(&self.live)
                .get(&worker)
                .filter(|l| l.run_id == run_id)
                .map(|l| asked.iter().filter(|t| l.background.open.iter().any(|o| o.id == t.id)).cloned().collect());
            // The run ended meanwhile: whoever ended it says what is next for the task.
            let Some(kept) = kept else { return };
            if kept.is_empty() || Instant::now() >= until {
                break kept;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        // What is still there is not waited for a second time. The pass goes on now, unless
        // a turn runs or a prompt is on its way: the end of that turn runs the gate.
        let go_on = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(&worker).filter(|l| l.run_id == run_id) else { return };
            let due = l.background.give_up(hold, &asked, &kept);
            let quiet = !l.turn_active && l.queued == 0;
            if due && !quiet {
                l.background.end_hold(Instant::now());
            }
            due && quiet
        };
        // "after 5 min" for work that one wait waited the whole budget for; else the budget
        // went on several waits of the run, and this work may have run for less.
        let whole = gate::span(budget);
        let waited = if wait >= budget { format!("after {whole}") } else { format!("after Codly waited {whole} in all") };
        self.say_stopped(&task_id, &asked, &kept, &waited, budget, go_on);
        if go_on {
            self.pass_goes_on(&worker, &run_id, task_id).await;
        }
    }

    /// The task's record says what Codly stopped of `left` when the wait ran out (`waited`:
    /// that wait in words, "after 5 min"; `budget`: the run's whole wait), and what it could
    /// not stop (`kept`).
    fn say_stopped(&self, task_id: &str, left: &[BackgroundTask], kept: &[BackgroundTask], waited: &str, budget: Duration, go_on: bool) {
        let task = match self.task(task_id) {
            Ok(task) => task,
            Err(e) => {
                tracing::warn!("the stop of {task_id}'s background work was not recorded: {e}");
                return;
            }
        };
        let stopped: Vec<&BackgroundTask> = left.iter().filter(|t| !kept.contains(t)).collect();
        let acted = match (stopped.is_empty(), kept.is_empty()) {
            (false, true) => format!("Codly stopped {}", it_or_them(left)),
            (false, false) => format!("Codly stopped {}. It could not stop {}", names(stopped.iter().copied()), names(kept)),
            (true, _) => format!("Codly could not stop {}", it_or_them(left)),
        };
        let next = match (go_on, kept.is_empty()) {
            (false, _) => "",
            (true, true) => " and checks the work now",
            (true, false) => ", and checks the work as it is",
        };
        let summary = format!("{}: {} still ran {waited}. {acted}{next}", task.key, names(left));
        let ws = self.store.task_workspace(task_id).ok().flatten();
        let detail = json!({ "plan_id": task.plan_id, "stopped": stopped, "not_stopped": kept, "wait_secs": budget.as_secs() });
        if let Err(e) = self.decide(ws.as_deref(), Some(task_id), "gate", summary, detail) {
            tracing::warn!("the stop of {}'s background work was not recorded: {e}", task.key);
        }
    }

    /// The wait of a pass of the gate is over, and the pass goes on. The wait stood until
    /// here (the watchdog); the pass that starts waits anew, for what is left of the run's
    /// budget, for work that the run got meanwhile.
    async fn pass_goes_on(self: Arc<Self>, worker: &str, run_id: &str, task_id: Id) {
        if let Some(l) = lock(&self.live).get_mut(worker).filter(|l| l.run_id == run_id) {
            l.background.end_hold(Instant::now());
        }
        self.gate(task_id).await;
    }
}

impl super::Core {
    /// How long the gate waits for a worker's background work, in all the waits of one run,
    /// before Codly stops it (`WAIT` until set; the tests shorten it).
    pub fn set_background_wait(&self, wait: Duration) {
        *write(&self.inner.background_wait) = wait;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, kind: &str, description: &str) -> BackgroundTask {
        BackgroundTask { id: id.into(), kind: kind.into(), description: description.into() }
    }

    #[test]
    fn the_gate_waits_for_what_the_cli_lists_until_the_list_is_empty() {
        let mut b = Background::default();
        assert!(b.waited_for().is_empty() && !b.holds_gate());
        b.listed(&[task("tid-1", "command", "sleep 8; echo woke")]);
        assert_eq!(b.waited_for(), [task("tid-1", "command", "sleep 8; echo woke")]);
        let t0 = Instant::now();
        assert_eq!(b.begin_hold(t0), Some(1));
        assert_eq!(b.begin_hold(t0), None, "one wait at a time");
        assert_eq!(b.settled(), None, "the command still runs");
        // The list is the whole list each time: an empty one means nothing is left.
        b.listed(&[]);
        assert_eq!(b.settled(), Some(1));
        // A turn that starts ends the wait: its end runs the gate.
        b.turn_started(t0);
        assert_eq!((b.settled(), b.holds_gate()), (None, false));
        b.listed(&[task("tid-2", "command", "npm run dev")]);
        assert_eq!(b.begin_hold(t0), Some(2), "a later wait has its own number");
    }

    #[test]
    fn the_waits_of_one_run_share_one_budget() {
        let (budget, min) = (Duration::from_secs(300), Duration::from_secs(60));
        let t0 = Instant::now();
        let mut b = Background::default();
        // A dev server that never ends and a test run: the first wait has the whole budget.
        b.listed(&[task("tid-1", "command", "npm run dev"), task("tid-2", "command", "npm test")]);
        assert_eq!(b.left(t0, budget), budget);
        b.begin_hold(t0).unwrap();
        assert_eq!(b.left(t0, budget), budget);
        assert_eq!(b.left(t0 + min, budget), 4 * min, "the wait that runs is used up as it goes");
        // Minute 4: the tests end, and the CLI starts a turn by itself. The pass at the end
        // of that turn waits for the server with the minute that is left, not 5 more.
        b.listed(&[task("tid-1", "command", "npm run dev")]);
        b.turn_started(t0 + 4 * min);
        assert_eq!(b.left(t0 + 4 * min + min / 2, budget), min, "no wait runs during the turn: its time is not used up");
        b.begin_hold(t0 + 4 * min + min / 2).unwrap();
        assert_eq!(b.left(t0 + 4 * min + min / 2, budget), min);
        // That wait runs out, and a pass that waits after it has nothing left: Codly stops
        // what is open at once.
        b.end_hold(t0 + 5 * min + min / 2);
        assert_eq!(b.left(t0 + 6 * min, budget), Duration::ZERO);
        b.begin_hold(t0 + 6 * min).unwrap();
        b.end_hold(t0 + 7 * min);
        assert_eq!(b.left(t0 + 7 * min, budget), Duration::ZERO, "never below nothing");
        // A wait that is over twice counts once.
        b.end_hold(t0 + 9 * min);
        assert_eq!(b.waited, 6 * min);
        // A new run of the task has its own `Background`: a new budget.
        assert_eq!(Background::default().left(t0 + 9 * min, budget), budget);
        // The task's record says a wait in whole seconds, rounded up.
        assert_eq!(whole_secs(Duration::from_millis(1200)), Duration::from_secs(2));
        assert_eq!(whole_secs(Duration::from_secs(60)), Duration::from_secs(60));
    }

    #[test]
    fn work_that_codly_asked_to_stop_is_not_asked_for_again_while_it_looks() {
        let (server, helper) = (task("tid-1", "command", "npm run dev"), task("tid-2", "helper", "Run the tests"));
        let t0 = Instant::now();
        let mut b = Background::default();
        b.listed(std::slice::from_ref(&server));
        let first = b.begin_hold(t0).unwrap();
        // The wait ran out: Codly asks the CLI to stop the server, and looks at its list.
        assert_eq!(b.stop_now(std::slice::from_ref(&server)), [server.clone()]);
        // Meanwhile a turn of the worker starts and ends with a helper at work. The budget
        // is used up, so the wait of the pass at its end runs out at once: only the helper
        // is asked for.
        b.turn_started(t0);
        b.listed(&[server.clone(), helper.clone()]);
        let second = b.begin_hold(t0).unwrap();
        assert_eq!(b.stop_now(&[server.clone(), helper.clone()]), [helper.clone()]);
        assert!(b.stop_now(&[server.clone(), helper.clone()]).is_empty(), "both stops are looked at");
        // The first look is over (the server is still listed): the later wait still has the
        // helper, whose stop is looked at by its own wait.
        assert!(!b.give_up(first, std::slice::from_ref(&server), std::slice::from_ref(&server)));
        assert_eq!(b.waited_for(), [helper.clone()]);
        // The helper is gone from the list: nothing is left, and its wait is the run's.
        b.listed(std::slice::from_ref(&server));
        assert!(b.give_up(second, std::slice::from_ref(&helper), &[]));
        assert!(b.stopping.is_empty() && b.waited_for().is_empty());
    }

    #[test]
    fn work_whose_wait_ran_out_is_not_waited_for_again() {
        let mut b = Background::default();
        b.listed(&[task("tid-1", "command", "npm run dev"), task("tid-2", "helper", "Run the tests")]);
        b.let_go.insert("tid-1".into());
        assert_eq!(b.waited_for(), [task("tid-2", "helper", "Run the tests")]);
        // Still listed (the stop did not end it): no second wait for it.
        b.listed(&[task("tid-1", "command", "npm run dev")]);
        assert!(b.waited_for().is_empty());
        // Gone from the list: forgotten, so the same id later would be new work.
        b.listed(&[]);
        assert!(b.let_go.is_empty());
    }

    #[test]
    fn a_pass_is_due_when_the_stop_left_a_later_wait_with_nothing_to_wait_for() {
        let helper = task("tid-2", "helper", "Run the tests");
        let mut b = Background::default();
        b.listed(std::slice::from_ref(&helper));
        let t0 = Instant::now();
        let first = b.begin_hold(t0).unwrap();
        // The wait ran out and Codly asked the CLI to stop the helper. While Codly looks at
        // what is left, a short turn of the worker starts and ends, and the pass at its end
        // waits for the same helper.
        assert_eq!(b.stop_now(std::slice::from_ref(&helper)), [helper.clone()]);
        b.turn_started(t0);
        let second = b.begin_hold(t0).unwrap();
        assert_ne!(first, second);
        // The later wait asks for no second stop of it: the first one is still looked at.
        assert!(b.stop_now(std::slice::from_ref(&helper)).is_empty());
        // The helper is still listed, and nobody waits for it a second time: the later wait
        // has nothing left, so a pass is due now.
        assert!(b.give_up(first, std::slice::from_ref(&helper), std::slice::from_ref(&helper)));
        assert!(b.waited_for().is_empty());
        assert_eq!(b.settled(), Some(second));
    }

    #[test]
    fn a_pass_is_due_after_the_stop_only_for_a_wait_that_nobody_else_ends() {
        let server = task("tid-1", "command", "npm run dev");
        // No turn since the wait began: the pass goes on, with or without the work.
        let (t0, asked) = (Instant::now(), std::slice::from_ref(&server));
        let mut b = Background::default();
        b.listed(asked);
        let hold = b.begin_hold(t0).unwrap();
        assert!(b.give_up(hold, asked, asked));
        // A turn started and still runs: its end runs the gate.
        let mut b = Background::default();
        b.listed(asked);
        let hold = b.begin_hold(t0).unwrap();
        b.turn_started(t0);
        assert!(!b.give_up(hold, asked, asked));
        // A later wait is for work the run got meanwhile: its own end runs the pass.
        let mut b = Background::default();
        b.listed(asked);
        let hold = b.begin_hold(t0).unwrap();
        b.turn_started(t0);
        b.listed(&[server.clone(), task("tid-3", "command", "cargo watch")]);
        let later = b.begin_hold(t0).unwrap();
        assert!(!b.give_up(hold, asked, asked));
        assert_eq!(b.waited_for(), [task("tid-3", "command", "cargo watch")]);
        assert_eq!((b.settled(), b.held()), (None, Some(later)));
    }

    #[test]
    fn a_subagent_on_its_own_thread_is_open_while_it_is_at_work() {
        let mut b = Background::default();
        b.helper("thread-1", "Ada (explorer): find the callers", true);
        b.helper("thread-1", "Ada (explorer): find the callers", true);
        assert_eq!(b.waited_for(), [task("thread-1", "helper", "Ada (explorer): find the callers")], "one entry per subagent");
        // The row of a call that is not a subagent at work changes nothing.
        b.helper("call-9", "wait for Ada", false);
        assert_eq!(b.waited_for().len(), 1);
        b.helper("thread-1", "Ada (explorer): find the callers", false);
        assert!(b.waited_for().is_empty());
    }

    #[test]
    fn open_work_is_named_in_plain_words() {
        assert_eq!(name(&task("a", "command", "npm run dev")), "the background command `npm run dev`");
        assert_eq!(name(&task("b", "helper", "Run the\n  tests")), "the helper `Run the tests`");
        assert_eq!(name(&task("c", "task", "")), "a background task");
        let long = name(&task("d", "command", &"x".repeat(200)));
        assert!(long.chars().count() < 110 && long.ends_with("…`"), "{long}");
        let all = [task("a", "command", "npm run dev"), task("b", "helper", "Run the tests")];
        assert_eq!(names(&all), "the background command `npm run dev`, the helper `Run the tests`");
    }
}
