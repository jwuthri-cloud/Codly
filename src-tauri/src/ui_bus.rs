//! The UI bus: one ordered Tauri Channel per webview carrying coalesced batches of
//! `EventRow`s at ≈30 Hz. The pump parks on the broadcast receiver (zero wakeups when
//! idle), arms a 33 ms deadline on the first event, merges consecutive text deltas of
//! the same item, and drops ephemeral deltas while the window is hidden.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::model::{EventKind, EventRow};
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::{Instant, sleep_until};

#[derive(Clone, Serialize)]
pub struct UiBatch {
    pub events: Vec<EventRow>,
    /// Events were skipped: the UI must take a fresh snapshot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resync: bool,
}

/// A browser tab on `codly serve` (see web.rs); false once it is gone.
pub type WebSink = Box<dyn Fn(&UiBatch) -> bool + Send + Sync>;

#[derive(Default)]
pub struct UiBus {
    sink: Mutex<Option<Channel<UiBatch>>>,
    web: Mutex<Vec<WebSink>>,
    pub visible: AtomicBool,
}

const WINDOW: Duration = Duration::from_millis(33);
const MAX_BATCH: usize = 4096;

impl UiBus {
    pub fn new() -> Arc<Self> {
        Arc::new(UiBus { sink: Mutex::new(None), web: Mutex::new(vec![]), visible: AtomicBool::new(true) })
    }

    pub fn set_sink(&self, ch: Channel<UiBatch>) {
        *self.sink.lock().unwrap() = Some(ch);
    }

    pub fn add_web(&self, sink: WebSink) {
        self.web.lock().unwrap().push(sink);
    }

    fn send(&self, batch: UiBatch) {
        self.web.lock().unwrap().retain(|s| s(&batch)); // web sinks only queue, never block
        let ch = self.sink.lock().unwrap().clone(); // never hold the lock across send
        if let Some(ch) = ch {
            if ch.send(batch).is_err() {
                *self.sink.lock().unwrap() = None;
            }
        }
    }

    /// Runs for the app's lifetime.
    pub async fn pump(self: Arc<Self>, mut rx: broadcast::Receiver<EventRow>) {
        let mut buf: Vec<EventRow> = Vec::with_capacity(512);
        let mut lagged = false;
        loop {
            match rx.recv().await {
                Ok(ev) => self.push(&mut buf, ev),
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("ui bus lagged by {n} events");
                    lagged = true;
                }
                Err(RecvError::Closed) => return,
            }
            let deadline = sleep_until(Instant::now() + WINDOW);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    biased;
                    _ = &mut deadline => break,
                    ev = rx.recv() => match ev {
                        Ok(ev) => { self.push(&mut buf, ev); if buf.len() >= MAX_BATCH { break } }
                        Err(RecvError::Lagged(n)) => { tracing::warn!("ui bus lagged by {n} events"); lagged = true }
                        Err(RecvError::Closed) => break,
                    },
                }
            }
            if !buf.is_empty() || lagged {
                self.send(UiBatch { events: std::mem::take(&mut buf), resync: std::mem::take(&mut lagged) });
            }
        }
    }

    fn push(&self, buf: &mut Vec<EventRow>, ev: EventRow) {
        let ephemeral = ev.kind.is_ephemeral();
        // A browser tab counts as watching: it cannot report the hidden window's state.
        if ephemeral && !self.visible.load(Ordering::Relaxed) && self.web.lock().unwrap().is_empty() {
            return;
        }
        if let Some(last) = buf.last_mut() {
            if let (
                EventKind::TextDelta { item_id: a, text: t0 },
                EventKind::TextDelta { item_id: b, text: t1 },
            )
            | (
                EventKind::ReasoningDelta { item_id: a, text: t0 },
                EventKind::ReasoningDelta { item_id: b, text: t1 },
            ) = (&mut last.kind, &ev.kind)
            {
                if a == b && last.agent_id == ev.agent_id {
                    t0.push_str(t1);
                    return;
                }
            }
            if let (EventKind::ToolOutput { call_id: a, chunk: c0 }, EventKind::ToolOutput { call_id: b, chunk: c1 }) =
                (&mut last.kind, &ev.kind)
            {
                if a == b {
                    c0.push_str(c1);
                    return;
                }
            }
        }
        buf.push(ev);
    }
}
