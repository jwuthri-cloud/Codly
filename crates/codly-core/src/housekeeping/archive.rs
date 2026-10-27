//! Board archiving: what leaves the board, what gets flagged, and where archived text lives.
//!
//! Rules (days since the item was last touched; a threshold of 0 switches that rule off):
//! - a finished task (done or cancelled) is archived after `done_days` (default 2);
//! - an untouched task is FLAGGED after `stale_flag_days` (14) and archived after
//!   `stale_archive_days` (21); a task a worker is on right now is never judged;
//! - an open question that waited `question_flag_days` (7) is FLAGGED and never archived —
//!   somebody has to answer it.
//!
//! `judge` is pure and unit-tested. Archiving never deletes anything from the database (the
//! plan graph and the event log still point at the task); the full text is copied to an
//! append-only JSONL file so it stays searchable under the board's "Archived" filter.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Task, TaskStatus};

pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ArchivePolicy {
    /// Archive a finished task after this many days (0 = never).
    pub done_days: i64,
    /// Flag an untouched task after this many days (0 = never).
    pub stale_flag_days: i64,
    /// Archive an untouched task after this many days (0 = never; never sooner than the flag).
    pub stale_archive_days: i64,
    /// Flag an unanswered question after this many days (0 = never). Questions are never archived.
    pub question_flag_days: i64,
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        ArchivePolicy { done_days: 2, stale_flag_days: 14, stale_archive_days: 21, question_flag_days: 7 }
    }
}

