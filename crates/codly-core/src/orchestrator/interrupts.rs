//! What follows an interrupt that the CLI does not answer.
//!
//! An adapter gives an interrupted turn a few seconds to end. Past that it ends the process
//! (on the shared Codex server: the thread) and leaves a record (`adapters::gave_up`). The exit
//! that follows is Codly's doing, not a crash: the agent is not failed, and its task's worker
//! is not restarted "after an unexpected exit". Who asked for the interrupt says what is next
//! (`Live::interrupt` keeps who that was until the turn ends):
//!
//! - **The watchdog**, for a silent turn: the same as for a run the watchdog stops itself
//!   (`watchdog::stalled_run_gave_up`). A worker's task goes back in line, blocked from its
//!   third start on.
//! - **You, the circuit breaker, a tool block**: the turn was to stop, so the worker's task
//!   waits for you, Blocked, as after a Stop. A restart would go on with the work, and the
//!   pause or the tool block that came with the interrupt ended with the process.
//!
//! A review ends without a verdict, as when its reviewer exits by itself. A Lead or a 1:1
//! agent resumes its session with your next message.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Inner, Live, lock, watchdog};
use crate::adapters::{self, AgentCmd};
use crate::model::*;

/// Who asked a run to stop its turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Asker {
    You,
    /// The backstop for an agent whose tools you blocked (`control_tool_started`).
    ToolBlock,
    Breaker,
    /// The watchdog, for a turn that gave no output for `quiet`.
    Watchdog { quiet: Duration },
}

/// "interrupted by you": how the turn was stopped, for the reason on a task. None: nobody
/// is on record (the interrupt went out before Codly saw the turn).
fn interrupted(by: Option<Asker>) -> &'static str {
    match by {
        Some(Asker::You) => "interrupted by you",
        Some(Asker::ToolBlock) => "interrupted because you blocked its tools",
        Some(Asker::Breaker) => "interrupted by the circuit breaker",
        Some(Asker::Watchdog { .. }) | None => "interrupted",
    }
}

/// What a live run remembers about the interrupt of its turn (`Live::interrupt`).
#[derive(Debug, Default)]
pub(super) struct Interrupt {
    /// Who asked the turn that runs to stop. The turn's end answers it.
    by: Option<Asker>,
    /// The adapter gave the turn up and ends the run.
    gave_up: bool,
}

impl Interrupt {
    /// An event of the run came (`track_live`).
    pub(super) fn seen(&mut self, kind: &EventKind) {
        match kind {
            EventKind::Decision { decision_type, .. } if decision_type == adapters::GAVE_UP => self.gave_up = true,
            // The turn ended in time. (An adapter that gave up closes the turn at the exit.)
            EventKind::TurnCompleted { .. } if !self.gave_up => self.by = None,
            _ => {}
        }
    }

    /// The adapter gave the turn up: the run is ending.
    pub(super) fn gave_up(&self) -> bool {
        self.gave_up
    }
}

/// A run that its adapter ended because the turn did not stop, as `on_exit` saw it.
pub(super) struct GaveUp {
    by: Option<Asker>,
    run: watchdog::Look,
}

impl Live {
    /// Ask the run to stop its turn, and note who asked. False: its process is gone.
    pub(super) fn send_interrupt(&mut self, by: Asker) -> bool {
        if !self.handle.send(AgentCmd::Interrupt) {
            return false;
        }
        // An idle run has no turn to stop. And the watchdog does not take over a turn that
        // somebody asked to stop already: what follows stays theirs to say.
        let busy = self.turn_active || self.queued > 0;
        if busy && !(matches!(by, Asker::Watchdog { .. }) && self.interrupt.by.is_some()) {
            self.interrupt.by = Some(by);
        }
        true
    }

    /// Some when the run's adapter gave its interrupted turn up: the exit is that, not a crash.
    pub(super) fn gave_up(&self, agent_id: &str) -> Option<GaveUp> {
        self.interrupt.gave_up().then(|| GaveUp { by: self.interrupt.by, run: watchdog::Look::of(agent_id, self, Instant::now()) })
    }
}

impl Inner {
    /// Ask an agent's live run to stop its turn. False: it has none, or its process is gone.
    pub(super) fn interrupt_run(&self, agent_id: &str, by: Asker) -> bool {
        lock(&self.live).get_mut(agent_id).is_some_and(|l| l.send_interrupt(by))
    }

