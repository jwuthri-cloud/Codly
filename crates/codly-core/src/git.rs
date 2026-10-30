//! Git operations by shelling out to the user's `git` (argv, never a shell string).
//! Background reads use GIT_OPTIONAL_LOCKS=0 GIT_TERMINAL_PROMPT=0 LC_ALL=C.
//! See docs/research/agents-backend-orchestration.md §6.6–6.8 and CRITIQUE.md C8, D7.
//!
//! Hook and signing policy (CRITIQUE G9): Codly's own commits (task snapshots) and its
//! merges into `codly/<plan>/integration` are internal checkpoints on Codly-owned branches.
//! They run with `core.hooksPath=/dev/null` and `commit.gpgsign=false`, so a pre-commit
//! formatter, a commit-msg linter, a post-commit auto-push or a signing prompt can never
//! rewrite, block or publish a snapshot; the task's verify commands are the quality gate.
//! The final merge into the user's own branch (`merge(.., internal = false)`, used by
//! `merge_plan`) runs the user's hooks and signing config: that commit is theirs.
//!
//! An agent can edit everything in its worktree, its `.git` pointer file included: pointed at
//! a repository it wrote, git would read that repository's config (`core.fsmonitor`, filter
//! drivers) and run its programs as Codly, outside any sandbox. `check_worktree` refuses a
//! worktree whose git dir is not under the main repository's `.git/worktrees/`; the orchestrator
//! calls it before every commit in an agent's worktree, and internal git calls also turn
//! `core.fsmonitor` off.
//!
//! Callers serialize mutations of one repository (the orchestrator holds a per-workspace
//! lock); reads may run concurrently.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

/// Identity used for Codly's commits when the repository has none configured.
pub const CODLY_NAME: &str = "Codly";
pub const CODLY_EMAIL: &str = "codly@localhost";

/// A hung git (lock wait, fsmonitor, huge repo) must not wedge the orchestrator forever.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Variables that would redirect git away from the `-C` directory (set e.g. when Codly or
/// its tests run inside a git hook).
const LOCATION_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
];

#[derive(Debug, Clone)]
pub struct Git {
    bin: PathBuf,
    env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name; None when detached.
    pub branch: Option<String>,
}

/// Result of `git merge-tree --write-tree` (no working tree or index touched).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCheck {
    pub clean: bool,
    pub conflicts: Vec<String>,
    /// git's informational messages ("CONFLICT (content): Merge conflict in f.txt").
    pub messages: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged {
        sha: String,
    },
    /// The branch was already contained in HEAD (idempotent re-merge).
    AlreadyMerged {
        sha: String,
    },
    /// The merge conflicted and was aborted; the working tree is back to HEAD.
    Conflict {
        files: Vec<String>,
        messages: String,
    },
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Out {
    fn err(&self, what: &str) -> anyhow::Error {
        let msg = if self.stderr.trim().is_empty() { self.stdout.trim() } else { self.stderr.trim() };
        anyhow!("git {what} failed: {msg}")
    }
}

impl Git {
    /// `bin` = resolved git binary; `env` = the base child environment (captured login env).
    pub fn new(bin: PathBuf, mut env: HashMap<String, String>) -> Git {
        for k in LOCATION_VARS {
            env.remove(*k);
        }
        Git { bin, env }
    }

    /// Resolve git on `env["PATH"]`, preferring Homebrew's over the Xcode shim.
    pub fn detect(env: HashMap<String, String>) -> Git {
        let brew = Path::new("/opt/homebrew/bin/git");
        let bin = if brew.exists() {
            brew.to_path_buf()
        } else {
            let path = env.get("PATH").cloned().unwrap_or_default();
            let cwd = std::env::temp_dir();
            which::which_in("git", Some(path), cwd).unwrap_or_else(|_| PathBuf::from("/usr/bin/git"))
        };
        Git::new(bin, env)
    }

