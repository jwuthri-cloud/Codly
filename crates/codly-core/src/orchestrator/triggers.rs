//! Triggers at run time: the config in `kv`, the 30-second loop that fires due schedules at
//! the Lead and applies the context rules, and the `Core` methods the shell calls. The
//! decisions are pure and live in `crate::triggers`; this file only reads the world, asks
//! them, and acts.
//!
//! - A schedule posts its prompt in the first request's thread (author "codly") and hands
//!   the same text to that Lead as a turn. With no repo open it waits (it is not "missed").
//! - Compact asks a live, idle Claude agent to `/compact` (the other CLIs have no way to be
//!   asked through Codly yet: logged, not retried). Clear stops an idle agent and forgets its
//!   session, so its next message starts a fresh conversation. Neither touches an agent that
//!   is mid-turn or holds a task: it is retried on the next pass.
//! - An agent seen for the first time only starts its clock; rules never fire on startup.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{Core, Inner};
use crate::model::*;
use crate::triggers::*;

/// `kv` key of the whole `TriggersConfig` JSON.
const KEY: &str = "triggers";
const PASS: Duration = Duration::from_secs(30);

/// Serializes read-modify-write of the config (the loop and the UI both write it).
static EDIT: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Inner {
    fn triggers(&self) -> TriggersConfig {
        match self.store.kv_get(KEY) {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("triggers unreadable, using defaults: {e}");
                TriggersConfig::default()
            }),
            _ => TriggersConfig::default(),
        }
    }

    /// Change the stored config under the lock; the closure sees the latest.
    fn edit_triggers<T>(&self, f: impl FnOnce(&mut TriggersConfig) -> T) -> anyhow::Result<T> {
        let _g = EDIT.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = self.triggers();
        let out = f(&mut cfg);
        self.store.kv_set(KEY, &serde_json::to_string(&cfg)?)?;
        Ok(out)
    }

    /// One pass: due schedules, then the context rules. Never holds a lock across an await.
    async fn trigger_pass(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(ws) = self.workspace() else { return Ok(()) };
        let now = now_ms();
        let cfg = self.triggers();

        for id in due_schedules(&cfg.schedules, now) {
            let Some(s) = cfg.schedules.iter().find(|s| s.id == id) else { continue };
            let sent = self.fire_schedule(&ws, s).await;
            let entry = TriggerLog {
                ts: now,
                kind: "schedule".into(),
                label: s.name.clone(),
                detail: match &sent {
                    Ok(()) => "sent to the Lead".into(),
                    Err(e) => format!("could not reach the Lead: {e}"),
                },
                ok: sent.is_ok(),
            };
            // Fired or failed, the clock restarts: a broken Lead must not be retried every pass.
            self.edit_triggers(|c| {
                if let Some(s) = c.schedules.iter_mut().find(|s| s.id == id) {
                    s.last_fired_at = Some(now);
                }
                c.state.push_log(entry);
            })?;
        }

        if cfg.context.compact.enabled || cfg.context.clear.enabled {
            for agent in self.store.agents(&ws.id)? {
                self.context_pass(&cfg.context, &agent, now).await?;
            }
        }
        Ok(())
    }

    async fn fire_schedule(self: &Arc<Self>, ws: &Workspace, s: &Schedule) -> anyhow::Result<()> {
        let thread = "lead";
        let text = format!("Scheduled “{}”: {}", s.name, s.prompt);
        self.chat(Some(&ws.id), thread, "codly", &text, None)?;
        self.send_to_lead(ws, thread, text, None).await
    }

    async fn context_pass(self: &Arc<Self>, rules: &ContextRules, agent: &Agent, now: i64) -> anyhow::Result<()> {
        let stamp = self.triggers().state.context.get(&agent.id).cloned();
        let Some(stamp) = stamp else {
            // First sight: start both clocks now.
            return self.edit_triggers(|c| {
                c.state.context.insert(agent.id.clone(), ContextStamp { compact_at: now, clear_at: now });
            });
        };
        let percent = context_percent(agent.usage.context_used, agent.usage.context_window);
        let Some(action) = decide_context(rules, &stamp, percent, now) else { return Ok(()) };
        // Busy now, or nothing to compact = try again next pass (no stamp, no log).
        let live = self.is_live(&agent.id);
        let busy = live && !self.is_quiet(&agent.id);
        let holds_task = agent.task_id.is_some() && action == ContextAction::Clear;
        if busy || holds_task || (!live && action == ContextAction::Compact) {
            return Ok(());
        }
        let result = match action {
            ContextAction::Compact => self.compact_agent(agent, &rules.compact_focus),
            ContextAction::Clear => self.clear_agent(agent).await,
        };
        let (kind, note) = match action {
            ContextAction::Compact => ("compact", "asked it to compact"),
            ContextAction::Clear => ("clear", "started a fresh conversation"),
        };
        let pct = percent.map(|p| format!(" at {p}%")).unwrap_or_default();
        let entry = TriggerLog {
            ts: now,
            kind: kind.into(),
            label: agent.name.clone(),
            detail: match &result {
                Ok(()) => format!("{note}{pct}"),
                Err(e) => format!("not done{pct}: {e}"),
            },
            ok: result.is_ok(),
        };
        self.edit_triggers(|c| {
            let st = c.state.context.entry(agent.id.clone()).or_default();
            match action {
                ContextAction::Compact => st.compact_at = now,
                // A cleared conversation is empty: both clocks restart.
                ContextAction::Clear => *st = ContextStamp { compact_at: now, clear_at: now },
            }
            c.state.push_log(entry);
        })
    }

    /// `/compact [focus]` into a live Claude conversation.
    fn compact_agent(&self, agent: &Agent, focus: &str) -> anyhow::Result<()> {
        if agent.provider != Provider::Claude {
            anyhow::bail!("{} cannot be asked to compact through Codly yet", agent.provider.label());
        }
        if !self.is_live(&agent.id) {
            anyhow::bail!("it is not running");
        }
        let focus = focus.trim();
        self.send_prompt(&agent.id, if focus.is_empty() { "/compact".into() } else { format!("/compact {focus}") })
    }

    /// Forget the conversation: stop the process and drop the session id.
    async fn clear_agent(self: &Arc<Self>, agent: &Agent) -> anyhow::Result<()> {
        if !self.stop_and_wait(&agent.id).await {
            anyhow::bail!("it did not stop in time");
        }
        self.update_agent(&agent.id, |a| {
            a.session_id = None;
            a.status = AgentStatus::Offline;
            a.activity = None;
            a.activity_label = None;
            a.usage.context_used = None;
        })?;
        Ok(())
    }
}

/// The loop: one pass every 30 s. A pass runs as its own task so a panic fails that pass.
pub(super) async fn run(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(PASS).await;
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let me = inner.clone();
        match tokio::spawn(async move { me.trigger_pass().await }).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("trigger pass failed: {e}"),
            Err(e) => tracing::error!("trigger pass panicked: {e}"),
        }
    }
}

impl Core {
    /// The triggers config: schedules (with last/next run), context rules, network mode, log.
    pub async fn triggers(&self) -> anyhow::Result<TriggersConfig> {
        Ok(self.inner.triggers())
    }

    /// Save the config from the UI. Codly-owned fields (log, clocks) are kept; returns what was stored.
    pub async fn set_triggers(&self, cfg: TriggersConfig) -> anyhow::Result<TriggersConfig> {
        self.inner.edit_triggers(|old| {
            *old = normalize(cfg, old, now_ms());
            old.clone()
        })
    }

    /// May `peer` do `action` on this machine right now? The one gate every inbound path asks.
    pub async fn network_gate(&self, action: NetworkAction, peer: &str) -> Gate {
        network_gate(&self.inner.triggers().network, action, peer)
    }
}
