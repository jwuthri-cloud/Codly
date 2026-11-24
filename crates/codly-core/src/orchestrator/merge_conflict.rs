//! The last step, merging finished work into the user's branch, is never a dead end.
//!
//! - `merge_state` says what stands between a plan's branch (or a solo agent's) and the
//!   checked-out branch: the files that conflict, read with `git merge-tree` (nothing is
//!   touched), and the plan's checks that failed on the result of this merge.
//! - `resolve_merge_conflict` has an agent resolve a conflict. Codly brings the user's branch
//!   into the Codly branch inside Codly's own worktree, never in the user's checkout. For a
//!   plan, one more task of the plan resolves the conflicts in the plan's integration worktree
//!   and goes through the gate with the plan's verify commands; the plan then finishes again
//!   (its integration checks run on the result) and can be merged. For a solo agent, the agent
//!   itself resolves them in its worktree, and Codly commits the merge when its turn ends.
//! - `check_merge_result` runs the plan's verify commands on the result of the merge, in a
//!   scratch worktree, before the user's branch moves. Only when that result is new: the user's
//!   branch has commits the plan was never checked with. When a check fails there and nothing
//!   conflicts, `resolve_merge_conflict` gives the plan one more task that makes the checks
//!   pass, on the merge Codly committed in the plan's worktree.
//!
//! The task that works on the plan's own branch is reviewed like every task, but its reviewer
//! reads only what was done by hand (`resolution_diff`): each conflict as git left it against
//! how it was resolved, and the commits after the merge, not the user's own commits that the
//! merge brought in. The checks its gate ran on the plan's branch as it is are not run a second
//! time when the plan finishes again (`checked_in_place`). Its write scope is checked on the
//! same files (`changed_by_hand`): what git merged by itself is the user's work, not the task's.
//!
//! A merge is only ever committed by Codly, and never while a file still has conflict markers.
//! A resolution that does not finish leaves nothing behind: the plan's branch goes back to
//! where it was and the new files its worker left go (`settle_resolution`), and a solo agent's
//! merge can be stopped (`stop_resolving_merge`). Nor does one that could not start: a step
//! that fails after Codly began the merge takes the merge back (`take_back_start`), so the
//! user's branch is never in the plan's branch with no task on it and no check of the result.
//! The start is on record before the merge (`MergeStart`), so one that Codly was closed in
//! the middle of is taken back too, before the plan is merged, finished or resolved again.
//!
//! What was in the plan's worktree before such a task started, and is on no branch (a check's
//! output, what worktree setup left), is not the task's work: the gate's commit of the task
//! leaves it out (`there_before`) and the task's record says so (`note_there_before`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::pull_request::PrSource;
use super::{Core, Inner, gate, prompts, row};
use crate::git::{LeftOut, MergeOutcome, name_list};
use crate::model::*;

/// One merge check at a time: they share one scratch worktree per office.
static MERGE_CHECK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The key, in a decision's detail, of the branch a resolving task brings into its plan.
const BRINGS_IN: &str = "brings_in";

/// What stands between a finished plan's branch (or a solo agent's) and the checked-out branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MergeState {
    /// The checked-out branch the merge would go into.
    pub target: String,
    /// The Codly branch that would be merged: the plan's, or the solo agent's.
    pub branch: String,
    /// The project folder: where the user runs git to resolve a conflict by hand.
    pub repo: String,
    /// Files both branches changed in a way git cannot combine. Empty: it merges cleanly.
    pub conflicts: Vec<String>,
    /// Who works on it now: the plan's task that resolves the conflicts (or makes the plan's
    /// checks pass on the merge), while it is not finished; the solo agent, while the merge
    /// Codly started is open in its worktree.
    pub resolving: Option<Id>,
    /// The plan's checks that failed on the result of this very merge the last time it was
    /// tried (the merge was not made). Empty once either branch moved.
    pub failed_checks: Vec<String>,
    /// Why the plan's checks could not run on the result of this very merge the last time it
    /// was tried (the worktree for them could not be set up; the merge was not made). None
    /// once either branch moved.
    pub checks_not_run: Option<String>,
    /// How many commits the checked-out branch has that the Codly branch was not built on: it
    /// is behind by that many, and the merge brings the two together. 0 when the Codly branch
    /// has nothing to merge: it is merged already, or it has no commit of its own.
    pub behind: u32,
}

/// The plan's checks that failed on the result of merging `tip` into `head`, or why they could
/// not run on it (kv `merge_check:<plan>`): what the plan's card shows until one of the two
/// moves.
#[derive(Serialize, Deserialize)]
struct FailedChecks {
    head: String,
    tip: String,
    failed: Vec<String>,
    #[serde(default)]
    not_run: Option<String>,
}

fn failed_key(plan_id: &str) -> String {
    format!("merge_check:{plan_id}")
}

/// Where a plan stood when a task began to resolve its conflict with the user's branch (kv
/// `resolution:<plan>`): what a resolution that does not finish puts back.
#[derive(Serialize, Deserialize)]
struct Resolution {
    task: Id,
    /// The plan's branch before the user's branch was brought in.
    tip: String,
    base_commit: Option<String>,
    base_branch: Option<String>,
    /// The new (untracked) paths that were in the plan's worktree before the task started, as
    /// `Git::untracked` lists them: what worktree setup and the checks left there. A resolution
    /// that does not finish removes the new paths that are not in this list. None in a record
    /// from before Codly kept the list: then no file is removed.
    #[serde(default)]
    untracked: Option<Vec<String>>,
}

fn resolution_key(plan_id: &str) -> String {
    format!("resolution:{plan_id}")
}

impl Resolution {
    /// The record of where `plan` stands when `task` starts on its own branch, at `before`.
    fn of(plan: &Plan, task: &Task, before: &str, untracked: Vec<String>) -> Self {
        Resolution {
            task: task.id.clone(),
            tip: before.into(),
            base_commit: plan.base_commit.clone(),
            base_branch: plan.base_branch.clone(),
            untracked: Some(untracked),
        }
    }
}

/// What `add_in_place` writes, in the caller's store transaction (`review_back.rs`): the
/// record of where the plan stands, the new task, and the commit its work is measured from. A
/// caller that writes more for the task in the same transaction leaves none of it when one
/// write fails: a record without its task, or for a task that never started, would make a
/// later `settle_resolution` take back work that never ran. Returns the task's event, to
/// publish after the commit.
pub(super) fn add_in_place_tx(tx: &rusqlite::Connection, ws_id: Option<&str>, plan: &Plan, task: &Task, before: &str, untracked: Vec<String>) -> Result<EventRow> {
    let undo = serde_json::to_string(&Resolution::of(plan, task, before, untracked))?;
    tx.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [resolution_key(&plan.id), undo],
    )?;
    let mut ev = row(None, Some(&task.id), None, EventKind::TaskChanged { task: task.clone() });
    ev.id = crate::store::insert_event(tx, ws_id, &ev)?;
    // What the task changes is measured from the plan's branch as it was.
    tx.execute("UPDATE task SET base_commit = ?2 WHERE id = ?1", [task.id.as_str(), before])?;
    Ok(ev)
}

/// A resolution that Codly starts (kv `merge_start:<plan>`): written before the user's branch
/// is brought into the plan's branch, removed when the plan runs again. One that is still
/// there while the plan does not build on `head` was cut short: the merge may be on the
/// plan's branch with no task on it. `settle_resolution` takes it back.
#[derive(Serialize, Deserialize)]
struct MergeStart {
    /// The plan's branch before the user's branch was brought in.
    tip: String,
    /// The user's branch.
    target: String,
    /// The commit of the user's branch: the plan builds on it once the start is complete.
    head: String,
    /// The run of Codly that began it (`THIS_RUN`).
    run: Id,
}

fn start_key(plan_id: &str) -> String {
    format!("merge_start:{plan_id}")
}

/// In the detail of the line that says a start was taken back: the start's record as the
/// store kept it, so a later `settle_resolution` that finds the record does not say it twice.
const TAKEN_BACK: &str = "taken_back";

/// This run of Codly. A start on record from another run was cut short when Codly closed;
/// one from this run, when a step of it failed and its record could not be removed.
static THIS_RUN: LazyLock<Id> = LazyLock::new(new_id);

/// Why Codly's commit of a task on its plan's own branch leaves a path out, in the record.
const THERE_BEFORE: &str = "was there before the task";

/// The plan's checks as they ran on the result of merging it, before the user's branch moved.
pub(super) struct MergeChecked {
    /// The user's branch and the plan's branch as they were checked: the merge is made only if
    /// neither moved since.
    pub head: String,
    pub tip: String,
    passed: Vec<String>,
    /// Not run: the approval rules see a risk in them, and nobody allowed them.
    refused: Vec<HeldCheck>,
}

impl MergeChecked {
    /// What ran, for the line that says the plan is merged.
    pub(super) fn said(&self, target: &str) -> String {
        let mut s = format!(" {target} had changed since the plan was checked.");
        if !self.passed.is_empty() {
            s.push_str(&format!(" Codly ran the plan's checks on the merge result first: {} passed.", code_list(&self.passed)));
        }
        if !self.refused.is_empty() {
            let list: Vec<String> = self.refused.iter().map(|h| format!("`{}` ({})", h.command, h.risks.join(", "))).collect();
            s.push_str(&format!(" Not run on the merge result, because they need your OK: {}.", list.join(", ")));
        }
        s
    }

    pub(super) fn detail(&self) -> serde_json::Value {
        json!({ "passed": self.passed, "refused": self.refused })
    }
}

fn code_list(cmds: &[String]) -> String {
    cmds.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ")
}

/// "a.rs still has conflict markers".
fn markers_left(files: &[String]) -> String {
    format!("{} still {} conflict markers", name_list(files), if files.len() == 1 { "has" } else { "have" })
}

/// The user's branch as git reads it, whatever tag or other ref shares its name.
fn branch_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// What a merge into the user's branch says when it conflicts. `who` can resolve it: "an agent
/// (on the plan's card)", or the solo agent by name.
pub(super) fn conflict_error(target: &str, branch: &str, files: &[String], who: &str) -> String {
    format!(
        "merging into {target} conflicts in {}; the merge was aborted and your branch is unchanged. \
         {who} can resolve it, or you can do it yourself: git merge {branch}",
        files.join(", ")
    )
}

