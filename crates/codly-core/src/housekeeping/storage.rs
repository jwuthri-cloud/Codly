//! Storage: what Codly leaves on disk and in a repository, and what may be removed.
//!
//! Three kinds of leftovers:
//! - worktrees under an office's worktree folder (`<data>/worktrees/<office>/`), each with its
//!   branch, its state and its size;
//! - `codly/…` branches that no worktree has checked out;
//! - raw run logs (`<data>/runs/<run id>/`) and artifact copies (`<data>/artifacts/`).
//!
//! The report also lists the copies of Codly's own database and what a start from a copy, or
//! with no data, moved aside (`crate::store::backup` makes, lists and removes them).
//!
//! What "nothing to lose" means (`WorkState::Merged`): no uncommitted file (ignored files do
//! not count: build output, installed dependencies), and every commit is contained in the
//! branch the user has checked out, in the repository's default branch, or, for a task of a
//! plan, in the plan's integration branch.
//!
//! Rules:
//! - `sweep` (the automatic pass) removes only worktrees and branches with nothing to lose
//!   whose owner is finished (`Owner::finished`) and not in use (`Owner::locked`);
//! - `delete_worktree` and `delete_branch` (the user's own click) refuse to lose anything unless
//!   `force` says the user confirmed it, and a forced delete keeps the branch unless the user
//!   asked for it too;
//! - a worktree is judged with the record of its folder and the record of the branch it has
//!   checked out (`Owners::worktree`): a lock on either keeps the folder and the branch;
//! - a run's folder is removed by `prune_runs` only when nothing was written there for the
//!   whole retention time and the run is not in `keep`.
//!
//! This module knows git and the file system; who owns what (`Owners`) comes from the Core
//! (`orchestrator/housekeeping/storage.rs`), which also records every removal.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::git::{Git, WorktreeInfo};
use crate::store::backup::{DbBackup, SetAside};

/// Uncommitted files named per worktree in a report (the count is always complete).
const DIRTY_SHOWN: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoragePolicy {
    /// Delete an agent run's raw log this many days after its last line (0 = keep every log).
    pub log_days: i64,
    /// The background sweep also removes what finished plans left behind: clean, merged
    /// worktrees and merged `codly/…` branches.
    pub clean_merged: bool,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        StoragePolicy { log_days: 30, clean_merged: true }
    }
}

impl StoragePolicy {
    pub fn normalized(mut self) -> Self {
        self.log_days = self.log_days.clamp(0, 3650);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WorkState {
    /// Nothing to lose: no uncommitted file, and every commit is in another branch.
    Merged,
    /// Commits that no other branch has; no uncommitted file.
    Unmerged,
    /// Files that are not committed (it can have unmerged commits too).
    Dirty,
    /// Codly could not read it (`StoredWorktree::locked` says why).
    Unknown,
}

/// One of Codly's worktrees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoredWorktree {
    pub path: String,
    /// None = on no branch (a team member's folder between tasks).
    pub branch: Option<String>,
    pub head: Option<String>,
    /// Whose it is ("Task t1: Fix the login"); None = Codly has no record of it.
    pub owner: Option<String>,
    pub state: WorkState,
    /// The first uncommitted files; `dirty_count` is how many there are.
    pub dirty_files: Vec<String>,
    pub dirty_count: i64,
    /// Commits that no other branch has.
    pub unmerged_commits: i64,
    /// Why it cannot be deleted now (an agent works there, its task is not finished).
    pub locked: Option<String>,
    /// Its last commit or the last change of its folder, whichever is newer (unix ms).
    pub last_used_at: i64,
    /// Size on disk. Files that macOS shares between copies count in full.
    pub size_bytes: i64,
}

/// A `codly/…` branch that no worktree has checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoredBranch {
    pub name: String,
    pub head: String,
    pub owner: Option<String>,
    /// Commits that no other branch has (0 = deleting it loses nothing).
    pub unmerged_commits: i64,
    pub locked: Option<String>,
    pub last_commit_at: i64,
}

/// What one office (repository) holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OfficeStorage {
    pub office_id: String,
    pub name: String,
    pub repo_path: String,
    pub worktrees: Vec<StoredWorktree>,
    pub branches: Vec<StoredBranch>,
    /// Why this office could not be read (its folder is gone, git failed).
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DirUsage {
    /// Run folders, or artifact files.
    pub items: i64,
    pub bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StorageReport {
    pub offices: Vec<OfficeStorage>,
    /// Raw run logs of every office (`runs/`).
    pub logs: DirUsage,
    /// Copies of the files agents made, of every office (`artifacts/`).
    pub artifacts: DirUsage,
    /// Copies of Codly's database, newest first (`backups/`).
    pub backups: Vec<DbBackup>,
    /// What a start from a backup, or with no data, moved aside in Codly's data folder, newest
    /// first: folders of older backups, and databases that could not be used.
    pub set_aside: Vec<SetAside>,
}

/// What a cleanup removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StorageCleanup {
    pub worktrees: i64,
    pub branches: i64,
    /// Run logs (one folder per agent run).
    pub logs: i64,
    /// Disk space that came free.
    pub bytes: i64,
    /// What could not be cleaned, and why ("my-repo: git status failed: …").
    pub problems: Vec<String>,
}

/// What Codly's records say about a worktree or a branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owner {
    pub label: Option<String>,
    /// Why it must stay for now.
    pub locked: Option<String>,
    /// Its task and plan are over: the automatic sweep may remove it when nothing is lost.
    pub finished: bool,
    /// A branch that also counts as "merged into" (a task's integration branch).
    pub merged_into: Option<String>,
}

