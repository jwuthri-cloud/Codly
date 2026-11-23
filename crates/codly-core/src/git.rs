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
//! What Codly's commit of an agent's work leaves out (`commit_work`): a new folder with more
//! than `MAX_NEW_FOLDER_FILES` files, or a new file over `MAX_NEW_FILE_BYTES`, is build output,
//! a dependency folder or a download that the repository forgot to ignore (`node_modules`,
//! `target`, `coverage`), not work. A plain `add -A` would put it in the commit, in the diff
//! and in the scope check. The commit takes the rest and reports what it left
//! (`WorkCommit::left_out`); the files stay in the worktree. "New" is untracked: a path that
//! somebody staged (`git add`) is work, whatever its size, and is committed. A checkpoint
//! (`snapshot`) leaves out the same paths.
//!
//! The user's checkout is theirs too: Codly never merges into it while they are in the middle
//! of a merge, rebase or cherry-pick of their own, only aborts a merge it started itself, and
//! takes a merge back (`undo_merge`) only while that loses nothing.
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

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

/// Identity used for Codly's commits when the repository has none configured.
pub const CODLY_NAME: &str = "Codly";
pub const CODLY_EMAIL: &str = "codly@localhost";

/// A hung git (lock wait, fsmonitor, huge repo) must not wedge the orchestrator forever.
/// The limit of a status-sized call: a read, a ref, a config value.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// The limit of a call whose time grows with the size of the repository, with the user's hooks
/// or with the network (`long_running`): a large or LFS repository needs minutes for a checkout,
/// `add -A` or a push.
const GIT_LONG_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// A git that ran out of time is asked to stop, so that it removes its lock files, and killed
/// if it has not stopped after this long.
const GIT_STOP_GRACE: Duration = Duration::from_secs(2);
/// In the error of a call that ran out of time (`transient` reads it).
const TIMED_OUT: &str = "timed out after";
/// What git says when another git process holds a lock file of the repository
/// ("Unable to create '…/index.lock': File exists.").
const LOCKED: &str = ".lock': File exists";
/// In the error of a merge that ran out of time and could not be aborted.
const MERGE_LEFT: &str = "the merge is still in progress";
/// Stable links to the newest forwarded SSH agent, under the home directory (see `Git::push`).
const AGENT_LINKS: [&str; 2] = [".ssh/agent.sock", ".ssh/ssh_auth_sock"];

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
    /// The time limit of a status-sized call, and of a `long_running` one.
    quick: Duration,
    long: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub files: Vec<String>,
}

/// `commit_work` does not commit a new folder with more files than this (see the module doc).
pub const MAX_NEW_FOLDER_FILES: usize = 2_000;
/// `commit_work` does not commit a new file bigger than this (see the module doc).
pub const MAX_NEW_FILE_BYTES: u64 = 25 << 20;

/// A new path that `commit_work` left in the worktree, untracked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeftOut {
    /// A new folder (`path` ends in `/`) with more than `MAX_NEW_FOLDER_FILES` files.
    Folder { path: String, files: usize },
    /// A new file of more than `MAX_NEW_FILE_BYTES`.
    File { path: String, bytes: u64 },
}

impl LeftOut {
    pub fn path(&self) -> &str {
        match self {
            LeftOut::Folder { path, .. } | LeftOut::File { path, .. } => path,
        }
    }
}

/// `node_modules/ (20000 new files)`, `data/dump.bin (41 MB)`.
impl std::fmt::Display for LeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeftOut::Folder { path, files } => write!(f, "{path} ({files} new files)"),
            LeftOut::File { path, bytes } => write!(f, "{path} ({} MB)", bytes >> 20),
        }
    }
}