/// A task that resolves a conflict with the user's branch works in its plan's integration
/// worktree itself, on the integration branch: its branch is the plan's. Returns that branch
/// and worktree; None for every other task.
pub(super) fn in_place(task: &Task, plan: &Plan) -> Option<(String, PathBuf)> {
    let branch = plan.integration_branch.as_ref().filter(|b| task.branch.as_ref() == Some(*b))?;
    Some((branch.clone(), PathBuf::from(plan.integration_worktree.as_ref()?)))
}

/// The key of a task that resolves a conflict with the user's branch, and of one that makes
/// the plan's checks pass on the merge with it.
const RESOLVE: &str = "resolve";
const FIX_CHECKS: &str = "fix-checks";
/// The key of a task that makes the changes the user asked for in a review of the finished
/// plan (`review_back.rs`): no merge is made for it.
pub(super) const REVIEW: &str = "review";

/// "resolve", then "resolve-2"…: a plan can need this more than once.
pub(super) fn next_key(tasks: &[Task], stem: &str) -> String {
    let mut n = 1;
    loop {
        let key = if n == 1 { stem.to_string() } else { format!("{stem}-{n}") };
        if !tasks.iter().any(|t| t.key == key) {
            return key;
        }
        n += 1;
    }
}

/// Does this task make the plan's checks pass on a merge without a conflict (`fix_task`)?
/// Else, of the tasks that work on their plan's own branch, it resolves a conflict.
fn fixes_checks(task: &Task) -> bool {
    task.key.starts_with(FIX_CHECKS)
}

/// Does this task make the changes the user asked for in review (`review_back.rs`)?
pub(super) fn answers_review(task: &Task) -> bool {
    task.key.starts_with(REVIEW)
}

/// Why the gate does not review a task on its plan's own branch whose diff is empty: no merge
/// was made for the changes you asked for in review, so that task changed nothing at all.
pub(super) fn nothing_to_review(task: &Task) -> String {
    match answers_review(task) {
        true => format!("{}: no review, the task changed nothing", task.key),
        false => format!("{}: no review, nothing was changed by hand in the merge with your branch", task.key),
    }
}

/// What the reviewer of a task that worked on its plan's own branch reads after the brief: the
/// diff there is not the task's whole change. For a task that resolved a conflict:
const RESOLVE_REVIEW_NOTE: &str = "\n\nAbout the diff: this task worked on a merge of the plan's branch with the user's branch. \
     The diff shows only what the worker changed by hand, in the order Codly committed it. In the merge, the removed lines \
     from <<<<<<< to >>>>>>> are a conflict as git left it, with both sides, and the added lines are how the worker resolved \
     it. What git merged by itself is the user's own work: it is not in the diff and not for review. Check that each \
     resolution keeps what both sides changed.";

/// For a task that made the plan's checks pass: that merge had no conflict, so the note
/// speaks of none.
const FIX_REVIEW_NOTE: &str = "\n\nAbout the diff: this task worked on the plan's branch after Codly merged the user's branch into it. \
     There was no conflict: the diff shows only what the worker changed after that merge, in the order Codly committed it. \
     What the merge brought in is the user's own work: it is not in the diff and not for review. Check that the change \
     makes the plan's checks pass and keeps what both sides changed.";

/// For a task that made the changes the user asked for in review: no merge was made, so the
/// diff is the task's own commits.
const ASKED_REVIEW_NOTE: &str = "\n\nAbout the diff: this task worked on the plan's finished branch; the diff is what it changed there, \
     not the plan's whole work.";

/// The note for the reviewer of `task`, which worked on its plan's own branch.
pub(super) fn review_note(task: &Task) -> &'static str {
    if answers_review(task) {
        ASKED_REVIEW_NOTE
    } else if fixes_checks(task) {
        FIX_REVIEW_NOTE
    } else {
        RESOLVE_REVIEW_NOTE
    }
}

/// Why no agent takes the work that is left on a cancelled plan. `fix_checks`: that work is a
/// fix of the plan's checks, which failed on a merge without a conflict.
fn cancelled_refusal(fix_checks: bool) -> String {
    let left = if fix_checks { "fix the checks" } else { "resolve the conflict" };
    format!("this plan was cancelled, so no agent works on it any more; {left} yourself")
}

/// What both briefs say about the new files that were in the plan's worktree before the task.
pub(super) const NOT_COMMITTED: &str = "Files that were in this worktree before you started and are on no branch are not committed.";

fn resolve_goal(target: &str, files: &[String]) -> String {
    format!(
        "The plan is finished, but {target} (the user's branch) changed the same lines meanwhile. Codly started merging \
         {target} into this worktree, which holds the plan's finished work. These files have conflict markers \
         (<<<<<<< ======= >>>>>>>): {}.\n\
         Resolve every conflict so that the plan's work and the changes on {target} are both kept, and remove all markers. \
         Then make sure the project still builds and the verify commands pass.\n\
         Your write scope is the files with conflicts. If the two sides only work together after a change in another file, \
         make that change and name the file in scope_exceptions in your report.\n\
         Do not run git commands that change the repository (commit, merge, checkout, reset, stash): Codly commits the \
         merge when you report. {NOT_COMMITTED}",
        files.join(", ")
    )
}

/// The task that resolves a plan's conflicts with `target`, in the plan's own worktree.
fn resolve_task(plan: &Plan, key: String, target: &str, files: &[String], verify: Vec<String>, branch: &str, wt: &Path) -> Task {
    let now = now_ms();
    Task {
        id: new_id(),
        plan_id: Some(plan.id.clone()),
        key,
        title: format!("Resolve conflicts with {target}"),
        goal: resolve_goal(target, files),
        acceptance: vec![
            "No conflict marker (<<<<<<<, =======, >>>>>>>) is left in any file".into(),
            format!("The plan's work and the changes on {target} are both kept and work together"),
        ],
        write_scope: files.to_vec(),
        verify,
        depends_on: vec![],
        status: TaskStatus::Pending,
        assignee: None,
        agent_hint: None,
        account_hint: None,
        consultant: None,
        // It works in the plan's worktree: a team member's own conversation lives in another.
        fresh_session: true,
        branch: Some(branch.into()),
        worktree: Some(wt.to_string_lossy().into_owned()),
        attempt: 0,
        status_reason: None,
        summary: None,
        created_at: now,
        updated_at: now,
    }
}

fn fix_goal(target: &str, failed: &[String]) -> String {
    format!(
        "The plan is finished, and {target} (the user's branch) got new commits meanwhile. The two merge without a conflict, \
         but on the result of that merge these checks fail: {}. Codly merged {target} into this worktree, which holds the \
         plan's finished work, and committed the merge.\n\
         Find out why the checks fail now. Fix it so that the plan's work and the changes on {target} are both kept and work \
         together. Then make sure the verify commands pass.\n\
         Your write scope is the files the plan changed. If the fix needs a change in another file, make that change and \
         name the file in scope_exceptions in your report.\n\
         Do not run git commands that change the repository (commit, merge, checkout, reset, stash): Codly commits your work \
         when you report. {NOT_COMMITTED}",
        code_list(failed)
    )
}

/// The task that makes the plan's checks pass on the merge with `target`, in the plan's own
/// worktree: like the task that resolves a conflict. `scope`: the files the plan changed.
#[allow(clippy::too_many_arguments)]
fn fix_task(plan: &Plan, key: String, target: &str, failed: &[String], scope: &[String], verify: Vec<String>, branch: &str, wt: &Path) -> Task {
    let mut acceptance: Vec<String> = failed.iter().map(|c| format!("`{c}` passes")).collect();
    acceptance.push(format!("The plan's work and the changes on {target} are both kept and work together"));
    Task {
        title: format!("Fix the checks with {target} merged in"),
        goal: fix_goal(target, failed),
        acceptance,
        ..resolve_task(plan, key, target, scope, verify, branch, wt)
    }
}

/// The files of `changed` that are outside the write scope of a task that works on its plan's
/// own branch. That scope is a list of file names as git gives them (`resolve_task`,
/// `fix_task`), not of globs: a name like `routes/[id]/page.tsx` is compared as text. As in
/// every write scope, a name stands for that path and what is under it, and an empty scope is
/// unrestricted.
pub(super) fn outside_scope(scope: &[String], changed: &[String]) -> Vec<String> {
    let under = |file: &str, name: &str| file.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    let within = |file: &str| scope.is_empty() || scope.iter().any(|name| under(file, name.as_str()));
    changed.iter().filter(|file| !within(file.as_str())).cloned().collect()
}

/// Are two new paths of a worktree, as `Git::untracked` lists them (a folder that is new as
/// a whole is one entry ending in `/`), the same place: the same path, or one is a folder
/// that holds the other?
fn same_place(a: &str, b: &str) -> bool {
    a == b || (a.ends_with('/') && b.starts_with(a)) || (b.ends_with('/') && a.starts_with(b))
}

/// Remove what is new in one of Codly's own worktrees: the untracked paths `now` that were not
/// there `before` (both as `Git::untracked` lists them). A path under a folder that was there
/// before stays, and so does a folder that holds a path from before. Returns what was removed.
fn remove_new(wt: &Path, now: &[String], before: &[String]) -> std::io::Result<Vec<String>> {
    let was_there = |path: &str| before.iter().any(|b| same_place(b, path));
    let mut removed = vec![];
    for path in now.iter().filter(|p| !was_there(p)) {
        let full = wt.join(path);
        // A link is removed, never followed.
        match std::fs::symlink_metadata(&full) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&full)?,
            Ok(_) => std::fs::remove_file(&full)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
        removed.push(path.clone());
    }
    Ok(removed)
}

/// What a solo agent is asked, in its own conversation, when the user has it resolve a conflict.
fn resolve_request(target: &str, files: &[String]) -> String {
    format!(
        "Please resolve the merge conflicts with {target}. Codly started merging {target} into your worktree, and these \
         files have conflict markers (<<<<<<< ======= >>>>>>>): {}. Resolve every conflict so that your work and the \
         changes on {target} are both kept, and remove all markers. Do not run git commands that change the repository \
         (commit, merge, checkout, reset, stash): Codly commits the merge when your turn ends.",
        files.join(", ")
    )
}

