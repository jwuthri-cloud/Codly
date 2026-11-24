//! Required checks: the repo owner's own commands, which decide what "passes".
//!
//! The Lead writes a plan's verify commands, so "the gate passed" is only as strong as the
//! command a model chose. `.codly/checks` is the owner's list: one shell command on each line
//! (a line that starts with `#` is a comment). Once you turn it on in Settings, the gate runs
//! each of its commands for every task, after the task's own verify commands, and again on
//! every finished plan, after its integration checks. "Check the work" runs them in the
//! worktree of an agent you chat with, whose work has no gate.
//!
//! The file is text from the repo, and its commands run without asking. So they run only for
//! the exact text you turned on, read from your checkout (never from a worktree, where an
//! agent could change it); Settings shows that text and what the approval rules see in each
//! command. When the file is not that text any more, none of its commands runs and nothing
//! passes in their place: a task waits for you, and a finished plan says that it was not
//! checked, until you turn the new text on or turn the checks off. That choice is all the
//! waiting tasks need: Codly checks each of them again by itself, a few at a time, and the
//! tasks that share a team member's worktree one after the other. The tasks still to check
//! are in the store until each is through (`Rechecks`): when Codly closes in the middle, the
//! next start checks the rest, or the end of its restart hold does (`resume.rs`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use ts_rs::TS;

use super::worktree_setup::fingerprint;
use super::{Core, Inner, approvals, lock, prompts, row, scheduler};
use crate::model::*;

/// The repo's required checks, relative to the repo root.
pub const CHECKS_FILE: &str = ".codly/checks";
const MAX_FILE_BYTES: u64 = 16 * 1024;
/// Where you read the checks and turn them on or off.
const SETTINGS: &str = "Settings → Safety → Required checks";

/// One command of the repo's checks file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RequiredCheck {
    pub command: String,
    /// What the approval rules see in it ("network", "deletes files"…), as they do for a
    /// verify command. Shown before you turn the checks on; it does not hold the command.
    pub risks: Vec<String>,
}

/// The open repo's required checks, as Settings shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RequiredChecks {
    /// The text of `.codly/checks` in your checkout. None = no file, or one with no command.
    pub text: Option<String>,
    /// The fingerprint of that text: send it back to turn the checks on.
    pub sha: Option<String>,
    /// The commands of that text, in its order.
    pub commands: Vec<RequiredCheck>,
    /// You turned the checks on, for this exact text.
    pub on: bool,
    /// You turned on another text: the file changed since, is gone, or cannot be read (see
    /// `problem`). None of its commands runs, and tasks wait, until you turn the checks on
    /// again or off.
    pub changed: bool,
    /// Why Codly does not read the file that is there (a link, not a file, too long, not
    /// text). `text` and `sha` are None then.
    pub problem: Option<String>,
}

/// Turn the repo's required checks on or off.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct ChecksChoice {
    /// `RequiredChecks::sha` of the text you read; None = off.
    pub allow: Option<String>,
}

/// One required check Codly ran ("Check the work").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CheckResult {
    pub command: String,
    pub ok: bool,
    /// None: it ran out of time, a signal ended it, or it did not start.
    pub exit_code: Option<i64>,
    /// Codly stopped it at the time limit for checks (Settings).
    pub timed_out: bool,
    /// The last lines of its output.
    pub tail: String,
}

/// The result of one "Check the work".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CheckedWork {
    pub results: Vec<CheckResult>,
    /// When the checks ended (Unix ms).
    pub at: i64,
}

/// What "Check the work" has for an agent now, as its Git tab shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentCheck {
    /// Its required checks run now: the result comes when they end.
    pub running: bool,
    /// The last result, while it is about the agent's files as they are now. None: Codly did
    /// not check this work, the agent works, or its files changed since (a turn of the agent,
    /// files you put back, a merge into its branch, your own edits).
    pub last: Option<CheckedWork>,
}

/// The last "Check the work" of an agent, as the store keeps it.
#[derive(Serialize, Deserialize)]
struct Checked {
    work: CheckedWork,
    /// The agent's files when the checks ended (`Git::worktree_tree`): the result is about
    /// these files and no others.
    tree: String,
}

/// What the gate does about the repo's required checks.
pub(super) enum Required {
    /// You did not turn them on.
    Off,
    /// On, for the text the file has now: its commands, in its order.
    Run(Vec<String>),
    /// On, for a text the file does not have any more: none of them runs. Why, for the user.
    Stale(String),
}

/// What the required checks say about a finished plan.
pub(super) struct PlanChecks {
    /// The commands that ran and failed.
    pub failed: Vec<String>,
    /// The checks are on and did not run: why, and what to do, for the user.
    pub not_run: Option<String>,
}

fn checks_key(ws_id: &str) -> String {
    format!("required_checks:{ws_id}")
}

/// Where the store keeps the result of an agent's last "Check the work", until the agent's
/// files change or the agent is let go (`Inner::forget_agent`).
pub(super) fn checked_key(agent_id: &str) -> String {
    format!("agent_check:{agent_id}")
}

/// The commands of a checks file: its lines that are not blank and not a comment, once each.
fn commands(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && seen.insert(*l)).map(String::from).collect()
}