/// What `Git::commit_work` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkCommit {
    /// None when nothing changed, apart from what was left out.
    pub commit: Option<Commit>,
    pub left_out: Vec<LeftOut>,
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
        Git { bin, env, quick: GIT_TIMEOUT, long: GIT_LONG_TIMEOUT }
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
        self.run_on(dir, args, read, None).await
    }

    /// `run` with `index` as the index file (`GIT_INDEX_FILE`) in place of the worktree's own.
    async fn run_on(&self, dir: &Path, args: &[&str], read: bool, index: Option<&Path>) -> Result<Out> {
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
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        let limit = if long_running(args) { self.long } else { self.quick };
        let mut child = cmd.spawn().map_err(|e| anyhow!("cannot run git ({}): {e}", self.bin.display()))?;
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let ended = tokio::time::timeout(limit, async { tokio::try_join!(child.wait(), read_all(stdout), read_all(stderr)) }).await;
        let Ok(ended) = ended else {
            // Asked to stop first: git then removes its lock files (a killed one leaves them, and
            // every later call fails on them). Only a git that still runs is asked: `id` is None
            // once git ended and was reaped, and its pid may be another process's by then (what
            // git left running, a hook's background job or a shared ssh connection, can hold the
            // pipes open long after git). Dropping `child` kills a git that has not stopped by
            // the end of the grace.
            if let Some(pid) = child.id() {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
                let _ = tokio::time::timeout(GIT_STOP_GRACE, child.wait()).await;
            }
            bail!("git {} {TIMED_OUT} {}s", subcommand(args), limit.as_secs());
        };
        let (status, stdout, stderr) = ended?;
        Ok(Out {
            ok: status.success(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    /// Run and require exit 0; returns stdout.
    async fn ok(&self, dir: &Path, args: &[&str], read: bool) -> Result<String> {
        let out = self.run(dir, args, read).await?;
        if !out.ok {
            return Err(out.err(subcommand(args)));
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

    /// The best common ancestor of two revisions, if they share history.
    pub async fn merge_base(&self, repo: &Path, a: &str, b: &str) -> Result<Option<String>> {
        let out = self.run(repo, &["merge-base", a, b], true).await?;
        Ok(out.ok.then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty()))
    }

    /// The upstream of the checked-out branch and how far HEAD is (ahead, behind) of it, as
    /// last fetched. None when the branch tracks nothing.
    pub async fn ahead_behind(&self, repo: &Path) -> Result<Option<(String, u32, u32)>> {
        let up = self.run(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], true).await?;
        if !up.ok {
            return Ok(None);
        }
        let counts = self.ok(repo, &["rev-list", "--left-right", "--count", "HEAD...@{u}"], true).await?;
        let mut n = counts.split_whitespace().map(|x| x.parse::<u32>().unwrap_or(0));
        Ok(Some((up.stdout.trim().to_string(), n.next().unwrap_or(0), n.next().unwrap_or(0))))
    }

    /// Commit the user's changes to tracked files in their own checkout (`commit -a`): their
    /// hooks and signing apply, as when they commit by hand. Untracked files are left alone.
    pub async fn commit_tracked(&self, repo: &Path, message: &str) -> Result<String> {
        self.ok(repo, &["commit", "--all", "--quiet", "-m", message], false).await?;
        self.head(repo).await?.ok_or_else(|| anyhow!("commit produced no HEAD"))
    }

    /// Push `branch` to `remote` (sets the upstream when it has none). A long-running server
    /// keeps the SSH agent socket of the login that started it, which is gone once that SSH
    /// session ends, so the agent is the first live one of: that socket, then the stable links
    /// a `~/.ssh/rc` keeps pointing at the newest forwarded agent (`AGENT_LINKS`). SSH runs in
    /// batch mode, so a missing key fails at once instead of waiting on a prompt.
    pub async fn push(&self, repo: &Path, remote: &str, branch: &str) -> Result<()> {
        let mut env = self.env.clone();
        let home = self.env.get("HOME").map(PathBuf::from);
        let live = self
            .env
            .get("SSH_AUTH_SOCK")
            .map(PathBuf::from)
            .into_iter()
            .chain(home.iter().flat_map(|h| AGENT_LINKS.iter().map(move |l| h.join(l))))
            .find(|p| p.exists()); // follows links: a dangling one does not count
        if let Some(sock) = live {
            env.insert("SSH_AUTH_SOCK".into(), sock.to_string_lossy().into_owned());
        }
        env.entry("GIT_SSH_COMMAND".into()).or_insert_with(|| "ssh -o BatchMode=yes".into());
        let pusher = Git { env, ..self.clone() };
        let refspec = format!("{branch}:refs/heads/{branch}");
        let out = pusher.run(repo, &["push", "--set-upstream", remote, &refspec], false).await?;
        if out.ok {
            return Ok(());
        }
        let err = out.stderr.trim();
        if err.contains("Permission denied (publickey)") || err.contains("authentication agent") || err.contains("Host key verification failed") {
            bail!(
                "the remote refused the SSH key, so nothing was pushed. Connect to this machine with agent forwarding (ssh -A, or ForwardAgent yes) from a computer that has your GitHub key, then push again. ({err})"
            );
        }
        if err.contains("[rejected]") || err.contains("non-fast-forward") || err.contains("fetch first") {
            bail!("{remote} has commits your branch does not have, so the push was refused. Pull (git pull --rebase) first, then push again.");
        }
        Err(out.err("push"))
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

    /// The untracked paths the repository's ignore rules hide (so `add -A` never picks them
    /// up), relative to `repo`. A folder that is ignored as a whole is one entry ending in
    /// `/`: git does not look inside it.
    pub async fn ignored(&self, repo: &Path) -> Result<Vec<String>> {
        let all = split_z(&self.ok(repo, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"], true).await?);
        // git also names a folder that only holds ignored files, just before those files. No
        // rule ignores that folder itself, so only its files are kept.
        let holds_next = |i: usize, path: &str| path.ends_with('/') && all.get(i + 1).is_some_and(|next| next.starts_with(path));
        Ok(all.iter().enumerate().filter(|(i, path)| !holds_next(*i, path)).map(|(_, path)| path.clone()).collect())
    }

    /// The files inside `dirs`, folders of `repo` that `ignored` lists as one entry each.
    pub async fn ignored_files(&self, repo: &Path, dirs: &[&str]) -> Result<Vec<String>> {
        let specs: Vec<String> = dirs.iter().map(|dir| format!(":(literal){dir}")).collect();
        let mut args = vec!["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--"];
        args.extend(specs.iter().map(String::as_str));
        Ok(split_z(&self.ok(repo, &args, true).await?))
    }

    /// The files git tracks in the checkout `dir` under `paths` (top-level files, or folders
    /// with everything below them), relative to `dir`. Names compare without case, as the
    /// macOS filesystem does.
    pub async fn tracked(&self, dir: &Path, paths: &[&str]) -> Result<Vec<String>> {
        let specs: Vec<String> = paths.iter().map(|path| format!(":(icase){path}")).collect();
        let mut args = vec!["ls-files", "-z", "--"];
        args.extend(specs.iter().map(String::as_str));
        Ok(split_z(&self.ok(dir, &args, true).await?))
    }

    /// Do the ignore rules of the checkout `wt` hide `rel` (so `add -A` there never picks it
    /// up)? `dir` = `rel` is a folder. The path does not have to exist there yet.
    pub async fn is_ignored(&self, wt: &Path, rel: &str, dir: bool) -> Result<bool> {
        let path = if dir { format!("{rel}/") } else { rel.to_string() };
        let out = self.run(wt, &["check-ignore", "-q", "--no-index", "--", &path], true).await?;
        // Exit 1 with no message = not ignored; anything else is git's own failure.
        if !out.ok && !out.stderr.trim().is_empty() {
            return Err(out.err("check-ignore"));
        }
        Ok(out.ok)
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

    /// What the checkout is in the middle of ("a merge", "a rebase"…), if anything.
    pub async fn in_progress(&self, wt: &Path) -> Result<Option<&'static str>> {
        const MARKS: [(&str, &str); 5] = [
            ("MERGE_HEAD", "a merge"),
            ("rebase-merge", "a rebase"),
            ("rebase-apply", "a rebase"),
            ("CHERRY_PICK_HEAD", "a cherry-pick"),
            ("REVERT_HEAD", "a revert"),
        ];
        let dir = self.git_path(wt, "--git-dir").await?;
        Ok(MARKS.iter().find(|(mark, _)| dir.join(mark).exists()).map(|(_, what)| *what))
    }

    /// Is `wt` in the middle of merging `tip`? In the user's checkout (`internal` = false) that
    /// is the only merge Codly may abort; in its own worktrees any merge is Codly's.
    async fn merging(&self, wt: &Path, tip: &str, internal: bool) -> bool {
        matches!(self.rev_parse(wt, "MERGE_HEAD").await, Ok(Some(head)) if internal || head == tip)
    }

    /// Is `rev` on a local branch other than `except`?
    async fn on_another_branch(&self, repo: &Path, rev: &str, except: &str) -> Result<bool> {
        let heads = self.read(repo, &["for-each-ref", "--contains", rev, "--format=%(refname)", "refs/heads"]).await?;
        Ok(heads.lines().filter_map(|r| r.strip_prefix("refs/heads/")).any(|b| b != except))
    }

    /// Why the merge commit `merged` of `source`, made on `branch` on top of `before`, cannot be
    /// taken back right now; None when it can. Taking it back moves `branch` to `before`, so it
    /// is only safe while nothing else depends on the merge: the checkout is still on `branch`
    /// at that very commit, no remote branch has it, the merged work stays on a branch, and no
    /// tracked file has uncommitted changes. Every way out it names keeps undo possible: a
    /// commit on top of the merge (also one that finishes an operation in progress) ends it.
    pub async fn undo_merge_blocker(&self, repo: &Path, branch: &str, merged: &str, before: &str, source: &str) -> Result<Option<String>> {
        if let Some(op) = self.in_progress(repo).await? {
            return Ok(Some(format!(
                "your checkout has {op} in progress; abort it first (finishing it would put a commit on top of the merge and make undo impossible)"
            )));
        }
        match self.current_branch(repo).await? {
            Some(b) if b == branch => {}
            Some(b) => return Ok(Some(format!("your checkout is on {b} now, not {branch}"))),
            None => return Ok(Some(format!("your checkout is not on {branch} now"))),
        }
        let head = self.head(repo).await?.unwrap_or_default();
        if head != merged {
            return Ok(Some(format!("{branch} has changed since this merge (it is at {} now)", short(&head))));
        }
        let on_top = self.rev_parse(repo, &format!("{merged}^1")).await?.as_deref() == Some(before);
        let Some(work) = self.rev_parse(repo, &format!("{merged}^2")).await?.filter(|_| on_top) else {
            return Ok(Some(format!("{} is not a merge made on top of {}", short(merged), short(before))));
        };
        let remotes = self.read(repo, &["for-each-ref", "--contains", merged, "--format=%(refname)", "refs/remotes"]).await?;
        let pushed = remotes.lines().filter_map(|r| r.strip_prefix("refs/remotes/")).find(|r| !r.ends_with("/HEAD"));
        if let Some(remote) = pushed {
            return Ok(Some(format!("the merge is already on {remote}; undo it there with git revert -m 1 {}", short(merged))));
        }
        // A deleted `source` is put back by the undo itself. One that points somewhere else now
        // cannot be: without another branch at the merged work, only `branch` still holds it.
        if !self.on_another_branch(repo, &work, branch).await? && self.branch_exists(repo, source).await? {
            return Ok(Some(format!(
                "{source} was moved and no other branch has the merged work, so undoing would leave that work on no branch; keep it first with git branch <new name> {}",
                short(&work)
            )));
        }
        let dirty = self.changed_paths(repo, &["HEAD"]).await?;
        if !dirty.is_empty() {
            return Ok(Some(format!(
                "your checkout has uncommitted changes in {}; put them aside with git stash first (a commit on top of the merge would make undo impossible)",
                name_list(&dirty)
            )));
        }
        Ok(None)
    }

    // ───────────── writes ─────────────

    /// A worktree on no branch (detached at `rev`): a team member's home between tasks.
    pub async fn worktree_add_detached(&self, repo: &Path, path: &Path, rev: &str) -> Result<()> {
        self.ok(repo, &["worktree", "prune"], false).await?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let fresh = !path.exists();
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let added = self.ok(repo, &[c1, hooks, c2, fsmonitor, "worktree", "add", "--detach", &path.to_string_lossy(), rev], false).await;
        self.added(repo, path, fresh, added).await
    }

    /// The end of a `worktree add` at `path`. A failed one (a checkout that ran out of time, a
    /// full disk) can leave a registered worktree with only a part of its files: the next try
    /// would take it for a whole one (`worktree_add` reuses a worktree on the same branch), and
    /// the missing files would be committed as deletions. So what a failed add made is removed.
    /// `fresh` = nothing was at `path` before the add: only then is what is there now its own.
    async fn added(&self, repo: &Path, path: &Path, fresh: bool, added: Result<String>) -> Result<()> {
        let Err(e) = added else { return Ok(()) };
        if !fresh {
            return Err(e);
        }
        // git refuses to remove a worktree it never finished: then the folder goes, and the
        // prune forgets it.
        let _ = self.ok(repo, &["worktree", "remove", "--force", &path.to_string_lossy()], false).await;
        if path.exists()
            && let Err(left) = tokio::fs::remove_dir_all(path).await
        {
            bail!("{e:#} (and the unfinished worktree at {} could not be removed: {left})", path.display());
        }
        if let Err(prune) = self.ok(repo, &["worktree", "prune"], false).await {
            tracing::warn!("after a failed worktree add: {prune:#}");
        }
        Err(e)
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
        // An internal checkout, like `switch`: the user's post-checkout hook does not run.
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let added = if self.branch_exists(repo, branch).await? {
            self.ok(repo, &[c1, hooks, c2, fsmonitor, "worktree", "add", &p, branch], false).await
        } else {
            self.ok(repo, &[c1, hooks, c2, fsmonitor, "worktree", "add", "-b", branch, &p, start], false).await
        };
        self.added(repo, path, true, added).await
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

    /// Commit everything in a worktree (`add -A` + commit), without the new folders and files
    /// that are too big to be work (see the module doc): those stay untracked and are
    /// reported. No commit when nothing else changed. Also concludes a merge in progress
    /// (MERGE_HEAD), which is how a worker's conflict resolution becomes a merge commit
    /// without the worker touching `.git`.
    pub async fn commit_work(&self, wt: &Path, message: &str) -> Result<WorkCommit> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let left_out = self.too_big(wt).await?;
        if !left_out.is_empty() {
            tracing::info!("commit in {}: left out: {}", wt.display(), left_out.iter().map(LeftOut::to_string).collect::<Vec<_>>().join(", "));
        }
        let add = add_all_but(&left_out);
        let mut args = vec![c1, hooks, c2, fsmonitor];
        args.extend(add.iter().map(String::as_str));
        self.ok(wt, &args, false).await?;
        let files =
            split_z(&self.ok(wt, &[c1, hooks, c2, fsmonitor, "diff", "--cached", "--name-only", "--no-renames", "-z"], false).await?);
        let merging = self.in_merge(wt).await;
        if files.is_empty() && !merging {
            return Ok(WorkCommit { commit: None, left_out });
        }
        let mut args = self.internal_flags(wt).await?;
        args.extend(["commit", "--no-verify", "--quiet", "-m", message].map(String::from));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.ok(wt, &args, false).await?;
        let sha = self.head(wt).await?.ok_or_else(|| anyhow!("commit produced no HEAD"))?;
        Ok(WorkCommit { commit: Some(Commit { sha, files }), left_out })
    }

    /// `commit_work`, for a caller that only needs the commit. None when nothing changed.
    pub async fn commit_all(&self, wt: &Path, message: &str) -> Result<Option<Commit>> {
        Ok(self.commit_work(wt, message).await?.commit)
    }

    /// The new (untracked, not ignored) folders and files of a worktree that are too big to
    /// be work: what `commit_work` and `snapshot` leave out.
    async fn too_big(&self, wt: &Path) -> Result<Vec<LeftOut>> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let list = [c1, hooks, c2, fsmonitor, "ls-files", "-z", "--others", "--exclude-standard"];
        let files = split_z(&self.ok(wt, &list, true).await?);
        let mut folders = vec![];
        // Only with this many new files can a folder hold too many. Then git says which
        // folders are new as a whole (one entry each, ending in `/`).
        if files.len() > MAX_NEW_FOLDER_FILES {
            let mut roots = list.to_vec();
            roots.extend(["--directory", "--no-empty-directory"]);
            folders = oversized_folders(&split_z(&self.ok(wt, &roots, true).await?), &files, MAX_NEW_FOLDER_FILES);
        }
        let in_folder = |file: &str| folders.iter().any(|(folder, _)| file.starts_with(folder.as_str()));
        let big = files.iter().filter(|f| !in_folder(f.as_str())).filter_map(|f| {
            let meta = std::fs::symlink_metadata(wt.join(f)).ok()?;
            (meta.is_file() && meta.len() > MAX_NEW_FILE_BYTES).then(|| LeftOut::File { path: f.clone(), bytes: meta.len() })
        });
        let big: Vec<LeftOut> = big.collect();
        Ok(folders.into_iter().map(|(path, files)| LeftOut::Folder { path, files }).chain(big).collect())
    }

    /// Merge `branch` into the worktree's HEAD with a merge commit. On conflict the merge is
    /// aborted and the conflicting paths returned. `internal` = a Codly checkpoint merge
    /// (hooks and signing off, see the module doc); false for the user's own branch, which is
    /// refused while the user is in the middle of a merge, rebase or cherry-pick there.
    pub async fn merge(&self, wt: &Path, branch: &str, message: &str, internal: bool) -> Result<MergeOutcome> {
        let tip = self.rev_parse(wt, branch).await?.ok_or_else(|| anyhow!("unknown branch {branch}"))?;
        if self.is_ancestor(wt, &tip, "HEAD").await? {
            let sha = self.head(wt).await?.unwrap_or(tip);
            return Ok(MergeOutcome::AlreadyMerged { sha });
        }
        // `git merge` would fail there, and aborting "the" merge would throw away the conflict
        // resolutions the user has not committed yet.
        if !internal && let Some(op) = self.in_progress(wt).await? {
            bail!("your checkout has {op} in progress; finish or abort it, then merge again (nothing was changed)");
        }
        let mut args = if internal { self.internal_flags(wt).await? } else { self.identity_flags(wt).await? };
        args.extend(["merge", "--no-ff", "--no-edit", "-m", message].map(String::from));
        if internal {
            args.push("--no-verify".into());
        }
        args.push(branch.into());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = match self.run(wt, &args, false).await {
            Ok(out) => out,
            // Stopped mid-merge (a slow hook hit the time limit): don't leave the checkout half-merged.
            Err(e) => {
                // Stopped after the merge commit was made (a slow post-merge hook): the branch
                // has moved, so the merge is reported as what it is.
                if matches!(self.is_ancestor(wt, &tip, "HEAD").await, Ok(true))
                    && let Some(sha) = self.head(wt).await?
                {
                    tracing::warn!("{e:#}, after the merge of {branch} was made: the merge stands");
                    // git runs that hook before it forgets the merge, so the checkout still
                    // says "merging": only that note is left to remove (`--quit` changes no file).
                    if self.merging(wt, &tip, internal).await {
                        self.ok(wt, &["merge", "--quit"], false).await?;
                    }
                    return Ok(MergeOutcome::Merged { sha });
                }
                let left = self.merging(wt, &tip, internal).await && self.ok(wt, &["merge", "--abort"], false).await.is_err();
                let note = if left { format!("{MERGE_LEFT}: run `git merge --abort`") } else { "the merge was aborted, nothing changed".to_string() };
                bail!("{e:#} ({note})");
            }
        };
        if out.ok {
            let sha = self.head(wt).await?.ok_or_else(|| anyhow!("merge produced no HEAD"))?;
            return Ok(MergeOutcome::Merged { sha });
        }
        if self.merging(wt, &tip, internal).await {
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
    /// uncommitted until `commit_all` or `conclude_merge`). Used when a task branch falls
    /// behind integration.
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

    /// Put the branch of one of Codly's own worktrees back to `sha`: the commits after it, a
    /// merge left open and the changes to tracked files go. Never for the user's checkout.
    pub async fn reset_to(&self, wt: &Path, sha: &str) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(wt, &[c1, hooks, c2, fsmonitor, "reset", "--hard", "--quiet", sha], false).await.map(drop)
    }

    /// Take back the merge commit `merged` of `source` in the user's checkout: `branch` goes
    /// back to `before`, the commit it was at. Refused with the reason unless
    /// `undo_merge_blocker` finds it safe. The merged work always stays on a branch: when
    /// `source` was deleted after the merge and no other branch has the work, `source` is put
    /// back first (returns true). `reset --keep` moves the branch and the merged files only, and
    /// stops instead of overwriting anything that is not committed.
    pub async fn undo_merge(&self, repo: &Path, branch: &str, merged: &str, before: &str, source: &str) -> Result<bool> {
        if let Some(why) = self.undo_merge_blocker(repo, branch, merged, before, source).await? {
            bail!("the merge cannot be undone: {why}");
        }
        let work = format!("{merged}^2");
        let restored = !self.on_another_branch(repo, &work, branch).await?;
        if restored {
            self.create_branch(repo, source, &work).await?;
        }
        self.ok(repo, &["reset", "--keep", "--quiet", before], false).await?;
        Ok(restored)
    }
}

/// Everything a child writes to one of its pipes, until every process that holds the pipe has
/// closed it.
async fn read_all(pipe: Option<impl tokio::io::AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut out).await?;
    }
    Ok(out)
}

/// The git subcommand of `args`, after the `-c name=value` pairs before it.
fn subcommand<'a>(args: &[&'a str]) -> &'a str {
    let mut rest = args;
    while let ["-c", _, after @ ..] = rest {
        rest = after;
    }
    rest.first().copied().unwrap_or("")
}

/// Is this a call whose time grows with the size of the repository, with the user's hooks or
/// with the network (`GIT_LONG_TIMEOUT`)? A checkout, `add -A`, a commit, a merge, a push.
fn long_running(args: &[&str]) -> bool {
    match subcommand(args) {
        "add" | "commit" | "merge" | "switch" | "reset" | "read-tree" | "push" => true,
        // `worktree add` checks every file out and `worktree remove` deletes them; `prune` and
        // `list` only read the list of worktrees.
        "worktree" => args.iter().skip_while(|a| **a != "worktree").nth(1).is_some_and(|a| matches!(*a, "add" | "remove")),
        _ => false,
    }
}

/// Did a git call fail in a way that passes by itself: another git process held a lock file
/// of the repository (an editor's git panel, a watcher), or the call ran out of time and was
/// stopped? Then the same call can be made again after a moment: a call that found a lock
/// changed nothing, and one that was stopped left nothing that the next one does not redo.
/// (Not a merge that could not be aborted: that one needs you.)
pub fn transient(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}");
    text.contains(LOCKED) || (text.contains(TIMED_OUT) && !text.contains(MERGE_LEFT))
}

