//! Worker retries: a worker that could not start, that exited by itself, or whose session ran
//! out of context is started again after a wait, and its task says why meanwhile. Failures in
//! a row are counted per task here, apart from `Task::attempt` (which counts every start: after
//! a restart of Codly, a usage-limit hand-off, your own Retry). A row ends when the worker
//! completes a turn (the restart worked), not with time: a worker that fails again and again
//! with no turn completed in between is stopped, however long each run lasted. A cause only you
//! can fix, and one failure too many, block the task.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::{Inner, lock};
use crate::model::*;

/// The waits before a failed worker starts again, one per retry. The failure after the last
/// retry blocks the task.
pub(super) const WORKER_RETRY_WAITS: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(30)];

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

/// A start error only you can fix: no wait installs a CLI, signs you in, turns an account on
/// or trusts a folder's own CLI configuration. (The texts are `runs.rs`'s and `trust.rs`'s.
/// One this list does not know is retried, then blocks with its own words.)
pub(super) fn needs_you(error: &str) -> bool {
    const CAUSES: [&str; 6] =
        ["not installed", "not signed in", "turned off", "no longer available", "binary not found", super::trust::NOT_STARTED];
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

struct Failed {
    /// Failures in a row: since the worker last completed a turn.
    n: u32,
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
    /// The next failure of the task's worker: its number in the row, and the wait before its
    /// next start (None: that one is one too many).
    fn next(&self, task_id: &str) -> (u32, Option<Duration>) {
        let n = self.failed.get(task_id).map_or(1, |f| f.n + 1);
        (n, self.waits.get(n as usize - 1).copied())
    }

    /// That failure happened at `now`: the task waits, or (no wait) its row ends.
    fn record(&mut self, task_id: &str, n: u32, wait: Option<Duration>, now: i64) {
        match wait {
            Some(w) => self.failed.insert(task_id.to_string(), Failed { n, not_before: now + w.as_millis() as i64 }),
            None => self.failed.remove(task_id),
        };
    }

    /// The row ends: the worker completed a turn, the task is done, or you chose Retry (the
    /// task then starts now).
    pub(super) fn forget(&mut self, task_id: &str) {
        self.failed.remove(task_id);
    }

    /// Task → the time before which it does not start again, for the scheduler.
    pub(super) fn holds(&self, now: i64) -> HashMap<Id, i64> {
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
        // One step under the lock, from the count to the task's write: a second report of the
        // same failure (the exit of a run, and its brief that did not go out) finds the task
        // moved and counts nothing. A scheduling pass reads the tasks before the holds, so it
        // never sees this task Ready without its wait.
        let (n, wait, reason) = {
            let mut r = lock(&self.retries);
            let (n, wait) = if failure.needs_you() { (1, None) } else { r.next(task_id) };
            let reason = match wait {
                Some(w) => {
                    let next = if full { "starts a new session" } else { "starts it again" };
                    format!("{what}; Codly {next} in {} (retry {n} of {})", in_secs(w), r.waits.len())
                }
                None if n == 1 => format!("blocked: {what}"),
                None => format!("blocked: the worker failed {n} times in a row (last: {what}); fix the cause, then Retry"),
            };
            // Checked in the write: only a task this worker still runs is its to fail.
            let mut mine = false;
            let written = self.update_task(task_id, |t| {
                mine = matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) && t.assignee.as_deref() == Some(agent_id);
                if mine {
                    t.status = if wait.is_some() { TaskStatus::Ready } else { TaskStatus::Blocked };
                    t.status_reason = Some(reason.clone());
                }
                Ok(())
            });
            if !mine || written.is_err() {
                return false;
            }
            r.record(task_id, n, wait, now_ms());
            if full {
                r.fresh.insert(task_id.to_string());
            }
            (n, wait, reason)
        };
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
    use crate::orchestrator::{Core, CoreConfig, StartOptions, row};

    const S: i64 = 1_000;
    const MIN: i64 = 60 * S;

    /// One more failure of the task's worker at `now`, counted as `worker_failed` does.
    fn fail(r: &mut Retries, task_id: &str, now: i64) -> (u32, Option<Duration>) {
        let (n, wait) = r.next(task_id);
        r.record(task_id, n, wait, now);
        (n, wait)
    }

    #[test]
    fn a_row_of_failures_waits_longer_each_time_then_ends() {
        let mut r = Retries::default();
        assert_eq!(r.next("t"), (1, Some(Duration::from_secs(5))));
        assert!(r.failed.is_empty(), "looking at the next failure counts nothing");
        assert_eq!(fail(&mut r, "t", 0), (1, Some(Duration::from_secs(5))));
        assert_eq!(r.holds(S), HashMap::from([("t".to_string(), 5 * S)]), "it does not start before its wait is over");
        assert!(r.holds(5 * S).is_empty(), "then it may start");
        assert_eq!(fail(&mut r, "t", 6 * S), (2, Some(Duration::from_secs(30))));
        // The third failure in a row is one too many: no wait, and the row ends.
        assert_eq!(fail(&mut r, "t", 40 * S), (3, None));
        assert_eq!(fail(&mut r, "t", 41 * S).0, 1, "after a block the count starts again");
    }

    /// A worker that dies after 11 minutes of work each time is stopped at its third failure:
    /// time alone never ends a row.
    #[test]
    fn failures_far_apart_are_still_in_a_row() {
        let mut r = Retries::default();
        assert_eq!(fail(&mut r, "t", 0).0, 1);
        assert_eq!(fail(&mut r, "t", 11 * MIN).0, 2);
        assert_eq!(fail(&mut r, "t", 22 * MIN), (3, None));
        assert!(r.failed.is_empty());
    }

    #[test]
    fn a_completed_turn_or_your_retry_ends_the_row() {
        let mut r = Retries::default();
        fail(&mut r, "t", 0);
        fail(&mut r, "t", S);
        assert_eq!(fail(&mut r, "u", S).0, 1, "another task has its own row");
        r.forget("t");
        assert_eq!(r.holds(2 * S).keys().collect::<Vec<_>>(), ["u"], "no wait is left over");
        assert_eq!(fail(&mut r, "t", 2 * S).0, 1, "the next failure is the first of a new row");
        assert_eq!(r.next("u").0, 2);
    }

    /// A run that exits right after its handshake is reported twice: by its exit, and by its
    /// brief that did not go out. The first report moves the task; the second one finds a task
    /// that is not this run's to fail any more, and counts nothing.
    #[tokio::test]
    async fn a_second_report_of_the_same_failure_counts_nothing() {
        let root = tempfile::tempdir().unwrap();
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let inner = &core.inner;
        // An office with no folder: no scheduling pass touches its task.
        let ws = Workspace {
            id: new_id(),
            repo_path: root.path().join("gone").to_string_lossy().into_owned(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        inner.store.upsert_workspace(&ws).unwrap();
        let ada = inner.create_agent(&ws.id, "Ada", Provider::Claude, Role::Worker, 0, None).unwrap();
        let task = Task {
            id: new_id(),
            plan_id: None,
            key: "a1".into(),
            title: "Write the greeting".into(),
            goal: "Create a text file".into(),
            acceptance: vec![],
            write_scope: vec![],
            verify: vec![],
            depends_on: vec![],
            status: TaskStatus::Running,
            assignee: Some(ada.id.clone()),
            agent_hint: None,
            account_hint: None,
            consultant: None,
            fresh_session: false,
            branch: None,
            worktree: None,
            attempt: 1,
            status_reason: None,
            summary: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        inner.emit(Some(&ws.id), row(None, Some(&task.id), None, EventKind::TaskChanged { task: task.clone() })).unwrap();

        assert!(inner.worker_failed(&task.id, &ada.id, Failure::Exit { code: Some(1), error: None }));
        assert!(!inner.worker_failed(&task.id, &ada.id, Failure::Start("that agent is not running".into())));
        let now = inner.task(&task.id).unwrap();
        assert_eq!(
            (now.status, now.status_reason.as_deref()),
            (TaskStatus::Ready, Some("the worker exited unexpectedly (exit 1); Codly starts it again in 5 s (retry 1 of 2)"))
        );
        let r = lock(&inner.retries);
        assert_eq!(r.next(&task.id).0, 2, "one failure is on record");
        assert!(r.holds(now_ms()).contains_key(&task.id), "and its wait stands");
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
            "Gemini CLI is not started in this folder: it has its own Gemini CLI configuration that you have not trusted",
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
