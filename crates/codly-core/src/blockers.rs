//! "Ask me" blockers: a decision an agent needs from you, raised with `codly_ask_user`.
//!
//! A blocker is a card: a plain-language explanation, whether it is a heads-up (no action
//! needed today) or blocks the agent's task, and your reply. It is event-sourced: every
//! change is a `BlockerChanged` event and the current state is the latest event per id, so
//! there is no table to migrate and the log stays the source of truth.
//!
//! This file is the pure part (types, folding the log, the reminder rule). The orchestrator
//! side (raising, answering, resuming the waiting task) is `orchestrator/ask_me.rs`.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{EventKind, Id};

/// An open question that has waited this long is flagged for a reminder.
pub const REMIND_AFTER_DAYS: i64 = 2;
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BlockerSeverity {
    /// The agent's task waits for your answer.
    Blocking,
    /// FYI, no action needed today; the agent carries on.
    HeadsUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BlockerStatus {
    Open,
    Answered,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Blocker {
    pub id: Id,
    pub workspace_id: Option<Id>,
    /// The agent that asks (a worker or a Lead).
    pub agent_id: Id,
    /// The task that waits (blocking blockers of a worker).
    pub task_id: Option<Id>,
    /// The request thread the answer goes to when the asker is a Lead ("lead", "lead:<id>").
    pub thread: String,
    /// One line.
    pub title: String,
    /// The explanation in plain language: what is needed, why, what each answer changes.
    pub body: String,
    pub severity: BlockerSeverity,
    pub status: BlockerStatus,
    pub answer: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub answered_at: Option<i64>,
    /// Set when it has waited `REMIND_AFTER_DAYS` (again every period after that).
    pub reminded_at: Option<i64>,
}

/// Has this open blocker waited long enough for a (new) reminder?
pub fn needs_reminder(b: &Blocker, now: i64) -> bool {
    b.status == BlockerStatus::Open && now - b.reminded_at.unwrap_or(b.created_at) >= REMIND_AFTER_DAYS * DAY_MS
}

/// The current state of every blocker of a workspace (None = all), oldest first: the latest
/// `BlockerChanged` event per id.
pub fn list(db: &Connection, workspace_id: Option<&str>) -> Result<Vec<Blocker>> {
    let mut stmt =
        db.prepare_cached("SELECT payload FROM event WHERE type = 'blocker_changed' AND (?1 IS NULL OR workspace_id = ?1) ORDER BY id")?;
    let rows = stmt.query_map([workspace_id], |r| r.get::<_, String>(0))?;
    let mut out: Vec<Blocker> = vec![];
    for payload in rows {
        let Ok(EventKind::BlockerChanged { blocker }) = serde_json::from_str::<EventKind>(&payload?) else { continue };
        match out.iter_mut().find(|b| b.id == blocker.id) {
            Some(slot) => *slot = blocker,
            None => out.push(blocker),
        }
    }
    out.sort_by_key(|b| b.created_at);
    Ok(out)
}

/// One blocker by id.
pub fn get(db: &Connection, id: &str) -> Result<Option<Blocker>> {
    Ok(list(db, None)?.into_iter().find(|b| b.id == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventRow, new_id, now_ms};
    use crate::store::Store;

    pub(crate) fn blocker(ws: &str, title: &str, severity: BlockerSeverity) -> Blocker {
        let now = now_ms();
        Blocker {
            id: new_id(),
            workspace_id: Some(ws.into()),
            agent_id: "agent".into(),
            task_id: None,
            thread: "lead".into(),
            title: title.into(),
            body: "Which database should the app use?".into(),
            severity,
            status: BlockerStatus::Open,
            answer: None,
            created_at: now,
            updated_at: now,
            answered_at: None,
            reminded_at: None,
        }
    }

    fn put(store: &Store, ws: &str, b: &Blocker) {
        let ev = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind: EventKind::BlockerChanged { blocker: b.clone() } };
        store.append_event(Some(ws), &ev).unwrap();
    }

    #[test]
    fn the_latest_event_per_blocker_wins_and_workspaces_are_apart() {
        let store = Store::open_in_memory().unwrap();
        let mut a = blocker("w1", "Which database?", BlockerSeverity::Blocking);
        a.created_at -= 10;
        let b = blocker("w1", "FYI: tests are slow", BlockerSeverity::HeadsUp);
        let other = blocker("w2", "Elsewhere", BlockerSeverity::Blocking);
        put(&store, "w1", &a);
        put(&store, "w1", &b);
        put(&store, "w2", &other);
        let mut answered = a.clone();
        answered.status = BlockerStatus::Answered;
        answered.answer = Some("sqlite".into());
        put(&store, "w1", &answered);

        let w1 = store.with(|db| list(db, Some("w1"))).unwrap();
        assert_eq!(w1.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Which database?", "FYI: tests are slow"]);
        assert_eq!(w1[0].status, BlockerStatus::Answered);
        assert_eq!(w1[0].answer.as_deref(), Some("sqlite"));
        assert_eq!(store.with(|db| list(db, None)).unwrap().len(), 3);
        assert_eq!(store.with(|db| get(db, &other.id)).unwrap().unwrap().title, "Elsewhere");
        assert!(store.with(|db| get(db, "nope")).unwrap().is_none());
    }

    #[test]
    fn only_open_blockers_that_waited_get_a_reminder_and_again_each_period() {
        let day = DAY_MS;
        let mut b = blocker("w", "q", BlockerSeverity::Blocking);
        let t0 = b.created_at;
        assert!(!needs_reminder(&b, t0 + day));
        assert!(needs_reminder(&b, t0 + REMIND_AFTER_DAYS * day));
        b.reminded_at = Some(t0 + REMIND_AFTER_DAYS * day);
        assert!(!needs_reminder(&b, t0 + REMIND_AFTER_DAYS * day + day));
        assert!(needs_reminder(&b, t0 + 2 * REMIND_AFTER_DAYS * day));
        b.status = BlockerStatus::Answered;
        assert!(!needs_reminder(&b, t0 + 99 * day));
    }
}