    /// The run's adapter ended it because its turn did not stop after an interrupt (`on_exit`).
    pub(super) async fn interrupt_gave_up(self: Arc<Self>, ended: GaveUp) {
        let GaveUp { by, run } = ended;
        if let Some(Asker::Watchdog { quiet }) = by {
            return self.stalled_run_gave_up(run, quiet).await;
        }
        // A Lead or a 1:1 agent: the adapter said what happened, and your next message resumes it.
        let Some(t) = run.task_id.as_deref().and_then(|id| self.task(id).ok()) else { return };
        match run.role {
            // As for a reviewer that exited by itself: the review ends without a verdict.
            Role::Reviewer if t.status == TaskStatus::Reviewing => self.review_done(t.id.clone(), run.run_id.clone()).await,
            Role::Worker => self.block_interrupted(&run, by, &t),
            _ => {}
        }
    }

    /// The turn was to stop, so the worker's task waits for you, as after a Stop.
    fn block_interrupted(&self, run: &watchdog::Look, by: Option<Asker>, t: &Task) {
        let name = self.agent(&run.agent_id).map_or_else(|_| "its worker".into(), |a| a.name);
        let how = interrupted(by);
        let reason = format!("{how}; the CLI did not stop, so Codly ended its process; move it to Ready to resume with {name}");
        // Checked in the write: moved, stopped or cancelled by you meanwhile, it stays as it is.
        let mut blocked = false;
        let _ = self.update_task(&t.id, |t| {
            blocked = watchdog::working(t, &run.agent_id);
            if blocked {
                t.status = TaskStatus::Blocked;
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        if !blocked {
            return;
        }
        // Its worker is free, and a reviewer that was reading it: a move to Ready starts from
        // the worker's session, with fresh fix rounds.
        self.release_task_agents(t);
        let ws = run.ws.as_deref();
        let summary = format!("{name} was {how} on {} and did not stop. Codly ended its process and blocked its task.", t.key);
        let _ = self.decide(ws, Some(&t.id), "interrupt", summary, json!({ "plan_id": t.plan_id, "agent_id": run.agent_id, "run_id": run.run_id }));
        let text = format!(
            "{name} was {how} on {} \"{}\" and did not stop, so Codly ended its process. The task is blocked until you move it to Ready.",
            t.key, t.title
        );
        let _ = self.chat(ws, &self.task_thread(t), "codly", &text, None);
        self.kick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_ended(status: TurnStatus) -> EventKind {
        EventKind::TurnCompleted { status, result_text: None, duration_ms: None }
    }

    #[test]
    fn a_turn_that_ends_in_time_answers_the_interrupt() {
        let mut i = Interrupt { by: Some(Asker::You), gave_up: false };
        i.seen(&EventKind::Error { message: "tool call failed".into(), fatal: false });
        assert_eq!(i.by, Some(Asker::You), "the turn still runs");
        i.seen(&turn_ended(TurnStatus::Interrupted));
        assert_eq!((i.by, i.gave_up), (None, false), "an exit later on is a crash, not this interrupt");
    }

    #[test]
    fn a_turn_the_adapter_gave_up_keeps_who_asked_until_the_exit() {
        let mut i = Interrupt { by: Some(Asker::Breaker), gave_up: false };
        i.seen(&adapters::gave_up("Claude", Duration::from_secs(8)));
        // The adapter closes the turn at the exit: that end is not an answer in time.
        i.seen(&turn_ended(TurnStatus::Interrupted));
        assert_eq!((i.by, i.gave_up), (Some(Asker::Breaker), true));
        // Another decision of the run is not the adapter's record.
        let mut other = Interrupt::default();
        other.seen(&EventKind::Decision { id: new_id(), decision_type: "watchdog".into(), method: "rule".into(), summary: String::new(), detail: json!({}) });
        assert!(!other.gave_up);
    }

    #[test]
    fn the_reason_names_who_interrupted() {
        assert_eq!(interrupted(Some(Asker::You)), "interrupted by you");
        assert_eq!(interrupted(Some(Asker::Breaker)), "interrupted by the circuit breaker");
        assert_eq!(interrupted(Some(Asker::ToolBlock)), "interrupted because you blocked its tools");
        assert_eq!(interrupted(None), "interrupted");
    }
}