/// Who owns the worktrees (by path) and the branches (by name) of one office.
#[derive(Debug, Clone, Default)]
pub struct Owners {
    worktrees: HashMap<PathBuf, Owner>,
    branches: HashMap<String, Owner>,
}

impl Owner {
    /// Two records for one thing, on the safe side: locked if either is, finished only if
    /// both are.
    fn join(self, other: Owner) -> Owner {
        Owner {
            label: self.label.or(other.label),
            locked: self.locked.or(other.locked),
            finished: self.finished && other.finished,
            merged_into: self.merged_into.or(other.merged_into),
        }
    }
}

impl Owners {
    pub fn add_worktree(&mut self, path: &Path, owner: Owner) {
        let key = canonical(path);
        let joined = match self.worktrees.remove(&key) {
            Some(known) => known.join(owner),
            None => owner,
        };
        self.worktrees.insert(key, joined);
    }

    pub fn add_branch(&mut self, name: &str, owner: Owner) {
        let joined = match self.branches.remove(name) {
            Some(known) => known.join(owner),
            None => owner,
        };
        self.branches.insert(name.to_string(), joined);
    }

    /// The worktree is in use: it stays whatever else is known about it.
    pub fn lock_worktree(&mut self, path: &Path, why: String) {
        self.worktrees.entry(canonical(path)).or_default().locked = Some(why);
    }

    /// What is known about the worktree at `path` that has `branch` checked out: the record
    /// of the folder, joined with the record of that branch. A team member's folder has no
    /// task record of its own: the task it holds is known by its branch. A folder without a
    /// record of its own is never finished.
    fn worktree(&self, path: &Path, branch: Option<&str>) -> Owner {
        let folder = self.worktrees.get(&canonical(path)).cloned().unwrap_or_default();
        match branch.and_then(|name| self.branches.get(name)) {
            Some(held) => folder.join(held.clone()),
            None => folder,
        }
    }

    fn branch(&self, name: &str) -> Owner {
        self.branches.get(name).cloned().unwrap_or_default()
    }
}

/// One thing a cleanup removed, for the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removed {
    /// The worktree folder (None: only a branch was deleted).
    pub worktree: Option<String>,
    /// The branch that was deleted (None: it stays, or the worktree was on none).
    pub branch: Option<String>,
    /// The commit it was at: `git branch <name> <head>` brings its commits back until git
    /// prunes them.
    pub head: Option<String>,
    pub bytes: i64,
}

/// One repository and the folder that holds Codly's worktrees of it.
pub struct Site<'a> {
    pub git: &'a Git,
    pub repo: &'a Path,
    /// `<data>/worktrees/<office>/`.
    pub root: &'a Path,
}