/// The repo's checks file. None = no file, or one with no command; Err = a file Codly does
/// not read (a link, not a file, too long, not text).
fn read_checks(repo: &Path) -> Result<Option<String>> {
    let path = repo.join(CHECKS_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => bail!("cannot read {CHECKS_FILE}: {e}"),
    };
    if !meta.is_file() {
        bail!("{CHECKS_FILE} is not a regular file");
    }
    if meta.len() > MAX_FILE_BYTES {
        bail!("{CHECKS_FILE} is over {} KB", MAX_FILE_BYTES >> 10);
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {CHECKS_FILE}"))?;
    Ok(Some(text).filter(|t| !commands(t).is_empty()))
}

/// Said where required checks that are on did not run: why, and what to do.
pub(super) fn not_run(why: &str) -> String {
    format!("Codly did not run your required checks: {why}. Read them in {SETTINGS} and turn them on again, or turn them off.")
}

/// "1 required check", "3 required checks".
fn count(n: usize) -> String {
    format!("{n} required check{}", if n == 1 { "" } else { "s" })
}

/// How many commands the Lead's board names; the rest is a count.
const BOARD_COMMANDS: usize = 6;

/// The required checks on the Lead's board (`prompts::board_header`): the commands Codly runs
/// for every task, or why it runs none of them now. None: they are off. The commands are the
/// text you read and turned on.
pub(super) fn board_line(required: &Required) -> Option<String> {
    match required {
        Required::Off => None,
        Required::Run(cmds) => {
            let mut list: Vec<String> = cmds.iter().take(BOARD_COMMANDS).map(|c| format!("`{}`", prompts::clip(c, 60))).collect();
            if cmds.len() > BOARD_COMMANDS {
                list.push(format!("{} more", cmds.len() - BOARD_COMMANDS));
            }
            Some(format!(
                "Required by the user ({CHECKS_FILE}; Codly runs these for every task after its verify commands, and for every finished plan): {}",
                list.join(" · ")
            ))
        }
        // Why (`Stale`'s text) is written for the user; each waiting task's line has it.
        Required::Stale(_) => Some(format!(
            "Required by the user ({CHECKS_FILE}): Codly runs none of them now, the file is not the text the user turned on. Each task waits \
             when its worker finishes, until the user turns the checks on again, or off, in {SETTINGS}; Codly then checks the waiting tasks \
             again by itself."
        )),
    }
}

/// An agent's place in `Inner::checking` while "Check the work" runs: given back when the
/// run ends, however it ends.
struct Checking<'a> {
    inner: &'a Inner,
    agent_id: Id,
}

impl Drop for Checking<'_> {
    fn drop(&mut self) {
        lock(&self.inner.checking).remove(&self.agent_id);
    }
}

/// How often a re-check looks whether the gate is through with a task.
const SETTLE_POLL: Duration = Duration::from_millis(50);

/// The tasks of an office that Codly still checks again after your choice, as the store keeps
/// them (kv `required_rechecks:<office>`): written when the queue is built, shortened as each
/// task is through, gone at its end. A row that is there at a start was cut short by a quit.
#[derive(Serialize, Deserialize)]
struct Rechecks {
    /// You turned the checks on (or off), for the record of what became of the tasks.
    on: bool,
    tasks: Vec<Id>,
}

const RECHECKS: &str = "required_rechecks:";

fn rechecks_key(ws_id: &str) -> String {
    format!("{RECHECKS}{ws_id}")
}

/// What became of a task that waited for your required checks, after your choice.
enum Rechecked {
    /// Codly checked it again: the pass ran, and the task went where its checks sent it.
    Again,
    /// Its team member (the name) is on other work in the worktree they share, so the pass
    /// could not run there: the task is back in line for the member, whose next turn on it
    /// ends with the checks.
    Back(String),
    /// It still waits for you: why.
    Waits(String),
}

impl Inner {
    // ───────────── the switch ─────────────

