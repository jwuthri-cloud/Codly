//! The Git tab of an agent's sheet: read-only state of its worktree.
//!
//! Status of the working tree, local branches, and the recent commit log. Every git argv is a
//! fixed list (the only variable is the `-C` directory, the agent's own worktree), and the
//! worktree is checked to be a real linked worktree of the repo before git runs in it (an agent
//! can rewrite its `.git` pointer; see `git.rs`). The parsers are pure and unit-tested.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::Core;

/// One changed path in the worktree (`git status --porcelain`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitFileStatus {
    /// The two-letter porcelain code ("M ", " M", "A ", "??", "R "…).
    pub code: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    pub sha: String,
    /// Unix ms of the branch tip's commit.
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitLogEntry {
    /// Abbreviated sha.
    pub sha: String,
    pub subject: String,
    pub author: String,
    /// Unix ms of the commit.
    pub ts: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentGit {
    /// None when the agent has no worktree (the Lead, an agent not yet started).
    pub worktree: Option<String>,
    /// Short branch name; None when detached.
    pub branch: Option<String>,
    pub files: Vec<GitFileStatus>,
    pub branches: Vec<GitBranch>,
    pub log: Vec<GitLogEntry>,
    /// Why part of the view is empty, when git could not answer.
    pub note: Option<String>,
}

const LOG_LIMIT: &str = "-n30";
const SEP: char = '\u{1f}';

/// `git log --format=%h%x1f%an%x1f%ct%x1f%s`, one commit per line.
pub fn parse_log(out: &str) -> Vec<GitLogEntry> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, SEP);
            let (sha, author, ts, subject) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(GitLogEntry { sha: sha.into(), author: author.into(), ts: ts.parse::<i64>().ok()? * 1000, subject: subject.into() })
        })
        .collect()
}

/// `git status --porcelain=v1 -z`: `XY path\0`, and a renamed or copied path is followed by
/// its origin as a second NUL-separated field (dropped: the new path is what changed).
pub fn parse_status(out: &str) -> Vec<GitFileStatus> {
    let mut files = vec![];
    let mut parts = out.split('\0');
    while let Some(p) = parts.next() {
        if p.len() < 4 || !p.is_char_boundary(3) {
            continue;
        }
        let (code, path) = p.split_at(2);
        if code.contains(['R', 'C']) {
            parts.next();
        }
        files.push(GitFileStatus { code: code.into(), path: path[1..].into() });
    }
    files
}

/// `git for-each-ref --format=%(HEAD)%1f%(refname:short)%1f%(objectname:short)%1f%(committerdate:unix) refs/heads`.
pub fn parse_branches(out: &str) -> Vec<GitBranch> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, SEP);
            let (head, name, sha, ts) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(GitBranch { name: name.into(), current: head == "*", sha: sha.into(), ts: ts.parse::<i64>().ok()? * 1000 })
        })
        .collect()
}

impl Core {
    /// Git state of an agent's worktree. Never mutates the repository.
    pub async fn agent_git(&self, agent_id: &str) -> anyhow::Result<AgentGit> {
        let inner = &self.inner;
        let agent = inner.agent(agent_id)?;
        let Some(wt) = agent.worktree.clone().filter(|w| Path::new(w).is_dir()) else {
            return Ok(AgentGit { worktree: agent.worktree, note: Some("This agent has no worktree yet.".into()), ..Default::default() });
        };
        let wt_path = Path::new(&wt);
        let mut view = AgentGit { worktree: Some(wt.clone()), ..Default::default() };
        let repo = inner.require_ws()?.repo_path;
        if let Err(e) = inner.git.check_worktree(Path::new(&repo), wt_path).await {
            view.note = Some(format!("{e}"));
            return Ok(view);
        }
        let git = &inner.git;
        view.branch = git.current_branch(wt_path).await.unwrap_or(None);
        match git.read(wt_path, &["status", "--porcelain=v1", "-z", "--untracked-files=normal"]).await {
            Ok(o) => view.files = parse_status(&o),
            Err(e) => view.note = Some(format!("{e}")),
        }
        let refs = "--format=%(HEAD)%1f%(refname:short)%1f%(objectname:short)%1f%(committerdate:unix)";
        if let Ok(o) = git.read(wt_path, &["for-each-ref", "--sort=-committerdate", "--count=30", refs, "refs/heads"]).await {
            view.branches = parse_branches(&o);
        }
        // A repository without commits has no log; that is not an error worth showing.
        if let Ok(o) = git.read(wt_path, &["log", LOG_LIMIT, "--format=%h%x1f%an%x1f%ct%x1f%s"]).await {
            view.log = parse_log(&o);
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_commit_log() {
        let out = "a1b2c3d\u{1f}Ana\u{1f}1760000000\u{1f}Add dark mode\nf00ba12\u{1f}Leo\u{1f}1759990000\u{1f}Fix: subject with \u{1f} stray separator\n\nbroken line\n";
        let log = parse_log(out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0], GitLogEntry { sha: "a1b2c3d".into(), author: "Ana".into(), ts: 1_760_000_000_000, subject: "Add dark mode".into() });
        assert!(log[1].subject.starts_with("Fix: subject with "));
        assert!(parse_log("").is_empty());
    }

    #[test]
    fn parses_status_including_renames_and_untracked() {
        let out = " M src/a.rs\0A  new.txt\0R  moved.rs\0old.rs\0?? scratch/n.md\0";
        let files = parse_status(out);
        let got: Vec<(&str, &str)> = files.iter().map(|f| (f.code.as_str(), f.path.as_str())).collect();
        assert_eq!(got, [(" M", "src/a.rs"), ("A ", "new.txt"), ("R ", "moved.rs"), ("??", "scratch/n.md")]);
        assert!(parse_status("").is_empty());
    }

    #[test]
    fn parses_branches() {
        let out = "*\u{1f}feat/x\u{1f}abc1234\u{1f}1760000000\n \u{1f}main\u{1f}def5678\u{1f}1750000000\n";
        let b = parse_branches(out);
        assert_eq!(b.len(), 2);
        assert!(b[0].current && !b[1].current);
        assert_eq!(b[1].name, "main");
    }

    #[tokio::test]
    async fn reads_a_real_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let sh = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(d).status().unwrap().success());
        sh(&["init", "-q", "-b", "main"]);
        sh(&["config", "user.email", "t@t"]);
        sh(&["config", "user.name", "Tess"]);
        std::fs::write(d.join("a.txt"), "1").unwrap();
        sh(&["add", "-A"]);
        sh(&["commit", "-q", "-m", "first"]);
        std::fs::write(d.join("a.txt"), "2").unwrap();
        let git = crate::git::Git::from_process_env();
        let log = parse_log(&git.read(d, &["log", LOG_LIMIT, "--format=%h%x1f%an%x1f%ct%x1f%s"]).await.unwrap());
        assert_eq!((log.len(), log[0].subject.as_str(), log[0].author.as_str()), (1, "first", "Tess"));
        let st = parse_status(&git.read(d, &["status", "--porcelain=v1", "-z"]).await.unwrap());
        assert_eq!(st, [GitFileStatus { code: " M".into(), path: "a.txt".into() }]);
    }
}
