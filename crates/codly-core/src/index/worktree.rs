//! A search asked from an agent's worktree. The index covers the user's checkout; an agent
//! works in a git worktree on another branch, where files were changed, added or removed
//! (by its own task, or by tasks merged into the plan's branch before it). This module
//! asks git which files those are and makes the answer say so: a match in a file the
//! worktree does not have is left out, a match in a file that differs there is marked, and
//! the files that exist only there are named, because the index cannot search them.
// ponytail: the files that differ are not searched as they are in the worktree. The upgrade is
// a small overlay index of those files, ranked together with the checkout's index.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow};

use super::query::Ranked;
use super::{Index, lang, walk};
use crate::git::Git;
use crate::model::Confidence;

/// First entry of `Citation::why` when the cited file is not the same in the asker's worktree.
const DIFFERS: &str = "differs in your worktree: read the file";

/// With more files than this that differ, the worktree is on other code altogether: the files
/// that are new there are not looked for one by one.
const MAX_COMPARED: usize = 2000;
/// New files named in a warning.
const MAX_NAMED: usize = 8;

/// An agent's worktree, compared with the checkout the index covers.
#[derive(Debug)]
pub struct WorktreeView {
    /// The worktree's root: citations are paths below it.
    root: PathBuf,
    /// What git said about the two checkouts, or why it could not.
    compared: Result<Compared, String>,
}

#[derive(Debug)]
struct Compared {
    /// What the index covers, as an agent can name it: `main@1a2b3c4`.
    indexed: String,
    /// Repo-relative paths whose file is not the same in the worktree and in the checkout:
    /// what the commits changed between the two, and what is uncommitted in either.
    differs: BTreeSet<String>,
}

impl WorktreeView {
    /// Compare `worktree` (the root of a git worktree of the repository) with `checkout` (the
    /// indexed one). Takes at most `within`; a view that could not be compared makes the
    /// search say so.
    pub async fn read(git: &Git, checkout: &Path, worktree: &Path, within: Duration) -> WorktreeView {
        let root = worktree.canonicalize().unwrap_or_else(|_| worktree.to_path_buf());
        let compared = match tokio::time::timeout(within, compare(git, checkout, &root)).await {
            Ok(Ok(c)) => Ok(c),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(_) => Err(format!("git did not answer in {} s", within.as_secs())),
        };
        WorktreeView { root, compared }
    }

    /// Make a ranked answer true for this worktree and cut it to `k` citations: see the module
    /// doc. `r` holds more than `k` candidates, so matches that are left out are replaced.
    pub(super) fn apply(&self, index: &Index, r: &mut Ranked, k: usize) {
        let mut kept = 0;
        let mut gone = 0;
        let keep: Vec<bool> = r
            .result
            .citations
            .iter()
            .map(|c| {
                let take = kept < k && self.root.join(&c.path).is_file();
                if kept < k && !take {
                    gone += 1;
                }
                kept += take as usize;
                take
            })
            .collect();
        let mut it = keep.iter();
        r.result.citations.retain(|_| it.next().copied().unwrap_or(false));
        let mut it = keep.iter();
        r.meta.retain(|_| it.next().copied().unwrap_or(false));

        let mut marked = 0;
        for (i, c) in r.result.citations.iter_mut().enumerate() {
            c.id = format!("C{}", i + 1);
            if self.compared.as_ref().is_ok_and(|v| v.differs.contains(&c.path)) {
                c.why.insert(0, DIFFERS.to_string());
                marked += 1;
            }
        }
        // The confidence was computed with the matches that are now left out. A lower one
        // comes with the rg patterns, as it does from the ranker.
        let confidence = match r.result.confidence {
            _ if r.result.citations.is_empty() => Confidence::Low,
            Confidence::Exact if !r.meta.iter().any(|m| m.level == Confidence::Exact) => Confidence::Medium,
            Confidence::High if gone > 0 => Confidence::Medium,
            c => c,
        };
        r.set_confidence(confidence);

        let warnings = &mut r.result.warnings;
        if gone > 0 {
            warnings.push(format!("Matches left out because the file is not in your worktree: {gone}."));
        }
        let view = match &self.compared {
            Ok(v) => v,
            Err(why) => {
                warnings.push(format!(
                    "The index covers the user's checkout, and your worktree could not be compared with it ({why}). \
                     Lines may be off and new files are not searched: check with rg."
                ));
                return;
            }
        };
        if marked > 0 {
            warnings.push(format!(
                "Cited files that differ in your worktree: {marked} (marked). The index covers the user's checkout ({}), \
                 so their lines may be off: read the file.",
                view.indexed
            ));
        }
        if view.differs.len() > MAX_COMPARED {
            warnings.push(format!(
                "{} files differ between your worktree and the user's checkout ({}), which the index covers: \
                 take every citation as a hint and check with rg.",
                view.differs.len(),
                view.indexed
            ));
            return;
        }
        let new = self.new_files(index, view);
        if !new.is_empty() {
            let more = match new.len().saturating_sub(MAX_NAMED) {
                0 => String::new(),
                n => format!(" and {n} more"),
            };
            warnings.push(format!(
                "Not searched, because they are new in your worktree: {}{more}. Search them with rg.",
                new.iter().take(MAX_NAMED).copied().collect::<Vec<_>>().join(", ")
            ));
        }
    }

