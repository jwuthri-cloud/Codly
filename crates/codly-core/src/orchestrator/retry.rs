//! Worker retries: a worker that could not start, that exited by itself, or whose session ran
//! out of context is started again after a wait, and its task says why meanwhile. Failures in
//! a row are counted per task here, apart from `Task::attempt` (which counts every start: after
//! a restart of Codly, a usage-limit hand-off, your own Retry). A cause only you can fix, and
//! one failure too many, block the task.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::{Inner, lock};
use crate::model::*;

/// The waits before a failed worker starts again, one per retry. The failure after the last
/// retry blocks the task.
pub(super) const WORKER_RETRY_WAITS: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(30)];

/// Failures further apart than this are not in a row: the count starts again.
const IN_A_ROW_MS: i64 = 10 * 60_000;

/// How a task's worker failed.
pub(super) enum Failure {
    /// It did not start: the error of `try_start_worker`.
    Start(String),
    /// Its process exited by itself (`code` None: a signal). `error`: the CLI's last error.
    Exit { code: Option<i64>, error: Option<String> },
    /// Its turn failed with this error, which says the session's context is full.
    ContextFull(String),
}

impl Failure {
    /// The CLI's own words, if any.
    fn error(&self) -> Option<&str> {
        match self {
            Failure::Start(e) | Failure::ContextFull(e) => Some(e),
            Failure::Exit { error, .. } => error.as_deref(),
        }
    }

    /// The session is full: the next run starts a new one.
    fn context_full(&self) -> bool {
        match self {
            Failure::Start(_) => false,
            Failure::ContextFull(_) => true,
            Failure::Exit { error, .. } => error.as_deref().is_some_and(context_full),
        }
    }

    /// Waiting cannot help: the task is blocked at once.
    fn needs_you(&self) -> bool {
        matches!(self, Failure::Start(e) if needs_you(e))
    }

    /// For the task's reason: "could not start the worker: …".
    fn what(&self) -> String {
        match self {
            Failure::Start(e) => format!("could not start the worker: {e}"),
            Failure::Exit { code, .. } if !self.context_full() => {
                format!("the worker exited unexpectedly ({})", code.map_or("a signal".to_string(), |c| format!("exit {c}")))
            }
            Failure::Exit { .. } | Failure::ContextFull(_) => "the worker's session ran out of context".to_string(),
        }
    }
}

/// A start error only you can fix: no wait installs a CLI, signs you in or turns an account on.
/// (The texts are `runs.rs`'s. One this list does not know is retried, then blocks with its
/// own words.)
pub(super) fn needs_you(error: &str) -> bool {
    const CAUSES: [&str; 5] = ["not installed", "not signed in", "turned off", "no longer available", "binary not found"];
    let e = error.to_lowercase();
    CAUSES.iter().any(|c| e.contains(c))
}

/// Does a CLI's error say that the session's context is full? Resuming that session would
/// fail the same way.
pub(super) fn context_full(error: &str) -> bool {
    const SIGNS: [&str; 5] = ["prompt is too long", "context_length_exceeded", "context window", "context length", "maximum context"];
    let e = error.to_lowercase();
    SIGNS.iter().any(|s| e.contains(s))
}