impl Site<'_> {
    /// The branches whose content is safe without Codly: the one the user has checked out and
    /// the default branch.
    async fn kept_branches(&self) -> Result<Vec<String>> {
        let mut kept: Vec<String> = self.git.checked_out_branch(self.repo).await?.into_iter().collect();
        let default = self.git.default_branch(self.repo).await?;
        if !kept.contains(&default) && self.git.branch_exists(self.repo, &default).await? {
            kept.push(default);
        }
        Ok(kept)
    }

    /// `kept` plus the owner's own target, without the branch that is judged.
    async fn targets(&self, kept: &[String], owner: &Owner, own: Option<&str>) -> Result<Vec<String>> {
        let mut targets = kept.to_vec();
        if let Some(extra) = owner.merged_into.as_deref()
            && !targets.iter().any(|t| t == extra)
            && self.git.branch_exists(self.repo, extra).await?
        {
            targets.push(extra.to_string());
        }
        targets.retain(|t| Some(t.as_str()) != own);
        Ok(targets)
    }

    /// Codly's worktrees of this repository whose folder is on disk, and every branch that a
    /// worktree of the repository (Codly's or not) has checked out.
    async fn worktrees(&self) -> Result<(Vec<WorktreeInfo>, HashSet<String>)> {
        let root = canonical(self.root);
        let all = self.git.worktrees(self.repo).await?;
        let checked_out = all.iter().filter_map(|w| w.branch.clone()).collect();
        Ok((all.into_iter().filter(|w| w.path.is_dir() && canonical(&w.path).starts_with(&root)).collect(), checked_out))
    }

    async fn judge_worktree(&self, w: &WorktreeInfo, kept: &[String], owner: &Owner) -> StoredWorktree {
        let mut out = StoredWorktree {
            path: w.path.to_string_lossy().into_owned(),
            branch: w.branch.clone(),
            head: w.head.clone(),
            owner: owner.label.clone(),
            state: WorkState::Unknown,
            dirty_files: vec![],
            dirty_count: 0,
            unmerged_commits: 0,
            locked: owner.locked.clone(),
            last_used_at: modified_ms(&w.path),
            size_bytes: 0,
        };
        // No git command runs in a folder that is not this repository's own worktree.
        let read = async {
            self.git.check_worktree(self.repo, &w.path).await?;
            let dirty = self.git.dirty_paths(&w.path).await?;
            let (unmerged, committed_at) = match (w.head.as_deref(), w.branch.as_deref()) {
                (Some(head), Some(branch)) => {
                    let targets = self.targets(kept, owner, Some(branch)).await?;
                    (self.git.commits_only_on(self.repo, head, &targets).await?, self.git.committed_at(self.repo, head).await?)
                }
                // On no branch: only the commits that no branch has would go with the folder.
                (Some(head), None) => (self.git.commits_on_no_branch(self.repo, head).await?, self.git.committed_at(self.repo, head).await?),
                (None, _) => (0, 0),
            };
            anyhow::Ok((dirty, unmerged, committed_at))
        };
        match read.await {
            Ok((dirty, unmerged, committed_at)) => {
                out.state = if !dirty.is_empty() {
                    WorkState::Dirty
                } else if unmerged > 0 {
                    WorkState::Unmerged
                } else {
                    WorkState::Merged
                };
                out.dirty_count = dirty.len() as i64;
                out.dirty_files = dirty.into_iter().take(DIRTY_SHOWN).collect();
                out.unmerged_commits = i64::from(unmerged);
                out.last_used_at = out.last_used_at.max(committed_at);
            }
            Err(e) => out.locked = Some(format!("Codly could not read it: {e}")),
        }
        out
    }

    /// Every Codly worktree (by path) and every `codly/…` branch without a worktree (by name),
    /// judged. Sizes are not measured here (`with_sizes`).
    pub async fn inspect(&self, owners: &Owners) -> Result<(Vec<StoredWorktree>, Vec<StoredBranch>)> {
        let kept = self.kept_branches().await?;
        let (mut mine, checked_out) = self.worktrees().await?;
        mine.sort_by(|a, b| a.path.cmp(&b.path));
        let mut worktrees = vec![];
        for w in &mine {
            worktrees.push(self.judge_worktree(w, &kept, &owners.worktree(&w.path, w.branch.as_deref())).await);
        }
        let mut branches = vec![];
        for b in self.git.codly_branches(self.repo).await? {
            if checked_out.contains(&b.name) {
                continue;
            }
            let owner = owners.branch(&b.name);
            let targets = self.targets(&kept, &owner, Some(&b.name)).await?;
            let unmerged = self.git.commits_only_on(self.repo, &b.head, &targets).await?;
            branches.push(StoredBranch {
                name: b.name,
                head: b.head,
                owner: owner.label,
                unmerged_commits: i64::from(unmerged),
                locked: owner.locked,
                last_commit_at: b.committed_at,
            });
        }
        Ok((worktrees, branches))
    }

    /// The automatic pass: remove the worktrees and branches of finished owners that have
    /// nothing to lose. A dirty, unmerged, unknown or locked one is never touched.
    pub async fn sweep(&self, owners: &Owners) -> Result<Vec<Removed>> {
        // A worktree whose folder is gone still holds its branch until git forgets it.
        let root = canonical(self.root);
        if let Some(gone) = self.git.worktrees(self.repo).await?.into_iter().find(|w| !w.path.exists() && w.path.starts_with(&root)) {
            self.git.worktree_remove(self.repo, &gone.path, None, false).await?;
        }
        let (worktrees, branches) = self.inspect(owners).await?;
        let mut removed = vec![];
        for w in worktrees {
            let path = PathBuf::from(&w.path);
            if !owners.worktree(&path, w.branch.as_deref()).finished || w.locked.is_some() || w.state != WorkState::Merged {
                continue;
            }
            match self.remove_worktree(&w, false, true).await {
                Ok(r) => removed.push(r),
                Err(e) => tracing::warn!("storage sweep: keeping {}: {e}", w.path),
            }
        }
        for b in branches {
            if !owners.branch(&b.name).finished || b.locked.is_some() || b.unmerged_commits != 0 {
                continue;
            }
            match self.git.delete_branch(self.repo, &b.name, &b.head).await {
                Ok(()) => removed.push(Removed { worktree: None, branch: Some(b.name), head: Some(b.head), bytes: 0 }),
                Err(e) => tracing::warn!("storage sweep: keeping {}: {e}", b.name),
            }
        }
        Ok(removed)
    }

    /// Remove a judged worktree, and its `codly/…` branch when `with_branch`. Without `force`
    /// git itself refuses a worktree that got an uncommitted file since it was judged.
    async fn remove_worktree(&self, w: &StoredWorktree, force: bool, with_branch: bool) -> Result<Removed> {
        let path = Path::new(&w.path);
        let bytes = measure(path.to_path_buf()).await.bytes;
        self.git.worktree_remove(self.repo, path, None, force).await?;
        let mut removed = Removed { worktree: Some(w.path.clone()), branch: None, head: w.head.clone(), bytes };
        if let (true, Some(branch), Some(head)) = (with_branch, w.branch.as_deref().filter(|b| b.starts_with("codly/")), w.head.as_deref()) {
            self.git
                .delete_branch(self.repo, branch, head)
                .await
                .map_err(|e| anyhow!("the worktree {} is deleted, but its branch stays: {e}", w.path))?;
            removed.branch = Some(branch.to_string());
        }
        Ok(removed)
    }

    /// The user deletes one worktree. `head` is the commit the user saw it at. One with nothing
    /// to lose goes with its branch. A dirty or unmerged one needs `force` (the user confirmed
    /// what is lost) and keeps its branch unless `with_branch`.
    pub async fn delete_worktree(&self, owners: &Owners, path: &Path, head: Option<&str>, force: bool, with_branch: bool) -> Result<Removed> {
        let want = canonical(path);
        let Some(info) = self.worktrees().await?.0.into_iter().find(|w| canonical(&w.path) == want) else {
            bail!("{} is not one of Codly's worktrees of this repository", path.display());
        };
        let w = self.judge_worktree(&info, &self.kept_branches().await?, &owners.worktree(&info.path, info.branch.as_deref())).await;
        if let Some(why) = &w.locked {
            bail!("it cannot be deleted now: {why}");
        }
        if w.head.as_deref() != head {
            bail!("this worktree has a new commit since the list was made; look at the list again");
        }
        if w.state == WorkState::Merged {
            return self.remove_worktree(&w, false, true).await;
        }
        if !force {
            bail!("not deleted: it has {}; confirm that this can go", lost(&w));
        }
        self.remove_worktree(&w, true, with_branch).await
    }

    /// The user deletes one `codly/…` branch that no worktree has checked out. `head` is the
    /// commit the user saw it at. Commits that no other branch has need `force`.
    pub async fn delete_branch(&self, owners: &Owners, name: &str, head: &str, force: bool) -> Result<Removed> {
        if !name.starts_with("codly/") {
            bail!("{name} is not one of Codly's branches");
        }
        let (_, branches) = self.inspect(owners).await?;
        let Some(b) = branches.into_iter().find(|b| b.name == name) else {
            bail!("{name} does not exist any more, or a worktree has it checked out");
        };
        if let Some(why) = &b.locked {
            bail!("it cannot be deleted now: {why}");
        }
        if b.head != head {
            bail!("{name} has a new commit since the list was made; look at the list again");
        }
        if b.unmerged_commits > 0 && !force {
            bail!("not deleted: {name} has {}; confirm that this can go", commits(b.unmerged_commits, true));
        }
        self.git.delete_branch(self.repo, name, head).await?;
        Ok(Removed { worktree: None, branch: Some(b.name), head: Some(b.head), bytes: 0 })
    }
}