    /// The files that differ, are in the worktree, and that the index has never seen in the
    /// checkout (and would index: not ignored, not a lockfile or a binary).
    fn new_files<'a>(&self, index: &Index, view: &'a Compared) -> Vec<&'a str> {
        let mut ignore = walk::IgnoreCheck::new(&self.root);
        view.differs
            .iter()
            .map(String::as_str)
            .filter(|rel| {
                if lang::skip_by_name(rel).is_some() || index.knows(rel) {
                    return false;
                }
                let abs = self.root.join(rel);
                std::fs::metadata(&abs).is_ok_and(|m| m.is_file() && m.len() <= walk::MAX_FILE_BYTES) && !ignore.is_ignored(&abs, false)
            })
            .collect()
    }
}

async fn compare(git: &Git, checkout: &Path, worktree: &Path) -> anyhow::Result<Compared> {
    let head = git.head(checkout).await?.ok_or_else(|| anyhow!("the repository has no commit yet"))?;
    let checkout_commit = [head.as_str()];
    let (branch, ours, theirs, committed) = tokio::join!(
        git.current_branch(checkout),
        git.status_z(checkout),
        git.status_z(worktree),
        // The worktree's files against the checkout's commit: what the branches changed, and
        // the worktree's uncommitted changes to tracked files.
        git.changed_paths(worktree, &checkout_commit),
    );
    let mut differs: BTreeSet<String> = committed.context("comparing the worktree with the checkout's commit")?.into_iter().collect();
    // Uncommitted and untracked files, on either side.
    for status in [ours.context("reading the checkout's status")?, theirs.context("reading the worktree's status")?] {
        differs.extend(status_paths(&status).map(String::from));
    }
    let short = &head[..head.len().min(7)];
    let indexed = match branch? {
        Some(b) => format!("{b}@{short}"),
        None => short.to_string(),
    };
    Ok(Compared { indexed, differs })
}

/// The paths of `git status --porcelain=v1 -z --no-renames`: one entry is "XY <path>".
fn status_paths(status: &str) -> impl Iterator<Item = &str> {
    status.split('\0').filter_map(|entry| entry.get(3..)).filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_entries_give_their_paths() {
        let status = " M src/a.rs\0?? notes/new file.md\0D  gone.rs\0";
        assert_eq!(status_paths(status).collect::<Vec<_>>(), ["src/a.rs", "notes/new file.md", "gone.rs"]);
        assert_eq!(status_paths("").count(), 0);
    }
}