impl Inner {
    async fn checked_out(&self, repo: &Path) -> Result<String> {
        self.git.current_branch(repo).await?.ok_or_else(|| anyhow!("your checkout is on a detached HEAD; switch to a branch first"))
    }

    /// The office and the Codly branch of a plan or a solo agent.
    async fn merge_side(&self, source: &PrSource) -> Result<(Workspace, String)> {
        match source {
            PrSource::Plan(id) => {
                let plan = self.plan(id)?;
                let branch = plan.integration_branch.ok_or_else(|| anyhow!("the plan never ran, so there is nothing to merge"))?;
                let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
                Ok((ws, branch))
            }
            PrSource::Agent(id) => {
                let (agent, ws, wt) = self.solo_side(id).await?;
                let branch = self.git.current_branch(&wt).await?.ok_or_else(|| anyhow!("{}'s worktree is on a detached HEAD", agent.name))?;
                Ok((ws, branch))
            }
        }
    }

    /// A solo agent, its office and its own worktree.
    async fn solo_side(&self, agent_id: &str) -> Result<(Agent, Workspace, PathBuf)> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo {
            bail!("only a solo agent has a branch of its own; a task's work merges with its plan");
        }
        let wt = PathBuf::from(agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to merge", agent.name))?);
        let ws_id = self.store.agent_workspace(agent_id)?.ok_or_else(|| anyhow!("{} is in no office", agent.name))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        if wt == Path::new(&ws.repo_path) {
            bail!("{} works in your project folder itself, so it has no branch of its own to merge", agent.name);
        }
        Ok((agent, ws, wt))
    }

    pub(super) async fn merge_state(&self, source: &PrSource) -> Result<MergeState> {
        let (ws, branch) = self.merge_side(source).await?;
        let repo = PathBuf::from(&ws.repo_path);
        let target = self.checked_out(&repo).await?;
        let head = self.git.head(&repo).await?;
        let tip = self.git.rev_parse(&repo, &branch).await?;
        let (conflicts, behind) = match &tip {
            // Everything on the branch is in the checked-out one already (it was merged, or it
            // has no commit of its own): there is no merge to make, so it is not behind.
            Some(tip) if self.git.is_ancestor(&repo, tip, "HEAD").await? => (vec![], 0),
            Some(tip) => (self.git.merge_tree_check(&repo, "HEAD", tip).await?.conflicts, self.git.commits_ahead(&repo, tip, "HEAD").await?),
            // The branch was deleted: there is nothing to merge, so nothing conflicts.
            None => (vec![], 0),
        };
        let (resolving, failed_checks, checks_not_run) = match source {
            PrSource::Plan(id) => {
                let resolving = self.open_resolution(&self.plan(id)?)?.map(|t| t.id);
                let current = self.failed_checks(id, head.as_deref(), tip.as_deref())?;
                let (failed, not_run) = current.map(|f| (f.failed, f.not_run)).unwrap_or_default();
                (resolving, failed, not_run)
            }
            PrSource::Agent(id) => {
                let (_, _, wt) = self.solo_side(id).await?;
                (self.git.in_merge(&wt).await.then(|| id.clone()), vec![], None)
            }
        };
        Ok(MergeState { target, branch, repo: ws.repo_path, conflicts, resolving, failed_checks, checks_not_run, behind })
    }

    /// What the plan's checks said about the result of merging `tip` into `head`, the last
    /// time that merge was tried. None: they passed or never ran, or one of the two moved since.
    fn failed_checks(&self, plan_id: &str, head: Option<&str>, tip: Option<&str>) -> Result<Option<FailedChecks>> {
        let failed: Option<FailedChecks> = self.store.kv_get(&failed_key(plan_id))?.map(|v| serde_json::from_str(&v)).transpose()?;
        Ok(failed.filter(|f| Some(f.head.as_str()) == head && Some(f.tip.as_str()) == tip))
    }