/// "2 files that are not committed (a.rs, b.rs) and 3 commits that no other branch has".
fn lost(w: &StoredWorktree) -> String {
    let mut parts = vec![];
    if w.dirty_count > 0 {
        let files = if w.dirty_count == 1 { "1 file that is".to_string() } else { format!("{} files that are", w.dirty_count) };
        let shown = &w.dirty_files[..w.dirty_files.len().min(3)];
        let names = match w.dirty_count - shown.len() as i64 {
            0 => shown.join(", "),
            more => format!("{} and {more} more", shown.join(", ")),
        };
        parts.push(format!("{files} not committed ({names})"));
    }
    if w.unmerged_commits > 0 {
        parts.push(commits(w.unmerged_commits, w.branch.is_some()));
    }
    parts.join(" and ")
}

/// Commits that go with a branch ("no other branch has") or with a worktree on no branch.
fn commits(n: i64, on_a_branch: bool) -> String {
    match (n, on_a_branch) {
        (1, true) => "1 commit that no other branch has".to_string(),
        (_, true) => format!("{n} commits that no other branch has"),
        (1, false) => "1 commit that is on no branch".to_string(),
        (_, false) => format!("{n} commits that are on no branch"),
    }
}

/// Fill in `size_bytes` (a walk of every worktree: slow on big ones).
pub async fn with_sizes(mut worktrees: Vec<StoredWorktree>) -> Vec<StoredWorktree> {
    for w in &mut worktrees {
        w.size_bytes = measure(PathBuf::from(&w.path)).await.bytes;
    }
    worktrees
}

/// `usage` off the async threads.
pub async fn measure(path: PathBuf) -> DirUsage {
    tokio::task::spawn_blocking(move || usage(&path)).await.unwrap_or_default()
}

/// Disk space under `path` (symlinks are not followed; a file with several names counts once)
/// and the number of entries directly in it. What cannot be read counts as nothing.
pub fn usage(path: &Path) -> DirUsage {
    let mut total = DirUsage::default();
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut stack = vec![(path.to_path_buf(), true)];
    while let Some((dir, top)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if top {
                total.items += 1;
            }
            if meta.is_dir() {
                stack.push((entry.path(), false));
            }
            if meta.nlink() > 1 && !meta.is_dir() && !seen.insert((meta.dev(), meta.ino())) {
                continue;
            }
            total.bytes += meta.blocks() as i64 * 512;
        }
    }
    total
}

/// What `prune_runs` removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunedRuns {
    pub runs: Vec<String>,
    pub bytes: i64,
}

/// Remove the run folders in `dir` where nothing was written for `max_age`, except the runs
/// in `keep` (live ones). A folder that cannot be removed is kept and logged.
pub fn prune_runs(dir: &Path, max_age: Duration, keep: &HashSet<String>, now: SystemTime) -> PrunedRuns {
    let mut pruned = PrunedRuns::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return pruned };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        // `file_type` does not follow a symlink: only real folders are run folders.
        if keep.contains(&id) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        let Some(last_write) = newest(&path) else { continue };
        if now.duration_since(last_write).is_ok_and(|age| age > max_age) {
            let bytes = usage(&path).bytes;
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    pruned.runs.push(id);
                    pruned.bytes += bytes;
                }
                Err(e) => tracing::warn!("could not remove the old run log {}: {e}", path.display()),
            }
        }
    }
    pruned
}

/// The newest modification time of a run folder and the files in it.
fn newest(dir: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::metadata(dir).and_then(|m| m.modified()).ok()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(t) = entry.metadata().and_then(|m| m.modified()) {
            newest = newest.max(t);
        }
    }
    Some(newest)
}

