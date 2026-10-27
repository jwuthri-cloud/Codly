//! Enforcement for the circuit breaker (policy: `breaker`). Events feed the tracker from the
//! ingest pipeline; a beat every `BEAT_MS` asks the policy what to do and does it with the
//! mechanisms that already exist: a prompt (rides the agent's next turn; Codex steers it into
//! the running turn), an interrupt, `stop_agent`. Every level change is a `Decision` on the
//! agent's timeline. The Lead is exempt: it only talks to you.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::breaker::{self, Action, BEAT_MS, BreakerConfig, BreakerLevel, Tracker, Transition};
use super::{Core, Inner, lock, row};
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
        let ws = self.workspace().map(|w| w.id);
        // Send first, record after: a note that could not be delivered is said so in the event.
        let delivered = match (t.action, &t.trip) {
            (Action::Steer, Some(trip)) => self.send_prompt(&t.agent, breaker::note(BreakerLevel::Steering, trip)).is_ok(),
            (Action::Constrain, Some(trip)) => {
                let interrupted = lock(&self.live).get(&t.agent).is_some_and(|l| l.handle.send(AgentCmd::Interrupt));
                interrupted && self.send_prompt(&t.agent, breaker::note(BreakerLevel::Constrained, trip)).is_ok()
            }
            (Action::Stop, _) => self.stop_agent(&t.agent).await.is_ok(),
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

    pub async fn breaker_status(&self) -> anyhow::Result<BreakerStatus> {
        let rt = lock(&self.inner.breaker);
        let agents = rt.tracker.snapshot().into_iter().map(|(agent_id, level, work_tokens)| AgentBreaker { agent_id, level, work_tokens }).collect();
        Ok(BreakerStatus { floor_tokens: rt.tracker.floor_tokens(), agents })
    }
}
