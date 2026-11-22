//! Enforcement for the circuit breaker (policy: `breaker`). Events feed the tracker from the
//! ingest pipeline; a beat every `BEAT_MS` asks the policy what to do and does it with the
//! mechanisms that already exist: a prompt (rides the agent's next turn; Codex steers it into
//! the running turn), an interrupt, `stop_agent`. Every level change is a `Decision` on the
//! agent's timeline. The Lead is exempt: it only talks to you.
//!
//! A task's worker that is constrained is also paused ("stop after this step"), so the turn in
//! which it answers the note never reaches the gate as finished work; Resume sends it back to
//! the task. A stop says on the task that the breaker did it, and why.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::breaker::{self, Action, BEAT_MS, BreakerConfig, BreakerLevel, Tracker, Transition, Trip};
use super::{Core, Inner, lock, row, scheduler};
use crate::adapters::AgentCmd;
use crate::model::*;

const KV_KEY: &str = "breaker";

/// The settings and the signals, behind one lock.
#[derive(Default)]
pub(super) struct Runtime {
    cfg: BreakerConfig,
    tracker: Tracker,
}

impl Runtime {
    /// The saved settings (defaults when none, or when the saved JSON is from another version).
    pub(super) fn load(store: &crate::store::Store) -> Runtime {
        let cfg = store.kv_get(KV_KEY).ok().flatten().and_then(|s| serde_json::from_str::<BreakerConfig>(&s).ok()).unwrap_or_default();
        Runtime { cfg: cfg.normalized(), tracker: Tracker::default() }
    }
}

/// What Settings shows next to the limits.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BreakerStatus {
    pub floor_tokens: i64,
    pub agents: Vec<AgentBreaker>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentBreaker {
    pub agent_id: Id,
    pub level: BreakerLevel,
    pub work_tokens: i64,
}

pub(super) async fn beat_loop(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(Duration::from_millis(BEAT_MS as u64)).await;
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        inner.breaker_beat().await;
    }
}

impl Inner {
    /// Called by the ingest pipeline for every persisted event.
    pub(super) fn breaker_observe(&self, agent_id: &str, kind: &EventKind) {
        lock(&self.breaker).tracker.observe(agent_id, now_ms(), kind);
    }

    /// One look at the floor: decide, enforce, record.
    pub(super) async fn breaker_beat(&self) {
        let leads: HashSet<Id> = lock(&self.live).iter().filter(|(_, l)| l.role == Role::Lead).map(|(id, _)| id.clone()).collect();
        let transitions = {
            let mut rt = lock(&self.breaker);
            let cfg = rt.cfg.clone();
            rt.tracker.beat(&cfg, now_ms(), &|id| leads.contains(id))
        };
        for t in transitions {
            self.breaker_enforce(t).await;
        }
    }

    async fn breaker_enforce(&self, t: Transition) {
        let Ok(agent) = self.agent(&t.agent) else { return };
        let ws = self.store.agent_workspace(&t.agent).ok().flatten();
        // The task it holds (as `stop_agent` reads it), and whether it is that task's worker.
        let held = agent.task_id.clone().or_else(|| lock(&self.live).get(&t.agent).and_then(|l| l.task_id.clone()));
        let task = held.and_then(|id| self.task(&id).ok());
        let work = task.as_ref().filter(|x| scheduler::is_active(x.status) && x.assignee.as_deref() == Some(t.agent.as_str()));
        // Send first, record after: a note that could not be delivered is said so in the event.
        let delivered = match (t.action, &t.trip) {
            (Action::Steer, Some(trip)) => self.send_prompt(&t.agent, breaker::note(BreakerLevel::Steering, trip)).is_ok(),
            (Action::Constrain, Some(trip)) => {
                let note = breaker::note(BreakerLevel::Constrained, trip);
                match work {
                    // A task's worker is paused too (one you paused already just gets the note, held).
                    Some(task) if !self.control_holding(&t.agent) => self.breaker_pause(ws.as_deref(), &agent, task, trip, note),
                    _ => {
                        let interrupted = lock(&self.live).get(&t.agent).is_some_and(|l| l.handle.send(AgentCmd::Interrupt));
                        interrupted && self.send_prompt(&t.agent, note).is_ok()
                    }
                }
            }
            (Action::Stop, trip) => {
                let why = trip.as_ref().map(|x| format!(": {}", x.reason())).unwrap_or_default();
                let stopped = self.stop_agent_as(&t.agent, &format!("the circuit breaker{why}")).await.is_ok();
                // Its task is blocked now: say so where you follow that task.
                if let Some(task) = task.as_ref().filter(|x| self.task(&x.id).is_ok_and(|now| now.status == TaskStatus::Blocked)) {
                    let text =
                        format!("The circuit breaker stopped {} on {} \"{}\"{why}. The task is blocked until you retry it.", agent.name, task.key, task.title);
                    let _ = self.chat(ws.as_deref(), &self.task_thread(task), "codly", &text, None);
                }
                stopped
            }
            _ => true,
        };
        let name = &agent.name;
        let (verb, why) = (
            match t.action {
                Action::Steer => "steered",
                Action::Constrain => "constrained",
                Action::Stop => "stopped",
                Action::None => "recovered",
            },
            t.trip.as_ref().map(|x| x.reason()),
        );
        let summary = match (&why, t.action) {
            (Some(w), _) => format!("Circuit breaker {verb} {name}: {w}"),
            (None, _) => format!("Circuit breaker: {name} is {} again", t.to.label()),
        };
        let detail = json!({
            "level": t.to,
            "from": t.from,
            "action": verb,
            "trip": t.trip.as_ref().map(|x| x.kind()),
            "reason": why,
            "delivered": delivered,
            "hard_stop": lock(&self.breaker).cfg.hard_stop,
        });
        let kind = EventKind::Decision { id: new_id(), decision_type: "breaker".into(), method: "rule".into(), summary, detail };
        if let Err(e) = self.emit(ws.as_deref(), row(Some(&t.agent), agent.task_id.as_deref(), agent.run_id.as_deref(), kind)) {
            tracing::warn!("breaker event not recorded: {e}");
        }
    }