fn modified_ms(path: &Path) -> i64 {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// Canonical path for comparisons (macOS: /var → /private/var); the input if it fails.
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A repository on `main` with one commit, and an empty folder for Codly's worktrees.
    struct Fixture {
        _dir: tempfile::TempDir,
        git: Git,
        repo: PathBuf,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let (repo, root) = (dir.path().join("repo"), dir.path().join("worktrees"));
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::create_dir_all(&root).unwrap();
            sh(&repo, &["init", "-q", "-b", "main"]);
            sh(&repo, &["config", "user.name", "T"]);
            sh(&repo, &["config", "user.email", "t@t"]);
            std::fs::write(repo.join("a.txt"), "a\n").unwrap();
            std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
            sh(&repo, &["add", "-A"]);
            sh(&repo, &["commit", "-qm", "init"]);
            Fixture { _dir: dir, git: Git::from_process_env(), repo, root }
        }

        fn site(&self) -> Site<'_> {
            Site { git: &self.git, repo: &self.repo, root: &self.root }
        }

        /// A worktree `<root>/<name>` on the new branch `codly/<name>`, started at `start`.
        async fn worktree(&self, name: &str, start: &str) -> PathBuf {
            let wt = self.root.join(name);
            self.git.worktree_add(&self.repo, &wt, &format!("codly/{name}"), start).await.unwrap();
            wt
        }

        fn commit(&self, wt: &Path, file: &str) {
            std::fs::write(wt.join(file), file).unwrap();
            sh(wt, &["add", "-A"]);
            sh(wt, &["commit", "-qm", file]);
        }

        async fn has_branch(&self, name: &str) -> bool {
            self.git.branch_exists(&self.repo, name).await.unwrap()
        }
    }

    fn finished() -> Owner {
        Owner { finished: true, ..Default::default() }
    }

    fn named<'a>(listed: &'a [StoredWorktree], name: &str) -> &'a StoredWorktree {
        listed.iter().find(|w| w.path.ends_with(&format!("/{name}"))).unwrap()
    }

    #[tokio::test]
    async fn the_sweep_removes_only_what_is_finished_and_has_nothing_to_lose() {
        let f = Fixture::new();
        let merged = f.worktree("merged", "main").await;
        f.commit(&merged, "m.txt");
        sh(&f.repo, &["merge", "-q", "--no-ff", "-m", "merge", "codly/merged"]);
        // Ignored build output is not work: it does not keep a worktree.
        std::fs::create_dir_all(merged.join("target")).unwrap();
        std::fs::write(merged.join("target/out.bin"), vec![b'x'; 8192]).unwrap();
        let dirty = f.worktree("dirty", "main").await;
        std::fs::write(dirty.join("notes.txt"), "not committed").unwrap();
        let unmerged = f.worktree("unmerged", "main").await;
        f.commit(&unmerged, "u.txt");
        let open = f.worktree("open", "main").await;
        let busy = f.worktree("busy", "main").await;

        let mut owners = Owners::default();
        for wt in [&merged, &dirty, &unmerged, &busy] {
            owners.add_worktree(wt, finished());
        }
        owners.lock_worktree(&busy, "Ana is working there now".into());
        // `open` has no finished owner: Codly keeps what it has no record of.

        let (listed, _) = f.site().inspect(&owners).await.unwrap();
        assert_eq!(
            ["merged", "dirty", "unmerged", "open", "busy"].map(|name| named(&listed, name).state),
            [WorkState::Merged, WorkState::Dirty, WorkState::Unmerged, WorkState::Merged, WorkState::Merged]
        );
        let d = named(&listed, "dirty");
        assert_eq!((d.dirty_files.as_slice(), d.dirty_count), (&["notes.txt".to_string()][..], 1));
        assert_eq!(named(&listed, "unmerged").unmerged_commits, 1);
        assert!(named(&listed, "busy").locked.is_some());

        let removed = f.site().sweep(&owners).await.unwrap();
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert_eq!(removed[0].branch.as_deref(), Some("codly/merged"));
        assert!(removed[0].bytes > 0, "the freed space is measured before the folder goes");
        assert!(!merged.exists() && !f.has_branch("codly/merged").await);
        for (wt, branch) in [(&dirty, "codly/dirty"), (&unmerged, "codly/unmerged"), (&open, "codly/open"), (&busy, "codly/busy")] {
            assert!(wt.exists() && f.has_branch(branch).await, "{branch} must survive the sweep");
        }
        assert_eq!(std::fs::read_to_string(dirty.join("notes.txt")).unwrap(), "not committed");
        assert!(f.site().sweep(&owners).await.unwrap().is_empty(), "a second sweep finds nothing more");
    }

    #[tokio::test]
    async fn a_task_merged_into_its_plan_has_nothing_to_lose_while_the_plan_branch_stays() {
        let f = Fixture::new();
        let integration = f.worktree("plan/integration", "main").await;
        let task = f.worktree("plan/t1", "main").await;
        f.commit(&task, "t1.txt");
        sh(&integration, &["merge", "-q", "--no-ff", "-m", "merge t1", "codly/plan/t1"]);

        let mut owners = Owners::default();
        owners.add_worktree(&task, finished());
        owners.add_worktree(&integration, finished());
        let (listed, _) = f.site().inspect(&owners).await.unwrap();
        assert!(listed.iter().all(|w| w.state == WorkState::Unmerged), "main has neither: {listed:?}");

        // The Core names the plan's branch as the place the task was merged into.
        let mut owners = Owners::default();
        owners.add_worktree(&task, Owner { merged_into: Some("codly/plan/integration".into()), ..finished() });
        owners.add_worktree(&integration, finished());
        let removed = f.site().sweep(&owners).await.unwrap();
        assert_eq!(removed.iter().map(|r| r.branch.as_deref()).collect::<Vec<_>>(), [Some("codly/plan/t1")]);
        assert!(!task.exists() && integration.exists() && f.has_branch("codly/plan/integration").await);
    }

    #[tokio::test]
    async fn a_folder_on_no_branch_loses_only_the_commits_that_no_branch_has() {
        let f = Fixture::new();
        let home = f.worktree("home", "main").await;
        f.commit(&home, "h.txt");
        // A team member's folder between tasks: detached, at the tip of its last task's branch.
        f.git.detach(&home).await.unwrap();
        let (listed, _) = f.site().inspect(&Owners::default()).await.unwrap();
        assert_eq!((listed[0].branch.as_deref(), listed[0].state), (None, WorkState::Merged), "the commit stays on codly/home");
        f.commit(&home, "more.txt");
        let (listed, _) = f.site().inspect(&Owners::default()).await.unwrap();
        assert_eq!((listed[0].state, listed[0].unmerged_commits), (WorkState::Unmerged, 1));
        let err = f.site().delete_worktree(&Owners::default(), &home, listed[0].head.as_deref(), false, false).await.unwrap_err().to_string();
        assert!(err.contains("1 commit that is on no branch"), "{err}");
    }

    #[tokio::test]
    async fn a_folder_is_judged_with_the_record_of_the_branch_it_holds() {
        let f = Fixture::new();
        let integration = f.worktree("plan/integration", "main").await;
        // A team member's folder has no task record: the task is known by the branch it holds.
        let home = f.root.join("team/ana");
        f.git.worktree_add(&f.repo, &home, "codly/plan/t3", "main").await.unwrap();
        f.commit(&home, "t3.txt");
        sh(&integration, &["merge", "-q", "--no-ff", "-m", "merge t3", "codly/plan/t3"]);
        let head = f.git.head(&home).await.unwrap();
        let records = |locked: Option<&str>| {
            let mut owners = Owners::default();
            owners.add_worktree(&home, Owner { label: Some("Ana's own folder".into()), ..Default::default() });
            let task = Owner { locked: locked.map(String::from), merged_into: Some("codly/plan/integration".into()), ..finished() };
            owners.add_branch("codly/plan/t3", task);
            owners
        };

        // The task's plan still runs: its lock and its plan's branch count for the folder too.
        let owners = records(Some("its plan is not finished"));
        let (listed, _) = f.site().inspect(&owners).await.unwrap();
        let w = named(&listed, "ana");
        assert_eq!((w.state, w.owner.as_deref(), w.locked.as_deref()), (WorkState::Merged, Some("Ana's own folder"), Some("its plan is not finished")));
        std::fs::write(home.join("draft.txt"), "x").unwrap();
        for (force, with_branch) in [(false, false), (true, false), (true, true)] {
            let err = f.site().delete_worktree(&owners, &home, head.as_deref(), force, with_branch).await.unwrap_err().to_string();
            assert!(err.contains("its plan is not finished"), "{err}");
        }
        assert!(home.join("draft.txt").exists() && f.has_branch("codly/plan/t3").await);
        std::fs::remove_file(home.join("draft.txt")).unwrap();

        // The plan is over. The folder itself is never finished, so the sweep leaves it; you
        // can delete it, and it has nothing to lose: its commit is in the plan's branch.
        let owners = records(None);
        assert!(f.site().sweep(&owners).await.unwrap().is_empty());
        let removed = f.site().delete_worktree(&owners, &home, head.as_deref(), false, false).await.unwrap();
        assert_eq!(removed.branch.as_deref(), Some("codly/plan/t3"));
        assert!(!home.exists() && !f.has_branch("codly/plan/t3").await);
    }

    #[tokio::test]
    async fn a_branch_named_like_a_folder_a_file_or_a_tag_is_read_as_the_branch() {
        let f = Fixture::new();
        // You work on the branch "docs" in a repository that has a folder "docs/".
        sh(&f.repo, &["switch", "-q", "-c", "docs"]);
        std::fs::create_dir(f.repo.join("docs")).unwrap();
        f.commit(&f.repo, "docs/guide.md");
        let merged = f.worktree("merged", "HEAD").await;
        f.commit(&merged, "m.txt");
        sh(&f.repo, &["merge", "-q", "--no-ff", "-m", "merge", "codly/merged"]);
        let unmerged = f.worktree("unmerged", "HEAD").await;
        f.commit(&unmerged, "u.txt");
        sh(&f.repo, &["branch", "codly/old", "HEAD"]);
        // A tag "docs" at an older commit, and a built program "main" next to the branch "main".
        sh(&f.repo, &["tag", "docs", "refs/heads/main"]);
        std::fs::write(f.repo.join("main"), "binary").unwrap();

        let mut owners = Owners::default();
        owners.add_worktree(&merged, finished());
        owners.add_worktree(&unmerged, finished());
        owners.add_branch("codly/old", finished());
        let (listed, branches) = f.site().inspect(&owners).await.unwrap();
        assert!(listed.iter().all(|w| w.locked.is_none()), "{listed:?}");
        assert_eq!(named(&listed, "merged").state, WorkState::Merged, "the branch docs has its commit; the tag docs does not");
        assert_eq!((named(&listed, "unmerged").state, named(&listed, "unmerged").unmerged_commits), (WorkState::Unmerged, 1));
        assert_eq!(branches.iter().map(|b| (b.name.as_str(), b.unmerged_commits)).collect::<Vec<_>>(), [("codly/old", 0)]);

        let removed = f.site().sweep(&owners).await.unwrap();
        assert_eq!(removed.iter().map(|r| r.branch.as_deref()).collect::<Vec<_>>(), [Some("codly/merged"), Some("codly/old")]);
        assert!(unmerged.exists() && f.has_branch("codly/unmerged").await);
    }

    #[tokio::test]
    async fn branches_without_a_worktree_go_only_when_merged_and_finished() {
        let f = Fixture::new();
        for name in ["codly/done", "codly/unmerged", "codly/unknown", "codly/busy", "feature"] {
            sh(&f.repo, &["branch", name, "main"]);
        }
        let wt = f.worktree("scratch", "codly/unmerged").await;
        f.commit(&wt, "s.txt");
        sh(&f.repo, &["branch", "-f", "codly/unmerged", "codly/scratch"]);

        let mut owners = Owners::default();
        owners.add_branch("codly/done", Owner { label: Some("Task t1: Fix the login".into()), ..finished() });
        owners.add_branch("codly/unmerged", finished());
        owners.add_branch("codly/busy", Owner { locked: Some("task t2 is not finished".into()), ..finished() });
        let (_, branches) = f.site().inspect(&owners).await.unwrap();
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["codly/busy", "codly/done", "codly/unknown", "codly/unmerged"], "checked-out and non-Codly branches are not listed");
        assert_eq!(branches[1].owner.as_deref(), Some("Task t1: Fix the login"));
        assert_eq!(branches[3].unmerged_commits, 1);

        let removed = f.site().sweep(&owners).await.unwrap();
        assert_eq!(removed.iter().map(|r| r.branch.as_deref()).collect::<Vec<_>>(), [Some("codly/done")]);
        for name in ["codly/unmerged", "codly/unknown", "codly/busy", "codly/scratch", "feature"] {
            assert!(f.has_branch(name).await, "{name} must stay");
        }
    }

    #[tokio::test]
    async fn deleting_work_needs_a_confirm_and_keeps_the_branch_unless_asked() {
        let f = Fixture::new();
        let owners = Owners::default();
        let wt = f.worktree("wip", "main").await;
        f.commit(&wt, "c.txt");
        std::fs::write(wt.join("draft.txt"), "x").unwrap();
        let head = f.git.head(&wt).await.unwrap();

        let err = f.site().delete_worktree(&owners, &wt, head.as_deref(), false, false).await.unwrap_err().to_string();
        assert!(err.contains("1 file that is not committed (draft.txt)") && err.contains("1 commit that no other branch has"), "{err}");
        let err = f.site().delete_worktree(&owners, &wt, Some("0000000000000000000000000000000000000000"), true, true).await.unwrap_err().to_string();
        assert!(err.contains("new commit since the list was made"), "{err}");
        assert!(wt.exists());

        let removed = f.site().delete_worktree(&owners, &wt, head.as_deref(), true, false).await.unwrap();
        assert_eq!((removed.branch, removed.head), (None, head.clone()));
        assert!(!wt.exists() && f.has_branch("codly/wip").await, "the commits stay on the branch");

        // The branch is listed now; it has a commit nobody else has, so it also needs a confirm.
        let head = head.unwrap();
        let err = f.site().delete_branch(&owners, "codly/wip", &head, false).await.unwrap_err().to_string();
        assert!(err.contains("1 commit that no other branch has"), "{err}");
        f.site().delete_branch(&owners, "codly/wip", &head, true).await.unwrap();
        assert!(!f.has_branch("codly/wip").await);

        // "Delete the branch too" takes both in one go.
        let wt = f.worktree("both", "main").await;
        f.commit(&wt, "d.txt");
        let head = f.git.head(&wt).await.unwrap();
        let removed = f.site().delete_worktree(&owners, &wt, head.as_deref(), true, true).await.unwrap();
        assert_eq!(removed.branch.as_deref(), Some("codly/both"));
        assert!(!wt.exists() && !f.has_branch("codly/both").await);
    }

    #[tokio::test]
    async fn deletes_are_refused_for_what_is_in_use_or_not_codlys() {
        let f = Fixture::new();
        let wt = f.worktree("busy", "main").await;
        let head = f.git.head(&wt).await.unwrap();
        let mut owners = Owners::default();
        owners.lock_worktree(&wt, "Ana is working there now".into());
        let err = f.site().delete_worktree(&owners, &wt, head.as_deref(), true, true).await.unwrap_err().to_string();
        assert!(err.contains("Ana is working there now"), "{err}");

        // The user's own checkout and worktrees outside Codly's folder are never Codly's to delete.
        let outside = f.repo.parent().unwrap().join("mine");
        f.git.worktree_add(&f.repo, &outside, "codly/mine", "main").await.unwrap();
        for path in [&f.repo, &outside] {
            let err = f.site().delete_worktree(&Owners::default(), path, head.as_deref(), true, true).await.unwrap_err().to_string();
            assert!(err.contains("not one of Codly's worktrees"), "{err}");
        }
        let head = head.unwrap();
        assert!(f.site().delete_branch(&Owners::default(), "main", &head, true).await.is_err());
        let err = f.site().delete_branch(&Owners::default(), "codly/mine", &head, true).await.unwrap_err().to_string();
        assert!(err.contains("a worktree has it checked out"), "{err}");
        assert!(wt.exists() && outside.exists() && f.has_branch("codly/mine").await && f.has_branch("main").await);
    }

    #[tokio::test]
    async fn a_merged_worktree_goes_with_its_branch_in_one_click() {
        let f = Fixture::new();
        let wt = f.worktree("ok", "main").await;
        let head = f.git.head(&wt).await.unwrap();
        let removed = f.site().delete_worktree(&Owners::default(), &wt, head.as_deref(), false, false).await.unwrap();
        assert_eq!(removed.branch.as_deref(), Some("codly/ok"));
        assert!(!wt.exists() && !f.has_branch("codly/ok").await);
    }

    #[tokio::test]
    async fn a_folder_that_is_not_the_repos_own_worktree_is_listed_but_never_read() {
        let f = Fixture::new();
        let wt = f.worktree("forged", "main").await;
        let fake = f.root.join("fake.git");
        sh(&f.root, &["init", "-q", "--bare", &fake.to_string_lossy()]);
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", fake.display())).unwrap();
        let mut owners = Owners::default();
        owners.add_worktree(&wt, finished());
        let (listed, _) = f.site().inspect(&owners).await.unwrap();
        assert_eq!(listed[0].state, WorkState::Unknown);
        assert!(listed[0].locked.as_deref().is_some_and(|why| why.contains("could not read")), "{:?}", listed[0].locked);
        assert!(f.site().sweep(&owners).await.unwrap().is_empty());
        assert!(wt.exists());
    }

    #[test]
    fn two_records_for_one_worktree_are_joined_on_the_safe_side() {
        let mut owners = Owners::default();
        let path = Path::new("/nowhere/wt");
        owners.add_worktree(path, Owner { label: Some("Task t1".into()), ..finished() });
        owners.add_worktree(path, Owner { locked: Some("task t2 is not finished".into()), ..Default::default() });
        let joined = owners.worktree(path, None);
        assert_eq!((joined.finished, joined.label.as_deref(), joined.locked.as_deref()), (false, Some("Task t1"), Some("task t2 is not finished")));
    }

    #[test]
    fn old_run_logs_go_and_live_or_recent_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let now = SystemTime::now();
        let age = |days: u64| now - Duration::from_secs(days * 86_400);
        let run = |id: &str, written: SystemTime| {
            let d = runs.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("raw.ndjson"), vec![b'x'; 4096]).unwrap();
            std::fs::File::open(d.join("raw.ndjson")).unwrap().set_modified(written).unwrap();
            std::fs::File::open(&d).unwrap().set_modified(written).unwrap();
        };
        run("old", age(40));
        run("recent", age(3));
        run("live", age(40));
        // A folder made long ago whose log was written to yesterday is not old.
        run("long", age(40));
        std::fs::File::open(runs.join("long/raw.ndjson")).unwrap().set_modified(age(1)).unwrap();
        std::fs::write(runs.join("stray.txt"), "not a run").unwrap();

        let keep = HashSet::from(["live".to_string()]);
        let pruned = prune_runs(&runs, Duration::from_secs(30 * 86_400), &keep, now);
        assert_eq!(pruned.runs, ["old"]);
        assert!(pruned.bytes > 0);
        assert!(!runs.join("old").exists());
        for kept in ["recent", "live", "long", "stray.txt"] {
            assert!(runs.join(kept).exists(), "{kept} must stay");
        }
        assert_eq!(usage(&runs).items, 4);
        assert_eq!(prune_runs(&dir.path().join("missing"), Duration::ZERO, &keep, now), PrunedRuns::default());
    }

    #[test]
    fn usage_does_not_follow_links_and_counts_a_shared_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two, outside) = (dir.path().join("one"), dir.path().join("two"), dir.path().join("out"));
        for d in [&one, &two, &outside] {
            std::fs::create_dir_all(d.join("sub")).unwrap();
        }
        let file = vec![b'x'; 256 * 1024];
        std::fs::write(one.join("sub/a.bin"), &file).unwrap();
        std::fs::write(two.join("sub/a.bin"), &file).unwrap();
        std::fs::hard_link(two.join("sub/a.bin"), two.join("same.bin")).unwrap();
        std::fs::write(outside.join("big.bin"), vec![b'x'; 4 * 1024 * 1024]).unwrap();
        std::os::unix::fs::symlink(&outside, two.join("link")).unwrap();
        let (u1, u2) = (usage(&one), usage(&two));
        assert_eq!((u1.items, u2.items), (1, 3));
        assert!(u1.bytes > 0 && u2.bytes < u1.bytes + 128 * 1024, "the second name and the link add no file: {u1:?} {u2:?}");
    }

    #[test]
    fn the_refusal_names_what_would_be_lost() {
        let mut w = StoredWorktree {
            path: "/w".into(),
            branch: None,
            head: None,
            owner: None,
            state: WorkState::Dirty,
            dirty_files: (1..=5).map(|i| format!("f{i}.rs")).collect(),
            dirty_count: 5,
            unmerged_commits: 2,
            locked: None,
            last_used_at: 0,
            size_bytes: 0,
        };
        assert_eq!(lost(&w), "5 files that are not committed (f1.rs, f2.rs, f3.rs and 2 more) and 2 commits that are on no branch");
        w.dirty_count = 0;
        w.branch = Some("codly/x".into());
        assert_eq!(lost(&w), "2 commits that no other branch has");
    }
}
