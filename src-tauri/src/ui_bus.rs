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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tokio::sync::mpsc;

    /// An event as the core sends it (`kind` in its JSON form).
    fn ev(id: i64, agent: &str, kind: Value) -> EventRow {
        serde_json::from_value(json!({ "id": id, "ts": 0, "agent_id": agent, "task_id": null, "run_id": null, "kind": kind })).unwrap()
    }
    fn text(agent: &str, item: &str, text: &str) -> EventRow {
        ev(0, agent, json!({ "type": "text_delta", "item_id": item, "text": text }))
    }
    fn thought(agent: &str, item: &str, text: &str) -> EventRow {
        ev(0, agent, json!({ "type": "reasoning_delta", "item_id": item, "text": text }))
    }
    fn output(call: &str, chunk: &str) -> EventRow {
        ev(0, "a1", json!({ "type": "tool_output", "call_id": call, "chunk": chunk }))
    }
    /// A stored event: the answer to an approval.
    fn answered(id: i64) -> EventRow {
        ev(id, "a1", json!({ "type": "approval_resolved", "approval_id": "ap1", "status": "approved", "by": "user" }))
    }

    /// A batch in short: streamed text by item or call, stored events by id and type.
    fn shape(events: &[EventRow]) -> Vec<String> {
        events
            .iter()
            .map(|e| match &e.kind {
                EventKind::TextDelta { item_id, text } => format!("{item_id}:{text}"),
                EventKind::ReasoningDelta { item_id, text } => format!("{item_id}?{text}"),
                EventKind::ToolOutput { call_id, chunk } => format!("{call_id}>{chunk}"),
                kind => format!("#{} {}", e.id, kind.type_name()),
            })
            .collect()
    }

    /// The next batch a browser tab gets.
    async fn next(batches: &mut mpsc::UnboundedReceiver<UiBatch>) -> UiBatch {
        tokio::time::timeout(Duration::from_secs(10), batches.recv()).await.expect("a batch within ten seconds").expect("the bus is still there")
    }

    #[test]
    fn streamed_text_is_joined_per_item_and_no_event_changes_place() {
        let bus = UiBus::new();
        let mut buf = Vec::new();
        let events = [
            text("a1", "m1", "He"),
            text("a1", "m1", "llo"),
            output("c1", "$ ls\n"),
            output("c1", "src\n"),
            answered(7),
            text("a1", "m1", "!"), // after a stored event: not joined across it
            text("a2", "m1", "another agent's item"),
            text("a2", "m2", "x"),
            thought("a2", "m2", "h"),
            thought("a2", "m2", "m"),
            output("c2", "y"),
        ];
        for e in events {
            bus.push(&mut buf, e);
        }
        assert_eq!(shape(&buf), ["m1:Hello", "c1>$ ls\nsrc\n", "#7 approval_resolved", "m1:!", "m1:another agent's item", "m2:x", "m2?hm", "c2>y"]);
    }

    #[test]
    fn a_hidden_window_gets_no_streamed_text_but_every_stored_event() {
        let bus = UiBus::new();
        bus.visible.store(false, Ordering::Relaxed);
        let mut buf = Vec::new();
        for e in [text("a1", "m1", "He"), output("c1", "src\n"), answered(7), thought("a1", "m1", "hm")] {
            bus.push(&mut buf, e);
        }
        assert_eq!(shape(&buf), ["#7 approval_resolved"]);
        // A browser tab on `codly serve` is watching, whatever the window does.
        bus.add_web(Box::new(|_| true));
        bus.push(&mut buf, text("a1", "m1", "llo"));
        assert_eq!(shape(&buf), ["#7 approval_resolved", "m1:llo"]);
    }

    #[tokio::test]
    async fn a_burst_is_one_batch_and_lost_events_ask_for_a_fresh_snapshot() {
        let bus = UiBus::new();
        let (tx, mut batches) = mpsc::unbounded_channel();
        bus.add_web(Box::new(move |b| tx.send(b.clone()).is_ok()));
        let (events, rx) = broadcast::channel(4);
        tokio::spawn(bus.clone().pump(rx));

        for e in [text("a1", "m1", "He"), text("a1", "m1", "llo"), answered(7)] {
            events.send(e).unwrap();
        }
        let batch = next(&mut batches).await;
        assert_eq!(shape(&batch.events), ["m1:Hello", "#7 approval_resolved"]);
        assert!(!batch.resync);

        // More events than the channel holds before the pump runs again: the first two are
        // lost, and the batch tells the UI to take a fresh snapshot.
        for id in 1..=6 {
            events.send(answered(id)).unwrap();
        }
        let batch = next(&mut batches).await;
        assert!(batch.resync);
        assert_eq!(shape(&batch.events), ["#3 approval_resolved", "#4 approval_resolved", "#5 approval_resolved", "#6 approval_resolved"]);

        // A tab that went away is forgotten with the next batch.
        drop(batches);
        events.send(answered(9)).unwrap();
        let forgotten = async {
            while !bus.web.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), forgotten).await.expect("the closed tab is dropped");
    }
}