    /// The plan's task that resolves a conflict with the user's branch (or makes the plan's
    /// checks pass on the merge with it), while it is not finished: until then a merge may be
    /// open in the plan's integration worktree.
    pub(super) fn open_resolution(&self, plan: &Plan) -> Result<Option<Task>> {
        let open = |t: &Task| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled) && in_place(t, plan).is_some();
        Ok(self.store.plan_tasks(&plan.id)?.into_iter().find(open))
    }

    /// `into`: the branch the user confirmed, for a plan that did not start from it.
    pub(super) async fn resolve_merge_conflict(self: &Arc<Self>, source: &PrSource, into: Option<&str>) -> Result<MergeState> {
        match source {
            PrSource::Plan(id) => self.resolve_plan_conflict(id, into).await?,
            PrSource::Agent(id) => self.resolve_agent_conflict(id).await?,
        }
        self.merge_state(source).await
    }

    /// The checked-out branch a resolution brings into the plan's branch. Like the merge
    /// itself, a branch the plan did not start from needs the user's word first (`into`): from
    /// then on the plan builds on that branch, and a merge of the plan brings all of it along.
    async fn resolve_target(&self, repo: &Path, plan: &Plan, into: Option<&str>) -> Result<String> {
        let target = self.checked_out(repo).await?;
        match into {
            Some(confirmed) if confirmed != target => bail!("your checkout is on {target} now, not {confirmed}; nothing was changed"),
            Some(_) => {}
            None => {
                if let Some(w) = self.other_branch_warning(repo, plan, &target).await? {
                    bail!("{} Nothing was changed: have it resolved again and confirm {target} to go on.", w.message);
                }
            }
        }
        Ok(target)
    }

    /// A resolution that did not finish leaves nothing in the plan's branch. The gate commits
    /// the merge of the user's branch before the task's checks run, so a resolution that then
    /// failed them (and was cancelled) is on the branch: the branch goes back to where it was
    /// before the user's branch was brought in, with the merge left open undone, and the plan
    /// builds again on what it built on before. A resolution whose task is done stays.
    /// The same for a start that was cut short (`MergeStart`: Codly was closed in the middle
    /// of it, or a step failed and so did its record): the merge may be on the plan's branch
    /// with no task on it, and a task that never started does not start later.
    /// Returns the plan as it is now. Call it holding `repo_lock`, when no task of the plan
    /// works: before the plan finishes, before another resolution, before the merge.
    pub(super) async fn settle_resolution(&self, plan: &Plan) -> Result<Plan> {
        let (key, start_key) = (resolution_key(&plan.id), start_key(&plan.id));
        let was: Option<Resolution> = self.store.kv_get(&key)?.map(|v| serde_json::from_str(&v)).transpose()?;
        let start_row = self.store.kv_get(&start_key)?;
        let start: Option<MergeStart> = start_row.as_deref().map(serde_json::from_str).transpose()?;
        if was.is_none() && start.is_none() {
            return Ok(plan.clone());
        }
        let forget: [(String, Option<String>); 2] = [(key, None), (start_key, None)];
        let task = match &was {
            Some(was) => self.store.task(&was.task)?,
            None => None,
        };
        // The last step of a start moves the plan onto the user's branch as it was then. A
        // start on record whose plan is not there did not get that far.
        let cut_short = start.filter(|s| plan.base_commit.as_deref() != Some(s.head.as_str()));
        let tip = match (&was, &cut_short) {
            (Some(_), _) if task.as_ref().is_some_and(|t| t.status == TaskStatus::Done) => None,
            (Some(was), _) => Some(was.tip.clone()),
            (None, Some(start)) => Some(start.tip.clone()),
            // A start that is complete, with no task: only its record was left.
            (None, None) => None,
        };
        let Some(tip) = tip else {
            self.store.kv_write(&forget)?;
            return Ok(plan.clone());
        };
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let branch = plan.integration_branch.clone().ok_or_else(|| anyhow!("the plan has no integration branch"))?;
        let wt = PathBuf::from(plan.integration_worktree.clone().ok_or_else(|| anyhow!("the plan has no integration worktree"))?);
        // The plan's worktree, back if a cleanup removed it: the branch moves in it.
        let fresh = !wt.exists();
        self.git.worktree_add(&repo, &wt, &branch, &branch).await?;
        if fresh {
            self.mark_new_worktree(&wt)?;
        }
        self.git.check_worktree(&repo, &wt).await?;
        let mut removed = vec![];
        if self.git.is_ancestor(&wt, &tip, "HEAD").await? {
            self.git.reset_to(&wt, &tip).await?;
            // The new files its worker left, which are on no branch, go too: the plan's checks
            // must not see them. What was new there before the task started stays: no commit
            // of the task took it in (`there_before`).
            if let Some(before) = was.as_ref().and_then(|w| w.untracked.as_ref()) {
                removed = remove_new(&wt, &self.git.untracked(&wt).await?, before)?;
            }
        } else {
            // Somebody else moved the branch since: only the open merge goes.
            self.git.abort_merge(&wt).await?;
        }
        let plan = match &was {
            Some(was) => self.update_plan(&plan.id, |p| {
                p.base_commit = was.base_commit.clone();
                p.base_branch = was.base_branch.clone();
                Ok(())
            })?,
            None => plan.clone(),
        };
        // Cut short before the plan ran again: its task never started. It does not start
        // later, on a merge that is gone.
        if cut_short.is_some()
            && let Some(t) = task.as_ref().filter(|t| t.status == TaskStatus::Pending)
        {
            self.update_task(&t.id, |t| {
                t.status = TaskStatus::Cancelled;
                t.status_reason = Some("Codly could not start it".into());
                Ok(())
            })?;
        }
        // A take-back that wrote its line and then could not remove the records of its start
        // (`take_back_start`): what happened is on record once already.
        let said = match (&cut_short, &start_row) {
            (Some(_), Some(row)) => self.start_taken_back(&plan.id, row)?,
            _ => false,
        };
        self.store.kv_write(&forget)?;
        if said {
            return Ok(plan);
        }
        let ws_id = Some(ws.id.as_str());
        let (summary, text) = match (&cut_short, task.as_ref().is_some_and(fixes_checks)) {
            (Some(start), _) => {
                // From another run of Codly: it was closed in the middle. From this run: a
                // step failed, and the record of the start could not be removed then.
                let when = if start.run == *THIS_RUN { "" } else { " when Codly closed" };
                let target = &start.target;
                (
                    format!("bringing {target} into {branch} did not finish{when}; {branch} is back where it was before it"),
                    format!(
                        "Bringing {target} into the plan's branch did not finish{when}. Codly took that merge back: the \
                         plan's branch is as its tasks left it."
                    ),
                )
            }
            // The task made the changes you asked for in review: no merge was made for it.
            (None, _) if task.as_ref().is_some_and(answers_review) => (
                format!("the changes you asked for in review did not finish: {branch} is back where it was before them"),
                "The changes you asked for in review were not finished. Codly took them back: the plan's branch is as its tasks left it."
                    .to_string(),
            ),
            // The task resolved a conflict, or it made the checks pass on a merge without one.
            (None, false) => (
                format!("the conflict resolution did not finish: {branch} is back where it was before it"),
                "The conflict with your branch was not resolved. Codly took the unfinished merge back: the plan's branch is as its tasks left it."
                    .to_string(),
            ),
            (None, true) => (
                format!("the fix of the plan's checks did not finish: {branch} is back where it was before it"),
                "The plan's checks still fail with your branch merged in. Codly took that merge back: the plan's branch is as its tasks left it."
                    .to_string(),
            ),
        };
        let on_task = task.as_ref().map(|t| t.id.as_str());
        self.decide(ws_id, on_task, "merge", summary, json!({ "plan_id": plan.id, "tip": tip, "removed": removed }))?;
        self.chat(ws_id, &plan.thread, "codly", &text, None)?;
        Ok(plan)
    }

    /// The plans with a start on record that was cut short (`MergeStart`: the plan does not
    /// build on the user's branch it brought in): their merge may be on the plan's branch with
    /// no task on it, until `settle_resolution` takes it back. A record that cannot be read,
    /// or whose plan is gone, is passed over.
    pub(super) fn starts_cut_short(&self) -> Result<Vec<Plan>> {
        let prefix = start_key("");
        let mut plans = vec![];
        for (key, row) in self.store.kv_prefixed(&prefix)? {
            let start: MergeStart = match serde_json::from_str(&row) {
                Ok(start) => start,
                Err(e) => {
                    tracing::warn!("the start of a conflict resolution in {key} cannot be read: {e}");
                    continue;
                }
            };
            if let Some(plan) = self.store.plan(&key[prefix.len()..])?
                && plan.base_commit.as_deref() != Some(start.head.as_str())
            {
                plans.push(plan);
            }
        }
        Ok(plans)
    }

    /// Is the take-back of this start (`row`, as the store keeps it) on record already?
    fn start_taken_back(&self, plan_id: &str, row: &str) -> Result<bool> {
        self.store.with(|db| {
            let sql = "SELECT EXISTS(SELECT 1 FROM decision WHERE plan_id = ?1 AND type = 'merge' AND json_extract(detail, ?2) = ?3)";
            Ok(db.query_row(sql, rusqlite::params![plan_id, format!("$.{TAKEN_BACK}"), row], |r| r.get(0))?)
        })
    }

    /// A plan an agent can still work on, and that is not in the user's branch yet.
    async fn resolvable_plan(&self, plan_id: &str) -> Result<Plan> {
        let plan = self.plan(plan_id)?;
        if plan.merged_sha.is_some() {
            bail!("this plan is already merged into your branch");
        }
        match plan.status {
            PlanStatus::Done | PlanStatus::Failed => Ok(plan),
            PlanStatus::Running | PlanStatus::Integrating => bail!("the plan is still running; wait until it is done"),
            PlanStatus::Cancelled => {
                // What is left to do is said in plain words also when it cannot be read (git
                // or the store fails; a project folder that is gone reads as no commit): then
                // it is called a conflict, and the error goes to the log.
                let fix_checks = self.checks_failed_on_merge(&plan).await.unwrap_or_else(|e| {
                    tracing::warn!("what is left to do on the cancelled plan {plan_id} was not read: {e:#}");
                    false
                });
                bail!("{}", cancelled_refusal(fix_checks))
            }
            PlanStatus::Planning | PlanStatus::AwaitingApproval => bail!("the plan never ran, so there is nothing to merge"),
        }
    }

    /// Did the plan's checks fail on the merge of its branch into the user's, as the two are
    /// now, the last time that merge was tried? Then what is left to do is a fix of the
    /// checks: that merge had no conflict.
    async fn checks_failed_on_merge(&self, plan: &Plan) -> Result<bool> {
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let head = self.git.head(&repo).await?;
        let tip = match plan.integration_branch.as_deref() {
            Some(branch) => self.git.rev_parse(&repo, branch).await?,
            None => None,
        };
        Ok(self.failed_checks(&plan.id, head.as_deref(), tip.as_deref())?.is_some_and(|f| !f.failed.is_empty()))
    }

    /// One more task of a finished plan starts on the plan's own branch, in the plan's
    /// worktree. If it does not finish, the plan goes back to where it is now
    /// (`settle_resolution`): its branch at `before`, and no new file but those in `untracked`.
    pub(super) fn add_in_place(&self, ws_id: Option<&str>, plan: &Plan, task: &Task, before: &str, untracked: Vec<String>) -> Result<()> {
        let undo = Resolution::of(plan, task, before, untracked);
        self.store.kv_set(&resolution_key(&plan.id), &serde_json::to_string(&undo)?)?;
        self.emit(ws_id, row(None, Some(&task.id), None, EventKind::TaskChanged { task: task.clone() }))?;
        // What the task changes is measured from the plan's branch as it was.
        self.store.set_task_commits(&task.id, Some(before), None)
    }

    /// The new paths that were in the plan's worktree before `task` started on the plan's own
    /// branch (`Resolution::untracked`, as `Git::untracked` lists them). They are on no
    /// branch and are not the task's work: the gate's commit of the task leaves them out, with
    /// what is under a folder among them. A file the worker makes is new after that list, and
    /// is committed like all its work. Empty for every other task.
    pub(super) fn there_before(&self, task: &Task) -> Result<Vec<String>> {
        let Some(plan_id) = task.plan_id.as_deref() else { return Ok(vec![]) };
        let was: Option<Resolution> = self.store.kv_get(&resolution_key(plan_id))?.map(|v| serde_json::from_str(&v)).transpose()?;
        Ok(was.filter(|w| w.task == task.id).and_then(|w| w.untracked).unwrap_or_default())
    }

    /// Say, in the task's record, which of those paths (`there_before`) the gate's commit in
    /// `wt` left out: the ones that are still new there. Like every path a commit of Codly's
    /// leaves out (`left_out.rs`), it is said once, and again when the paths change. A path
    /// that the commit left out for another reason too (`left_out`) is on record with that one.
    pub(super) async fn note_there_before(&self, ws: Option<&str>, task: &Task, wt: &Path, before: &[String], left_out: &[LeftOut]) -> Result<()> {
        if before.is_empty() {
            return Ok(());
        }
        let now = self.git.untracked(wt).await?;
        let said_already = |path: &str| left_out.iter().any(|l| same_place(l.path(), path));
        let left: Vec<&String> = before.iter().filter(|b| now.iter().any(|n| same_place(b.as_str(), n.as_str())) && !said_already(b.as_str())).collect();
        if left.is_empty() {
            return Ok(());
        }
        let paths = json!(left);
        let said = self.store.task_events(&task.id, "decision")?.into_iter().rev().find_map(|e| match e.kind {
            EventKind::Decision { detail, .. } if detail["reason"] == THERE_BEFORE => Some(detail["left_out"].clone()),
            _ => None,
        });
        if said.as_ref() == Some(&paths) {
            return Ok(());
        }
        let list = prompts::clip(&left.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "), 200);
        let summary = format!(
            "{}: Codly did not commit {list} ({THERE_BEFORE}). What was in the plan's worktree before the task started and is on no \
             branch is not the task's work. It is still in the worktree.",
            task.key
        );
        let detail = json!({ "plan_id": task.plan_id, "left_out": paths, "reason": THERE_BEFORE, "worktree": wt.to_string_lossy() });
        self.decide(ws, Some(&task.id), "commit", summary, detail).map(drop)
    }

    /// For a task that worked on its plan's own branch (it resolved a conflict with the user's
    /// branch, or made the checks pass on the merge with it): the diff its reviewer reads, which
    /// is what was changed by hand since `before` and not what the merge brought in from the
    /// user's branch (`Git::resolution_diff`). None for every other task.
    pub(super) async fn resolution_diff(&self, task: &Task, repo: &Path, before: &str) -> Result<Option<String>> {
        let Some(plan) = task.plan_id.as_deref().map(|p| self.plan(p)).transpose()? else { return Ok(None) };
        let Some((branch, _)) = in_place(task, &plan) else { return Ok(None) };
        self.git.resolution_diff(repo, before, &branch).await.map(Some)
    }

    /// For such a task, the files it changed by hand since `before`, all in all: what its
    /// write scope is checked on. These are the files of that diff, without those a later
    /// commit put back (a worker that takes an edit back after a scope round must pass the
    /// next one). What the merge brought in from the user's branch by itself is not among
    /// them. None for every other task.
    pub(super) async fn changed_by_hand(&self, task: &Task, repo: &Path, before: &str) -> Result<Option<Vec<String>>> {
        let Some(plan) = task.plan_id.as_deref().map(|p| self.plan(p)).transpose()? else { return Ok(None) };
        let Some((branch, _)) = in_place(task, &plan) else { return Ok(None) };
        let tip = branch.as_str();
        // The first commit since `before` is the merge with the user's branch.
        let commits = self.git.read(repo, &["rev-list", "--first-parent", "--reverse", &format!("{before}..{tip}"), "--"]).await?;
        let Some(merge) = commits.lines().next() else { return Ok(Some(vec![])) };
        let Some(theirs) = self.git.rev_parse(repo, &format!("{merge}^2")).await? else {
            // No merge was made (the user's branch is gone): every commit is the task's own.
            return self.git.changed_paths(repo, &[before, tip]).await.map(Some);
        };
        // In the merge commit: the files that are not as git merges them by itself (the
        // conflicts as they were resolved, and what else was edited before Codly committed).
        let log = ["log", "-1", "--format=", "--remerge-diff", "--name-only", "--no-renames", "-z", merge, "--"];
        let names = self.git.read(repo, &log).await?;
        let in_merge: Vec<&str> = names.split('\0').map(|name| name.trim_start_matches('\n')).filter(|name| !name.is_empty()).collect();
        // One of those that a later commit put back to how git merges it is no change any
        // more. Codly can tell where only one side changed the file since the two branches
        // parted: there git takes that side's file. (Where both sides changed it, the file
        // counts as changed by hand.)
        let base = self.git.merge_base(repo, before, &theirs).await?.ok_or_else(|| anyhow!("the plan's branch and the merged branch share no commit"))?;
        let ours_changed = self.git.changed_paths(repo, &[base.as_str(), before]).await?;
        let theirs_changed = self.git.changed_paths(repo, &[base.as_str(), theirs.as_str()]).await?;
        let since_ours = self.git.changed_paths(repo, &[before, tip]).await?;
        let since_theirs = self.git.changed_paths(repo, &[theirs.as_str(), tip]).await?;
        let as_git_merges = |file: &str| {
            let has = |list: &[String]| list.iter().any(|f| f == file);
            (!has(&theirs_changed) && !has(&since_ours)) || (!has(&ours_changed) && !has(&since_theirs))
        };
        let mut files: Vec<String> = in_merge.iter().copied().filter(|file| !as_git_merges(file)).map(String::from).collect();
        // After the merge commit: what the later commits changed, all in all.
        let later = self.git.changed_paths(repo, &[merge, tip]).await?;
        files.extend(later.into_iter().filter(|file| !in_merge.contains(&file.as_str())));
        Ok(Some(files))
    }

    /// The checks that already passed on the plan's branch as it is in `wt`: the last task that
    /// worked on that branch itself ran them in its gate, after its last commit, and nothing
    /// moved the branch since. The plan does not run them a second time when it finishes
    /// again, and the record says so. Empty when no such task was the last to move the branch.
    pub(super) async fn checked_in_place(&self, plan: &Plan, wt: &Path) -> Result<Vec<String>> {
        let tasks = self.store.plan_tasks(&plan.id)?;
        let last = tasks.iter().filter(|t| t.status == TaskStatus::Done && in_place(t, plan).is_some()).max_by_key(|t| t.updated_at);
        let Some(task) = last else { return Ok(vec![]) };
        let (_, committed) = self.store.task_commits(&task.id)?;
        if committed.is_none() || committed != self.git.head(wt).await? {
            return Ok(vec![]);
        }
        // Its gate's results after its last commit: the last one of each command counts.
        let since = self.store.task_events(&task.id, "committed")?.last().map_or(0, |e| e.id);
        let mut results: Vec<(String, bool)> = vec![];
        for e in self.store.task_events(&task.id, "verified")?.into_iter().filter(|e| e.id > since) {
            if let EventKind::Verified { command, ok, .. } = e.kind {
                results.retain(|(c, _)| *c != command);
                results.push((command, ok));
            }
        }
        let passed: Vec<String> = results.into_iter().filter(|(_, ok)| *ok).map(|(command, _)| command).collect();
        if !passed.is_empty() {
            let summary = format!("{} passed in task {} on the plan's branch as it is: the plan does not run {} a second time", code_list(&passed), task.key, if passed.len() == 1 { "it" } else { "them" });
            self.decide(Some(&plan.workspace_id), None, "merge", summary, json!({ "plan_id": plan.id, "task_id": task.id, "checks": passed }))?;
        }
        Ok(passed)
    }

    /// A resolution that could not start leaves nothing behind. A step failed (`why`) after
    /// Codly began to bring `target` into the plan's branch, so the merge may be open in the
    /// plan's worktree, or committed on its branch with no task on it. Left there, the next
    /// merge of the plan would find the user's branch in the plan's already and run no check
    /// on the result. So the plan's branch goes back to `before`, a task that reached the
    /// store is cancelled, and the plan is as it was: the next merge checks its result again.
    /// Call it holding `repo_lock`. Returns what the caller says: `why`, and what became of
    /// the merge. That is one of three things: it could not be taken back (the start stays on
    /// record, and `settle_resolution` tries again before the plan is merged); it was taken
    /// back and that could not be written down (the same record says it later); or both worked.
    /// When only the records of the start cannot be removed after the line is written, both
    /// worked: `settle_resolution` removes them later and finds the line (`TAKEN_BACK`).
    async fn take_back_start(&self, plan: &Plan, target: &str, branch: &str, wt: &Path, before: &str, why: anyhow::Error) -> anyhow::Error {
        // The merge commit, a merge left open, and what it changed in the files. No worker
        // ran there, so no new file is its own.
        if let Err(e) = self.git.reset_to(wt, before).await {
            return anyhow!(
                "{why:#}. Codly could not take the merge of {target} back ({e:#}): {branch} may have {target} merged in with no check on the result. \
                 Put it back with: git -C {} reset --hard {before}",
                wt.display()
            );
        }
        let key = resolution_key(&plan.id);
        let noted = || -> Result<()> {
            if let Some(saved) = self.store.kv_get(&key)? {
                let was: Resolution = serde_json::from_str(&saved)?;
                if self.store.task(&was.task)?.is_some() {
                    self.update_task(&was.task, |t| {
                        t.status = TaskStatus::Cancelled;
                        t.status_reason = Some("Codly could not start it".into());
                        Ok(())
                    })?;
                }
            }
            let start = self.store.kv_get(&start_key(&plan.id))?;
            let summary = format!("bringing {target} into {branch} did not start ({}); {branch} is back where it was before it", prompts::clip(&format!("{why:#}"), 160));
            let mut detail = json!({ "plan_id": plan.id, "tip": before });
            detail[TAKEN_BACK] = json!(start);
            self.decide(Some(&plan.workspace_id), None, "merge", summary, detail)?;
            Ok(())
        };
        if let Err(e) = noted() {
            return anyhow!("{why:#}. Codly took the merge of {target} back: {branch} is as it was. It could not write that down: {e:#}");
        }
        // Last: while the line above is not on record, the start is.
        if let Err(e) = self.store.kv_write(&[(key, None), (start_key(&plan.id), None)]) {
            tracing::warn!("the start of a resolution of plan {} stays on record after it was taken back: {e:#}", plan.id);
        }
        anyhow!("{why:#}. Codly took the merge of {target} back: {branch} is as it was")
    }

    /// Bring the user's branch into the plan's integration branch, in the plan's own worktree.
    /// With conflicts, one more task of the plan resolves them there. Without, and with checks
    /// of the plan that failed on that very merge, the merge is committed at once and one more
    /// task makes them pass. Either way the plan runs again: it finishes, with its integration
    /// checks on the result, when that task is done. Without a conflict and without a check
    /// that is known to fail, nothing is brought in: the merge itself checks its result.
    /// A step that fails once the merge has begun takes the merge back (`take_back_start`),
    /// and so does the plan's next pass when Codly was closed in the middle (`MergeStart`).
    async fn resolve_plan_conflict(self: &Arc<Self>, plan_id: &str, into: Option<&str>) -> Result<()> {
        let plan = self.resolvable_plan(plan_id).await?;
        let branch = plan.integration_branch.clone().ok_or_else(|| anyhow!("the plan never ran, so there is nothing to merge"))?;
        let wt = PathBuf::from(plan.integration_worktree.clone().ok_or_else(|| anyhow!("the plan has no integration worktree"))?);
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        // Before anything is set up: a branch the plan did not start from is yours to confirm.
        self.resolve_target(&repo, &plan, into).await?;
        // The plan's worktree, back if a cleanup removed it, with the project's dependencies.
        {
            let _g = self.repo_lock.lock().await;
            if self.git.rev_parse(&repo, &branch).await?.is_none() {
                bail!("the plan's branch {branch} was deleted, so there is nothing to resolve");
            }
            let fresh = !wt.exists();
            self.git.worktree_add(&repo, &wt, &branch, &branch).await?;
            if fresh {
                self.mark_new_worktree(&wt)?;
            }
        }
        self.prepare_worktree(&ws, &wt, None, Some(&plan.thread)).await?;

        let _g = self.repo_lock.lock().await;
        // Read again under the lock: of two clicks, only the first starts a resolution. One
        // that did not finish, and that the plan's last pass could not take back, goes first.
        let plan = self.settle_resolution(&self.resolvable_plan(plan_id).await?).await?;
        let target = self.resolve_target(&repo, &plan, into).await?;
        let theirs = branch_ref(&target);
        let head = self.git.head(&repo).await?.ok_or_else(|| anyhow!("the repository has no commits yet"))?;
        if self.git.is_ancestor(&repo, &head, &branch).await? {
            bail!("{branch} already has everything on {target}, so there is nothing to resolve; merge it");
        }
        self.git.check_worktree(&repo, &wt).await?;
        // A merge that could not be committed last time is still open: this one starts over.
        self.git.abort_merge(&wt).await?;
        let before = self.git.head(&wt).await?.ok_or_else(|| anyhow!("{branch} has no commits"))?;
        // The plan's checks that failed on this very merge when you tried it.
        let failed = self.failed_checks(&plan.id, Some(&head), Some(&before))?.map(|f| f.failed).unwrap_or_default();
        // Nothing conflicts and no check is known to fail: there is nothing for an agent to do,
        // and a merge brought in here would reach your branch without the plan's checks on it.
        // The merge itself runs them on the result first. (A git too old to say goes on.)
        if failed.is_empty() && self.git.merge_tree_check(&repo, &before, &head).await.is_ok_and(|check| check.clean) {
            bail!("{branch} and {target} merge without a conflict now, so there is nothing to resolve; merge the plan (Codly runs its checks on the result first)");
        }
        // Read before the merge, like everything above that can fail without it: the plan's
        // tasks (for the new task's key), its verify commands, and the write scope of a task
        // that fixes the checks, which is the files the plan changed, measured from where the
        // plan started. From the merge on, a step that fails is taken back.
        let tasks = self.store.plan_tasks(&plan.id)?;
        let verify = self.plan_verify_commands(&plan)?;
        let scope = match plan.base_commit.as_deref().filter(|_| !failed.is_empty()) {
            Some(base) => self.git.changed_paths(&repo, &[format!("{base}...{before}").as_str()]).await?,
            None => vec![],
        };
        // What worktree setup and the checks left there: it stays out of the merge Codly
        // commits here, and it stays if the task does not finish.
        let untracked = self.git.untracked(&wt).await?;
        let ws_id = Some(ws.id.as_str());
        // On record before the merge: if Codly is closed in the middle of what follows, the
        // plan's next pass, merge or resolution finds the start and takes the merge back.
        let start = MergeStart { tip: before.clone(), target: target.clone(), head: head.clone(), run: THIS_RUN.clone() };
        self.store.kv_set(&start_key(&plan.id), &serde_json::to_string(&start)?)?;
        let started: Result<()> = async {
            let files = self.git.merge_for_resolution(&wt, &theirs).await?;
            let mut detail = json!({ "plan_id": plan.id, "target_sha": head, "files": files });
            detail[BRINGS_IN] = json!(target);
            let text = if !files.is_empty() {
                let task = resolve_task(&plan, next_key(&tasks, RESOLVE), &target, &files, verify, &branch, &wt);
                self.add_in_place(ws_id, &plan, &task, &before, untracked)?;
                let summary = format!("you had an agent resolve the conflicts of {branch} with {target} in {}: task {}", name_list(&files), task.key);
                self.decide_by(ws_id, Some(&task.id), "merge", "human", summary, detail)?;
                format!(
                    "{target} and this plan both changed {}. Codly brought {target} into the plan's branch, in its own worktree \
                     (your checkout is not touched). An agent resolves the conflicts there: that is task {}. Then the plan's \
                     checks run again and you can merge it.",
                    name_list(&files),
                    task.key
                )
            } else {
                let message = format!("codly: merge {target} into {branch}");
                // The merge as git made it, and nothing else that is in the worktree.
                let c = self.git.conclude_merge(&wt, &message).await?;
                self.emit(ws_id, row(None, None, None, EventKind::Committed { sha: c.sha, files: c.files, message }))?;
                self.decide_by(ws_id, None, "merge", "human", format!("you had {target} brought into {branch}; it merged without a conflict"), detail)?;
                if failed.is_empty() {
                    format!(
                        "Codly brought {target} into the plan's branch, in its own worktree: no conflict after all. The plan's \
                         checks run again on the result; then you can merge it."
                    )
                } else {
                    // The plan's checks failed on this very merge when you tried it: one more
                    // task of the plan makes them pass, on the merge Codly committed just now.
                    let task = fix_task(&plan, next_key(&tasks, FIX_CHECKS), &target, &failed, &scope, verify, &branch, &wt);
                    self.add_in_place(ws_id, &plan, &task, &before, untracked)?;
                    let summary = format!("you had an agent fix {} on {branch} with {target} merged in: task {}", code_list(&failed), task.key);
                    self.decide_by(ws_id, Some(&task.id), "merge", "human", summary, json!({ "plan_id": plan.id, "target_sha": head, "failed_checks": failed }))?;
                    format!(
                        "{target} and this plan merge without a conflict, but {} failed on the result. Codly brought {target} into \
                         the plan's branch, in its own worktree (your checkout is not touched), and task {} makes the checks pass \
                         there. Then the plan's checks run again and you can merge it.",
                        code_list(&failed),
                        task.key
                    )
                }
            };
            self.chat(ws_id, &plan.thread, "codly", &text, None)?;
            // Last, when nothing can fail any more: from here the plan runs, and a running plan
            // with nothing left to do finishes at the next pass. It builds on the user's
            // branch as it is now: its diff shows the plan's work only.
            self.update_plan(&plan.id, |p| {
                p.status = PlanStatus::Running;
                p.base_commit = Some(head.clone());
                p.base_branch = Some(target.clone());
                Ok(())
            })
            .map(drop)
        }
        .await;
        if let Err(why) = started {
            return Err(self.take_back_start(&plan, &target, &branch, &wt, &before, why).await);
        }
        // The start is complete. A record of it that stays says no more than that: the plan
        // builds on `head` now (`settle_resolution`).
        if let Err(e) = self.store.kv_write(&[(start_key(&plan.id), None)]) {
            tracing::warn!("the start of a resolution of plan {} stays on record: {e:#}", plan.id);
        }
        drop(_g);
        self.kick();
        Ok(())
    }

    /// Before a worker starts (again) on a task that resolves a conflict in place: the merge it
    /// resolves is open in the worktree. A stop that undid the merge (its rounds ran out, Codly
    /// restarted) starts it again here, with the user's branch as it is now.
    pub(super) async fn reopen_resolution(&self, task: &Task, repo: &Path, wt: &Path) -> Result<()> {
        let target = self.store.task_events(&task.id, "decision")?.iter().find_map(|e| match &e.kind {
            EventKind::Decision { detail, .. } => detail[BRINGS_IN].as_str().map(branch_ref),
            _ => None,
        });
        let Some(target) = target else { return Ok(()) };
        let _g = self.repo_lock.lock().await;
        self.git.check_worktree(repo, wt).await?;
        // Open already, or concluded (the worker only has checks left to fix), or the user
        // deleted that branch since: nothing to start.
        if self.git.in_merge(wt).await || self.git.rev_parse(wt, &target).await?.is_none() || self.git.is_ancestor(wt, &target, "HEAD").await? {
            return Ok(());
        }
        self.git.merge_for_resolution(wt, &target).await.map(drop)
    }

    /// A solo agent's worktree is committed (after a turn, before a merge or a push) only when
    /// no merge is left half-resolved in it: committed, the conflict markers would be in its
    /// branch.
    pub(super) async fn check_resolved(&self, agent: &Agent, wt: &Path) -> Result<()> {
        let left = self.unresolved(wt).await?;
        if left.is_empty() {
            return Ok(());
        }
        bail!("{} has not finished resolving a merge: {}. Ask it to finish, then try again.", agent.name, markers_left(&left))
    }

    /// The files of a merge open in `wt` that still have conflict markers. Empty: no merge is
    /// open there, or it is resolved.
    async fn unresolved(&self, wt: &Path) -> Result<Vec<String>> {
        if !self.git.in_merge(wt).await {
            return Ok(vec![]);
        }
        Ok(gate::files_with_markers(wt, &self.git.unmerged_files(wt).await?))
    }

    /// A solo agent's turn ended: may Codly commit its worktree? Not while a merge in it still
    /// has conflict markers. The merge stays open for the agent's next turn, and its
    /// conversation says that this turn's work is not committed, and how to go on.
    pub(super) async fn may_snapshot(&self, ws: Option<&str>, agent: &Agent, wt: &Path) -> Result<bool> {
        let left = self.unresolved(wt).await?;
        if left.is_empty() {
            return Ok(true);
        }
        let text = format!(
            "{name}'s turn ended, but {}, so Codly did not commit this turn's work. Ask {name} to finish resolving, or stop \
             resolving in its Git tab.",
            markers_left(&left),
            name = agent.name
        );
        self.chat(ws, &format!("agent:{}", agent.id), "codly", &text, None)?;
        Ok(false)
    }

    /// A solo agent whose process writes no file now. A stopped agent counts as working until
    /// its process is gone.
    fn solo_at_rest(&self, agent: &Agent) -> bool {
        let resting = matches!(agent.status, AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed | AgentStatus::Offline);
        (agent.run_id.is_none() || resting) && self.is_quiet(&agent.id)
    }

    /// Stop a solo agent's conflict resolution: the merge Codly started in its worktree is
    /// taken out, and its branch is as it was before. The conflict itself stays.
    pub(super) async fn stop_resolving_merge(&self, agent_id: &str) -> Result<MergeState> {
        let (agent, ws, wt) = self.solo_side(agent_id).await?;
        if !self.solo_at_rest(&agent) {
            bail!("{} is still working; stop its turn first, then stop resolving", agent.name);
        }
        {
            let _g = self.repo_lock.lock().await;
            self.git.check_worktree(Path::new(&ws.repo_path), &wt).await?;
            if !self.git.abort_merge(&wt).await? {
                bail!("{} is not resolving a merge", agent.name);
            }
        }
        let ws_id = Some(ws.id.as_str());
        self.decide_by(ws_id, None, "merge", "human", format!("you stopped {}'s conflict resolution; the merge was taken out of its worktree", agent.name), json!({ "agent_id": agent_id }))?;
        let text = format!(
            "You stopped the conflict resolution. Codly took the merge out of {}'s worktree: its branch is as it was before. The \
             conflict with your branch is still there.",
            agent.name
        );
        self.chat(ws_id, &format!("agent:{agent_id}"), "codly", &text, None)?;
        self.merge_state(&PrSource::Agent(agent_id.to_string())).await
    }

    /// Bring the user's branch into a solo agent's branch, in the agent's own worktree, and ask
    /// the agent to resolve the conflicts in its conversation. Codly commits the merge when
    /// that turn ends (`solo_snapshot`).
    async fn resolve_agent_conflict(self: &Arc<Self>, agent_id: &str) -> Result<()> {
        let (agent, ws, wt) = self.solo_side(agent_id).await?;
        if !self.solo_at_rest(&agent) {
            bail!("{} is still working; ask again when its turn ends", agent.name);
        }
        let repo = PathBuf::from(&ws.repo_path);
        let ws_id = Some(ws.id.as_str());
        let thread = format!("agent:{agent_id}");
        let target = self.checked_out(&repo).await?;
        let theirs = branch_ref(&target);
        let files = {
            let _g = self.repo_lock.lock().await;
            self.git.check_worktree(&repo, &wt).await?;
            self.check_resolved(&agent, &wt).await?;
            // What its last turn left uncommitted is the agent's side of the merge.
            let message = format!("codly(solo/{}): before merging {target}", agent.name);
            if let Some(c) = self.commit_noted(ws_id, Some(agent_id), &agent.name, &wt, &message).await? {
                self.emit(ws_id, row(Some(agent_id), None, None, EventKind::Committed { sha: c.sha, files: c.files, message }))?;
            }
            if self.git.is_ancestor(&wt, &theirs, "HEAD").await? {
                bail!("{}'s branch already has everything on {target}, so there is nothing to resolve; merge it", agent.name);
            }
            let files = self.git.merge_for_resolution(&wt, &theirs).await?;
            if files.is_empty() {
                let message = format!("codly(solo/{}): merge {target}", agent.name);
                if let Some(c) = self.commit_noted(ws_id, Some(agent_id), &agent.name, &wt, &message).await? {
                    self.emit(ws_id, row(Some(agent_id), None, None, EventKind::Committed { sha: c.sha, files: c.files, message }))?;
                }
            }
            files
        };
        if files.is_empty() {
            let text = format!("Codly brought {target} into {}'s branch, in its own worktree: no conflict after all. You can merge it now.", agent.name);
            self.chat(ws_id, &thread, "codly", &text, None)?;
            return Ok(());
        }
        if let Err(e) = self.chat_agent(agent_id, &resolve_request(&target, &files)).await {
            // Nobody resolves it: the worktree goes back to the agent's last commit.
            let _g = self.repo_lock.lock().await;
            self.git.abort_merge(&wt).await?;
            return Err(e);
        }
        let summary = format!("you had {} resolve the conflicts of its branch with {target} in {}", agent.name, name_list(&files));
        self.decide_by(ws_id, None, "merge", "human", summary, json!({ "agent_id": agent_id, "target": target, "files": files }))?;
        Ok(())
    }

    /// The plan's verify commands: those of every finished task that passed when the task was
    /// checked, then the plan's integration checks. Each once.
    pub(super) fn plan_verify_commands(&self, plan: &Plan) -> Result<Vec<String>> {
        let mut checks: Vec<String> = vec![];
        for t in self.store.plan_tasks(&plan.id)?.iter().filter(|t| t.status == TaskStatus::Done && in_place(t, plan).is_none()) {
            let runs = self.store.task_events(&t.id, "verified")?;
            for cmd in &t.verify {
                let last = runs.iter().rev().find_map(|e| match &e.kind {
                    EventKind::Verified { command, ok, .. } if command == cmd => Some(*ok),
                    _ => None,
                });
                if last == Some(true) && !checks.contains(cmd) {
                    checks.push(cmd.clone());
                }
            }
        }
        let integration = self.store.plan_json(&plan.id)?["integration_checks"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();
        for cmd in integration {
            if !checks.contains(&cmd) {
                checks.push(cmd);
            }
        }
        Ok(checks)
    }

    /// Before a plan is merged into the user's branch: run the plan's verify commands on the
    /// result of that merge, made in a scratch worktree (the user's checkout is not touched).
    /// None = there is nothing new to check: the plan has no commands, or the user's branch
    /// has no commit the plan was not checked with. Err = a check failed or the checks could
    /// not run (the plan's thread and its card say which), or the merge cannot be tried at all.
    pub(super) async fn check_merge_result(&self, plan: &Plan, ws: &Workspace, branch: &str, into: Option<&str>) -> Result<Option<MergeChecked>> {
        let repo = PathBuf::from(&ws.repo_path);
        let ws_id = Some(ws.id.as_str());
        let _one = MERGE_CHECK.lock().await;
        let scratch = self.worktree_root(ws).join("merge-check");
        let (target, head, tip, checks) = {
            let _g = self.repo_lock.lock().await;
            let target = self.merge_target(&repo, plan, branch, into).await?;
            let (Some(head), Some(tip)) = (self.git.head(&repo).await?, self.git.rev_parse(&repo, branch).await?) else { return Ok(None) };
            if self.git.is_ancestor(&repo, &head, &tip).await? || self.git.is_ancestor(&repo, &tip, &head).await? {
                return Ok(None);
            }
            let checks = self.plan_verify_commands(plan)?;
            if checks.is_empty() {
                return Ok(None);
            }
            // One left by a check that did not finish (Codly was closed) goes first.
            self.remove_scratch(&repo, &scratch).await?;
            self.git.worktree_add_detached(&repo, &scratch, &head).await?;
            self.mark_new_worktree(&scratch)?;
            let merged = self.git.merge(&scratch, &tip, "codly: merge check", true).await;
            if !matches!(merged, Ok(MergeOutcome::Merged { .. })) {
                self.remove_scratch(&repo, &scratch).await?;
            }
            match merged? {
                MergeOutcome::Merged { .. } => {}
                // The user's branch has it already (ruled out above): nothing to check.
                MergeOutcome::AlreadyMerged { .. } => return Ok(None),
                MergeOutcome::Conflict { files, .. } => bail!("{}", conflict_error(&target, branch, &files, "An agent (on the plan's card)")),
            }
            (target, head, tip, checks)
        };
        // The commands need the project's dependencies, like in every worktree Codly makes.
        let ready = self.prepare_worktree(ws, &scratch, None, Some(&plan.thread)).await;
        let ran = match &ready {
            Ok(()) => Some(self.run_merge_checks(plan, ws, &scratch, &checks).await),
            Err(_) => None,
        };
        {
            let _g = self.repo_lock.lock().await;
            self.remove_scratch(&repo, &scratch).await?;
        }
        // The worktree could not be set up, so no check ran: not merged, like a failed check,
        // and the plan's card offers the same way on (merge anyway).
        let Some(ran) = ran else {
            let why = ready.err().map(|e| format!("{e:#}")).unwrap_or_default();
            let record = FailedChecks { head, tip, failed: vec![], not_run: Some(why.clone()) };
            self.store.kv_set(&failed_key(&plan.id), &serde_json::to_string(&record)?)?;
            let text = format!(
                "Not merged: Codly could not run the plan's checks on \"{}\" merged into {target}, because the worktree for them \
                 was not set up: {why} {target} was not changed. {target} has commits the plan was never checked with; fix the \
                 setup and merge again, or merge anyway.",
                plan.summary
            );
            self.chat(ws_id, &plan.thread, "codly", &text, None)?;
            self.decide(ws_id, None, "merge", prompts::clip(&text, 200), json!({ "plan_id": plan.id, "target": target, "checks_not_run": why }))?;
            bail!("the plan's checks could not run on the merge result, so nothing was merged and {target} is unchanged: {why}");
        };
        let (passed, failed, refused) = ran?;
        if failed.is_empty() {
            self.store.kv_write(&[(failed_key(&plan.id), None)])?;
            return Ok(Some(MergeChecked { head, tip, passed, refused }));
        }
        let record = FailedChecks { head, tip, failed: failed.clone(), not_run: None };
        self.store.kv_set(&failed_key(&plan.id), &serde_json::to_string(&record)?)?;
        let list = code_list(&failed);
        let text = format!(
            "Not merged: with \"{}\" merged into {target}, {list} failed. {target} was not changed. {target} has commits the plan \
             was never checked with; have an agent fix it (on the plan's card), or merge anyway if you know why it fails.",
            plan.summary
        );
        self.chat(ws_id, &plan.thread, "codly", &text, None)?;
        let detail = json!({ "plan_id": plan.id, "target": target, "failed_checks": failed, "passed_checks": passed, "refused_checks": refused });
        self.decide(ws_id, None, "merge", prompts::clip(&text, 200), detail)?;
        bail!("with the plan merged into {target}, {list} failed, so nothing was merged and {target} is unchanged")
    }

    /// Run the plan's verify commands in the scratch worktree that holds the merge result:
    /// those that passed, those that failed, and those Codly refused to run.
    async fn run_merge_checks(&self, plan: &Plan, ws: &Workspace, wt: &Path, checks: &[String]) -> Result<(Vec<String>, Vec<String>, Vec<HeldCheck>)> {
        let mode = self.approval_mode_for(Some(&ws.id), Some(&plan.thread));
        let (mut passed, mut failed, mut refused) = (vec![], vec![], vec![]);
        for cmd in checks {
            let v = self.run_verify(wt, cmd, mode).await;
            self.emit(Some(&ws.id), row(None, None, None, EventKind::Verified { command: cmd.clone(), ok: v.ok, exit_code: v.code, tail: v.tail }))?;
            if !v.held.is_empty() {
                refused.push(HeldCheck { command: cmd.clone(), risks: v.held });
            } else if v.ok {
                passed.push(cmd.clone());
            } else {
                failed.push(cmd.clone());
            }
        }
        Ok((passed, failed, refused))
    }

    /// Remove the scratch worktree of a merge check, also when git no longer knows the folder
    /// as a worktree. It is Codly's own and holds nothing to keep.
    async fn remove_scratch(&self, repo: &Path, scratch: &Path) -> Result<()> {
        if let Err(e) = self.git.worktree_remove(repo, scratch, None, true).await {
            if !scratch.exists() {
                return Err(e);
            }
            std::fs::remove_dir_all(scratch)?;
            self.git.worktree_remove(repo, scratch, None, true).await?;
        }
        Ok(())
    }

    /// The plan is in the user's branch: what its checks said about another merge is history.
    pub(super) fn forget_failed_checks(&self, plan_id: &str) -> Result<()> {
        self.store.kv_write(&[(failed_key(plan_id), None)])
    }
}

impl Core {
    /// What stands between a plan's branch (or a solo agent's) and the checked-out branch:
    /// the files that conflict, and the plan's checks that failed on the merge result. Reads
    /// only; nothing in the repository changes.
    pub async fn merge_state(&self, source: &PrSource) -> Result<MergeState> {
        self.inner.merge_state(source).await
    }

    /// Have an agent resolve the conflict with the checked-out branch, in Codly's own worktree
    /// (the user's checkout is not touched). A plan gets one more task and runs again until it
    /// is resolved and checked; a solo agent resolves it in its own conversation. Refused, with
    /// what to confirm, for a plan that did not start from the checked-out branch.
    pub async fn resolve_merge_conflict(&self, source: &PrSource) -> Result<MergeState> {
        self.inner.resolve_merge_conflict(source, None).await
    }

    /// `resolve_merge_conflict` for a plan, after the user saw `merge_plan_warning` and chose to
    /// go on: brings `branch` into the plan, and only if that is still the checked-out branch.
    pub async fn resolve_merge_conflict_into(&self, source: &PrSource, branch: &str) -> Result<MergeState> {
        self.inner.resolve_merge_conflict(source, Some(branch)).await
    }

    /// Stop a solo agent's conflict resolution: the merge Codly started in the agent's worktree
    /// is taken out, and its branch is as it was before. Refused while the agent works.
    pub async fn stop_resolving_merge(&self, agent_id: &str) -> Result<MergeState> {
        self.inner.stop_resolving_merge(agent_id).await
    }

    /// `merge_plan` without the plan's checks on the merge result: the user saw that they
    /// failed (or chose not to wait) and merges anyway. `into` as for `merge_plan_into`.
    pub async fn merge_plan_unchecked(&self, plan_id: &str, into: Option<&str>) -> Result<String> {
        self.inner.merge_plan(plan_id, into, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        serde_json::from_value(json!({
            "id": "p", "workspace_id": "ws", "thread": "lead", "request": "", "summary": "Add sign-in", "assumptions": [],
            "open_questions": [], "status": "done", "base_commit": null, "integration_branch": "codly/sign-in/integration",
            "integration_worktree": "/data/worktrees/web/sign-in/integration", "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    #[test]
    fn only_a_task_on_the_plans_own_branch_works_in_place() {
        let plan = plan();
        let task = resolve_task(&plan, "resolve".into(), "main", &["a.rs".to_string()], vec!["make check".into()], "codly/sign-in/integration", Path::new("/data/worktrees/web/sign-in/integration"));
        assert_eq!(in_place(&task, &plan), Some(("codly/sign-in/integration".to_string(), PathBuf::from("/data/worktrees/web/sign-in/integration"))));
        assert_eq!((task.title.as_str(), task.status, &task.write_scope, &task.verify), ("Resolve conflicts with main", TaskStatus::Pending, &vec!["a.rs".to_string()], &vec!["make check".to_string()]));
        assert!(task.goal.contains("merging main into this worktree") && task.goal.contains("a.rs") && task.goal.contains("Do not run git commands"), "{}", task.goal);
        assert!(task.goal.ends_with("Codly commits the merge when you report. Files that were in this worktree before you started and are on no branch are not committed."), "{}", task.goal);

        // A task on its own branch, a task that never started, a plan that never ran: not in place.
        let own = Task { branch: Some("codly/sign-in/t1".into()), ..task.clone() };
        assert_eq!(in_place(&own, &plan), None);
        assert_eq!(in_place(&Task { branch: None, ..task.clone() }, &plan), None);
        let unstarted = Plan { integration_branch: None, integration_worktree: None, ..plan.clone() };
        assert_eq!(in_place(&Task { branch: None, ..task.clone() }, &unstarted), None);

        // A second conflict gets a key of its own.
        assert_eq!(next_key(&[], RESOLVE), "resolve");
        assert_eq!(next_key(&[task.clone()], RESOLVE), "resolve-2");
        assert_eq!(next_key(&[task.clone(), Task { key: "resolve-2".into(), ..task.clone() }], RESOLVE), "resolve-3");
        assert_eq!(next_key(&[task], FIX_CHECKS), "fix-checks");
    }

    #[test]
    fn the_task_that_fixes_the_checks_works_in_place_on_the_files_the_plan_changed() {
        let plan = plan();
        let failed = ["npm test".to_string()];
        let scope = ["src/theme.ts".to_string()];
        let task = fix_task(&plan, "fix-checks".into(), "main", &failed, &scope, vec!["npm test".into(), "npm run lint".into()], "codly/sign-in/integration", Path::new("/data/worktrees/web/sign-in/integration"));
        assert!(in_place(&task, &plan).is_some());
        assert_eq!((task.key.as_str(), task.title.as_str(), task.status, &task.write_scope[..]), ("fix-checks", "Fix the checks with main merged in", TaskStatus::Pending, &scope[..]));
        assert_eq!(task.verify, ["npm test", "npm run lint"]);
        assert_eq!(task.acceptance, ["`npm test` passes", "The plan's work and the changes on main are both kept and work together"]);
        assert!(task.goal.contains("merge without a conflict") && task.goal.contains("these checks fail: `npm test`") && task.goal.contains("Do not run git commands"), "{}", task.goal);
        assert!(task.goal.ends_with("Codly commits your work when you report. Files that were in this worktree before you started and are on no branch are not committed."), "{}", task.goal);
    }

    /// The write scope of these tasks names files, and a file name can hold what a glob reads
    /// as a pattern (a route folder like `[id]`, a brace with no end): it is its own scope.
    #[test]
    fn the_scope_of_a_task_in_place_is_file_names_not_patterns() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        let scope = s(&["src/routes/[slug]/+page.svelte", "app/{draft/page.tsx", "docs/a*.md", "src/lib"]);
        assert_eq!(outside_scope(&scope, &scope), Vec::<String>::new());
        // A name is no pattern for other files. It stands for what is under it, as a folder.
        let changed = s(&["src/routes/s/+page.svelte", "docs/about.md", "src/lib/api.ts", "src/library.ts", "docs/a*.md"]);
        assert_eq!(outside_scope(&scope, &changed), s(&["src/routes/s/+page.svelte", "docs/about.md", "src/library.ts"]));
        assert_eq!(outside_scope(&[], &changed), Vec::<String>::new(), "no scope: unrestricted");
    }

    /// A fix of the checks worked on a merge without a conflict: neither what its reviewer
    /// reads about the diff nor the refusal for a cancelled plan speaks of one.
    #[test]
    fn a_fix_of_the_checks_is_not_called_a_conflict() {
        let plan = plan();
        let (branch, wt) = ("codly/sign-in/integration", Path::new("/data/worktrees/web/sign-in/integration"));
        let resolve = resolve_task(&plan, "resolve-2".into(), "main", &["a.rs".to_string()], vec![], branch, wt);
        let fix = fix_task(&plan, "fix-checks-2".into(), "main", &["npm test".to_string()], &[], vec![], branch, wt);

        let note = review_note(&resolve);
        assert!(note.starts_with("\n\nAbout the diff: this task worked on a merge of the plan's branch with the user's branch."), "{note}");
        assert!(note.contains("from <<<<<<< to >>>>>>> are a conflict as git left it") && note.ends_with("Check that each resolution keeps what both sides changed."), "{note}");
        let note = review_note(&fix);
        assert!(note.starts_with("\n\nAbout the diff: this task worked on the plan's branch after Codly merged the user's branch into it. There was no conflict:"), "{note}");
        assert!(note.contains("the diff shows only what the worker changed after that merge") && note.contains("it is not in the diff and not for review"), "{note}");
        assert!(!note.contains("<<<<<<<") && !note.contains("resolution") && !note.contains("resolved"), "{note}");

        assert_eq!(cancelled_refusal(false), "this plan was cancelled, so no agent works on it any more; resolve the conflict yourself");
        assert_eq!(cancelled_refusal(true), "this plan was cancelled, so no agent works on it any more; fix the checks yourself");
    }

    #[test]
    fn a_review_task_with_nothing_to_review_is_not_called_a_merge() {
        let plan = plan();
        let (branch, wt) = ("codly/sign-in/integration", Path::new("/data/worktrees/web/sign-in/integration"));
        let mut review = resolve_task(&plan, "review-2".into(), "main", &[], vec![], branch, wt);
        assert_eq!(nothing_to_review(&review), "review-2: no review, the task changed nothing");
        review.key = "resolve".into();
        assert_eq!(nothing_to_review(&review), "resolve: no review, nothing was changed by hand in the merge with your branch");
    }

    #[test]
    fn only_what_is_new_in_a_worktree_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        for folder in ["setup", "mixed", "new/deep", "elsewhere"] {
            std::fs::create_dir_all(wt.join(folder)).unwrap();
        }
        for file in ["setup/kept.txt", "setup/added-later.txt", "mixed/old.log", "mixed/new.log", "new/deep/a.txt", "old.txt", "new.txt", "elsewhere/target.txt"] {
            std::fs::write(wt.join(file), "x\n").unwrap();
        }
        std::os::unix::fs::symlink(wt.join("elsewhere"), wt.join("link")).unwrap();
        let list = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        // As `Git::untracked` lists them: a folder that is new as a whole is one entry.
        let before = list(&["setup/", "mixed/old.log", "old.txt"]);
        let now = list(&["setup/", "mixed/", "new/", "old.txt", "new.txt", "link", "gone.txt"]);
        let removed = remove_new(wt, &now, &before).unwrap();
        assert_eq!(removed, ["new/", "new.txt", "link"]);
        assert!(!wt.join("new").exists() && !wt.join("new.txt").exists() && wt.join("link").symlink_metadata().is_err());
        assert!(wt.join("elsewhere/target.txt").exists(), "a link is removed, not what it points at");
        assert!(wt.join("setup/added-later.txt").exists(), "a folder that was there before is left alone");
        assert!(wt.join("mixed/old.log").exists() && wt.join("mixed/new.log").exists(), "a folder that holds a path from before stays whole");
        assert!(wt.join("old.txt").exists());
    }

    /// Two new paths, as git lists them, are the same place when they are one path, or when
    /// one is a folder that is new as a whole and holds the other.
    #[test]
    fn a_new_folder_stands_for_what_is_under_it() {
        for (a, b) in [("notes.tmp", "notes.tmp"), ("cache/", "cache/built.txt"), ("cache/built.txt", "cache/"), ("cache/", "cache/deep/")] {
            assert!(same_place(a, b), "{a} and {b}");
        }
        for (a, b) in [("notes.tmp", "notes.tmp2"), ("cache/", "cache2/built.txt"), ("cache", "cache/built.txt"), ("a/b.txt", "a/c.txt")] {
            assert!(!same_place(a, b), "{a} and {b}");
        }
    }

    #[test]
    fn the_merged_line_says_what_ran_on_the_merge_result() {
        let checked = |passed: &[&str], refused: &[(&str, &str)]| MergeChecked {
            head: "a".into(),
            tip: "b".into(),
            passed: passed.iter().map(|c| c.to_string()).collect(),
            refused: refused.iter().map(|(c, r)| HeldCheck { command: c.to_string(), risks: vec![r.to_string()] }).collect(),
        };
        assert_eq!(
            checked(&["npm test", "npm run lint"], &[]).said("main"),
            " main had changed since the plan was checked. Codly ran the plan's checks on the merge result first: `npm test`, `npm run lint` passed."
        );
        assert_eq!(
            checked(&[], &[("./deploy.sh", "network")]).said("main"),
            " main had changed since the plan was checked. Not run on the merge result, because they need your OK: `./deploy.sh` (network)."
        );
        assert_eq!(
            conflict_error("main", "codly/x/integration", &["a.rs".to_string(), "b.rs".to_string()], "An agent (on the plan's card)"),
            "merging into main conflicts in a.rs, b.rs; the merge was aborted and your branch is unchanged. An agent (on the plan's card) can resolve it, or you can do it yourself: git merge codly/x/integration"
        );
    }
}