/// The last `max` characters of `s`.
fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max { s.to_string() } else { format!("…{}", s.chars().skip(n - max).collect::<String>()) }
}

/// A commit id as people read it.
fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// "a.rs, b.rs, c.rs and 9 more".
pub(crate) fn name_list<S: AsRef<str>>(files: &[S]) -> String {
    let shown: Vec<&str> = files.iter().take(3).map(AsRef::as_ref).collect();
    match files.len().saturating_sub(3) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

fn split_z(s: &str) -> Vec<String> {
    s.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// `git add -A` for everything in a worktree but the paths in `left_out`.
fn add_all_but(left_out: &[LeftOut]) -> Vec<String> {
    let mut add = vec!["add".to_string(), "-A".to_string()];
    if !left_out.is_empty() {
        add.extend(["--".to_string(), ".".to_string()]);
        add.extend(left_out.iter().map(|l| format!(":(exclude,literal){}", l.path())));
    }
    add
}

/// The new folders with more than `max` files, as (folder, files in it). `roots`: the
/// untracked paths, where a folder that is new as a whole is one entry ending in `/`;
/// `files`: every untracked file. The files of a sub-folder that goes do not count for the
/// folders around it, so a new package keeps its sources and loses its dependency folder and
/// its build output: `web/node_modules/` and `web/.next/` go, `web/src/` stays. A dependency
/// folder goes as a whole, also when one package in it is over the limit by itself.
fn oversized_folders(roots: &[String], files: &[String], max: usize) -> Vec<(String, usize)> {
    let mut out = vec![];
    for root in roots.iter().filter(|r| r.ends_with('/')) {
        let inside: Vec<&str> = files.iter().map(String::as_str).filter(|f| f.starts_with(root.as_str())).collect();
        oversized_in(root, &inside, max, &mut out);
    }
    out
}

/// `oversized_folders` for the new folder `dir` and the files in it: adds the folders that go
/// to `out`, and returns how many of `files` are in them.
fn oversized_in(dir: &str, files: &[&str], max: usize, out: &mut Vec<(String, usize)>) -> usize {
    if files.len() <= max {
        return 0;
    }
    // By sub-folder, in the order of their names, so the answer does not change by run.
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in files {
        if let Some((child, _)) = f[dir.len()..].split_once('/') {
            children.entry(child).or_default().push(*f);
        }
    }
    let (found, mut gone) = (out.len(), 0);
    for (child, inside) in &children {
        gone += oversized_in(&format!("{dir}{child}/"), inside, max, out);
    }
    if files.len() - gone <= max {
        return gone;
    }
    // Still too many without them: the folder goes as a whole, with the ones found in it.
    out.truncate(found);
    out.push((dir.to_string(), files.len()));
    files.len()
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

/// A local branch as Settings → Storage lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// Short name ("codly/plan-1a2b/t1").
    pub name: String,
    pub head: String,
    /// When its last commit was made (unix ms).
    pub committed_at: i64,
}

/// Reads and deletes for Settings → Storage (`housekeeping/storage.rs`): what Codly left in a
/// repository, and what would be lost with it.
impl Git {
    /// Every local branch under `codly/`.
    pub async fn codly_branches(&self, repo: &Path) -> Result<Vec<BranchInfo>> {
        let out = self.read(repo, &["for-each-ref", "--format=%(objectname) %(committerdate:unix) %(refname)", "refs/heads/codly/"]).await?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, ' ');
                let (head, secs, name) = (parts.next()?, parts.next()?, parts.next()?.strip_prefix("refs/heads/")?);
                Some(BranchInfo { name: name.to_string(), head: head.to_string(), committed_at: secs.parse::<i64>().ok()? * 1000 })
            })
            .collect())
    }

    /// The branch checked out in `dir`, by its name under `refs/heads/` (None: on no branch).
    /// `current_branch` gives the name the way git shortens it: "heads/x" when a tag is also
    /// named x.
    pub async fn checked_out_branch(&self, dir: &Path) -> Result<Option<String>> {
        let out = self.run(dir, &["symbolic-ref", "--quiet", "HEAD"], true).await?;
        Ok(if out.ok { out.stdout.trim().strip_prefix("refs/heads/").map(String::from) } else { None })
    }

    /// How many commits `rev` has that none of the local branches `others` has. 0 = `rev` is
    /// contained in one of them: nothing is lost when only they stay. With no `others`, every
    /// commit counts.
    pub async fn commits_only_on(&self, repo: &Path, rev: &str, others: &[String]) -> Result<u32> {
        // Full ref names, and "--" to end the revisions: a branch can have the name of a tag,
        // or of a file or a folder in the checkout, and git refuses a name that is two things.
        let others: Vec<String> = others.iter().map(|branch| format!("refs/heads/{branch}")).collect();
        let mut args = vec!["rev-list", "--count", rev, "--not"];
        args.extend(others.iter().map(String::as_str));
        args.push("--");
        Ok(self.read(repo, &args).await?.trim().parse()?)
    }

    /// How many commits `rev` has that no local branch has: what goes with a worktree that is
    /// on no branch.
    pub async fn commits_on_no_branch(&self, repo: &Path, rev: &str) -> Result<u32> {
        Ok(self.read(repo, &["rev-list", "--count", rev, "--not", "--branches", "--"]).await?.trim().parse()?)
    }

    /// When the commit `rev` was made (unix ms).
    pub async fn committed_at(&self, repo: &Path, rev: &str) -> Result<i64> {
        Ok(self.read(repo, &["log", "-1", "--format=%ct", rev, "--"]).await?.trim().parse::<i64>()? * 1000)
    }

    /// The uncommitted files of a worktree: changed tracked files and untracked files that are
    /// not ignored (what `dirty(.., true)` counts). Call `check_worktree` first.
    pub async fn dirty_paths(&self, wt: &Path) -> Result<Vec<String>> {
        let out = self.read(wt, &["status", "--porcelain=v1", "-z", "--untracked-files=normal", "--no-renames"]).await?;
        // One entry is "XY <path>".
        Ok(out.split('\0').filter_map(|entry| entry.get(3..)).filter(|path| !path.is_empty()).map(String::from).collect())
    }

    /// Delete the local branch `branch` if it is still at `head`. Refused when it moved since,
    /// and when a worktree has it checked out.
    pub async fn delete_branch(&self, repo: &Path, branch: &str, head: &str) -> Result<()> {
        if self.worktrees(repo).await?.iter().any(|w| w.branch.as_deref() == Some(branch)) {
            bail!("{branch} is checked out in a worktree; keeping it");
        }
        let r = format!("refs/heads/{branch}");
        self.ok(repo, &["update-ref", "-d", &r, head], false).await.map(drop)
    }
}

/// A private index file; removed when dropped.
struct ScratchIndex(PathBuf);

/// The files of a worktree as git read them at one moment (`Git::snapshot`): `tree` names
/// them, and the private index they are staged in is what `Git::restore_tree` starts from.
pub struct Snapshot {
    index: ScratchIndex,
    tree: String,
}