    fn checks_status(&self, ws: &Workspace) -> Result<RequiredChecks> {
        let repo = Path::new(&ws.repo_path);
        // A file Codly does not read is said in `problem`: the way to turn the checks off stays.
        let (text, problem) = match read_checks(repo) {
            Ok(text) => (text, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let sha = text.as_deref().map(fingerprint);
        let allowed = self.store.kv_get(&checks_key(&ws.id))?;
        // The same rules as for a verify command (gate.rs), on each command's text.
        let commands = text
            .as_deref()
            .map(commands)
            .unwrap_or_default()
            .into_iter()
            .map(|command| {
                let risks = approvals::detect_risks(&ApprovalSubject::Command { command: command.clone(), cwd: None }, Some(repo));
                RequiredCheck { command, risks }
            })
            .collect();
        let on = sha.is_some() && sha == allowed;
        Ok(RequiredChecks { on, changed: allowed.is_some() && !on, problem, text, sha, commands })
    }

    fn choose_checks(&self, ws: &Workspace, choice: ChecksChoice) -> Result<()> {
        let key = checks_key(&ws.id);
        let Some(sha) = choice.allow else {
            if self.store.kv_get(&key)?.is_some() {
                self.store.kv_write(&[(key, None)])?;
                self.decide(Some(&ws.id), None, "approval_policy", format!("you turned off the required checks ({CHECKS_FILE})"), json!({}))?;
            }
            return Ok(());
        };
        let status = self.checks_status(ws)?;
        if status.text.is_none() {
            bail!("{}", status.problem.unwrap_or_else(|| format!("this repo has no {CHECKS_FILE} with a command in it")));
        }
        if status.sha.as_deref() != Some(sha.as_str()) {
            bail!("{CHECKS_FILE} changed since you read it. Read it again, then turn it on");
        }
        self.store.kv_set(&key, &sha)?;
        let summary = format!("you turned on the required checks: Codly runs {CHECKS_FILE} for every task and every finished plan");
        self.decide(Some(&ws.id), None, "approval_policy", summary, json!({ "sha": sha, "commands": status.commands }))?;
        Ok(())
    }

    /// Does the task wait for you because your required checks were not the text you turned
    /// on? Then the gate parked it with the `required` key, and nothing was decided about it
    /// since.
    fn waits_for_checks(&self, task: &Task) -> Result<bool> {
        if task.status != TaskStatus::Blocked {
            return Ok(false);
        }
        let last = self.store.last_task_event(&task.id, "decision")?;
        Ok(last.is_some_and(|e| matches!(&e.kind, EventKind::Decision { decision_type, detail, .. } if decision_type == "gate" && !detail["required"].is_null())))
    }

    /// The tasks of an office that wait for you because of your required checks.
    fn parked_for_checks(&self, ws_id: &str) -> Result<Vec<Task>> {
        let mut parked = vec![];
        for t in self.store.tasks(ws_id)? {
            if self.waits_for_checks(&t)? {
                parked.push(t);
            }
        }
        Ok(parked)
    }

    /// Wait until nothing works on the task: no pass of the gate runs for it, and it is not
    /// in flight (a fix round, a review, the merge queue). False: Codly is quitting.
    async fn settled(&self, task_id: &str) -> bool {
        loop {
            if self.shutting_down.load(Ordering::SeqCst) {
                return false;
            }
            let in_gate = lock(&self.gating).contains_key(task_id);
            // A task that is gone has nothing to wait for.
            if !in_gate && !self.task(task_id).is_ok_and(|t| scheduler::is_active(t.status)) {
                return true;
            }
            tokio::time::sleep(SETTLE_POLL).await;
        }
    }

    /// You turned the required checks on (`on`) or off: every task that waited for that
    /// choice is checked again, as Re-run checks on its card does, with no click on each.
    /// Not all at once: the checks of a few tasks run at a time (as many as workers may:
    /// `max_parallel`), so N waiting tasks do not start N test runs that push each other
    /// over the time limit. And the tasks of one team member, which share its worktree, go
    /// one after the other: a pass that began while the member's other task was checked
    /// there would send the task back to the member for a new turn. When all of them are
    /// through, the record says what became of each. One choice at a time: a second one
    /// (on, then off) waits here for the first one's tasks, then takes what still waits.
    /// `only`: the tasks a quit left in the queue (`Rechecks`), of those that still wait.
    async fn recheck_parked(self: Arc<Self>, ws: Workspace, on: bool, only: Option<Vec<Id>>) {
        let _one = self.rechecks.lock().await;
        if let Err(e) = self.recheck_lanes(&ws, on, only).await {
            tracing::warn!("re-checking the tasks that waited for the required checks: {e:#}");
        }
    }

    async fn recheck_lanes(self: &Arc<Self>, ws: &Workspace, on: bool, only: Option<Vec<Id>>) -> Result<()> {
        let mut parked = self.parked_for_checks(&ws.id)?;
        if let Some(only) = &only {
            parked.retain(|t| only.contains(&t.id));
        }
        let order: Vec<Id> = parked.iter().map(|t| t.id.clone()).collect();
        // In the store until each task is through: a quit leaves the rest for the next start.
        self.keep_rechecks(&ws.id, on, &order);
        let left = Arc::new(Mutex::new(order.clone()));
        // The tasks that share a worktree are one lane.
        let mut lanes: Vec<Vec<Task>> = vec![];
        for t in parked {
            match lanes.iter_mut().find(|l| t.worktree.is_some() && l[0].worktree == t.worktree) {
                Some(lane) => lane.push(t),
                None => lanes.push(vec![t]),
            }
        }
        let slots = Arc::new(Semaphore::new(self.settings().max_parallel.max(1) as usize));
        let mut running = JoinSet::new();
        for lane in lanes {
            let (me, slots, left, ws_id) = (self.clone(), slots.clone(), left.clone(), ws.id.clone());
            running.spawn(async move {
                // Held for the whole lane. (Nothing closes the semaphore.)
                let _slot = slots.acquire().await;
                let mut done = vec![];
                for t in lane {
                    let what = me.recheck_waiting(&t).await;
                    // Quitting: the task may not be through, so it stays in the queue. The next
                    // start passes over it if it waits no more.
                    if !me.shutting_down.load(Ordering::SeqCst) {
                        let mut still = lock(&left);
                        still.retain(|id| *id != t.id);
                        me.keep_rechecks(&ws_id, on, &still);
                    }
                    if let Some(what) = what {
                        done.push((t, what));
                    }
                }
                done
            });
        }
        let mut done = vec![];
        while let Some(lane) = running.join_next().await {
            match lane {
                Ok(tasks) => done.extend(tasks),
                Err(e) => tracing::warn!("a re-check of the tasks that waited for the required checks crashed: {e}"),
            }
        }
        // Quitting: the tasks that were not reached still wait, with Re-run checks on their
        // cards, and the next start checks them (`Rechecks`).
        if done.is_empty() || self.shutting_down.load(Ordering::SeqCst) {
            return Ok(());
        }
        done.sort_by_key(|(t, _)| order.iter().position(|id| *id == t.id));
        let (mut again, mut back, mut waiting, mut said) = (vec![], vec![], vec![], vec![]);
        for (t, what) in done {
            match what {
                Rechecked::Again => again.push(t.key),
                Rechecked::Back(who) => {
                    said.push(format!("{} goes back to {who} first ({who} is on other work in the same worktree)", t.key));
                    back.push(json!({ "task": t.key, "member": who }));
                }
                Rechecked::Waits(why) => waiting.push(format!("{} ({why})", t.key)),
            }
        }
        if !again.is_empty() {
            said.insert(0, format!("Codly checked {} again", again.join(", ")));
        }
        if !waiting.is_empty() {
            said.push(format!("still waiting for you: {}", waiting.join(", ")));
        }
        let summary = format!("after you turned the required checks {}: {}", if on { "on" } else { "off" }, said.join("; "));
        self.decide(Some(&ws.id), None, "gate", summary, json!({ "rechecked": again, "back_to_member": back, "still_waiting": waiting }))?;
        Ok(())
    }

    /// Check one waiting task again and wait until its pass is through, so the next task of
    /// its lane finds the worktree free. None: it does not wait for your checks any more (you
    /// moved it on from its card meanwhile), or Codly is quitting.
    async fn recheck_waiting(self: &Arc<Self>, task: &Task) -> Option<Rechecked> {
        // The pass that parked it ends a moment after it wrote the task's state (it still
        // lets the worker go): the re-run needs the task's gate free.
        if !self.settled(&task.id).await {
            return None;
        }
        let what = match self.recheck_now(task).await {
            Ok(what) => what?,
            Err(e) => Rechecked::Waits(e.to_string()),
        };
        if matches!(what, Rechecked::Again) {
            self.settled(&task.id).await;
        }
        Some(what)
    }

    /// Start the gate pass of a task that waits for your required checks (`recheck_task`, as
    /// its card does). None: it does not wait for them any more.
    async fn recheck_now(self: &Arc<Self>, task: &Task) -> Result<Option<Rechecked>> {
        if !self.waits_for_checks(&self.task(&task.id)?)? {
            return Ok(None);
        }
        let mark = self.store.last_task_event(&task.id, "decision")?.map_or(0, |e| e.id);
        let held = self.recheck_task(&task.id, vec![], super::gate::Rerun::Codly).await?;
        if !held.is_empty() {
            let cmds: Vec<String> = held.iter().map(|h| format!("`{}`", prompts::clip(&h.command, 80))).collect();
            return Ok(Some(Rechecked::Waits(format!("needs your OK for {}", cmds.join(", ")))));
        }
        // A pass that could not have the worktree (the task's team member is on other work
        // there) put the task back in line for the member, and the task's record says so.
        let mut since = self.store.task_events(&task.id, "decision")?.into_iter().filter(|e| e.id > mark);
        let back = since.find_map(|e| match e.kind {
            EventKind::Decision { detail, .. } => detail["waits_for"].as_str().map(String::from),
            _ => None,
        });
        Ok(Some(back.map_or(Rechecked::Again, Rechecked::Back)))
    }

    /// Keep the tasks of an office that are still to check again in the store, or take the
    /// row out when none is left. A row that cannot be written is not a reason to stop the
    /// re-checks: only a quit before their end would lose the rest.
    fn keep_rechecks(&self, ws_id: &str, on: bool, tasks: &[Id]) {
        let row = (!tasks.is_empty()).then(|| serde_json::to_string(&Rechecks { on, tasks: tasks.to_vec() })).transpose();
        let kept = row.map_err(anyhow::Error::from).and_then(|row| self.store.kv_write(&[(rechecks_key(ws_id), row)]));
        if let Err(e) = kept {
            tracing::warn!("the tasks still to check again are not in the store as they are now, so a quit may leave some waiting: {e:#}");
        }
    }

    /// Is a re-check that a quit cut short still to do? That is work that starts by itself
    /// (`resume.rs` holds it for your Resume): a queue of an office the scheduler works in
    /// (`scheduled`, with `offices` as the caller's look at them) in which a task still waits
    /// for your required checks. A queue that cannot be read, or whose office is gone, is taken
    /// out (`left_queue`).
    pub(super) fn rechecks_left(&self, offices: &mut HashMap<Id, bool>) -> Result<bool> {
        for (key, row) in self.store.kv_prefixed(RECHECKS)? {
            let Some((queue, ws)) = self.left_queue(&key, &row) else { continue };
            if !self.scheduled(offices, &ws.id)? {
                continue;
            }
            for id in &queue.tasks {
                if let Some(t) = self.store.task(id)?
                    && self.waits_for_checks(&t)?
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// The re-checks that a quit cut short go on: each office's queue starts again, with the
    /// tasks that were not through and still wait for your required checks. The queue of an
    /// office the scheduler passes over (its repo folder is not there) waits for a later start.
    pub(super) fn restart_rechecks(self: &Arc<Self>) {
        let rows = match self.store.kv_prefixed(RECHECKS) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("the re-checks a quit cut short cannot be read, so they do not go on now: {e:#}");
                return;
            }
        };
        let mut offices = HashMap::new();
        for (key, row) in rows {
            let Some((queue, ws)) = self.left_queue(&key, &row) else { continue };
            match self.scheduled(&mut offices, &ws.id) {
                Ok(true) => {
                    tokio::spawn(self.clone().recheck_parked(ws, queue.on, Some(queue.tasks)));
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("the re-checks a quit cut short in office {} do not go on now: {e:#}", ws.id),
            }
        }
    }

    /// The queue of store row `key` = `row`, with its office. None: the row cannot be read, or
    /// its office is gone. Such a row is taken out: it would hold the work of every later start
    /// for nothing.
    fn left_queue(&self, key: &str, row: &str) -> Option<(Rechecks, Workspace)> {
        let ws_id = &key[RECHECKS.len()..];
        let queue = serde_json::from_str::<Rechecks>(row)
            .map_err(anyhow::Error::from)
            .and_then(|queue| Ok((queue, self.store.workspace(ws_id)?.ok_or_else(|| anyhow!("its office is not there any more"))?)));
        match queue {
            Ok(queue) => Some(queue),
            Err(e) => {
                tracing::warn!("the re-checks a quit cut short in office {ws_id} are dropped: {e:#}");
                if let Err(e) = self.store.kv_write(&[(key.to_string(), None)]) {
                    tracing::warn!("they stay in the store: {e:#}");
                }
                None
            }
        }
    }

    // ───────────── for the gate ─────────────

    /// The required checks of the repo at `repo` (the checkout of workspace `ws_id`), as the
    /// gate must treat them now.
    pub(super) fn required_checks(&self, ws_id: &str, repo: &Path) -> Result<Required> {
        let Some(allowed) = self.store.kv_get(&checks_key(ws_id))? else { return Ok(Required::Off) };
        Ok(match read_checks(repo) {
            Ok(Some(text)) if fingerprint(&text) == allowed => Required::Run(commands(&text)),
            Ok(Some(_)) => Required::Stale(format!("{CHECKS_FILE} changed since you turned them on")),
            Ok(None) => Required::Stale(format!("{CHECKS_FILE} has no command any more")),
            Err(e) => Required::Stale(format!("{e:#}")),
        })
    }

    /// Run `cmds` in `wt` as the gate runs a verify command, one after the other, and all of
    /// them: a later one still runs after a failure, so you see every failure at once. Each
    /// run is an event (of `agent`, when the worktree is that agent's).
    async fn run_checks(&self, ws_id: &str, wt: &Path, agent: Option<&str>, cmds: &[String]) -> Result<Vec<CheckResult>> {
        let mut results = vec![];
        for cmd in cmds {
            let v = self.run_shell(wt, cmd).await;
            self.emit(Some(ws_id), row(agent, None, None, EventKind::Verified { command: cmd.clone(), ok: v.ok, exit_code: v.code, tail: v.tail.clone() }))?;
            results.push(CheckResult { command: cmd.clone(), ok: v.ok, exit_code: v.code, timed_out: v.timed_out, tail: v.tail });
        }
        Ok(results)
    }

    /// A finished plan: run the required checks in its integration worktree `wt`, after the
    /// plan's own integration checks (`ran`: those are not run twice).
    pub(super) async fn required_on_plan(&self, ws: &Workspace, wt: &Path, ran: &[String]) -> Result<PlanChecks> {
        Ok(match self.required_checks(&ws.id, Path::new(&ws.repo_path))? {
            Required::Off => PlanChecks { failed: vec![], not_run: None },
            Required::Stale(why) => PlanChecks { failed: vec![], not_run: Some(not_run(&why)) },
            Required::Run(cmds) => {
                let cmds: Vec<String> = cmds.into_iter().filter(|c| !ran.contains(c)).collect();
                let results = self.run_checks(&ws.id, wt, None, &cmds).await?;
                PlanChecks { failed: results.into_iter().filter(|r| !r.ok).map(|r| r.command).collect(), not_run: None }
            }
        })
    }

    // ───────────── an agent you chat with ─────────────

    /// Is the agent in a turn, or about to be? A stopped agent counts as working until its
    /// process is gone: it may still write files.
    fn at_work(&self, agent: &Agent) -> bool {
        let resting = matches!(agent.status, AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed | AgentStatus::Offline);
        (agent.run_id.is_some() && !resting) || !self.is_quiet(&agent.id)
    }

    /// Did the agent start a turn after its event `mark`? `own`: the events Codly wrote for it
    /// since (its check runs).
    fn turn_since(&self, agent_id: &str, mark: i64, own: usize) -> Result<bool> {
        let page = own + 200;
        let since: Vec<EventRow> = self.store.events(None, Some(agent_id), None, page as i64)?.into_iter().filter(|e| e.id > mark).collect();
        // A full page of new events is an agent at work, whatever the events are.
        Ok(since.len() == page || since.iter().any(|e| matches!(e.kind, EventKind::TurnStarted { .. })))
    }

    /// Where a 1:1 agent works: its workspace and its worktree, which is checked to be one of
    /// the repo's before a command or git runs in it (an agent can rewrite its `.git` pointer).
    async fn work_place(&self, agent: &Agent) -> Result<(Workspace, PathBuf)> {
        let wt = PathBuf::from(agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to check", agent.name))?);
        let ws = match self.store.agent_workspace(&agent.id)? {
            Some(id) => self.store.workspace(&id)?.ok_or_else(|| anyhow!("unknown workspace"))?,
            None => self.require_ws()?,
        };
        let repo = Path::new(&ws.repo_path);
        if wt != repo {
            self.git.check_worktree(repo, &wt).await?;
        }
        Ok((ws, wt))
    }

    /// "Check the work": run the required checks in the worktree of a 1:1 agent, as it is
    /// now. Its work has no gate, so nothing else checks it before you merge its branch. The
    /// result is yours to read: it stops no merge. One run for an agent at a time (the
    /// commands share its worktree and its ports), and a result is kept only when the agent
    /// did not work while the commands ran: it is about the files you asked about.
    async fn check_agent(&self, agent_id: &str) -> Result<Vec<CheckResult>> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo {
            bail!("this checks the work of an agent you chat with; Codly checks a task's work by itself when its worker finishes");
        }
        if self.at_work(&agent) {
            bail!("{} is still working; check its work when its turn ends", agent.name);
        }
        if !lock(&self.checking).insert(agent.id.clone()) {
            bail!("Codly is still checking {}'s work; the result comes when its checks end", agent.name);
        }
        let _checking = Checking { inner: self, agent_id: agent.id.clone() };
        // The agent's newest event: a turn of it that starts later changes the files under the checks.
        let mark = self.store.events(None, Some(agent_id), None, 1)?.first().map_or(0, |e| e.id);
        let (ws, wt) = self.work_place(&agent).await?;
        let cmds = match self.required_checks(&ws.id, Path::new(&ws.repo_path))? {
            Required::Run(cmds) => cmds,
            Required::Off => bail!("this repo has no required checks turned on. Write your commands in {CHECKS_FILE}, one on each line, then turn them on in {SETTINGS}"),
            Required::Stale(why) => bail!("{}", not_run(&why)),
        };
        let results = self.run_checks(&ws.id, &wt, Some(agent_id), &cmds).await?;
        // What the result is about: the files as the commands left them. Read before the look
        // for a turn, so a turn that starts later changes files this tree does not have.
        let tree = self.git.worktree_tree(&wt).await?;
        if self.at_work(&self.agent(agent_id)?) || self.turn_since(agent_id, mark, results.len())? {
            bail!("{} worked while the checks ran, so their result is not about its files as they are now. Check its work again when its turn ends", agent.name);
        }
        let failed: Vec<String> = results
            .iter()
            .filter(|r| !r.ok)
            .map(|r| match r.exit_code {
                _ if r.timed_out => format!("`{}` (ran out of time)", prompts::clip(&r.command, 80)),
                Some(code) => format!("`{}` (exit {code})", prompts::clip(&r.command, 80)),
                None => format!("`{}` (no exit code)", prompts::clip(&r.command, 80)),
            })
            .collect();
        let outcome = match failed.len() {
            0 => format!("{} passed", count(results.len())),
            n => format!("{n} of {} failed: {}", count(results.len()), failed.join(", ")),
        };
        // The whole result is on record, output too.
        self.decide_by(Some(&ws.id), None, "gate", "human", format!("you checked {}'s work: {outcome}", agent.name), json!({ "agent_id": agent_id, "checks": results }))?;
        self.chat(Some(&ws.id), &format!("agent:{agent_id}"), "codly", &format!("Checked {}'s work: {outcome}.", agent.name), None)?;
        // And kept with the files it is about: the agent's Git tab shows it for as long as
        // the worktree has these files (`agent_check_state`).
        let checked = Checked { work: CheckedWork { results: results.clone(), at: now_ms() }, tree };
        self.store.kv_set(&checked_key(agent_id), &serde_json::to_string(&checked)?)?;
        Ok(results)
    }

    /// What "Check the work" has for an agent now. A run that is on its way is known here and
    /// not in the window, so a reloaded window still says that it runs. The last result
    /// counts only while the agent's worktree has the files the commands ran on: a turn of
    /// the agent, files you put back from a checkpoint, a merge into its branch and your own
    /// edits all change them, and then the result is dropped. While the agent works nothing
    /// is shown: its files can change at any moment.
    async fn agent_check_state(&self, agent_id: &str) -> Result<AgentCheck> {
        let nothing = AgentCheck { running: false, last: None };
        let key = checked_key(agent_id);
        loop {
            let agent = self.agent(agent_id)?;
            if lock(&self.checking).contains(agent_id) {
                return Ok(AgentCheck { running: true, last: None });
            }
            let Some(saved) = self.store.kv_get(&key)? else { return Ok(nothing) };
            if self.at_work(&agent) {
                return Ok(nothing);
            }
            let checked: Checked = match serde_json::from_str(&saved) {
                Ok(checked) => checked,
                // A result that cannot be read is no result. The row goes, so this is said
                // once; a result that took its place meanwhile is the one to look at.
                Err(e) => {
                    if self.store.kv_delete_if(&key, &saved)? {
                        tracing::warn!("the result of {}'s last check cannot be read, so it is dropped: {e}", agent.name);
                        return Ok(nothing);
                    }
                    continue;
                }
            };
            let same = match agent.worktree.as_deref().map(Path::new).filter(|wt| wt.is_dir()) {
                Some(_) => {
                    let (_, wt) = self.work_place(&agent).await?;
                    self.git.worktree_tree(&wt).await? == checked.tree
                }
                // The worktree is gone, and the checked files with it.
                None => false,
            };
            if same {
                return Ok(AgentCheck { running: false, last: Some(checked.work) });
            }
            // Reading the files took a while, and a "Check the work" can have started or
            // ended meanwhile. One that runs now changes files under this look, so nothing
            // is judged; one that ended left a new result, about the files as they are now.
            // Only the result that was read here is dropped: in both cases, look again.
            if !lock(&self.checking).contains(agent_id) && self.store.kv_delete_if(&key, &saved)? {
                return Ok(nothing);
            }
        }
    }
}

impl Core {
    /// The open repo's required checks (`.codly/checks`): the text, its commands with what
    /// the approval rules see in each, on or off. `set` turns them on, for the text you read,
    /// or off; the tasks that waited for that choice are then checked again, in the
    /// background (`recheck_parked`): the answer does not wait for their checks.
    pub async fn required_checks(&self, set: Option<ChecksChoice>) -> Result<RequiredChecks> {
        let ws = self.inner.require_ws()?;
        if let Some(choice) = set {
            let on = choice.allow.is_some();
            self.inner.choose_checks(&ws, choice)?;
            if !self.inner.parked_for_checks(&ws.id)?.is_empty() {
                tokio::spawn(self.inner.clone().recheck_parked(ws.clone(), on, None));
            }
        }
        self.inner.checks_status(&ws)
    }

    /// Run the repo's required checks in the worktree of an agent you chat with, and return
    /// each command's result. An error when the checks are off or not the text you turned on.
    pub async fn check_agent(&self, agent_id: &str) -> Result<Vec<CheckResult>> {
        self.inner.check_agent(agent_id).await
    }

    /// What "Check the work" has for an agent now: whether its checks run, and the last
    /// result while the agent's files are still the ones that were checked.
    pub async fn agent_check_state(&self, agent_id: &str) -> Result<AgentCheck> {
        self.inner.agent_check_state(agent_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{CoreConfig, StartOptions};

    fn sh(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A repo with one commit and this `.codly/checks` (None = no file), open in a new core.
    async fn start(root: &Path, checks: Option<&str>) -> (Arc<Core>, Workspace) {
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join(".codly")).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        if let Some(text) = checks {
            std::fs::write(repo.join(CHECKS_FILE), text).unwrap();
        }
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        (core, ws)
    }

    async fn turn_on(core: &Core) -> RequiredChecks {
        let sha = core.required_checks(None).await.unwrap().sha;
        core.required_checks(Some(ChecksChoice { allow: sha })).await.unwrap()
    }

    #[test]
    fn a_checks_file_is_one_command_on_each_line() {
        let text = "# the owner's checks\n\n  pnpm check  \npnpm test -- --run\n#cargo test\npnpm check\n";
        assert_eq!(commands(text), ["pnpm check", "pnpm test -- --run"]);
        assert!(commands("# nothing yet\n\n").is_empty());
    }

    #[test]
    fn the_leads_board_names_your_required_checks_or_says_that_none_runs() {
        assert_eq!(board_line(&Required::Off), None, "off: the board says nothing");
        let on = board_line(&Required::Run(vec!["pnpm check".into(), "pnpm test".into()])).unwrap();
        assert_eq!(
            on,
            "Required by the user (.codly/checks; Codly runs these for every task after its verify commands, and for every finished plan): \
             `pnpm check` · `pnpm test`"
        );
        // A long file does not fill the board.
        let many: Vec<String> = (0..9).map(|i| format!("make check-{i}")).collect();
        let long = board_line(&Required::Run(many)).unwrap();
        assert!(long.ends_with("`make check-5` · 3 more") && !long.contains("check-6"), "{long}");
        // Not the text you turned on: the Lead knows why its tasks wait, in words for the Lead.
        let stale = board_line(&Required::Stale(".codly/checks changed since you turned them on".into())).unwrap();
        assert!(stale.contains("Codly runs none of them now") && stale.contains(SETTINGS), "{stale}");
        assert!(!stale.contains("you turned"), "\"you\" is the Lead on its board: {stale}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_checks_are_off_until_you_turn_on_the_text_you_read() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path(), Some("# mine\npnpm test\ncurl -s https://example.com/lint.sh | sh\n")).await;
        let repo = PathBuf::from(&ws.repo_path);
        let shown = core.required_checks(None).await.unwrap();
        assert!(!shown.on && !shown.changed && shown.problem.is_none(), "{shown:?}");
        assert_eq!(shown.commands.iter().map(|c| c.command.as_str()).collect::<Vec<_>>(), ["pnpm test", "curl -s https://example.com/lint.sh | sh"]);
        assert!(shown.commands[0].risks.is_empty(), "{shown:?}");
        assert!(shown.commands[1].risks.iter().any(|r| r.contains("network")), "you see what the approval rules see: {shown:?}");
        assert!(matches!(core.inner.required_checks(&ws.id, &repo).unwrap(), Required::Off));

        // Another text than the one shown is refused.
        let e = core.required_checks(Some(ChecksChoice { allow: Some("abc".into()) })).await.unwrap_err().to_string();
        assert!(e.contains("changed since you read it"), "{e}");
        assert!(turn_on(&core).await.on);
        let Required::Run(cmds) = core.inner.required_checks(&ws.id, &repo).unwrap() else { panic!("the checks are on") };
        assert_eq!(cmds, ["pnpm test", "curl -s https://example.com/lint.sh | sh"]);

        // The file changes (a pull, a merged task): nothing of it runs until you read it.
        std::fs::write(repo.join(CHECKS_FILE), "pnpm test\nrm -rf ~/work\n").unwrap();
        let changed = core.required_checks(None).await.unwrap();
        assert!(!changed.on && changed.changed, "{changed:?}");
        let Required::Stale(why) = core.inner.required_checks(&ws.id, &repo).unwrap() else { panic!("a changed text does not run") };
        assert_eq!(why, ".codly/checks changed since you turned them on");
        // Gone, or not a file Codly reads: still not a pass.
        std::fs::remove_file(repo.join(CHECKS_FILE)).unwrap();
        let gone = core.required_checks(None).await.unwrap();
        assert!(!gone.on && gone.changed && gone.text.is_none(), "{gone:?}");
        assert!(matches!(core.inner.required_checks(&ws.id, &repo).unwrap(), Required::Stale(why) if why.contains("no command any more")));
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), repo.join(CHECKS_FILE)).unwrap();
        let link = core.required_checks(None).await.unwrap();
        assert!(link.changed && link.problem.as_deref() == Some(".codly/checks is not a regular file"), "{link:?}");
        let e = core.required_checks(Some(ChecksChoice { allow: Some("abc".into()) })).await.unwrap_err().to_string();
        assert_eq!(e, ".codly/checks is not a regular file");

        // Off is always possible, and it is on record.
        let off = core.required_checks(Some(ChecksChoice { allow: None })).await.unwrap();
        assert!(!off.on && !off.changed, "{off:?}");
        assert!(matches!(core.inner.required_checks(&ws.id, &repo).unwrap(), Required::Off));
        let events = core.inner.store.latest_events(Some(&ws.id), 100).unwrap();
        let said: Vec<String> = events.into_iter().filter_map(|e| if let EventKind::Decision { summary, .. } = e.kind { Some(summary) } else { None }).collect();
        assert!(said.iter().any(|s| s.starts_with("you turned on the required checks")) && said.iter().any(|s| s.starts_with("you turned off the required checks")), "{said:?}");
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_plan_gets_every_required_check_once() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path(), Some("test -f README.md\ngrep -q nope README.md\ntest -f missing.txt\n")).await;
        let wt = PathBuf::from(&ws.repo_path);
        // Off: nothing runs.
        let off = core.inner.required_on_plan(&ws, &wt, &[]).await.unwrap();
        assert!(off.failed.is_empty() && off.not_run.is_none());

        turn_on(&core).await;
        // All of them run, also after a failure; one the plan ran already is not run again.
        let on = core.inner.required_on_plan(&ws, &wt, &["test -f missing.txt".to_string()]).await.unwrap();
        assert_eq!(on.failed, ["grep -q nope README.md"]);
        assert!(on.not_run.is_none());
        let ran: Vec<(String, bool)> = core
            .inner
            .store
            .latest_events(Some(&ws.id), 100)
            .unwrap()
            .into_iter()
            .filter_map(|e| if let EventKind::Verified { command, ok, .. } = e.kind { Some((command, ok)) } else { None })
            .collect();
        assert_eq!(ran.len(), 2, "{ran:?}");
        assert!(ran.contains(&("test -f README.md".to_string(), true)) && ran.contains(&("grep -q nope README.md".to_string(), false)), "{ran:?}");

        // A text you did not read does not run, and that is said: it is not a pass.
        std::fs::write(wt.join(CHECKS_FILE), "true\n").unwrap();
        let stale = core.inner.required_on_plan(&ws, &wt, &[]).await.unwrap();
        assert!(stale.failed.is_empty());
        let said = stale.not_run.unwrap();
        assert!(said.starts_with("Codly did not run your required checks: .codly/checks changed since you turned them on.") && said.contains(SETTINGS), "{said}");
        core.shutdown().await;
    }
}