impl ArchivePolicy {
    /// Clamp to sane values: no negatives, and an item is flagged before it is archived.
    pub fn normalized(mut self) -> Self {
        self.done_days = self.done_days.clamp(0, 3650);
        self.stale_flag_days = self.stale_flag_days.clamp(0, 3650);
        self.stale_archive_days = self.stale_archive_days.clamp(0, 3650);
        self.question_flag_days = self.question_flag_days.clamp(0, 3650);
        if self.stale_archive_days != 0 && self.stale_flag_days > self.stale_archive_days {
            self.stale_archive_days = self.stale_flag_days;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemKind {
    Task,
    Question,
}

/// What the policy needs to know about one board item.
#[derive(Debug, Clone)]
pub struct BoardItem {
    pub id: String,
    pub kind: ItemKind,
    /// Finished for good (done or cancelled).
    pub finished: bool,
    /// A worker is on it right now (running, verifying, reviewing, merging).
    pub live: bool,
    /// Last time anything happened to it (unix ms).
    pub touched_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FlagReason {
    /// Nobody touched it for a long time; it will be archived soon.
    Stale,
    /// An agent asked you something and it has waited too long.
    QuestionWaiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ArchiveReason {
    Done,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Flag(FlagReason),
    Archive(ArchiveReason),
}

/// Whole days between `touched_at` and `now` (never negative).
pub fn idle_days(touched_at: i64, now: i64) -> i64 {
    (now - touched_at).max(0) / DAY_MS
}

fn reached(days: i64, threshold: i64) -> bool {
    threshold > 0 && days >= threshold
}

/// The one decision. Pure: the same item, clock and policy always give the same verdict.
pub fn judge(item: &BoardItem, now: i64, policy: &ArchivePolicy) -> Verdict {
    let days = idle_days(item.touched_at, now);
    match item.kind {
        ItemKind::Question => {
            if reached(days, policy.question_flag_days) { Verdict::Flag(FlagReason::QuestionWaiting) } else { Verdict::Keep }
        }
        ItemKind::Task if item.live => Verdict::Keep,
        ItemKind::Task if item.finished => {
            if reached(days, policy.done_days) { Verdict::Archive(ArchiveReason::Done) } else { Verdict::Keep }
        }
        ItemKind::Task => {
            if reached(days, policy.stale_archive_days) {
                Verdict::Archive(ArchiveReason::Stale)
            } else if reached(days, policy.stale_flag_days) {
                Verdict::Flag(FlagReason::Stale)
            } else {
                Verdict::Keep
            }
        }
    }
}

/// The board's view of a task for `judge`.
pub fn task_item(t: &Task) -> BoardItem {
    BoardItem {
        id: t.id.clone(),
        kind: ItemKind::Task,
        finished: matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled),
        live: matches!(t.status, TaskStatus::Running | TaskStatus::Verifying | TaskStatus::Reviewing | TaskStatus::Merging),
        touched_at: t.updated_at,
    }
}

/// One thing the board wants your attention on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BoardFlag {
    pub id: String,
    pub kind: ItemKind,
    pub reason: FlagReason,
    pub days: i64,
    pub title: String,
}

/// An archived item: the board card's text, frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ArchivedItem {
    pub id: String,
    pub kind: ItemKind,
    /// "t3" for tasks.
    pub key: String,
    pub title: String,
    /// Goal, acceptance, summary and status note, joined: what the search looks through.
    pub text: String,
    /// Status when it left the board.
    pub status: String,
    pub reason: ArchiveReason,
    pub plan_id: Option<String>,
    pub touched_at: i64,
    pub archived_at: i64,
}

/// The archived form of a task.
pub fn archive_task(t: &Task, reason: ArchiveReason, now: i64) -> ArchivedItem {
    let mut parts: Vec<String> = vec![];
    if !t.goal.trim().is_empty() {
        parts.push(t.goal.trim().to_string());
    }
    parts.extend(t.acceptance.iter().map(|a| format!("- {}", a.trim())));
    if let Some(s) = t.summary.as_deref().filter(|s| !s.trim().is_empty()) {
        parts.push(format!("Result: {}", s.trim()));
    }
    if let Some(s) = t.status_reason.as_deref().filter(|s| !s.trim().is_empty()) {
        parts.push(format!("Note: {}", s.trim()));
    }
    ArchivedItem {
        id: t.id.clone(),
        kind: ItemKind::Task,
        key: t.key.clone(),
        title: t.title.clone(),
        text: parts.join("\n"),
        status: crate::store::enum_str(&t.status),
        reason,
        plan_id: t.plan_id.clone(),
        touched_at: t.updated_at,
        archived_at: now,
    }
}

/// Case-insensitive match: every word of `query` appears in the key, title or text.
pub fn matches(item: &ArchivedItem, query: &str) -> bool {
    let hay = format!("{} {} {}", item.key, item.title, item.text).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// The archive file of a workspace: `<data>/archive/<workspace>.jsonl`, one item per line.
pub struct ArchiveFile {
    path: PathBuf,
}

impl ArchiveFile {
    pub fn new(path: PathBuf) -> Self {
        ArchiveFile { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every archived item, newest first. Unreadable lines are skipped, never fatal.
    pub fn load(&self) -> Vec<ArchivedItem> {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return vec![] };
        let mut items: Vec<ArchivedItem> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        items.sort_by(|a, b| b.archived_at.cmp(&a.archived_at));
        items
    }

    /// Items matching `query` (all of them for an empty one), newest first.
    pub fn search(&self, query: &str) -> Vec<ArchivedItem> {
        let q = query.trim();
        self.load().into_iter().filter(|i| q.is_empty() || matches(i, q)).collect()
    }

    /// Append the items not archived yet (by id); returns how many were new.
    pub fn append(&self, items: &[ArchivedItem]) -> anyhow::Result<usize> {
        let have: HashSet<String> = self.load().into_iter().map(|i| i.id).collect();
        let fresh: Vec<&ArchivedItem> = items.iter().filter(|i| !have.contains(&i.id)).collect();
        if fresh.is_empty() {
            return Ok(0);
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).with_context(|| format!("open {}", self.path.display()))?;
        for i in &fresh {
            writeln!(f, "{}", serde_json::to_string(i)?)?;
        }
        Ok(fresh.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const D: i64 = DAY_MS;
    fn task(finished: bool, live: bool, idle_days: i64) -> BoardItem {
        BoardItem { id: "t".into(), kind: ItemKind::Task, finished, live, touched_at: NOW - idle_days * D - 1000 }
    }
    fn question(idle_days: i64) -> BoardItem {
        BoardItem { id: "q".into(), kind: ItemKind::Question, finished: false, live: false, touched_at: NOW - idle_days * D }
    }

    #[test]
    fn done_tasks_archive_after_two_days() {
        let p = ArchivePolicy::default();
        assert_eq!(judge(&task(true, false, 1), NOW, &p), Verdict::Keep);
        assert_eq!(judge(&task(true, false, 2), NOW, &p), Verdict::Archive(ArchiveReason::Done));
        assert_eq!(judge(&task(true, false, 40), NOW, &p), Verdict::Archive(ArchiveReason::Done));
    }

    #[test]
    fn untouched_tasks_are_flagged_at_14_and_archived_at_21() {
        let p = ArchivePolicy::default();
        assert_eq!(judge(&task(false, false, 13), NOW, &p), Verdict::Keep);
        assert_eq!(judge(&task(false, false, 14), NOW, &p), Verdict::Flag(FlagReason::Stale));
        assert_eq!(judge(&task(false, false, 20), NOW, &p), Verdict::Flag(FlagReason::Stale));
        assert_eq!(judge(&task(false, false, 21), NOW, &p), Verdict::Archive(ArchiveReason::Stale));
    }

    #[test]
    fn a_task_a_worker_is_on_is_never_judged() {
        assert_eq!(judge(&task(false, true, 90), NOW, &ArchivePolicy::default()), Verdict::Keep);
    }

    #[test]
    fn a_waiting_question_is_flagged_and_never_archived() {
        let p = ArchivePolicy::default();
        assert_eq!(judge(&question(6), NOW, &p), Verdict::Keep);
        assert_eq!(judge(&question(7), NOW, &p), Verdict::Flag(FlagReason::QuestionWaiting));
        assert_eq!(judge(&question(365), NOW, &p), Verdict::Flag(FlagReason::QuestionWaiting));
    }

    #[test]
    fn zero_switches_a_rule_off() {
        let p = ArchivePolicy { done_days: 0, stale_flag_days: 0, stale_archive_days: 0, question_flag_days: 0 };
        assert_eq!(judge(&task(true, false, 99), NOW, &p), Verdict::Keep);
        assert_eq!(judge(&task(false, false, 99), NOW, &p), Verdict::Keep);
        assert_eq!(judge(&question(99), NOW, &p), Verdict::Keep);
    }

    #[test]
    fn normalizing_never_archives_before_flagging() {
        let p = ArchivePolicy { done_days: -3, stale_flag_days: 30, stale_archive_days: 10, question_flag_days: 7 }.normalized();
        assert_eq!((p.done_days, p.stale_flag_days, p.stale_archive_days), (0, 30, 30));
    }

    #[test]
    fn a_clock_behind_the_item_counts_as_zero_days() {
        assert_eq!(idle_days(NOW + 5 * D, NOW), 0);
        assert_eq!(judge(&task(true, false, 0), NOW - 10 * D, &ArchivePolicy::default()), Verdict::Keep);
    }

    fn item(id: &str, title: &str, text: &str, at: i64) -> ArchivedItem {
        ArchivedItem {
            id: id.into(),
            kind: ItemKind::Task,
            key: "t1".into(),
            title: title.into(),
            text: text.into(),
            status: "done".into(),
            reason: ArchiveReason::Done,
            plan_id: None,
            touched_at: 0,
            archived_at: at,
        }
    }

    #[test]
    fn archived_text_is_kept_in_a_file_and_stays_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let f = ArchiveFile::new(dir.path().join("archive").join("ws.jsonl"));
        assert!(f.load().is_empty());
        let a = item("a", "Add rate limiter", "Token bucket per API key\nResult: shipped", 10);
        let b = item("b", "Fix login redirect", "Safari drops the cookie", 20);
        assert_eq!(f.append(&[a.clone(), b.clone()]).unwrap(), 2);
        assert_eq!(f.append(&[a.clone()]).unwrap(), 0, "already archived");
        assert_eq!(f.load().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b", "a"], "newest first");
        assert_eq!(f.search("bucket KEY").len(), 1);
        assert_eq!(f.search("safari cookie")[0].id, "b");
        assert!(f.search("nothing like this").is_empty());
        assert_eq!(f.search("").len(), 2);
    }

    #[test]
    fn a_corrupt_line_does_not_hide_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let f = ArchiveFile::new(dir.path().join("ws.jsonl"));
        f.append(&[item("a", "One", "", 1)]).unwrap();
        std::fs::OpenOptions::new().append(true).open(f.path()).unwrap().write_all(b"not json\n").unwrap();
        f.append(&[item("b", "Two", "", 2)]).unwrap();
        assert_eq!(f.load().len(), 2);
    }
}