impl Snapshot {
    /// The tree of everything that was in the worktree then.
    pub fn tree(&self) -> &str {
        &self.tree
    }
}

impl Drop for ScratchIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Checkpoints (`orchestrator/checkpoints.rs`): the files of a worktree kept as a commit that is
/// on no branch, and the way back to it. Nothing here moves HEAD or a branch, and nothing writes
/// the worktree's own index: the files are staged in a private index file. Like every git call
/// in an agent's worktree, these come after `check_worktree`.
impl Git {
    /// Everything in the worktree, staged in a private index: tracked and untracked files, no
    /// ignored one, and nothing that `commit_work` leaves out (a huge new folder or file is
    /// not stored, and not read again at every checkpoint). The index starts as a copy of the
    /// worktree's own, whose file stats keep git from reading the files that did not change.
    async fn stage_apart(&self, wt: &Path) -> Result<ScratchIndex> {
        let left_out = self.too_big(wt).await?;
        let dir = self.git_path(wt, "--git-dir").await?;
        let index = ScratchIndex(dir.join(format!("codly-index-{}", uuid::Uuid::new_v4())));
        match std::fs::copy(dir.join("index"), &index.0) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // nothing was ever staged there
            Err(e) => bail!("cannot copy the index of {}: {e}", wt.display()),
        }
        let add = add_all_but(&left_out);
        self.on_index(wt, &index, &add.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        Ok(index)
    }

    /// An internal git write (no hooks, no fsmonitor program) that works on a private index.
    async fn on_index(&self, wt: &Path, index: &ScratchIndex, args: &[&str]) -> Result<String> {
        let mut full: Vec<&str> = Self::SAFE.to_vec();
        full.extend_from_slice(args);
        let out = self.run_on(wt, &full, false, Some(&index.0)).await?;
        if !out.ok {
            return Err(out.err(args.first().copied().unwrap_or("")));
        }
        Ok(out.stdout)
    }

    /// Everything in the worktree now (tracked and untracked files, no ignored one, and
    /// nothing that `commit_work` leaves out), read once. This is the slow part of a
    /// checkpoint: git reads every file that changed. It writes objects only: files and
    /// folders that did not change are already stored.
    pub async fn snapshot(&self, wt: &Path) -> Result<Snapshot> {
        let index = self.stage_apart(wt).await?;
        let tree = self.on_index(wt, &index, &["write-tree"]).await?.trim().to_string();
        Ok(Snapshot { index, tree })
    }

    /// The tree of everything in the worktree now (`snapshot`, for a caller that restores
    /// nothing).
    pub async fn worktree_tree(&self, wt: &Path) -> Result<String> {
        Ok(self.snapshot(wt).await?.tree)
    }

    /// A commit of `tree` on top of `parent` that no branch points at. An internal checkpoint:
    /// no hooks, no signing (see the module doc).
    pub async fn commit_tree(&self, wt: &Path, tree: &str, parent: Option<&str>, message: &str) -> Result<String> {
        let mut args = self.internal_flags(wt).await?;
        args.extend(["commit-tree", "-m", message].map(String::from));
        if let Some(parent) = parent {
            args.extend(["-p", parent].map(String::from));
        }
        args.push(tree.into());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(self.ok(wt, &args, false).await?.trim().to_string())
    }

    /// Create the ref `name` at `sha`. Refused when the ref exists: a checkpoint is never
    /// written over.
    pub async fn create_ref(&self, repo: &Path, name: &str, sha: &str) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(repo, &[c1, hooks, c2, fsmonitor, "update-ref", name, sha, ""], false).await.map(drop)
    }

    pub async fn delete_ref(&self, repo: &Path, name: &str) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(repo, &[c1, hooks, c2, fsmonitor, "update-ref", "-d", name], false).await.map(drop)
    }

    /// Put the worktree's files back to `tree`: files that changed or were deleted since come
    /// back, files that are not in `tree` are removed, and ignored files are left alone. So
    /// is what `present` left out (a huge new folder or file; `tree` can only write a file
    /// there that it has itself): it is not kept, so it must not be removed. HEAD, the branch
    /// and the worktree's own index stay as they are, so the difference shows as uncommitted
    /// changes. `present` is the worktree as it is (`snapshot`): its tree is what to keep if
    /// the present state must not be lost, and with the files already read, this call only
    /// writes the files that differ.
    pub async fn restore_tree(&self, wt: &Path, present: Snapshot, tree: &str) -> Result<()> {
        self.on_index(wt, &present.index, &["read-tree", "--reset", "-u", tree]).await.map(drop)
    }
}

/// For the last merge, into the user's branch (`orchestrator/merge_conflict.rs`, `workspace.rs`).
impl Git {
    /// What was changed by hand between `base` and `tip`, along the first parents, oldest
    /// first. For a merge commit: the difference between what git merges by itself (a
    /// conflict as git leaves it, markers included) and what was committed (`--remerge-diff`,
    /// git ≥ 2.36), so a conflict that was resolved by taking one side whole shows too. For
    /// every other commit: its own diff. What a merge brought in without a conflict is not in
    /// it. Empty: nothing was changed by hand.
    pub async fn resolution_diff(&self, repo: &Path, base: &str, tip: &str) -> Result<String> {
        let range = format!("{base}..{tip}");
        let log = ["log", "--first-parent", "--reverse", "--format=", "--remerge-diff", "-p"];
        let diff = ["--no-color", "--no-ext-diff", "--find-renames", "-U3", range.as_str(), "--"];
        self.ok(repo, &[log, diff].concat(), true).await
    }

    /// How many commits `rev` has that `base` does not have.
    pub async fn commits_ahead(&self, repo: &Path, base: &str, rev: &str) -> Result<u32> {
        Ok(self.read(repo, &["rev-list", "--count", &format!("{base}..{rev}"), "--"]).await?.trim().parse()?)
    }