    /// The process environment and the first `git` on its PATH (tests, CLI tools).
    pub fn from_process_env() -> Git {
        Git::new(PathBuf::from("git"), std::env::vars().collect())
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    async fn run(&self, dir: &Path, args: &[&str], read: bool) -> Result<Out> {
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.env_clear()
            .envs(&self.env)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_EDITOR", "true")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if read {
            // status/diff must not take index.lock while an agent or Codly commits.
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        let child = cmd.spawn().map_err(|e| anyhow!("cannot run git ({}): {e}", self.bin.display()))?;
        let out = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| anyhow!("git {} timed out after {}s", args.first().unwrap_or(&""), GIT_TIMEOUT.as_secs()))??;
        Ok(Out {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Run and require exit 0; returns stdout.
    async fn ok(&self, dir: &Path, args: &[&str], read: bool) -> Result<String> {
        let out = self.run(dir, args, read).await?;
        if !out.ok {
            return Err(out.err(args.first().copied().unwrap_or("")));
        }
        Ok(out.stdout)
    }

    // ───────────── reads ─────────────

    /// A read-only git command (no index lock, fsmonitor off) for views that only display
    /// repository state. Callers pass fixed argv; nothing here is built from user text.
    pub async fn read(&self, dir: &Path, args: &[&str]) -> Result<String> {
        let mut full = vec!["-c", "core.fsmonitor=false"];
        full.extend_from_slice(args);
        self.ok(dir, &full, true).await
    }

    pub async fn is_repo(&self, path: &Path) -> bool {
        matches!(self.run(path, &["rev-parse", "--is-inside-work-tree"], true).await, Ok(o) if o.ok && o.stdout.trim() == "true")
    }

    pub async fn toplevel(&self, path: &Path) -> Result<PathBuf> {
        let out = self.run(path, &["rev-parse", "--show-toplevel"], true).await?;
        if !out.ok {
            bail!("not a git repository: {}", path.display());
        }
        Ok(PathBuf::from(out.stdout.trim()))
    }

    /// None when HEAD is detached.
    pub async fn current_branch(&self, repo: &Path) -> Result<Option<String>> {
        let out = self.run(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"], true).await?;
        Ok(out.ok.then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty()))
    }

    /// None for a repository without commits.
    pub async fn head(&self, repo: &Path) -> Result<Option<String>> {
        self.rev_parse(repo, "HEAD").await
    }

    /// The commit a revision names, or None if it does not resolve.
    pub async fn rev_parse(&self, repo: &Path, rev: &str) -> Result<Option<String>> {
        let spec = format!("{rev}^{{commit}}");
        let out = self.run(repo, &["rev-parse", "--verify", "--quiet", &spec], true).await?;
        Ok(out.ok.then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty()))
    }

    /// origin/HEAD, else main/master if they exist, else the current branch, else
    /// init.defaultBranch, else "main".
    pub async fn default_branch(&self, repo: &Path) -> Result<String> {
        let out = self.run(repo, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"], true).await?;
        if out.ok
            && let Some(b) = out.stdout.trim().strip_prefix("origin/")
            && !b.is_empty()
        {
            return Ok(b.to_string());
        }
        for b in ["main", "master"] {
            if self.branch_exists(repo, b).await? {
                return Ok(b.to_string());
            }
        }
        if let Some(b) = self.current_branch(repo).await? {
            return Ok(b);
        }
        let out = self.run(repo, &["config", "--get", "init.defaultBranch"], true).await?;
        let b = out.stdout.trim();
        Ok(if out.ok && !b.is_empty() { b.to_string() } else { "main".into() })
    }

    /// Uncommitted changes. `include_untracked` also counts untracked, non-ignored files
    /// (use it before deleting a worktree; leave it off for "may I merge here").
    pub async fn dirty(&self, path: &Path, include_untracked: bool) -> Result<bool> {
        let untracked = if include_untracked { "--untracked-files=normal" } else { "--untracked-files=no" };
        let out = self.ok(path, &["status", "--porcelain=v1", "-z", untracked], true).await?;
        Ok(!out.is_empty())
    }

    /// `diff --name-only <revs>`: ["HEAD"] gives the uncommitted tracked files, [base, branch]
    /// what a branch changed.
    pub async fn changed_paths(&self, path: &Path, revs: &[&str]) -> Result<Vec<String>> {
        let mut args = vec!["-c", "core.fsmonitor=false", "diff", "--name-only", "-z", "--no-renames"];
        args.extend_from_slice(revs);
        args.push("--");
        let out = self.ok(path, &args, true).await?;
        Ok(out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
    }

    pub async fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        let r = format!("refs/heads/{branch}");
        Ok(self.run(repo, &["show-ref", "--verify", "--quiet", &r], true).await?.ok)
    }

    /// Is `ancestor` contained in `descendant`?
    pub async fn is_ancestor(&self, repo: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
        Ok(self.run(repo, &["merge-base", "--is-ancestor", ancestor, descendant], true).await?.ok)
    }

    pub async fn worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>> {
        let out = self.ok(repo, &["worktree", "list", "--porcelain"], true).await?;
        let mut list = vec![];
        let mut cur: Option<WorktreeInfo> = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                list.extend(cur.take());
                cur = Some(WorktreeInfo { path: PathBuf::from(p), head: None, branch: None });
            } else if let Some(c) = cur.as_mut() {
                if let Some(h) = line.strip_prefix("HEAD ") {
                    c.head = Some(h.to_string());
                } else if let Some(b) = line.strip_prefix("branch ") {
                    c.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                }
            }
        }
        list.extend(cur);
        Ok(list)
    }

    /// Files changed on HEAD since it diverged from `base` (`base...HEAD`), renames split
    /// into delete + add so a moved-away file counts against the write scope too.
    pub async fn changed_files(&self, wt: &Path, base: &str) -> Result<Vec<String>> {
        let range = format!("{base}...HEAD");
        let out = self.ok(wt, &["diff", "--name-only", "--no-renames", "-z", &range], true).await?;
        Ok(split_z(&out))
    }

    /// Unified diff of `head` against its merge-base with `base` (`base...head`).
    pub async fn diff(&self, repo: &Path, base: &str, head: &str) -> Result<String> {
        let range = format!("{base}...{head}");
        self.ok(repo, &["diff", "--no-color", "--no-ext-diff", "--find-renames", "-U3", &range], true).await
    }

    /// Is `rel` ignored by the repository's ignore rules (so `add -A` never picks it up)?
    pub async fn is_ignored(&self, repo: &Path, rel: &str) -> Result<bool> {
        Ok(self.run(repo, &["check-ignore", "-q", "--no-index", rel], true).await?.ok)
    }

    pub async fn ls_files(&self, repo: &Path) -> Result<Vec<String>> {
        Ok(split_z(&self.ok(repo, &["ls-files", "-z"], true).await?))
    }

    /// Would merging `theirs` into `ours` conflict? Uses `git merge-tree --write-tree`
    /// (git ≥ 2.38): no index or working tree is touched, so it is safe to run while
    /// agents work. Exit 0 = clean, 1 = conflicts, anything else = error.
    pub async fn merge_tree_check(&self, repo: &Path, ours: &str, theirs: &str) -> Result<MergeCheck> {
        let out = self.run(repo, &["merge-tree", "--write-tree", "--name-only", "--messages", ours, theirs], true).await?;
        if out.ok {
            return Ok(MergeCheck { clean: true, conflicts: vec![], messages: String::new() });
        }
        let mut lines = out.stdout.lines();
        let tree = lines.next().unwrap_or_default();
        if tree.len() < 40 || !tree.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(out.err("merge-tree"));
        }
        let conflicts: Vec<String> = lines.by_ref().take_while(|l| !l.is_empty()).map(str::to_string).collect();
        let messages = lines.collect::<Vec<_>>().join("\n");
        Ok(MergeCheck { clean: false, conflicts, messages })
    }

