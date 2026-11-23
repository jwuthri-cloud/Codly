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
//! What is open comes from the adapters. Claude Code says its whole list at every change
//! (`EventKind::BackgroundTasks`). A Codex subagent works on a thread of its own, and its turn
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

/// How long the gate waits for a worker's background work before Codly stops it.
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
    /// A pass of the gate waits for this work: the number of that wait. A turn that starts
    /// ends the wait, because the turn's end runs the gate again.
    hold: Option<u64>,
    /// The waits that began in this run.
    holds: u64,
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
    }

    /// A turn of the run started.
    pub(super) fn turn_started(&mut self) {
        self.hold = None;
    }

    /// Does a pass of the gate wait for this work? Then Codly is about to act on the task.
    pub(super) fn holds_gate(&self) -> bool {
        self.hold.is_some()
    }

    /// The open work that the gate waits for.
    fn waited_for(&self) -> Vec<BackgroundTask> {
        self.open.iter().filter(|t| !self.let_go.contains(&t.id)).cloned().collect()
    }

    /// The wait of the gate, when nothing it waits for is left.
    fn settled(&self) -> Option<u64> {
        self.hold.filter(|_| self.waited_for().is_empty())
    }

    /// A pass of the gate waits from now on, unless one does already. Some = the new wait.
    fn begin_hold(&mut self) -> Option<u64> {
        if self.hold.is_some() {
            return None;
        }
        self.holds += 1;
        self.hold = Some(self.holds);
        self.hold
    }
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
    /// the work when the wait runs out.
    pub(super) fn hold_for_background(self: &Arc<Self>, task: &Task, worker: &str, ws: Option<&str>) -> Result<bool> {
        let (open, begun, run_id) = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(worker).filter(|l| l.task_id.as_deref() == Some(task.id.as_str())) else { return Ok(false) };
            let open = l.background.waited_for();
            if open.is_empty() {
                l.background.hold = None;
                return Ok(false);
            }
            (open, l.background.begin_hold(), l.run_id.clone())
        };
        let wait = *read(&self.background_wait);
        let reason = format!(
            "its worker left {} running; Codly checks the work when that ends, and stops {} after {}",
            names(&open),
            it_or_them(&open),
            gate::span(wait)
        );
        self.update_task(&task.id, |t| {
            if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        })?;
        let Some(hold) = begun else { return Ok(true) };
        let detail = json!({ "plan_id": task.plan_id, "background": open, "wait_secs": wait.as_secs() });
        self.decide(ws, Some(&task.id), "gate", format!("{}: not checked yet, {reason}", task.key), detail)?;
        tokio::spawn(self.clone().background_wait_over(task.id.clone(), worker.to_string(), run_id, hold, wait));
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

    /// The wait `hold` of a pass of the gate ran out. Still that wait, with work still open:
    /// Codly asks the CLI to stop each piece, looks at what the CLI then says is left, writes
    /// both in the task's record, and runs the gate on the work as it is. The wait stands
    /// until the gate runs, so the watchdog does not take the worker for idle meanwhile.
    async fn background_wait_over(self: Arc<Self>, task_id: Id, worker: Id, run_id: Id, hold: u64, wait: Duration) {
        tokio::time::sleep(wait).await;
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let you_hold = self.control_holding(&worker);
        let (left, provider) = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(&worker).filter(|l| l.run_id == run_id && !l.stopping && l.background.hold == Some(hold)) else { return };
            // A worker you hold stays out of the gate: your Resume runs it, and it waits anew.
            if you_hold {
                l.background.hold = None;
                return;
            }
            let left = l.background.waited_for();
            // Nothing left: the pass goes on by itself (`background_changed`). A prompt on
            // its way: the end of its turn runs the gate.
            if left.is_empty() || l.turn_active || l.queued > 0 {
                return;
            }
            // An idle Codex agent takes an interrupt as: stop the turns of my subagents
            // (codex.rs `interrupt`; from the types, not verified on a live run).
            if l.provider == Provider::Codex {
                l.handle.send(AgentCmd::Interrupt);
            }
            (left, l.provider)
        };
        if provider == Provider::Claude {
            // `stop_task` is recorded on a background command (part 2 §6), not on a subagent.
            // Its answer is the same whether anything was stopped or not, so nobody waits for
            // it: the CLI's next list of its background tasks is what Codly reads below.
            for t in &left {
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
                .map(|l| left.iter().filter(|t| l.background.open.iter().any(|o| o.id == t.id)).cloned().collect());
            // The run ended meanwhile: whoever ended it says what is next for the task.
            let Some(kept) = kept else { return };
            if kept.is_empty() || Instant::now() >= until {
                break kept;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        // What is still there is not waited for a second time. The pass goes on now, unless
        // a turn started meanwhile: the end of that turn runs the gate.
        let go_on = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(&worker).filter(|l| l.run_id == run_id) else { return };
            l.background.let_go.extend(kept.iter().map(|t| t.id.clone()));
            l.background.hold == Some(hold)
        };
        let Ok(task) = self.task(&task_id) else { return };
        let stopped: Vec<&BackgroundTask> = left.iter().filter(|t| !kept.contains(t)).collect();
        let acted = match (stopped.is_empty(), kept.is_empty()) {
            (false, true) => format!("Codly stopped {}", it_or_them(&left)),
            (false, false) => format!("Codly stopped {}. It could not stop {}", names(stopped.iter().copied()), names(&kept)),
            (true, _) => format!("Codly could not stop {}", it_or_them(&left)),
        };
        let next = match (go_on, kept.is_empty()) {
            (false, _) => "",
            (true, true) => " and checks the work now",
            (true, false) => ", and checks the work as it is",
        };
        let summary = format!("{}: {} still ran after {}. {acted}{next}", task.key, names(&left), gate::span(wait));
        let ws = self.store.task_workspace(&task_id).ok().flatten();
        let detail = json!({ "plan_id": task.plan_id, "stopped": stopped, "not_stopped": kept, "wait_secs": wait.as_secs() });
        if let Err(e) = self.decide(ws.as_deref(), Some(&task_id), "gate", summary, detail) {
            tracing::warn!("the stop of {}'s background work was not recorded: {e}", task.key);
        }
        if go_on {
            // This wait ends with the pass that starts now: it waits anew for work that the
            // run got meanwhile.
            if let Some(l) = lock(&self.live).get_mut(&worker).filter(|l| l.run_id == run_id && l.background.hold == Some(hold)) {
                l.background.hold = None;
            }
            self.gate(task_id).await;
        }
    }
}

impl super::Core {
    /// How long the gate waits for a worker's background work before Codly stops it (`WAIT`
    /// until set; the tests shorten it).
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
        assert_eq!(b.begin_hold(), Some(1));
        assert_eq!(b.begin_hold(), None, "one wait at a time");
        assert_eq!(b.settled(), None, "the command still runs");
        // The list is the whole list each time: an empty one means nothing is left.
        b.listed(&[]);
        assert_eq!(b.settled(), Some(1));
        // A turn that starts ends the wait: its end runs the gate.
        b.turn_started();
        assert_eq!((b.settled(), b.holds_gate()), (None, false));
        b.listed(&[task("tid-2", "command", "npm run dev")]);
        assert_eq!(b.begin_hold(), Some(2), "a later wait has its own number");
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