    /// The new (untracked, not ignored) paths of a worktree. A folder that is new as a whole
    /// is one entry ending in `/`: git does not look inside it.
    pub async fn untracked(&self, wt: &Path) -> Result<Vec<String>> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let list = [c1, hooks, c2, fsmonitor, "ls-files", "-z", "--others", "--exclude-standard", "--directory", "--no-empty-directory"];
        Ok(split_z(&self.ok(wt, &list, true).await?))
    }

    /// Point the ref `name` at `sha`, whatever it pointed at before.
    pub async fn set_ref(&self, repo: &Path, name: &str, sha: &str) -> Result<()> {
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        self.ok(repo, &[c1, hooks, c2, fsmonitor, "update-ref", name, sha], false).await.map(drop)
    }

    /// Commit the merge that is open in a worktree as git made it (`merge_for_resolution`
    /// without a conflict): the index is committed as it is and nothing is added, so a new
    /// (untracked) file of the worktree stays out of the merge commit and stays untracked. An
    /// error when no merge is open, or when a file of it is still in conflict.
    pub async fn conclude_merge(&self, wt: &Path, message: &str) -> Result<Commit> {
        if !self.in_merge(wt).await {
            bail!("no merge is open in {}, so there is none to commit", wt.display());
        }
        let [c1, hooks, c2, fsmonitor] = Self::SAFE;
        let files =
            split_z(&self.ok(wt, &[c1, hooks, c2, fsmonitor, "diff", "--cached", "--name-only", "--no-renames", "-z"], false).await?);
        let mut args = self.internal_flags(wt).await?;
        args.extend(["commit", "--no-verify", "--quiet", "-m", message].map(String::from));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.ok(wt, &args, false).await?;
        let sha = self.head(wt).await?.ok_or_else(|| anyhow!("commit produced no HEAD"))?;
        Ok(Commit { sha, files })
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
    /// The user's own git steps: commit tracked changes (untracked files stay out), see how far
    /// the branch is ahead of its remote, push it (setting the upstream), and see a refused push.
    #[tokio::test]
    async fn commit_ahead_behind_and_push() {
        let (d, repo) = repo();
        let remote = d.path().join("remote.git");
        sh(d.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
        sh(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
        let git = Git::from_process_env();
        assert_eq!(git.ahead_behind(&repo).await.unwrap(), None, "no upstream yet");

        git.push(&repo, "origin", "main").await.unwrap();
        assert_eq!(git.ahead_behind(&repo).await.unwrap(), Some(("origin/main".into(), 0, 0)), "push set the upstream");

        std::fs::write(repo.join("a.txt"), "a\nchanged\n").unwrap();
        std::fs::write(repo.join("scratch.txt"), "not mine to commit").unwrap();
        let sha = git.commit_tracked(&repo, "my edit").await.unwrap();
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(sha.as_str()));
        assert_eq!(git.status_z(&repo).await.unwrap(), "?? scratch.txt\0", "untracked files stay out of the commit");
        assert_eq!(git.ahead_behind(&repo).await.unwrap(), Some(("origin/main".into(), 1, 0)));
        assert_eq!(git.merge_base(&repo, "HEAD", "origin/main").await.unwrap(), git.rev_parse(&repo, "origin/main").await.unwrap());

        // Someone else pushed first: the push is refused with a message that says what to do.
        let other = d.path().join("other");
        sh(d.path(), &["clone", "-q", &remote.to_string_lossy(), &other.to_string_lossy()]);
        sh(&other, &["config", "user.name", "O"]);
        sh(&other, &["config", "user.email", "o@o"]);
        std::fs::write(other.join("b.txt"), "b\n").unwrap();
        sh(&other, &["add", "-A"]);
        sh(&other, &["commit", "-qm", "theirs"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        let e = git.push(&repo, "origin", "main").await.unwrap_err().to_string();
        assert!(e.contains("Pull (git pull --rebase) first"), "{e}");
    }

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

    /// Which new folders are too big to be work: the folders that hold the files go, and the
    /// new package around them stays, with its sources.
    #[test]
    fn oversized_folders_are_the_ones_that_hold_the_files() {
        let names = |dir: &str, n: usize| (0..n).map(|i| format!("{dir}{i}.js")).collect::<Vec<_>>();
        let strings = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let found = |roots: &[&str], files: &[String]| oversized_folders(&strings(roots), files, 10);
        let gone = |name: &str, files: usize| (name.to_string(), files);
        let mut files = strings(&["notes.md", "pkg/package.json", "pkg/src/index.ts"]);
        // A dependency folder inside a new package: many small folders, none of them big.
        for m in 0..4 {
            files.extend(names(&format!("pkg/node_modules/m{m}/"), 5));
        }
        // Build output in two halves, neither of them big: the folder that holds both goes.
        files.extend(names("target/debug/deps/", 8));
        files.extend(names("target/debug/build/", 8));
        files.push("target/CACHEDIR.TAG".into());
        // Flat, and exactly at the limit.
        files.extend(names("coverage/", 11));
        files.extend(names("small/", 10));
        files.sort();
        assert_eq!(
            found(&["coverage/", "notes.md", "pkg/", "small/", "target/"], &files),
            [gone("coverage/", 11), gone("pkg/node_modules/", 20), gone("target/debug/", 16)],
        );
        // New files next to tracked ones are not a new folder, however many they are.
        assert!(oversized_folders(&strings(&["src/a.rs", "src/b.rs", "src/c.rs"]), &strings(&["src/a.rs", "src/b.rs", "src/c.rs"]), 2).is_empty());

        // A new package with two heavy folders, neither of which holds nearly all the files:
        // both go, and the sources next to them are work.
        let mut web = strings(&["web/package.json"]);
        web.extend(names("web/node_modules/big/", 15));
        for m in 0..37 {
            web.extend(names(&format!("web/node_modules/m{m}/"), 5));
        }
        web.extend(names("web/.next/", 30));
        web.extend(names("web/src/", 5));
        let left = found(&["web/"], &web);
        // The dependency folder goes as a whole, not only the one package that is big by itself.
        assert_eq!(left, [gone("web/.next/", 30), gone("web/node_modules/", 200)]);
        assert!(!left.iter().any(|(folder, _)| "web/src/0.js".starts_with(folder.as_str()) || "web/package.json".starts_with(folder.as_str())));

        // Two heavy folders and nothing else: the folder around them is not what goes.
        let mut two = names("gen/a/", 30);
        two.extend(names("gen/b/", 12));
        assert_eq!(found(&["gen/"], &two), [gone("gen/a/", 30), gone("gen/b/", 12)]);
        // A folder that is still too big without its heavy folder goes as a whole.
        let mut rest = names("out/cache/", 100);
        for m in 0..3 {
            rest.extend(names(&format!("out/day{m}/"), 4));
        }
        assert_eq!(found(&["out/"], &rest), [gone("out/", 112)]);
    }

    /// Codly's commit takes the work and leaves out what is too big to be work: the files
    /// stay in the worktree, untracked, and the result says what was left. A checkpoint leaves
    /// out the same, and what somebody staged is work.
    #[tokio::test]
    async fn a_commit_leaves_out_huge_new_folders_and_files() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        let fill = |dir: &str, n: usize| {
            for i in 0..n {
                let sub = repo.join(dir).join(format!("m{}", i % 40));
                std::fs::create_dir_all(&sub).unwrap();
                std::fs::write(sub.join(format!("{i}.js")), "x").unwrap();
            }
        };
        // A new package: its sources are work, its dependency folder and its build output
        // are not (two heavy folders: neither holds nearly all of the package's files).
        fill("pkg/node_modules", MAX_NEW_FOLDER_FILES + 1);
        fill("pkg/.next", MAX_NEW_FOLDER_FILES + 1);
        std::fs::create_dir_all(repo.join("pkg/src")).unwrap();
        std::fs::write(repo.join("pkg/src/index.ts"), "export {};\n").unwrap();
        // A new folder at the limit is work (fixtures, generated code).
        fill("fixtures", MAX_NEW_FOLDER_FILES);
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        // One byte over the limit (a sparse file: nothing is written).
        std::fs::File::create(repo.join("dump.bin")).unwrap().set_len(MAX_NEW_FILE_BYTES + 1).unwrap();

        let snap = git.commit_work(&repo, "snapshot").await.unwrap();
        let left = [
            LeftOut::Folder { path: "pkg/.next/".into(), files: MAX_NEW_FOLDER_FILES + 1 },
            LeftOut::Folder { path: "pkg/node_modules/".into(), files: MAX_NEW_FOLDER_FILES + 1 },
            LeftOut::File { path: "dump.bin".into(), bytes: MAX_NEW_FILE_BYTES + 1 },
        ];
        assert_eq!(snap.left_out, left);
        assert_eq!(left[1..].iter().map(LeftOut::to_string).collect::<Vec<_>>(), ["pkg/node_modules/ (2001 new files)", "dump.bin (25 MB)"]);
        let committed = snap.commit.unwrap().files;
        assert_eq!(committed.len(), MAX_NEW_FOLDER_FILES + 2, "the fixtures, a.txt and the package's source");
        assert!(committed.contains(&"a.txt".to_string()) && committed.contains(&"pkg/src/index.ts".to_string()));
        assert!(!committed.iter().any(|f| f.starts_with("pkg/node_modules/") || f.starts_with("pkg/.next/") || f == "dump.bin"));
        // Nothing was deleted or staged: the files are still there, untracked.
        assert!(repo.join("dump.bin").exists() && repo.join("pkg/node_modules/m0/0.js").exists());
        assert_eq!(git.read(&repo, &["ls-files", "--", "pkg/node_modules", "pkg/.next", "dump.bin"]).await.unwrap(), "");
        let untracked = "?? dump.bin\0?? pkg/.next/\0?? pkg/node_modules/\0";
        assert_eq!(git.read(&repo, &["status", "--porcelain=v1", "-z"]).await.unwrap(), untracked);

        // The next one has nothing to commit, and says the same.
        let again = git.commit_work(&repo, "snapshot").await.unwrap();
        assert_eq!((again.commit, again.left_out.as_slice()), (None, &left[..]));
        assert_eq!(git.commit_all(&repo, "snapshot").await.unwrap(), None);

        // A checkpoint leaves the same paths out (they are not stored, and not read again at
        // every turn), and a restore leaves them where they are.
        let tree = git.worktree_tree(&repo).await.unwrap();
        let kept = git.read(&repo, &["ls-tree", "-r", "--name-only", &tree]).await.unwrap();
        assert_eq!(kept.lines().count(), MAX_NEW_FOLDER_FILES + 2, "what the commit has, and no more");
        assert!(kept.contains("pkg/src/index.ts") && !kept.contains("node_modules") && !kept.contains(".next") && !kept.contains("dump.bin"));
        std::fs::write(repo.join("later.txt"), "made later\n").unwrap();
        let present = git.snapshot(&repo).await.unwrap();
        git.restore_tree(&repo, present, &tree).await.unwrap();
        assert!(!repo.join("later.txt").exists(), "the restore ran");
        assert!(repo.join("dump.bin").exists() && repo.join("pkg/node_modules/m0/0.js").exists() && repo.join("pkg/.next/m0/0.js").exists());
        assert_eq!(git.read(&repo, &["status", "--porcelain=v1", "-z"]).await.unwrap(), untracked);

        // What somebody staged is work, whatever its size: it is committed.
        sh(&repo, &["add", "dump.bin"]);
        let staged = git.commit_work(&repo, "staged").await.unwrap();
        assert_eq!(staged.commit.unwrap().files, ["dump.bin"]);
        assert_eq!(staged.left_out, left[..2]);

        // Once the repository ignores the rest, there is nothing to leave out.
        std::fs::write(repo.join(".gitignore"), "node_modules/\n.next/\n").unwrap();
        let ignored = git.commit_work(&repo, "ignore").await.unwrap();
        assert_eq!(ignored.commit.unwrap().files, [".gitignore"]);
        assert!(ignored.left_out.is_empty());
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

    /// A commit on a new branch `name` (from main) that sets `file` (None deletes it); back on
    /// main afterwards.
    fn branch_with(repo: &Path, name: &str, file: &str, content: Option<&str>) {
        sh(repo, &["checkout", "-q", "-b", name, "main"]);
        match content {
            Some(c) => std::fs::write(repo.join(file), c).unwrap(),
            None => std::fs::remove_file(repo.join(file)).unwrap(),
        }
        sh(repo, &["add", "-A"]);
        sh(repo, &["commit", "-qm", name]);
        sh(repo, &["checkout", "-q", "main"]);
    }

    /// A commit on the checked-out branch that sets `file`.
    fn commit_file(repo: &Path, file: &str, content: &str) {
        std::fs::write(repo.join(file), content).unwrap();
        sh(repo, &["add", "-A"]);
        sh(repo, &["commit", "-qm", file]);
    }

    /// Runs git and expects it to stop (a conflict).
    fn stops(repo: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(repo).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(!out.status.success(), "git {args:?} was expected to stop on a conflict");
    }

    /// The user is resolving a merge of their own: Codly's merge into their checkout is refused
    /// before git runs, and their merge and the resolution they have not committed stay.
    #[tokio::test]
    async fn a_merge_the_user_has_in_progress_is_left_alone() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "theirs", "a.txt", Some("theirs\n"));
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "a.txt", "mine\n");
        stops(&repo, &["merge", "theirs"]);
        let theirs = git.rev_parse(&repo, "theirs").await.unwrap();
        std::fs::write(repo.join("a.txt"), "resolved by hand\n").unwrap();

        let err = git.merge(&repo, "plan", "merge plan", false).await.unwrap_err().to_string();
        assert!(err.contains("a merge in progress") && err.contains("nothing was changed"), "{err}");
        assert_eq!(git.rev_parse(&repo, "MERGE_HEAD").await.unwrap(), theirs, "their merge is still in progress");
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "resolved by hand\n");
        assert!(!repo.join("b.txt").exists(), "nothing of the plan was merged");

        // Once they conclude it, the same merge goes through.
        sh(&repo, &["commit", "-qam", "merge theirs"]);
        assert!(matches!(git.merge(&repo, "plan", "merge plan", false).await.unwrap(), MergeOutcome::Merged { .. }));
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "resolved by hand\n");
        assert!(repo.join("b.txt").exists());
    }

    /// The same for a cherry-pick the user stopped in: no merge on top of it.
    #[tokio::test]
    async fn a_cherry_pick_in_progress_blocks_the_merge() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "theirs", "a.txt", Some("theirs\n"));
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "a.txt", "mine\n");
        stops(&repo, &["cherry-pick", "theirs"]);
        assert_eq!(git.in_progress(&repo).await.unwrap(), Some("a cherry-pick"));
        let err = git.merge(&repo, "plan", "merge plan", false).await.unwrap_err().to_string();
        assert!(err.contains("a cherry-pick in progress"), "{err}");
        sh(&repo, &["cherry-pick", "--abort"]);
        assert_eq!(git.in_progress(&repo).await.unwrap(), None);
    }

    /// Undo merge: the branch goes back to the commit before the merge, the merged work stays
    /// on its own branch, and files that are not committed are never touched.
    #[tokio::test]
    async fn undo_merge_puts_the_branch_back() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "c.txt", "c\n");
        let before = git.head(&repo).await.unwrap().unwrap();
        let MergeOutcome::Merged { sha } = git.merge(&repo, "plan", "merge plan", false).await.unwrap() else { panic!("expected a merge") };
        std::fs::write(repo.join("notes.txt"), "not committed\n").unwrap();
        assert_eq!(git.undo_merge_blocker(&repo, "main", &sha, &before, "plan").await.unwrap(), None);

        assert!(!git.undo_merge(&repo, "main", &sha, &before, "plan").await.unwrap(), "no branch had to be put back");
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(before.as_str()));
        assert_eq!(git.current_branch(&repo).await.unwrap().as_deref(), Some("main"));
        assert!(!repo.join("b.txt").exists(), "the merged file is gone from the checkout");
        assert!(repo.join("c.txt").exists());
        assert_eq!(std::fs::read_to_string(repo.join("notes.txt")).unwrap(), "not committed\n", "untracked files stay");
        assert!(!git.dirty(&repo, false).await.unwrap());
        assert!(git.diff(&repo, "main", "plan").await.unwrap().contains("+b"), "the work is still on its branch");

        // Undone once: the same undo is refused, and the merge can be made again.
        let again = git.undo_merge(&repo, "main", &sha, &before, "plan").await.unwrap_err().to_string();
        assert!(again.contains("main has changed since this merge"), "{again}");
        assert!(matches!(git.merge(&repo, "plan", "merge plan", false).await.unwrap(), MergeOutcome::Merged { .. }));
    }

    /// Undo merge is refused, with the reason, whenever it could lose or rewrite something:
    /// uncommitted changes, a later commit, another branch checked out, a merge already pushed.
    #[tokio::test]
    async fn undo_merge_says_why_it_is_not_safe() {
        let (d, repo) = repo();
        let remote = d.path().join("remote.git");
        sh(d.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
        sh(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
        let git = Git::from_process_env();
        git.push(&repo, "origin", "main").await.unwrap();
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "c.txt", "c\n");
        let before = git.head(&repo).await.unwrap().unwrap();
        let MergeOutcome::Merged { sha } = git.merge(&repo, "plan", "merge plan", false).await.unwrap() else { panic!("expected a merge") };
        let why = async || git.undo_merge_blocker(&repo, "main", &sha, &before, "plan").await.unwrap().unwrap_or_default();
        assert_eq!(why().await, "", "an unpushed merge at the tip of a clean checkout can be undone");

        // A tracked file with uncommitted changes, staged or not.
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        let dirty = "your checkout has uncommitted changes in a.txt; put them aside with git stash first (a commit on top of the merge would make undo impossible)";
        assert_eq!(why().await, dirty);
        sh(&repo, &["add", "a.txt"]);
        let err = git.undo_merge(&repo, "main", &sha, &before, "plan").await.unwrap_err().to_string();
        assert!(err.contains("cannot be undone") && err.contains("uncommitted changes in a.txt"), "{err}");
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(sha.as_str()), "nothing moved");
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "edited\n");
        // The way out it names keeps undo possible.
        sh(&repo, &["stash", "-q"]);
        assert_eq!(why().await, "");
        sh(&repo, &["stash", "pop", "-q"]);
        sh(&repo, &["reset", "-q", "--hard"]);

        // The wrong commit to go back to.
        let wrong = git.rev_parse(&repo, "plan").await.unwrap().unwrap();
        let odd = git.undo_merge_blocker(&repo, "main", &sha, &wrong, "plan").await.unwrap().unwrap();
        assert!(odd.contains("is not a merge made on top of"), "{odd}");

        // Another branch checked out, at the same commit.
        sh(&repo, &["checkout", "-q", "-b", "other"]);
        assert_eq!(why().await, "your checkout is on other now, not main");
        sh(&repo, &["checkout", "-q", "--detach"]);
        assert_eq!(why().await, "your checkout is not on main now");
        sh(&repo, &["checkout", "-q", "main"]);

        // A merge of their own in progress.
        branch_with(&repo, "theirs", "d.txt", Some("d\n"));
        sh(&repo, &["merge", "-q", "--no-ff", "--no-commit", "theirs"]);
        let merging = "your checkout has a merge in progress; abort it first (finishing it would put a commit on top of the merge and make undo impossible)";
        assert_eq!(why().await, merging);
        sh(&repo, &["merge", "--abort"]);
        assert_eq!(why().await, "", "after the abort it names, the merge can still be undone");

        // Pushed: undoing it here would rewrite what the remote already has.
        git.push(&repo, "origin", "main").await.unwrap();
        let pushed = why().await;
        assert!(pushed.starts_with("the merge is already on origin/main; undo it there with git revert -m 1 "), "{pushed}");

        // A commit after the merge.
        commit_file(&repo, "a.txt", "later\n");
        let moved = why().await;
        assert!(moved.starts_with("main has changed since this merge (it is at "), "{moved}");
        assert!(git.undo_merge(&repo, "main", &sha, &before, "plan").await.is_err());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "later\n");
    }

    /// The merged branch was deleted after the merge (git allows it: it is merged). Undo puts it
    /// back at the merged work first, so that work is never left on no branch.
    #[tokio::test]
    async fn undo_merge_puts_a_deleted_branch_back() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "c.txt", "c\n");
        let before = git.head(&repo).await.unwrap().unwrap();
        let work = git.rev_parse(&repo, "plan").await.unwrap();
        let MergeOutcome::Merged { sha } = git.merge(&repo, "plan", "merge plan", false).await.unwrap() else { panic!("expected a merge") };
        sh(&repo, &["branch", "-q", "-d", "plan"]);
        assert_eq!(git.undo_merge_blocker(&repo, "main", &sha, &before, "plan").await.unwrap(), None);

        assert!(git.undo_merge(&repo, "main", &sha, &before, "plan").await.unwrap(), "the branch was put back");
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(before.as_str()));
        assert_eq!(git.rev_parse(&repo, "plan").await.unwrap(), work, "the work is on its branch again");
        assert!(matches!(git.merge(&repo, "plan", "merge plan", false).await.unwrap(), MergeOutcome::Merged { .. }));
    }

    /// The merged branch points somewhere else now, so undo cannot put it back: refused while
    /// no other branch has the merged work, and fine once one does.
    #[tokio::test]
    async fn undo_merge_never_leaves_the_work_on_no_branch() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "plan", "b.txt", Some("b\n"));
        commit_file(&repo, "c.txt", "c\n");
        let before = git.head(&repo).await.unwrap().unwrap();
        let work = git.rev_parse(&repo, "plan").await.unwrap().unwrap();
        let MergeOutcome::Merged { sha } = git.merge(&repo, "plan", "merge plan", false).await.unwrap() else { panic!("expected a merge") };
        sh(&repo, &["branch", "-q", "-f", "plan", &before]);

        let why = git.undo_merge_blocker(&repo, "main", &sha, &before, "plan").await.unwrap().unwrap();
        let keep = format!("keep it first with git branch <new name> {}", short(&work));
        assert!(why.starts_with("plan was moved and no other branch has the merged work") && why.ends_with(&keep), "{why}");
        assert!(git.undo_merge(&repo, "main", &sha, &before, "plan").await.is_err());
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(sha.as_str()), "the merge is still there");

        sh(&repo, &["branch", "-q", "kept", &work]);
        assert!(!git.undo_merge(&repo, "main", &sha, &before, "plan").await.unwrap(), "kept has the work: nothing to put back");
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(before.as_str()));
        assert_eq!(git.rev_parse(&repo, "plan").await.unwrap().as_deref(), Some(before.as_str()), "plan stays where the user moved it");
        assert_eq!(git.rev_parse(&repo, "kept").await.unwrap().as_deref(), Some(work.as_str()));
    }

    /// The merge deleted a file and the user has since made a new, uncommitted file of that
    /// name: undo would have to bring the old one back over it, so git stops and the file stays.
    #[tokio::test]
    async fn undo_merge_never_overwrites_an_untracked_file() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "plan", "a.txt", None);
        commit_file(&repo, "c.txt", "c\n");
        let before = git.head(&repo).await.unwrap().unwrap();
        let MergeOutcome::Merged { sha } = git.merge(&repo, "plan", "merge plan", false).await.unwrap() else { panic!("expected a merge") };
        assert!(!repo.join("a.txt").exists());
        std::fs::write(repo.join("a.txt"), "a new file of mine\n").unwrap();

        assert!(git.undo_merge(&repo, "main", &sha, &before, "plan").await.is_err());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "a new file of mine\n");
        assert_eq!(git.head(&repo).await.unwrap().as_deref(), Some(sha.as_str()), "the merge is still there");
    }

    /// What a new worktree lacks: the ignored paths, a folder ignored as a whole as one entry.
    #[tokio::test]
    async fn ignored_lists_ignored_untracked_paths_and_whole_folders_once() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        std::fs::write(repo.join(".gitignore"), "node_modules/\n.env\nlocal.json\n").unwrap();
        std::fs::create_dir_all(repo.join("pkg/node_modules/dep")).unwrap();
        std::fs::write(repo.join("pkg/index.js"), "tracked\n").unwrap();
        std::fs::write(repo.join("pkg/local.json"), "{}\n").unwrap();
        std::fs::write(repo.join("pkg/node_modules/dep/a.js"), "a\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "ignore rules"]);
        std::fs::create_dir_all(repo.join("node_modules/dep")).unwrap();
        std::fs::write(repo.join("node_modules/dep/a.js"), "a\n").unwrap();
        std::fs::write(repo.join(".env"), "A=1\n").unwrap();
        std::fs::write(repo.join("notes.txt"), "untracked, not ignored\n").unwrap();
        // A folder that only holds ignored files is not ignored itself: its files are listed.
        std::fs::create_dir_all(repo.join("fresh")).unwrap();
        std::fs::write(repo.join("fresh/local.json"), "{}\n").unwrap();

        assert_eq!(git.ignored(&repo).await.unwrap(), [".env", "fresh/local.json", "node_modules/", "pkg/local.json", "pkg/node_modules/"]);
        assert_eq!(git.ignored_files(&repo, &["node_modules/", "pkg/node_modules/"]).await.unwrap(), ["node_modules/dep/a.js", "pkg/node_modules/dep/a.js"]);
    }

    /// The rules of the checkout that is asked decide, also for a path that is not there yet.
    #[tokio::test]
    async fn is_ignored_reads_the_rules_of_the_checkout_it_is_asked_about() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        std::fs::write(repo.join(".gitignore"), "node_modules/\n/secrets/\n.env\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "ignore rules"]);
        let wt = repo.parent().unwrap().join("wt");
        git.worktree_add(&repo, &wt, "codly/test/ignore", "HEAD").await.unwrap();
        // A rule you did not commit: your checkout has it, the worktree does not.
        std::fs::write(repo.join(".gitignore"), "node_modules/\n/secrets/\n.env\nconfig/local.json\n").unwrap();

        for (rel, dir) in [("node_modules", true), ("packages/a/node_modules", true), ("secrets/dev.env", false), (".env", false)] {
            assert!(git.is_ignored(&wt, rel, dir).await.unwrap(), "{rel}");
        }
        assert!(!git.is_ignored(&wt, "node_modules", false).await.unwrap(), "the rule names a folder");
        assert!(!git.is_ignored(&wt, "src", true).await.unwrap());
        assert!(git.is_ignored(&repo, "config/local.json", false).await.unwrap());
        assert!(!git.is_ignored(&wt, "config/local.json", false).await.unwrap(), "the worktree's own rules decide");
    }

    /// A branch can have the name of a folder or a file in the checkout, or of a tag. The
    /// storage reads still mean the branch, and git does not stop on a name that is two things.
    #[tokio::test]
    async fn storage_reads_take_a_branch_name_for_the_branch_only() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        let base = git.head(&repo).await.unwrap().unwrap();
        sh(&repo, &["switch", "-q", "-c", "docs"]);
        std::fs::create_dir(repo.join("docs")).unwrap();
        commit_file(&repo, "docs/guide.md", "guide\n");
        let head = git.head(&repo).await.unwrap().unwrap();
        // A tag named like the checked-out branch, and a built program named like the default one.
        sh(&repo, &["tag", "docs", &base]);
        std::fs::write(repo.join("main"), "binary").unwrap();

        assert_eq!(git.checked_out_branch(&repo).await.unwrap().as_deref(), Some("docs"));
        assert_eq!(git.commits_only_on(&repo, &head, &["main".to_string()]).await.unwrap(), 1);
        let kept = ["docs".to_string(), "main".to_string()];
        assert_eq!(git.commits_only_on(&repo, &head, &kept).await.unwrap(), 0, "the branch docs has the commit; the tag docs does not");
        assert_eq!(git.commits_only_on(&repo, &head, &[]).await.unwrap(), 2);
        assert_eq!(git.commits_on_no_branch(&repo, &head).await.unwrap(), 0);
        assert!(git.committed_at(&repo, &head).await.unwrap() > 0);

        sh(&repo, &["switch", "-q", "--detach"]);
        assert_eq!(git.checked_out_branch(&repo).await.unwrap(), None);
    }

    /// What `git` prints for a question about `dir`.
    fn out(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A worktree with work in every state: a tracked file changed, a file staged, a file that
    /// is untracked (in a new folder) and a file that the repository ignores.
    async fn worktree_with_work(d: &Path, repo: &Path, git: &Git) -> PathBuf {
        std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        commit_file(repo, "b.txt", "b\n");
        let wt = d.join("wt");
        git.worktree_add(repo, &wt, "work", "main").await.unwrap();
        std::fs::write(wt.join("a.txt"), "a changed\n").unwrap();
        std::fs::write(wt.join("staged.txt"), "staged\n").unwrap();
        sh(&wt, &["add", "staged.txt"]);
        std::fs::create_dir(wt.join("new")).unwrap();
        std::fs::write(wt.join("new/untracked.txt"), "untracked\n").unwrap();
        std::fs::write(wt.join("build.log"), "ignored\n").unwrap();
        wt
    }

    /// A checkpoint's tree has the tracked and the untracked files and no ignored one. Taking
    /// it changes neither the worktree's index, nor HEAD, nor a branch, nor a file.
    #[tokio::test]
    async fn a_worktree_tree_has_untracked_files_and_leaves_the_index_and_head_alone() {
        let (d, repo) = repo();
        let git = Git::from_process_env();
        let wt = worktree_with_work(d.path(), &repo, &git).await;
        let status = out(&wt, &["status", "--porcelain=v1"]);
        let head = out(&wt, &["rev-parse", "HEAD"]);
        let refs = out(&wt, &["for-each-ref"]);
        // Read after `status`, which may refresh the index itself.
        let admin = git.git_path(&wt, "--git-dir").await.unwrap();
        let index = std::fs::read(admin.join("index")).unwrap();

        let tree = git.worktree_tree(&wt).await.unwrap();
        assert_eq!(std::fs::read(admin.join("index")).unwrap(), index, "the worktree's own index is untouched");
        let files = out(&wt, &["ls-tree", "-r", "--name-only", &tree]);
        assert_eq!(files.lines().collect::<Vec<_>>(), [".gitignore", "a.txt", "b.txt", "new/untracked.txt", "staged.txt"]);
        assert_eq!(out(&wt, &["show", &format!("{tree}:a.txt")]), "a changed\n", "the file as it is now, not as committed");
        assert_eq!(out(&wt, &["status", "--porcelain=v1"]), status);
        assert_eq!(out(&wt, &["rev-parse", "HEAD"]), head);
        assert_eq!(out(&wt, &["for-each-ref"]), refs, "no branch or ref moved");
        let left: Vec<String> = std::fs::read_dir(&admin).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!left.iter().any(|f| f.starts_with("codly-index")), "the private index is removed: {left:?}");
        assert_eq!(git.worktree_tree(&wt).await.unwrap(), tree, "nothing changed: the same tree");

        // The commit is on no branch: only the private ref names it, and it is never replaced.
        let sha = git.commit_tree(&wt, &tree, Some(head.trim()), "codly(checkpoint): test").await.unwrap();
        git.create_ref(&repo, "refs/codly/checkpoints/a/1", &sha).await.unwrap();
        assert_eq!(out(&wt, &["rev-parse", "HEAD"]), head);
        assert_eq!(out(&repo, &["branch", "--contains", &sha]), "", "on no branch");
        assert_eq!(out(&repo, &["rev-parse", "refs/codly/checkpoints/a/1"]).trim(), sha);
        assert!(git.create_ref(&repo, "refs/codly/checkpoints/a/1", head.trim()).await.is_err(), "a checkpoint is never written over");
        git.delete_ref(&repo, "refs/codly/checkpoints/a/1").await.unwrap();
        assert_eq!(out(&repo, &["for-each-ref", "refs/codly"]), "");
    }

    /// Restore: changed and deleted files come back, files made later go, ignored files stay,
    /// HEAD and the index do not move. The state that a restore replaced can be restored too.
    #[tokio::test]
    async fn restore_tree_puts_files_back_and_can_be_undone() {
        let (d, repo) = repo();
        let git = Git::from_process_env();
        let wt = worktree_with_work(d.path(), &repo, &git).await;
        let before = git.worktree_tree(&wt).await.unwrap();

        // A turn's work: a file changed, a tracked and an untracked file deleted, a file made in
        // a new folder, Codly's commit of all that, and one more file after the commit.
        std::fs::write(wt.join("a.txt"), "a changed again\n").unwrap();
        std::fs::remove_file(wt.join("b.txt")).unwrap();
        std::fs::remove_file(wt.join("new/untracked.txt")).unwrap();
        std::fs::create_dir(wt.join("later")).unwrap();
        std::fs::write(wt.join("later/made.txt"), "made later\n").unwrap();
        std::fs::write(wt.join("build.log"), "ignored, written later\n").unwrap();
        git.commit_all(&wt, "the turn").await.unwrap().unwrap();
        std::fs::write(wt.join("after.txt"), "not committed\n").unwrap();
        let head = out(&wt, &["rev-parse", "HEAD"]);
        let after = git.worktree_tree(&wt).await.unwrap();
        let admin = git.git_path(&wt, "--git-dir").await.unwrap();
        let index = std::fs::read(admin.join("index")).unwrap();

        let present = git.snapshot(&wt).await.unwrap();
        assert_eq!(present.tree(), after);
        git.restore_tree(&wt, present, &before).await.unwrap();
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "a changed\n");
        assert_eq!(std::fs::read_to_string(wt.join("b.txt")).unwrap(), "b\n", "a deleted file is back");
        assert_eq!(std::fs::read_to_string(wt.join("new/untracked.txt")).unwrap(), "untracked\n", "also one that was never committed");
        assert!(!wt.join("later").exists() && !wt.join("after.txt").exists(), "files made later are gone");
        assert_eq!(std::fs::read_to_string(wt.join("build.log")).unwrap(), "ignored, written later\n", "ignored files are left alone");
        assert_eq!(git.worktree_tree(&wt).await.unwrap(), before, "the files are exactly as they were");
        assert_eq!(out(&wt, &["rev-parse", "HEAD"]), head, "HEAD did not move");
        assert_eq!(out(&wt, &["symbolic-ref", "--short", "HEAD"]).trim(), "work");
        assert_eq!(std::fs::read(admin.join("index")).unwrap(), index, "the worktree's own index is untouched");
        assert!(git.dirty(&wt, true).await.unwrap(), "the difference from HEAD shows as uncommitted changes");

        // Undo: back to the state that the restore replaced.
        let present = git.snapshot(&wt).await.unwrap();
        git.restore_tree(&wt, present, &after).await.unwrap();
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "a changed again\n");
        assert!(!wt.join("b.txt").exists() && !wt.join("new").exists());
        assert_eq!(std::fs::read_to_string(wt.join("later/made.txt")).unwrap(), "made later\n");
        assert_eq!(std::fs::read_to_string(wt.join("after.txt")).unwrap(), "not committed\n");
        assert_eq!(git.worktree_tree(&wt).await.unwrap(), after);
        assert_eq!(out(&wt, &["rev-parse", "HEAD"]), head);
    }

    /// A checkout, a commit, a merge or a push may take minutes in a large repository; a read
    /// may not. The command decides, wherever the `-c` flags put it.
    #[test]
    fn a_call_gets_the_time_limit_of_what_it_does() {
        let [c1, hooks, c2, fsmonitor] = Git::SAFE;
        assert_eq!(subcommand(&[c1, hooks, c2, fsmonitor, "add", "-A"]), "add");
        assert_eq!(subcommand(&["status", "--porcelain=v1"]), "status");
        assert_eq!(subcommand(&[]), "");
        let long: [&[&str]; 7] = [
            &["push", "--set-upstream", "origin", "x:refs/heads/x"],
            &[c1, hooks, c2, fsmonitor, "worktree", "add", "-b", "t1", "/w", "main"],
            &["worktree", "remove", "--force", "/w"],
            &[c1, hooks, c2, fsmonitor, "add", "-A"],
            &[c1, hooks, "commit", "--no-verify", "-m", "x"],
            &["merge", "--abort"],
            &[c1, hooks, c2, fsmonitor, "switch", "t1"],
        ];
        for args in long {
            assert!(long_running(args), "{args:?}");
        }
        let quick: [&[&str]; 6] = [
            &["worktree", "prune"],
            &["worktree", "list", "--porcelain"],
            &["status", "--porcelain=v1", "-z"],
            &["merge-base", "--is-ancestor", "a", "b"],
            &["merge-tree", "--write-tree", "a", "b"],
            &["-c", "core.fsmonitor=false", "diff", "--name-only", "add"],
        ];
        for args in quick {
            assert!(!long_running(args), "{args:?}");
        }
    }

    /// A call that runs out of time is asked to stop before it is killed, so that git removes
    /// its lock files; the error names the command and the limit, and the call can be made again.
    #[tokio::test]
    async fn a_call_out_of_time_is_asked_to_stop_first() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, lock, stopped) = (dir.path().join("git"), dir.path().join("held.lock"), dir.path().join("stopped"));
        // Stands for a git that holds a lock file and, as git does, removes it when asked to stop.
        let body = format!(
            "#!/bin/sh\ntrap 'rm -f \"{lock}\"; touch \"{stopped}\"; kill $!; exit 1' TERM\ntouch \"{lock}\"\nsleep 30 &\nwait\n",
            lock = lock.display(),
            stopped = stopped.display()
        );
        executable(&fake, &body);
        let mut git = Git::new(fake, std::env::vars().collect());
        git.quick = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let e = git.ok(dir.path(), &["-c", "core.fsmonitor=false", "status"], true).await.unwrap_err();
        assert_eq!(e.to_string(), "git status timed out after 1s");
        assert!(transient(&e));
        assert!(stopped.exists(), "it was asked to stop, not killed at once");
        assert!(!lock.exists(), "so it removed its lock file");
        assert!(started.elapsed() < Duration::from_secs(15), "the `sleep 30` was not waited for");
    }

    /// Git ended and left something running that holds its pipes (a hook's background job, a
    /// shared ssh connection): the call still runs out of time, but git's process id is no
    /// longer git's, so nothing is asked to stop and nothing is waited for.
    #[tokio::test]
    async fn a_git_that_ended_is_not_asked_to_stop() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("git");
        executable(&fake, "#!/bin/sh\nsleep 6 &\nexit 0\n");
        let mut git = Git::new(fake, std::env::vars().collect());
        git.quick = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let e = git.ok(dir.path(), &["-c", "core.fsmonitor=false", "status"], true).await.unwrap_err();
        assert_eq!(e.to_string(), "git status timed out after 1s");
        assert!(started.elapsed() < git.quick + GIT_STOP_GRACE, "no process was given the time to stop: {:?}", started.elapsed());
    }

    /// Another git process holds the index lock: the call changes nothing and fails in a way
    /// that is told apart from a real failure, with the command's name in the error.
    #[tokio::test]
    async fn a_held_lock_file_is_a_failure_that_passes_by_itself() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        let lock = repo.join(".git/index.lock");
        std::fs::write(&lock, "").unwrap();
        let e = git.commit_all(&repo, "work").await.unwrap_err();
        assert!(transient(&e), "{e:#}");
        assert!(e.to_string().starts_with("git add failed:"), "{e}");
        std::fs::remove_file(&lock).unwrap();
        let commit = git.commit_all(&repo, "work").await.unwrap().expect("the same call works once the lock is gone");
        assert_eq!(commit.files, ["b.txt"]);

        let e = git.merge(&repo, "no-such-branch", "m", true).await.unwrap_err();
        assert!(!transient(&e), "{e:#}");
        let left = anyhow!("git merge timed out after 900s ({MERGE_LEFT}: run `git merge --abort`)");
        assert!(!transient(&left), "a merge that could not be aborted needs you");
    }

    /// A `worktree add` that fails after it made the worktree (stopped in the middle of the
    /// checkout) leaves nothing behind: the next try makes a whole worktree instead of reusing
    /// one with files missing. A folder that was there before is not Codly's to remove.
    #[tokio::test]
    async fn a_failed_worktree_add_leaves_no_half_made_worktree() {
        let (d, repo) = repo();
        let wt = d.path().join("wt");
        // A git that makes the worktree, loses one of its files and fails.
        let broken = d.path().join("git-stopped");
        let body = format!(
            "#!/bin/sh\ncase \" $* \" in\n*\" worktree add \"*) git \"$@\" && rm \"{}\"; exit 1 ;;\nesac\nexec git \"$@\"\n",
            wt.join("a.txt").display()
        );
        executable(&broken, &body);
        let stopped = Git::new(broken, std::env::vars().collect());
        let base = stopped.head(&repo).await.unwrap().unwrap();
        assert!(stopped.worktree_add(&repo, &wt, "t1", &base).await.is_err());
        assert!(!wt.exists(), "the half-made worktree is removed");
        let git = Git::from_process_env();
        assert_eq!(git.worktrees(&repo).await.unwrap().len(), 1, "and git has forgotten it");
        git.worktree_add(&repo, &wt, "t1", &base).await.unwrap();
        assert!(wt.join("a.txt").exists(), "the next try checks every file out");

        let taken = d.path().join("taken");
        std::fs::create_dir(&taken).unwrap();
        std::fs::write(taken.join("keep.txt"), "mine\n").unwrap();
        assert!(git.worktree_add_detached(&repo, &taken, &base).await.is_err(), "git does not add a worktree over a folder with files");
        assert_eq!(std::fs::read_to_string(taken.join("keep.txt")).unwrap(), "mine\n");
    }

    /// Internal checkouts run none of the user's hooks: a `post-checkout` hook stays quiet when
    /// Codly makes a worktree.
    #[tokio::test]
    async fn worktree_adds_do_not_run_the_users_hooks() {
        let (d, repo) = repo();
        let ran = d.path().join("hook-ran");
        executable(&repo.join(".git/hooks/post-checkout"), &format!("#!/bin/sh\ntouch \"{}\"\n", ran.display()));
        let git = Git::from_process_env();
        let base = git.head(&repo).await.unwrap().unwrap();
        git.worktree_add(&repo, &d.path().join("wt"), "t1", &base).await.unwrap();
        git.worktree_add_detached(&repo, &d.path().join("home"), &base).await.unwrap();
        assert!(!ran.exists(), "the post-checkout hook ran");
    }

    /// The user's slow `post-merge` hook runs out of time after the merge commit was made: the
    /// merge is reported as made (their branch moved), not as "aborted, nothing changed".
    #[tokio::test]
    async fn a_merge_stopped_after_its_commit_is_reported_as_made() {
        let (_d, repo) = repo();
        branch_with(&repo, "work", "w.txt", Some("w\n"));
        commit_file(&repo, "c.txt", "c\n");
        executable(&repo.join(".git/hooks/post-merge"), "#!/bin/sh\nexec sleep 8\n");
        let mut git = Git::from_process_env();
        git.long = Duration::from_secs(3);
        let merged = git.merge(&repo, "work", "merge work", false).await.unwrap();
        let head = git.head(&repo).await.unwrap().unwrap();
        assert_eq!(merged, MergeOutcome::Merged { sha: head });
        assert!(repo.join("w.txt").exists() && !git.in_merge(&repo).await);
        assert_eq!(git.rev_parse(&repo, "HEAD^2").await.unwrap(), git.rev_parse(&repo, "work").await.unwrap(), "HEAD is the merge of work");
    }

    /// A branch `name` whose merge with main is resolved by hand: main changed a.txt (a
    /// conflict) and added c.txt (no conflict). `resolved` is a.txt as the merge commits it.
    /// Returns the branch as it was before the merge.
    async fn resolved_merge(git: &Git, repo: &Path, name: &str, resolved: &str) -> String {
        branch_with(repo, name, "a.txt", Some("from the plan\n"));
        commit_file(repo, "a.txt", "from main\n");
        commit_file(repo, "c.txt", "only on main\n");
        sh(repo, &["checkout", "-q", name]);
        let before = git.head(repo).await.unwrap().unwrap();
        assert_eq!(git.merge_for_resolution(repo, "main").await.unwrap(), ["a.txt"]);
        commit_file(repo, "a.txt", resolved);
        before
    }

    /// The diff of a resolution has what was done by hand and nothing else: each conflict as
    /// git left it against how it was resolved, then the commits made after the merge. What
    /// the merge brought in without a conflict is not in it.
    #[tokio::test]
    async fn a_resolution_diff_shows_the_conflicts_and_the_later_commits_only() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        let before = resolved_merge(&git, &repo, "plan", "from main and the plan\n").await;
        let diff = git.resolution_diff(&repo, &before, "plan").await.unwrap();
        assert!(diff.contains("diff --git a/a.txt b/a.txt") && diff.contains("-<<<<<<<") && diff.contains("->>>>>>>"), "{diff}");
        assert!(diff.contains("-from the plan\n") && diff.contains("-from main\n") && diff.contains("+from main and the plan\n"), "{diff}");
        // (A conflict marker names main's last commit, "c.txt": only the file's own diff counts.)
        assert!(!diff.contains("a/c.txt") && !diff.contains("only on main"), "what merged by itself is left out: {diff}");
        assert_eq!(diff.matches("diff --git").count(), 1, "{diff}");

        // A commit after the merge (a fix round) follows with its own diff.
        commit_file(&repo, "d.txt", "a later fix\n");
        let diff = git.resolution_diff(&repo, &before, "plan").await.unwrap();
        let (merge, fix) = (diff.find("+from main and the plan").unwrap(), diff.find("+a later fix").unwrap());
        assert!(merge < fix && !diff.contains("a/c.txt") && diff.matches("diff --git").count() == 2, "{diff}");
    }

    /// A conflict that was resolved by taking one side whole shows in the diff of the
    /// resolution (a combined diff would leave it out), and a merge without a conflict that
    /// nobody changed has an empty one.
    #[tokio::test]
    async fn a_resolution_diff_shows_a_side_that_was_dropped() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        let before = resolved_merge(&git, &repo, "plan", "from the plan\n").await;
        let diff = git.resolution_diff(&repo, &before, "plan").await.unwrap();
        assert!(diff.contains("-from main\n") && !diff.contains("+from main"), "main's line is gone, and the diff says so: {diff}");

        sh(&repo, &["checkout", "-q", "main"]);
        branch_with(&repo, "clean", "e.txt", Some("e\n"));
        commit_file(&repo, "f.txt", "f\n");
        sh(&repo, &["checkout", "-q", "clean"]);
        let before = git.head(&repo).await.unwrap().unwrap();
        assert!(git.merge_for_resolution(&repo, "main").await.unwrap().is_empty());
        sh(&repo, &["commit", "-qm", "merge main"]);
        assert_eq!(git.resolution_diff(&repo, &before, "clean").await.unwrap().trim(), "");
        assert_eq!(git.commits_ahead(&repo, &before, "clean").await.unwrap(), 2, "the merge and the commit it brought in");
        assert_eq!(git.commits_ahead(&repo, "clean", &before).await.unwrap(), 0);
    }

    /// The new paths of a worktree: untracked files, a folder that is new as a whole as one
    /// entry, and nothing that is ignored or tracked.
    #[tokio::test]
    async fn untracked_lists_new_files_and_whole_new_folders() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        commit_file(&repo, ".gitignore", "*.log\n");
        assert!(git.untracked(&repo).await.unwrap().is_empty());
        std::fs::create_dir_all(repo.join("out/deep")).unwrap();
        for file in ["note.txt", "debug.log", "out/1.txt", "out/deep/2.txt"] {
            std::fs::write(repo.join(file), "x\n").unwrap();
        }
        assert_eq!(git.untracked(&repo).await.unwrap(), ["note.txt", "out/"]);
    }

    /// A ref that is set again moves, where `create_ref` would refuse.
    #[tokio::test]
    async fn set_ref_moves_a_ref_that_exists() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        let first = git.head(&repo).await.unwrap().unwrap();
        git.set_ref(&repo, "refs/codly/before-merge", &first).await.unwrap();
        commit_file(&repo, "b.txt", "b\n");
        let second = git.head(&repo).await.unwrap().unwrap();
        git.set_ref(&repo, "refs/codly/before-merge", &second).await.unwrap();
        assert_eq!(git.rev_parse(&repo, "refs/codly/before-merge").await.unwrap(), Some(second));
    }

    /// A merge without a conflict is committed as git made it: a new file that was in the
    /// worktree stays untracked and is not in the merge commit, and nobody changed the merge
    /// by hand. Without an open merge there is nothing to commit.
    #[tokio::test]
    async fn conclude_merge_commits_the_merge_and_adds_nothing() {
        let (_d, repo) = repo();
        let git = Git::from_process_env();
        branch_with(&repo, "plan", "e.txt", Some("e\n"));
        commit_file(&repo, "f.txt", "f\n");
        sh(&repo, &["checkout", "-q", "plan"]);
        let before = git.head(&repo).await.unwrap().unwrap();
        std::fs::write(repo.join("junit.xml"), "<testsuite/>\n").unwrap();
        let err = git.conclude_merge(&repo, "merge main").await.unwrap_err().to_string();
        assert!(err.starts_with("no merge is open in"), "{err}");
        assert_eq!(git.head(&repo).await.unwrap().unwrap(), before);

        assert!(git.merge_for_resolution(&repo, "main").await.unwrap().is_empty());
        let merge = git.conclude_merge(&repo, "merge main").await.unwrap();
        assert_eq!((git.head(&repo).await.unwrap().as_ref(), &merge.files[..]), (Some(&merge.sha), &["f.txt".to_string()][..]));
        assert!(!git.in_merge(&repo).await);
        assert_eq!(git.rev_parse(&repo, "HEAD^1").await.unwrap(), Some(before.clone()));
        assert_eq!(git.rev_parse(&repo, "HEAD^2").await.unwrap(), git.rev_parse(&repo, "main").await.unwrap());
        assert_eq!(git.untracked(&repo).await.unwrap(), ["junit.xml"]);
        assert!(!git.ls_files(&repo).await.unwrap().contains(&"junit.xml".to_string()));
        assert_eq!(git.resolution_diff(&repo, &before, "plan").await.unwrap().trim(), "");
    }
}