    pub async fn in_merge(&self, wt: &Path) -> bool {
        matches!(self.rev_parse(wt, "MERGE_HEAD").await, Ok(Some(_)))
    }

    /// Paths with unresolved conflicts in a merge in progress.
    pub async fn unmerged_files(&self, wt: &Path) -> Result<Vec<String>> {
        Ok(split_z(&self.ok(wt, &["diff", "--name-only", "--diff-filter=U", "-z"], true).await?))
    }

    // ───────────── writes ─────────────

    /// A worktree on no branch (detached at `rev`): a team member's home between tasks.
    pub async fn worktree_add_detached(&self, repo: &Path, path: &Path, rev: &str) -> Result<()> {
        self.ok(repo, &["worktree", "prune"], false).await?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        self.ok(repo, &["worktree", "add", "--detach", &path.to_string_lossy(), rev], false).await.map(drop)
    }

    /// Check out `branch` in a worktree, creating it at `start` when it does not exist.
    pub async fn switch(&self, wt: &Path, branch: &str, start: &str) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        if self.branch_exists(wt, branch).await? {
            self.ok(wt, &[c1, hooks, c2, fsmonitor, "switch", branch], false).await.map(drop)
        } else {
            self.ok(wt, &[c1, hooks, c2, fsmonitor, "switch", "-c", branch, start], false).await.map(drop)
        }
    }

    /// Leave the worktree's branch (HEAD detached at the same commit), so another worktree
    /// can check that branch out.
    pub async fn detach(&self, wt: &Path) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(wt, &[c1, hooks, c2, fsmonitor, "switch", "--detach"], false).await.map(drop)
    }

    pub async fn create_branch(&self, repo: &Path, branch: &str, start: &str) -> Result<()> {
        self.ok(repo, &["branch", branch, start], false).await.map(drop)
    }

    /// Check `branch` out at `path`, creating the branch from `start` if needed. Idempotent:
    /// an existing worktree of this repo on the same branch is reused.
    pub async fn worktree_add(&self, repo: &Path, path: &Path, branch: &str, start: &str) -> Result<()> {
        if path.exists() {
            let want = canonical(path);
            let wts = self.worktrees(repo).await?;
            return match wts.iter().find(|w| canonical(&w.path) == want) {
                Some(w) if w.branch.as_deref() == Some(branch) => Ok(()),
                Some(w) => bail!("{} is a worktree on {:?}, not {branch}", path.display(), w.branch),
                None => bail!("{} exists and is not a worktree of this repository", path.display()),
            };
        }
        // A worktree directory the user deleted is still registered until pruned.
        self.ok(repo, &["worktree", "prune"], false).await?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let p = path.to_string_lossy();
        if self.branch_exists(repo, branch).await? {
            self.ok(repo, &["worktree", "add", &p, branch], false).await?;
        } else {
            self.ok(repo, &["worktree", "add", "-b", branch, &p, start], false).await?;
        }
        Ok(())
    }

    /// Remove a worktree. Unless `force`, refuses when it has uncommitted (or untracked)
    /// changes, or when `merged_into` is given and its HEAD is not contained in that ref.
    /// The branch itself is always kept.
    pub async fn worktree_remove(&self, repo: &Path, path: &Path, merged_into: Option<&str>, force: bool) -> Result<()> {
        if !path.exists() {
            return self.ok(repo, &["worktree", "prune"], false).await.map(drop);
        }
        if !force {
            self.check_worktree(repo, path).await?; // `status` below runs in the agent's worktree
            if self.dirty(path, true).await? {
                bail!("{} has uncommitted changes; keeping it", path.display());
            }
            if let Some(target) = merged_into {
                let head = self.head(path).await?.ok_or_else(|| anyhow!("{} has no HEAD", path.display()))?;
                if !self.is_ancestor(repo, &head, target).await? {
                    bail!("{} has commits not merged into {target}; keeping it", path.display());
                }
            }
        }
        let p = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&p);
        self.ok(repo, &args, false).await?;
        self.ok(repo, &["worktree", "prune"], false).await.map(drop)
    }

    /// Is `wt` a linked worktree of `repo`, with its git dir under `repo`'s
    /// `.git/worktrees/`? (See the module doc: never commit through a `.git` an agent wrote.)
    pub async fn check_worktree(&self, repo: &Path, wt: &Path) -> Result<()> {
        let common = self.git_path(repo, "--git-common-dir").await?;
        let dir = self.git_path(wt, "--git-dir").await?;
        // …and the admin dir is this worktree's own (its `gitdir` file points back here), not
        // another worktree's: a `.git` aimed at the integration worktree would commit there.
        // (relative with `worktree.useRelativePaths`)
        let back = std::fs::read_to_string(dir.join("gitdir")).map(|s| canonical(&dir.join(s.trim()))).ok();
        if !dir.starts_with(common.join("worktrees")) || back != Some(canonical(&wt.join(".git"))) {
            bail!(
                "{} is not a worktree of {} (its git dir is {}); Codly will not run git there",
                wt.display(),
                repo.display(),
                dir.display()
            );
        }
        Ok(())
    }

    /// `git rev-parse <flag>` as a canonical absolute path. rev-parse reads config but runs
    /// none of its programs.
    async fn git_path(&self, dir: &Path, flag: &str) -> Result<PathBuf> {
        let out = self.ok(dir, &["-c", "core.fsmonitor=false", "rev-parse", flag], true).await?;
        Ok(canonical(&dir.join(out.trim())))
    }

    /// `-c` flags for every internal git write: no hooks, no fsmonitor program.
    const SAFE: [&'static str; 4] = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"];

    /// `-c` flags for an internal checkpoint commit (see the module doc), plus Codly's
    /// identity when the repository has none.
    async fn internal_flags(&self, wt: &Path) -> Result<Vec<String>> {
        let mut flags: Vec<String> = Self::SAFE.map(String::from).to_vec();
        flags.extend(["-c".into(), "commit.gpgsign=false".into()]);
        flags.extend(self.identity_flags(wt).await?);
        Ok(flags)
    }

    async fn identity_flags(&self, wt: &Path) -> Result<Vec<String>> {
        let mut flags = vec![];
        for (key, fallback) in [("user.name", CODLY_NAME), ("user.email", CODLY_EMAIL)] {
            let out = self.run(wt, &["config", "--get", key], true).await?;
            if !out.ok || out.stdout.trim().is_empty() {
                flags.push("-c".into());
                flags.push(format!("{key}={fallback}"));
            }
        }
        Ok(flags)
    }

    /// Snapshot everything in a worktree (`add -A` + commit). Returns None when nothing
    /// changed. Also concludes a merge in progress (MERGE_HEAD), which is how a worker's
    /// conflict resolution becomes a merge commit without the worker touching `.git`.
    pub async fn commit_all(&self, wt: &Path, message: &str) -> Result<Option<Commit>> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(wt, &[c1, hooks, c2, fsmonitor, "add", "-A"], false).await?;
        let files =
            split_z(&self.ok(wt, &[c1, hooks, c2, fsmonitor, "diff", "--cached", "--name-only", "--no-renames", "-z"], false).await?);
        let merging = self.in_merge(wt).await;
        if files.is_empty() && !merging {
            return Ok(None);
        }
        let mut args = self.internal_flags(wt).await?;
        args.extend(["commit", "--no-verify", "--quiet", "-m", message].map(String::from));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.ok(wt, &args, false).await?;
        let sha = self.head(wt).await?.ok_or_else(|| anyhow!("commit produced no HEAD"))?;
        Ok(Some(Commit { sha, files }))
    }

    /// Merge `branch` into the worktree's HEAD with a merge commit. On conflict the merge is
    /// aborted and the conflicting paths returned. `internal` = a Codly checkpoint merge
    /// (hooks and signing off, see the module doc); false for the user's own branch.
    pub async fn merge(&self, wt: &Path, branch: &str, message: &str, internal: bool) -> Result<MergeOutcome> {
        let tip = self.rev_parse(wt, branch).await?.ok_or_else(|| anyhow!("unknown branch {branch}"))?;
        if self.is_ancestor(wt, &tip, "HEAD").await? {
            let sha = self.head(wt).await?.unwrap_or(tip);
            return Ok(MergeOutcome::AlreadyMerged { sha });
        }
        let mut args = if internal { self.internal_flags(wt).await? } else { self.identity_flags(wt).await? };
        args.extend(["merge", "--no-ff", "--no-edit", "-m", message].map(String::from));
        if internal {
            args.push("--no-verify".into());
        }
        args.push(branch.into());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(wt, &args, false).await?;
        if out.ok {
            let sha = self.head(wt).await?.ok_or_else(|| anyhow!("merge produced no HEAD"))?;
            return Ok(MergeOutcome::Merged { sha });
        }
        if self.in_merge(wt).await {
            let files = self.unmerged_files(wt).await.unwrap_or_default();
            let messages = out.stdout.trim().to_string();
            self.ok(wt, &["merge", "--abort"], false).await?;
            if files.is_empty() {
                // Merged cleanly, but the commit was refused: a pre-merge-commit or commit-msg
                // hook (only the user's own merges run hooks).
                let output = [out.stderr.trim(), out.stdout.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
                bail!("a git hook stopped the merge (the merge was aborted, nothing changed): {}", tail_chars(&output, 2000));
            }
            return Ok(MergeOutcome::Conflict { files, messages });
        }
        Err(out.err("merge"))
    }

    /// Start merging `branch` into the worktree without committing, leaving conflict markers
    /// for an agent to resolve. Returns the conflicted paths (empty = merged cleanly, still
    /// uncommitted until `commit_all`). Used when a task branch falls behind integration.
    pub async fn merge_for_resolution(&self, wt: &Path, branch: &str) -> Result<Vec<String>> {
        let mut args = self.internal_flags(wt).await?;
        args.extend(["merge", "--no-ff", "--no-commit", "--no-verify", branch].map(String::from));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(wt, &args, false).await?;
        if self.in_merge(wt).await {
            return self.unmerged_files(wt).await;
        }
        if out.ok {
            return Ok(vec![]);
        }
        Err(out.err("merge"))
    }

    /// Abort a merge left in progress (crash recovery). Returns whether one was aborted.
    pub async fn abort_merge(&self, wt: &Path) -> Result<bool> {
        if !self.in_merge(wt).await {
            return Ok(false);
        }
        self.ok(wt, &["merge", "--abort"], false).await?;
        Ok(true)
    }
}