/// "5 s", or "a moment" under a second.
fn in_secs(wait: Duration) -> String {
    match wait.as_secs() {
        0 => "a moment".to_string(),
        s => format!("{s} s"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Failed {
    /// Failures in a row.
    n: u32,
    /// When the last one was (ms).
    at: i64,
    /// Its next start waits until then (ms).
    not_before: i64,
}

/// The retry state of every task whose worker failed lately.
pub(super) struct Retries {
    waits: Vec<Duration>,
    failed: HashMap<Id, Failed>,
    /// Tasks whose next run starts a new session: the last one ran out of context.
    fresh: HashSet<Id>,
}

impl Default for Retries {
    fn default() -> Self {
        Retries { waits: WORKER_RETRY_WAITS.to_vec(), failed: HashMap::new(), fresh: HashSet::new() }
    }
}

impl Retries {
    /// One more failure of the task's worker at `now`: its number in the row, the wait before
    /// its next start (None: that was one too many, and the row ends), and what was on record
    /// before (for `restore`).
    fn fail(&mut self, task_id: &str, now: i64) -> (u32, Option<Duration>, Option<Failed>) {
        let before = self.failed.get(task_id).copied();
        let n = before.filter(|f| now - f.at <= IN_A_ROW_MS).map_or(1, |f| f.n + 1);
        let wait = self.waits.get(n as usize - 1).copied();
        match wait {
            Some(w) => self.failed.insert(task_id.to_string(), Failed { n, at: now, not_before: now + w.as_millis() as i64 }),
            None => self.failed.remove(task_id),
        };
        (n, wait, before)
    }

    /// Take a failure back: the task was not this worker's to fail any more.
    fn restore(&mut self, task_id: &str, before: Option<Failed>) {
        match before {
            Some(f) => self.failed.insert(task_id.to_string(), f),
            None => self.failed.remove(task_id),
        };
    }

    /// Your Retry: the task starts now, with a new row.
    pub(super) fn forget(&mut self, task_id: &str) {
        self.failed.remove(task_id);
    }

    /// Task → the time before which it does not start again, for the scheduler. Rows that
    /// ended long ago are dropped here.
    pub(super) fn holds(&mut self, now: i64) -> HashMap<Id, i64> {
        self.failed.retain(|_, f| now - f.at <= IN_A_ROW_MS);
        self.failed.iter().filter(|(_, f)| f.not_before > now).map(|(id, f)| (id.clone(), f.not_before)).collect()
    }

    /// Does the task's next run start a new session?
    pub(super) fn fresh(&self, task_id: &str) -> bool {
        self.fresh.contains(task_id)
    }

    /// That run got its brief.
    pub(super) fn started(&mut self, task_id: &str) {
        self.fresh.remove(task_id);
    }
}

impl Inner {
    /// The worker of a task did not start, exited by itself or filled its session. The task
    /// goes back to Ready and starts again after a wait, with the reason on its card; on a
    /// cause only you can fix, or one failure too many, it is Blocked. A task you stopped or
    /// cancelled meanwhile, or one past its worker's part (in its checks, review or merge),
    /// stays as it is: false.
    pub(super) fn worker_failed(self: &Arc<Self>, task_id: &str, agent_id: &str, failure: Failure) -> bool {
        let Ok(task) = self.task(task_id) else { return false };
        let (full, what) = (failure.context_full(), failure.what());
        let (n, wait, before, retries) = {
            let mut r = lock(&self.retries);
            let retries = r.waits.len();
            if failure.needs_you() {
                (1, None, r.failed.remove(task_id), retries)
            } else {
                let (n, wait, before) = r.fail(task_id, now_ms());
                (n, wait, before, retries)
            }
        };
        let reason = match wait {
            Some(w) => {
                let next = if full { "starts a new session" } else { "starts it again" };
                format!("{what}; Codly {next} in {} (retry {n} of {retries})", in_secs(w))
            }
            None if n == 1 => format!("blocked: {what}"),
            None => format!("blocked: the worker failed {n} times in a row (last: {what}); fix the cause, then Retry"),
        };
        // Recorded before the task is Ready, so no scheduling pass starts it early. Checked in
        // the write: only a task this worker still runs is its to fail.
        let mut moved = false;
        let _ = self.update_task(task_id, |t| {
            moved = matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) && t.assignee.as_deref() == Some(agent_id);
            if moved {
                t.status = if wait.is_some() { TaskStatus::Ready } else { TaskStatus::Blocked };
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        if !moved {
            lock(&self.retries).restore(task_id, before);
            return false;
        }
        if full {
            lock(&self.retries).fresh.insert(task_id.to_string());
        }
        // A run of this task that is still up (its turn failed, or its brief did not go out)
        // stops, and the agent is free for the retry.
        self.release_worker(agent_id, task_id);
        let ws = self.store.task_workspace(task_id).ok().flatten();
        let detail = json!({
            "plan_id": task.plan_id, "error": failure.error(), "failures": n,
            "retry_in_ms": wait.map(|w| w.as_millis() as u64), "new_session": full,
        });
        let _ = self.decide(ws.as_deref(), Some(task_id), "retry", reason.clone(), detail);
        let me = self.clone();
        match (wait, ws) {
            (Some(w), _) => {
                // The pass that starts it again, on time (the slow tick could be seconds late).
                tokio::spawn(async move {
                    tokio::time::sleep(w).await;
                    me.kick();
                });
            }
            (None, Some(ws)) => {
                let (thread, text) = (self.task_thread(&task), format!("Task {} \"{}\" is {reason}.", task.key, task.title));
                tokio::spawn(async move { me.lead_followup(&ws, &thread, &text).await });
            }
            (None, None) => {}
        }
        self.kick();
        true
    }
}

impl super::Core {
    /// The waits before a failed worker starts again, one per retry (`WORKER_RETRY_WAITS`
    /// until set; the tests shorten them).
    pub fn set_worker_retry_waits(&self, waits: Vec<Duration>) {
        lock(&self.inner.retries).waits = waits;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000;

    #[test]
    fn a_row_of_failures_waits_longer_each_time_then_ends() {
        let mut r = Retries::default();
        assert_eq!(r.fail("t", 0), (1, Some(Duration::from_secs(5)), None));
        assert_eq!(r.holds(S), HashMap::from([("t".to_string(), 5 * S)]), "it does not start before its wait is over");
        assert!(r.holds(5 * S).is_empty(), "then it may start");
        let (n, wait, before) = r.fail("t", 6 * S);
        assert_eq!((n, wait), (2, Some(Duration::from_secs(30))));
        assert_eq!(before.map(|f| f.n), Some(1));
        // The third failure in a row is one too many: no wait, and the row ends.
        assert_eq!(r.fail("t", 40 * S).1, None);
        assert_eq!(r.fail("t", 41 * S).0, 1, "after a block the count starts again");
    }

    #[test]
    fn failures_far_apart_are_not_in_a_row() {
        let mut r = Retries::default();
        r.fail("t", 0);
        assert_eq!(r.fail("t", IN_A_ROW_MS + 1).0, 1);
        assert_eq!(r.fail("t", IN_A_ROW_MS + 2).0, 2);
        // Another task has its own row, and old rows are dropped.
        assert_eq!(r.fail("u", IN_A_ROW_MS + 3).0, 1);
        r.holds(3 * IN_A_ROW_MS);
        assert!(r.failed.is_empty());
    }

    #[test]
    fn a_failure_can_be_taken_back_and_your_retry_starts_at_once() {
        let mut r = Retries::default();
        let (_, _, before) = r.fail("t", 0);
        r.restore("t", before);
        assert!(r.holds(1).is_empty() && r.failed.is_empty());
        r.fail("t", 0);
        let (_, _, before) = r.fail("t", S);
        r.restore("t", before);
        assert_eq!(r.failed["t"].n, 1, "the earlier failure stays on record");
        r.forget("t");
        assert!(r.holds(S).is_empty());
    }

    #[test]
    fn only_causes_you_must_fix_block_at_once() {
        for e in [
            "Claude Code is not installed",
            "Codex is not signed in: run `codex login`",
            "the Claude Code account 'work' is turned off in Settings",
            "the Claude Code account 'work' is not signed in: run `claude /login` in a terminal",
            "the Codex account codex:/h/.codex-2 is no longer available",
            "Gemini CLI binary not found",
        ] {
            assert!(needs_you(e), "{e}");
            assert!(Failure::Start(e.into()).needs_you(), "{e}");
        }
        for e in ["Ada is already running", "Ada did not leave its chat in time", "Claude Code exited during startup", "the agent did not finish the handshake"] {
            assert!(!needs_you(e), "{e}");
        }
        // Only a start can need you: an exit is retried whatever the CLI said last.
        assert!(!Failure::Exit { code: Some(1), error: Some("not signed in".into()) }.needs_you());
    }

    #[test]
    fn a_full_context_is_told_from_other_errors() {
        for e in ["Prompt is too long", "error: context_length_exceeded", "This model's maximum context length is 200000 tokens", "the context window is full"] {
            assert!(context_full(e), "{e}");
        }
        for e in ["Invalid API key · Please run /login", "overloaded_error", "the CLI gave no reason"] {
            assert!(!context_full(e), "{e}");
        }
        assert_eq!(Failure::ContextFull("Prompt is too long".into()).what(), "the worker's session ran out of context");
        let exit = Failure::Exit { code: Some(1), error: Some("Prompt is too long".into()) };
        assert!(exit.context_full());
        assert_eq!(exit.what(), "the worker's session ran out of context");
    }

    #[test]
    fn reasons_say_what_happened_in_plain_words() {
        assert_eq!(Failure::Start("Ada is already running".into()).what(), "could not start the worker: Ada is already running");
        assert_eq!(Failure::Exit { code: Some(1), error: None }.what(), "the worker exited unexpectedly (exit 1)");
        assert_eq!(Failure::Exit { code: None, error: None }.what(), "the worker exited unexpectedly (a signal)");
        assert_eq!((in_secs(Duration::from_secs(30)), in_secs(Duration::from_millis(200))), ("30 s".to_string(), "a moment".to_string()));
    }
}
