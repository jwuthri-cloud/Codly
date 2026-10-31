//! Triggers at run time: the config in `kv`, the 30-second loop that fires due schedules at
//! the Lead, and the `Core` methods the shell calls. The
//! decisions are pure and live in `crate::triggers`; this file only reads the world, asks
//! them, and acts.
//!
//! - A schedule posts its prompt in the first request's thread (author "codly") and hands
//!   the same text to that Lead as a turn. With no repo open it waits (it is not "missed").

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

    /// One pass: due schedules. Never holds a lock across an await.
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

        Ok(())
    }

    async fn fire_schedule(self: &Arc<Self>, ws: &Workspace, s: &Schedule) -> anyhow::Result<()> {
        let thread = "lead";
        let text = format!("Scheduled “{}”: {}", s.name, s.prompt);
        self.chat(Some(&ws.id), thread, "codly", &text, None)?;
        self.send_to_lead(ws, thread, text, None).await
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
    /// The triggers config: schedules (with last/next run), network mode, log.
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