/// The last `max` characters of `s`.
fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max { s.to_string() } else { format!("…{}", s.chars().skip(n - max).collect::<String>()) }
}

fn split_z(s: &str) -> Vec<String> {
    s.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// Canonical path for comparisons (macOS: /var → /private/var); the input if it fails.
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Reads for the workspace IDE's Source tab (uncommitted work in a worktree).
impl Git {
    /// `status --porcelain=v1 -z -uall`, raw (parsed by `ide::parse_status`).
    pub async fn status_z(&self, wt: &Path) -> Result<String> {
        self.ok(wt, &["-c", "core.fsmonitor=false", "status", "--porcelain=v1", "-z", "-uall", "--no-renames"], true).await
    }

    /// Unified diff of one file's working-tree state against HEAD; an untracked file (or a
    /// repository without commits) diffs against nothing. `rel` must already be confined.
    pub async fn diff_file(&self, wt: &Path, rel: &str, untracked: bool) -> Result<String> {
        if !untracked {
            let out = self.run(wt, &["-c", "core.fsmonitor=false", "diff", "HEAD", "--no-color", "--no-ext-diff", "--no-textconv", "-U3", "--", rel], true).await?;
            if out.ok {
                return Ok(out.stdout);
            }
        }
        // `--no-index` exits 1 when the files differ, which is the normal case here.
        let out = self.run(wt, &["-c", "core.fsmonitor=false", "diff", "--no-index", "--no-color", "--no-ext-diff", "--no-textconv", "-U3", "--", "/dev/null", rel], true).await?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "T"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        (dir, repo)
    }

    fn executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// An agent rewrites its worktree's `.git` to point at a repository it wrote (whose config
    /// runs programs), or at another worktree: Codly refuses to run git there.
    #[tokio::test]
    async fn forged_worktree_git_dirs_are_refused() {
        let (d, repo) = repo();
        let git = Git::from_process_env();
        let base = git.head(&repo).await.unwrap().unwrap();
        let (wt, other) = (d.path().join("wt"), d.path().join("other"));
        git.worktree_add(&repo, &wt, "t1", &base).await.unwrap();
        git.worktree_add(&repo, &other, "t2", &base).await.unwrap();
        git.check_worktree(&repo, &wt).await.unwrap();
        assert!(git.check_worktree(&repo, &repo).await.is_err(), "the main checkout is not a task worktree");

        let real = std::fs::read_to_string(wt.join(".git")).unwrap();
        let fake = wt.join("fake.git");
        sh(d.path(), &["init", "-q", "--bare", &fake.to_string_lossy()]);
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", fake.display())).unwrap();
        let err = git.check_worktree(&repo, &wt).await.unwrap_err().to_string();
        assert!(err.contains("not a worktree"), "{err}");

        let other_admin = std::fs::read_to_string(other.join(".git")).unwrap();
        std::fs::write(wt.join(".git"), other_admin).unwrap();
        assert!(git.check_worktree(&repo, &wt).await.is_err(), "another worktree's admin dir");
        std::fs::write(wt.join(".git"), real).unwrap();
        git.check_worktree(&repo, &wt).await.unwrap();
    }

    /// The IDE's Source tab: status lists uncommitted work, diffs cover tracked and untracked files.
    #[tokio::test]
    async fn status_and_file_diffs_for_the_ide() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        assert_eq!(git.status_z(&repo).await.unwrap(), "");
        std::fs::write(repo.join("a.txt"), "a\nb\n").unwrap();
        std::fs::write(repo.join("new.txt"), "hello\n").unwrap();
        assert_eq!(git.status_z(&repo).await.unwrap(), " M a.txt\0?? new.txt\0");
        let tracked = git.diff_file(&repo, "a.txt", false).await.unwrap();
        assert!(tracked.contains("+b") && tracked.contains("a/a.txt"), "{tracked}");
        let fresh = git.diff_file(&repo, "new.txt", true).await.unwrap();
        assert!(fresh.contains("+hello") && fresh.contains("new file mode"), "{fresh}");
    }

    /// Codly's own commits never run a configured fsmonitor program.
    #[tokio::test]
    async fn internal_commits_do_not_run_fsmonitor() {
        let (d, repo) = repo();
        let marker = d.path().join("ran");
        let hook = d.path().join("fsmonitor.sh");
        executable(&hook, &format!("#!/bin/sh\ntouch '{}'\n", marker.display()));
        sh(&repo, &["config", "core.fsmonitor", &hook.to_string_lossy()]);
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        let git = Git::from_process_env();
        assert!(git.commit_all(&repo, "snapshot").await.unwrap().is_some());
        assert!(!marker.exists(), "fsmonitor ran during a Codly commit");
    }

    /// The user's own merge (merge_plan) runs their hooks; one that refuses the commit is
    /// reported as such, not as a conflict with no files.
    #[tokio::test]
    async fn a_failing_hook_is_not_a_conflict() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        sh(&repo, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "feature"]);
        sh(&repo, &["checkout", "-q", "main"]);
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "main moves"]);
        executable(&repo.join(".git/hooks/pre-merge-commit"), "#!/bin/sh\necho 'lint failed: b.txt' >&2\nexit 1\n");
        let err = git.merge(&repo, "feature", "merge feature", false).await.unwrap_err().to_string();
        assert!(err.contains("hook") && err.contains("lint failed: b.txt"), "{err}");
        assert!(!git.in_merge(&repo).await, "the merge was aborted");
        assert!(matches!(git.merge(&repo, "feature", "merge feature", true).await.unwrap(), MergeOutcome::Merged { .. }));
    }
}