    /// Constrain a task's worker. The turn that answers the note is a summary, not finished
    /// work: the worker is paused before its turn is interrupted, so no turn that ends from
    /// here on reaches the gate, and the note goes past the pause. Your Resume sends the
    /// worker back to its task. True = the note was delivered.
    fn breaker_pause(&self, ws: Option<&str>, agent: &Agent, task: &Task, trip: &Trip, note: String) -> bool {
        let interrupted = {
            // Under the `live` lock: an exit (which clears the controls) is before or after, not between.
            let live = lock(&self.live);
            let Some(l) = live.get(&agent.id) else { return false };
            // The note is its next turn: the pause holds from now and shows when a turn ends.
            lock(&self.controls).entry(agent.id.clone()).or_default().request_stop(true);
            l.handle.send(AgentCmd::Interrupt)
        };
        if !interrupted {
            return false; // its process is gone: the exit clears the pause
        }
        let delivered = self.deliver_prompt(&agent.id, note).is_ok();
        // Held until the resume (holding it also tells the UI about the pause).
        if let Err(e) = self.send_prompt(&agent.id, breaker::RESUME_NOTE.into()) {
            tracing::debug!("breaker resume note not held: {e}");
        }
        let why = trip.reason();
        let reason = format!("paused by the circuit breaker: {why}; resume {} to continue", agent.name);
        let _ = self.update_task(&task.id, |t| {
            if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        let text = format!(
            "The circuit breaker paused {} on {} \"{}\": {why}. Its work is not checked while it is paused. Resume {} to let it continue, or stop it.",
            agent.name, task.key, task.title, agent.name
        );
        let _ = self.chat(ws, &self.task_thread(task), "codly", &text, None);
        delivered
    }
}

impl Core {
    pub async fn breaker_config(&self) -> anyhow::Result<BreakerConfig> {
        Ok(lock(&self.inner.breaker).cfg.clone())
    }

    pub async fn set_breaker_config(&self, cfg: BreakerConfig) -> anyhow::Result<BreakerConfig> {
        let cfg = cfg.normalized();
        self.inner.store.kv_set(KV_KEY, &serde_json::to_string(&cfg)?)?;
        lock(&self.inner.breaker).cfg = cfg.clone();
        Ok(cfg)
    }

    /// Start the floor's token count and every level over (running agents keep running).
    pub async fn reset_breaker(&self) -> anyhow::Result<()> {
        lock(&self.inner.breaker).tracker.reset();
        Ok(())
    }

    /// Look at the floor now, as the beat loop does every `BEAT_MS` (the tests do not wait for it).
    pub async fn breaker_beat(&self) {
        self.inner.breaker_beat().await;
    }

    pub async fn breaker_status(&self) -> anyhow::Result<BreakerStatus> {
        let rt = lock(&self.inner.breaker);
        let agents = rt.tracker.snapshot().into_iter().map(|(agent_id, level, work_tokens)| AgentBreaker { agent_id, level, work_tokens }).collect();
        Ok(BreakerStatus { floor_tokens: rt.tracker.floor_tokens(), agents })
    }
}
