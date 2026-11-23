//! A new worktree, ready to build and test (docs/research/CRITIQUE.md G9). git checks out
//! tracked files only, so Codly does three things once for each worktree it creates (a task's,
//! a plan's integration, a team member's, a 1:1 agent's):
//! - copy: the checkout's git-ignored files and folders that the built-in list or the repo's
//!   `.worktreeinclude` names (gitignore syntax). A path is copied only when git ignores it
//!   in your checkout and on the worktree's branch too (an ignore rule you did not commit is
//!   not there), so `git add -A` in the new worktree does not take it into a Codly commit.
//!   The worktree can later be on a branch that does not ignore it (a team member's goes from
//!   task to task, an agent edits `.gitignore`): each copy is on the worktree's record
//!   (`Git::note_copied`), and Codly's commit leaves out what is on it and says so;
//! - ports: a block of `PORT_BLOCK` ports of its own, in `CODLY_PORT_BASE`, for the setup
//!   script, the verify commands and the agent;
//! - setup: the repo's `.codly/setup.sh`, read from your checkout. A script from a repo is
//!   code, so it runs only for the exact text you turned on in Settings, which shows the text
//!   and what the approval rules see in it. It runs like a verify command (the verify
//!   environment, a time limit, its last lines kept in a Decision), from a private copy of
//!   that text: its `#!` line names the program, `/bin/sh` without one.
//!
//! A worktree stays marked until this is done: a setup script that failed runs again at the
//! next start (a task you retry, a plan you run again).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::{Core, Inner, approvals, gate, lock, prompts};
use crate::model::*;

/// Names more git-ignored paths to copy into a new worktree (gitignore syntax, repo root).
pub const INCLUDE_FILE: &str = ".worktreeinclude";
/// The repo's setup script, relative to the repo root.
pub const SETUP_FILE: &str = ".codly/setup.sh";
/// `.codly/.gitignore`: git sees nothing of what Codly keeps in `.codly/` (pasted files),
/// except the setup script and the required checks (`required_checks.rs`), which you commit
/// so that they can be reviewed.
pub(super) const CODLY_IGNORE: &str = "*\n!/setup.sh\n!/checks\n";
/// `.codly/.gitignore` as earlier versions of Codly wrote it.
const OLD_CODLY_IGNORES: [&str; 2] = ["*\n", "*\n!/setup.sh\n"];
/// Copied when present and git-ignored, with or without an include file: installed
/// dependencies and local env files at the repo root. A later line wins, so `!/node_modules/`
/// in `.worktreeinclude` drops an entry.
const DEFAULT_INCLUDE: &[&str] = &["/node_modules/", "/.venv/", "/venv/", "/.env", "/.env.local", "/.env.development.local", "/.env.test.local"];

/// One copied file or folder.
const COPY_TIMEOUT: Duration = Duration::from_secs(120);
/// ponytail: a fixed limit for the setup script; a Setting when a repo needs longer.
const SETUP_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_SCRIPT_BYTES: u64 = 64 * 1024;

/// `cp` flags. macOS: `-c` clones on APFS (copy-on-write, near-free) and copies on other
/// volumes; other systems have no `-c`.
#[cfg(target_os = "macos")]
const CP_FLAGS: &str = "-cR";
#[cfg(not(target_os = "macos"))]
const CP_FLAGS: &str = "-R";

/// Port blocks: `PORT_FIRST`, `PORT_FIRST + PORT_BLOCK`, … (below the system's own range).
const PORT_FIRST: u16 = 20_000;
const PORT_BLOCK: u16 = 10;
const PORT_BLOCKS: u16 = 2_000;
pub(super) const PORT_VAR: &str = "CODLY_PORT_BASE";
const PORT_PREFIX: &str = "worktree_ports:";
const NEW_PREFIX: &str = "worktree_new:";

/// One preparation per worktree at a time: a second start waits, then finds it done. One
/// small entry per worktree prepared since the app started.
static PREPARING: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Two worktrees prepared together must not take the same port block.
static PORTS: Mutex<()> = Mutex::new(());

/// The open repo's worktree setup, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorktreeSetup {
    /// The text of `.codly/setup.sh` in your checkout. None = the repo has no setup script.
    pub script: Option<String>,
    /// The fingerprint of that text: send it back to turn the script on.
    pub sha: Option<String>,
    /// What the approval rules see in the script ("network", "deletes files"…), as they do
    /// for a verify command.
    pub risks: Vec<String>,
    /// You turned the script on, for this exact text.
    pub on: bool,
    /// You turned on another text: the script changed since (or Codly cannot read it any
    /// more, see `problem`). It does not run, and a new worktree's task waits, until you turn
    /// it on again or off.
    pub changed: bool,
    /// Why Codly does not run the file that is there (a link, not a file, too long, not
    /// text). `script` and `sha` are None then.
    pub problem: Option<String>,
    /// The lines of the repo's `.worktreeinclude` (no comments, no blank lines).
    pub include: Vec<String>,
}

/// Turn the repo's setup script on or off.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetupChoice {
    /// `WorktreeSetup::sha` of the text you read; None = off.
    pub allow: Option<String>,
}

fn setup_key(ws: &Workspace) -> String {
    format!("worktree_setup:{}", ws.id)
}

fn new_key(wt: &Path) -> String {
    format!("{NEW_PREFIX}{}", wt.display())
}

fn port_key(wt: &Path) -> String {
    format!("{PORT_PREFIX}{}", wt.display())
}

pub(super) fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The repo's setup script. None = no script (or an empty one); Err = a file Codly does not
/// run (a link, not a file, too long, not text).
fn read_script(repo: &Path) -> Result<Option<String>> {
    let path = repo.join(SETUP_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => bail!("cannot read {SETUP_FILE}: {e}"),
    };
    if !meta.is_file() {
        bail!("{SETUP_FILE} is not a regular file");
    }
    if meta.len() > MAX_SCRIPT_BYTES {
        bail!("{SETUP_FILE} is over {} KB", MAX_SCRIPT_BYTES >> 10);
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {SETUP_FILE}"))?;
    Ok(Some(text).filter(|t| !t.trim().is_empty()))
}

/// What a repo asks to copy into a new worktree.
struct Include {
    /// The built-in list, then the lines of the repo's `.worktreeinclude`.
    rules: Gitignore,
    /// The paths those lines start with (`line_path`).
    paths: Vec<String>,
    /// What could not be read: a line that is not a pattern, a file that does not open.
    problems: Vec<String>,
}

/// The path an include line starts with: its text before the first wildcard, when that has a
/// folder in it (`secrets/dev.env`, `config/` of `config/*.json`). None for a comment, a `!`
/// line and a bare name (`.dev.vars`, `*.local`), which matches at any depth.
fn line_path(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.starts_with(['#', '!']) {
        return None;
    }
    let line = line.strip_prefix('/').unwrap_or(line);
    let fixed = &line[..line.find(['*', '?', '[', '\\']).unwrap_or(line.len())];
    fixed.contains('/').then_some(fixed)
}

fn include_rules(repo: &Path) -> Include {
    let mut problems = vec![];
    let file = match std::fs::read_to_string(repo.join(INCLUDE_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            problems.push(format!("{INCLUDE_FILE}: {e}"));
            String::new()
        }
    };
    let mut rules = GitignoreBuilder::new(repo);
    for line in DEFAULT_INCLUDE.iter().copied().chain(file.lines()) {
        if let Err(e) = rules.add_line(None, line) {
            problems.push(format!("{INCLUDE_FILE}: {e}"));
        }
    }
    let rules = rules.build().unwrap_or_else(|e| {
        problems.push(format!("{INCLUDE_FILE}: {e}"));
        Gitignore::empty()
    });
    Include { rules, paths: file.lines().filter_map(line_path).map(String::from).collect(), problems }
}

/// The paths to copy: those of `ignored` (git's own list of ignored, untracked paths; a
/// folder ignored as a whole is one entry ending in `/`) that the rules name, by themselves
/// or by a parent folder. Such a folder is copied whole or not at all.
fn included(rules: &Gitignore, ignored: &[String]) -> Vec<String> {
    ignored
        .iter()
        .filter_map(|path| {
            let rel = path.trim_end_matches('/');
            (!rel.is_empty() && rules.matched_path_or_any_parents(rel, path.ends_with('/')).is_ignore()).then(|| rel.to_string())
        })
        .collect()
}

/// The folders of `ignored` to look inside: git ignores each as a whole and did not list
/// its files, no rule names the folder itself, and an include line names a path in it
/// (`secrets/dev.env` when `.gitignore` has `/secrets/`).
fn reached<'a>(include: &Include, ignored: &'a [String]) -> Vec<&'a str> {
    ignored
        .iter()
        .map(String::as_str)
        .filter(|dir| dir.ends_with('/') && include.paths.iter().any(|path| path.starts_with(*dir)))
        .filter(|dir| !include.rules.matched_path_or_any_parents(dir.trim_end_matches('/'), true).is_ignore())
        .collect()
}

/// What to copy for `file`, a file in a folder git ignores as a whole: the highest folder
/// on the way to it that the rules name, or the file itself. None = no rule names it.
fn named_part<'a>(rules: &Gitignore, file: &'a str) -> Option<&'a str> {
    let mut folders = file.match_indices('/').map(|(end, _)| &file[..end]);
    folders.find(|dir| rules.matched(*dir, true).is_ignore()).or_else(|| rules.matched(file, false).is_ignore().then_some(file))
}

/// The words that start a script: the program and the arguments of its `#!` line, `/bin/sh`
/// when it has none.
fn interpreter(text: &str) -> Vec<&str> {
    let named: Vec<&str> = text.lines().next().and_then(|first| first.strip_prefix("#!")).map(|rest| rest.split_whitespace().collect()).unwrap_or_default();
    if named.is_empty() { vec!["/bin/sh"] } else { named }
}

/// `.codly/.gitignore` as Codly wrote it before there was a setup script (or a checks file)
/// hides that file from git too (`git add` refuses it). Rewrite that old file, so the script
/// and the checks can be committed and reviewed.
pub(super) fn show_setup_file_to_git(repo: &Path) {
    let path = repo.join(".codly/.gitignore");
    if std::fs::read_to_string(&path).is_ok_and(|old| OLD_CODLY_IGNORES.contains(&old.as_str()))
        && let Err(e) = std::fs::write(&path, CODLY_IGNORE)
    {
        tracing::warn!("could not rewrite {}: {e}", path.display());
    }
}

/// Copy one file or folder; Err says why not, in words for the user.
async fn copy_path(src: &Path, dst: &Path) -> Result<()> {
    let cp = tokio::process::Command::new("/bin/cp").arg(CP_FLAGS).arg(src).arg(dst).stdin(Stdio::null()).kill_on_drop(true).output();
    match tokio::time::timeout(COPY_TIMEOUT, cp).await {
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => bail!("{}", prompts::clip(String::from_utf8_lossy(&out.stderr).trim(), 300)),
        Ok(Err(e)) => bail!("could not start cp: {e}"),
        Err(_) => bail!("the copy took more than {} s", COPY_TIMEOUT.as_secs()),
    }
}

/// How a setup script that did not succeed ended.
fn ending(out: &gate::VerifyOut) -> String {
    match out.code {
        _ if out.timed_out => format!("ran out of time ({} min)", SETUP_TIMEOUT.as_secs() / 60),
        Some(code) => format!("exited with code {code}"),
        None => "did not run to its end".to_string(),
    }
}

impl Inner {
    // ───────────── the setup script's switch ─────────────

    fn setup_status(&self, ws: &Workspace) -> Result<WorktreeSetup> {
        let repo = Path::new(&ws.repo_path);
        // A file Codly does not run is said in `problem`: the rest of the section, and the
        // way to turn the script off, stay.
        let (script, problem) = match read_script(repo) {
            Ok(script) => (script, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let sha = script.as_deref().map(fingerprint);
        let allowed = self.store.kv_get(&setup_key(ws))?;
        // The same rules as for a verify command (gate.rs), on the script's text.
        let risks = script
            .as_deref()
            .map(|text| approvals::detect_risks(&ApprovalSubject::Command { command: text.to_string(), cwd: None }, Some(repo)))
            .unwrap_or_default();
        let include = std::fs::read_to_string(repo.join(INCLUDE_FILE))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        let on = sha.is_some() && sha == allowed;
        let changed = allowed.is_some() && (problem.is_some() || (sha.is_some() && sha != allowed));
        Ok(WorktreeSetup { on, changed, problem, script, sha, risks, include })
    }

    fn choose_setup(&self, ws: &Workspace, choice: SetupChoice) -> Result<()> {
        let key = setup_key(ws);
        let Some(sha) = choice.allow else {
            if self.store.kv_get(&key)?.is_some() {
                self.store.kv_write(&[(key, None)])?;
                self.decide(Some(&ws.id), None, "approval_policy", format!("you turned off {SETUP_FILE} for new worktrees"), json!({}))?;
            }
            return Ok(());
        };
        let status = self.setup_status(ws)?;
        if status.script.is_none() {
            bail!("{}", status.problem.unwrap_or_else(|| format!("this repo has no {SETUP_FILE}")));
        }
        if status.sha.as_deref() != Some(sha.as_str()) {
            bail!("{SETUP_FILE} changed since you read it. Read it again, then turn it on");
        }
        self.store.kv_set(&key, &sha)?;
        let summary = format!("you turned on {SETUP_FILE}: Codly runs it in each new worktree");
        self.decide(Some(&ws.id), None, "approval_policy", summary, json!({ "sha": sha, "risks": status.risks }))?;
        Ok(())
    }

    // ───────────── a new worktree ─────────────

    /// Codly created `wt` just now: `prepare_worktree` makes it ready at its first use. Call
    /// this where the worktree is created (under the repo lock), so that a second start that
    /// finds the folder there waits for the first one's preparation.
    pub(super) fn mark_new_worktree(&self, wt: &Path) -> Result<()> {
        self.store.kv_set(&new_key(wt), "1")
    }

    /// Make `wt` ready, once: copy, ports, setup script. Does something only for a worktree
    /// still marked new. Before the setup script starts, a line says so: in `thread` when
    /// given (a plan's, a 1:1 agent's), else as a Decision on the task. Err = the repo's
    /// setup script did not succeed, in words for the user; the worktree stays marked, so the
    /// next start tries again.
    pub(super) async fn prepare_worktree(&self, ws: &Workspace, wt: &Path, task_id: Option<&str>, thread: Option<&str>) -> Result<()> {
        let one = lock(&PREPARING).entry(wt.to_path_buf()).or_default().clone();
        let _one = one.lock().await;
        let key = new_key(wt);
        if self.store.kv_get(&key)?.is_none() {
            return Ok(());
        }
        self.seed_worktree(ws, wt, task_id).await;
        self.assign_ports(wt)?;
        self.run_setup(ws, wt, task_id, thread).await?;
        self.store.kv_write(&[(key, None)])
    }

    /// The same for a 1:1 agent's own worktree. A setup script that fails is said once in the
    /// agent's conversation and the agent starts anyway: it can help you fix the script.
    pub(super) async fn prepare_solo_worktree(&self, ws: &Workspace, agent: &Agent, wt: &Path) {
        let thread = format!("agent:{}", agent.id);
        let Err(e) = self.prepare_worktree(ws, wt, None, Some(&thread)).await else { return };
        let text = format!("{}'s new worktree is not set up: {e} Codly does not run the script again for this agent.", agent.name);
        if let Err(e) = self.chat(Some(&ws.id), &thread, "codly", &text, None) {
            tracing::warn!("{text} ({e})");
        }
        if let Err(e) = self.store.kv_write(&[(new_key(wt), None)]) {
            tracing::warn!("could not clear the new-worktree mark of {}: {e}", wt.display());
        }
    }

    /// Environment a process gets because it runs in `wt`: the worktree's port block.
    pub(super) fn worktree_env(&self, wt: &Path) -> Vec<(String, String)> {
        let stored = self.store.kv_get(&port_key(wt)).unwrap_or_else(|e| {
            tracing::warn!("no {PORT_VAR} for {}: {e}", wt.display());
            None
        });
        let block = stored.and_then(|v| v.parse::<u16>().ok()).filter(|b| *b < PORT_BLOCKS);
        block.map(|b| (PORT_VAR.to_string(), (PORT_FIRST + b * PORT_BLOCK).to_string())).into_iter().collect()
    }

    /// Copy what the include rules name from the checkout, and record what was copied and
    /// what could not be. Never overwrites, never follows a link.
    async fn seed_worktree(&self, ws: &Workspace, wt: &Path, task_id: Option<&str>) {
        let repo = Path::new(&ws.repo_path);
        let include = include_rules(repo);
        let mut copied = vec![];
        let listed = self.copy_list(repo, &include).await;
        let mut problems = include.problems;
        match listed {
            Ok(paths) => {
                for rel in paths {
                    match self.seed_path(repo, wt, &rel).await {
                        Ok(true) => copied.push(rel),
                        Ok(false) => {}
                        Err(e) => problems.push(format!("{rel}: {e}")),
                    }
                }
            }
            Err(e) => problems.push(format!("could not list the ignored files: {e}")),
        }
        let worktree = wt.to_string_lossy();
        if !copied.is_empty() {
            let summary = format!("copied {} from your checkout into the new worktree (git-ignored files only)", copied.join(", "));
            let _ = self.decide(Some(&ws.id), task_id, "context", summary, json!({ "worktree": worktree, "copied": copied }));
        }
        if !problems.is_empty() {
            let summary = format!("could not copy into the new worktree: {}", problems.join("; "));
            let _ = self.decide(Some(&ws.id), task_id, "context", summary, json!({ "worktree": worktree, "problems": problems }));
        }
    }

    /// The checkout's paths to copy: what git ignores there and the include rules name.
    async fn copy_list(&self, repo: &Path, include: &Include) -> Result<Vec<String>> {
        let ignored = self.git.ignored(repo).await?;
        let mut paths = included(&include.rules, &ignored);
        let inside = reached(include, &ignored);
        if !inside.is_empty() {
            let files = self.git.ignored_files(repo, &inside).await?;
            let mut seen = HashSet::new();
            paths.extend(files.iter().filter_map(|file| named_part(&include.rules, file)).filter(|part| seen.insert(*part)).map(String::from));
        }
        Ok(paths)
    }

    /// Copy `rel` from the checkout into the worktree; false = nothing to do (a link, or the
    /// worktree has it). Err = not copied, and why: also a path the worktree's branch does
    /// not ignore, which the next `git add -A` there would take into a commit. The copy is
    /// made beside the worktrees and moved in when complete, so a copy that fails or runs out
    /// of time leaves nothing half-made in the worktree. It is on the worktree's record
    /// before it is there: no copy without a record. A copy that was not made goes off the
    /// record again: a file the agent later creates under that name is the agent's work.
    async fn seed_path(&self, repo: &Path, wt: &Path, rel: &str) -> Result<bool> {
        let (src, dst) = (repo.join(rel), wt.join(rel));
        let Ok(meta) = std::fs::symlink_metadata(&src) else { return Ok(false) };
        if meta.file_type().is_symlink() || std::fs::symlink_metadata(&dst).is_ok() {
            return Ok(false);
        }
        if !self.git.is_ignored(wt, rel, meta.is_dir()).await? {
            bail!("git does not ignore it on the worktree's branch, so a commit there would take it (commit the ignore rule first)");
        }
        self.git.note_copied(wt, rel).await?;
        let stage = self.data("worktrees").join(".copying").join(new_id());
        tokio::fs::create_dir_all(&stage).await.context("create a folder for the copy")?;
        let moved = async {
            let staged = stage.join("copy");
            copy_path(&src, &staged).await?;
            if let Some(dir) = dst.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            tokio::fs::rename(&staged, &dst).await.context("move the copy into the worktree")
        }
        .await;
        if let Err(e) = tokio::fs::remove_dir_all(&stage).await {
            tracing::warn!("could not remove {}: {e}", stage.display());
        }
        let Err(not_copied) = moved else { return Ok(true) };
        if let Err(e) = self.git.forget_copied(wt, rel).await {
            bail!("{not_copied:#}. It stays on the worktree's record of copies, so Codly would leave a file of that name out of its commits: {e:#}");
        }
        Err(not_copied)
    }

    /// Give `wt` the lowest port block no live worktree holds. A worktree that is gone
    /// gives its block back. No block left: the worktree gets no `CODLY_PORT_BASE`.
    pub(super) fn assign_ports(&self, wt: &Path) -> Result<()> {
        let _one = lock(&PORTS);
        let key = port_key(wt);
        let mut taken = HashSet::new();
        let mut writes = vec![];
        for (k, v) in self.store.kv_prefixed(PORT_PREFIX)? {
            if k == key {
                return Ok(());
            }
            if !Path::new(&k[PORT_PREFIX.len()..]).exists() {
                writes.push((k, None));
            } else if let Ok(block) = v.parse::<u16>() {
                taken.insert(block);
            }
        }
        if let Some(block) = (0..PORT_BLOCKS).find(|b| !taken.contains(b)) {
            writes.push((key, Some(block.to_string())));
        }
        self.store.kv_write(&writes)
    }

    /// Run the repo's setup script in `wt`, if you turned it on for this exact text.
    async fn run_setup(&self, ws: &Workspace, wt: &Path, task_id: Option<&str>, thread: Option<&str>) -> Result<()> {
        let Some(allowed) = self.store.kv_get(&setup_key(ws))? else { return Ok(()) };
        const ADVICE: &str = "Settings → Safety → New worktrees shows the script";
        let text = match read_script(Path::new(&ws.repo_path)) {
            Ok(Some(text)) => text,
            Ok(None) => return Ok(()),
            Err(e) => bail!("{e:#}, so Codly did not run it. Fix the file or turn the script off: {ADVICE}."),
        };
        if fingerprint(&text) != allowed {
            bail!("{SETUP_FILE} changed since you turned it on, so Codly did not run it. Read it and turn it on again, or turn it off: {ADVICE}.");
        }
        // Said before the wait: the script can take minutes, and nothing else shows it.
        let limit = SETUP_TIMEOUT.as_secs() / 60;
        let worktree = wt.to_string_lossy();
        let said = match thread {
            Some(thread) => self.chat(Some(&ws.id), thread, "codly", &format!("Setting up the new worktree: running {SETUP_FILE} (up to {limit} min)."), None).map(drop),
            None => self.decide(Some(&ws.id), task_id, "context", format!("running {SETUP_FILE} in the new worktree (up to {limit} min)"), json!({ "worktree": worktree })).map(drop),
        };
        if let Err(e) = said {
            tracing::warn!("could not say that {SETUP_FILE} runs in {worktree}: {e}");
        }
        let mut env = gate::verify_env(&self.base_env);
        env.extend(self.worktree_env(wt));
        env.push(("CODLY_REPO_PATH".to_string(), ws.repo_path.clone()));
        let out = self.run_script(wt, &text, env).await?;
        let summary = if out.ok { format!("ran {SETUP_FILE} in the new worktree") } else { format!("{SETUP_FILE} {} in the new worktree", ending(&out)) };
        let detail = json!({ "worktree": worktree, "ok": out.ok, "exit_code": out.code, "timed_out": out.timed_out, "tail": out.tail });
        let _ = self.decide(Some(&ws.id), task_id, "context", summary, detail);
        if out.ok {
            return Ok(());
        }
        let last = out.tail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output");
        bail!("{SETUP_FILE} {}: {}. Fix the script or turn it off: {ADVICE}.", ending(&out), prompts::clip(last.trim(), 300))
    }

    /// Run `text` as a script in `wt`, like a verify command. The script is a private copy
    /// of the text outside the worktree (the file in your checkout may change meanwhile, and
    /// a copy in the worktree could be committed), started by the program its `#!` line
    /// names, so `$0` is that copy and the folder it runs in is the worktree's top folder.
    async fn run_script(&self, wt: &Path, text: &str, env: Vec<(String, String)>) -> Result<gate::VerifyOut> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = self.data("worktrees").join(".setup").join(new_id());
        std::fs::create_dir_all(&dir).context("create a folder for the setup script")?;
        let file = dir.join("setup.sh");
        let written = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file).and_then(|mut f| f.write_all(text.as_bytes()));
        let out = match written {
            Ok(()) => {
                let words = interpreter(text).into_iter().map(crate::env::shell_quote).chain([crate::env::shell_quote(&file.to_string_lossy())]);
                let command = format!("exec {}", words.collect::<Vec<_>>().join(" "));
                Ok(gate::verify_in(wt, &command, env, SETUP_TIMEOUT).await)
            }
            Err(e) => Err(anyhow!("could not write the copy of {SETUP_FILE} that Codly runs: {e}")),
        };
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!("could not remove {}: {e}", dir.display());
        }
        out
    }
}

impl Core {
    /// The open repo's worktree setup: its `.worktreeinclude` lines and its setup script
    /// (text, risks, on or off). `set` turns the script on, for the text you read, or off.
    pub async fn worktree_setup(&self, set: Option<SetupChoice>) -> Result<WorktreeSetup> {
        let ws = self.inner.require_ws()?;
        if let Some(choice) = set {
            self.inner.choose_setup(&ws, choice)?;
        }
        self.inner.setup_status(&ws)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{CoreConfig, StartOptions};

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A repo with one commit; `ignore` is its `.gitignore`.
    fn temp_repo(root: &Path, ignore: &str) -> PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
        write(&repo.join(".gitignore"), ignore);
        write(&repo.join("config/app.json"), "{}\n");
        write(&repo.join("packages/a/package.json"), "{}\n");
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        repo
    }

    async fn start(root: &Path, repo: &Path) -> (Arc<Core>, Workspace) {
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        (core, ws)
    }

    /// A new worktree of the repo, as Codly creates one for a task: marked new, not prepared.
    async fn new_worktree(core: &Core, ws: &Workspace, name: &str) -> PathBuf {
        let wt = core.inner.worktree_root(ws).join("test").join(name);
        core.inner.git.worktree_add(Path::new(&ws.repo_path), &wt, &format!("codly/test/{name}"), "HEAD").await.unwrap();
        core.inner.mark_new_worktree(&wt).unwrap();
        wt
    }

    /// The summaries of the Decisions on record, oldest first.
    fn decisions(core: &Core, ws: &Workspace) -> Vec<String> {
        let events = core.inner.store.latest_events(Some(&ws.id), 300).unwrap();
        events.into_iter().filter_map(|e| if let EventKind::Decision { summary, .. } = e.kind { Some(summary) } else { None }).collect()
    }

    #[test]
    fn the_include_file_adds_to_the_built_in_list_and_can_drop_from_it() {
        let dir = tempfile::tempdir().unwrap();
        let lines = "# local config\nconfig/local.json\nnode_modules/\n!/.venv/\n.dev.vars\n[z-a]\nsecrets/dev.env\n/vendor/keys/*.pem\nvendor/cache/\n";
        write(&dir.path().join(INCLUDE_FILE), lines);
        let include = include_rules(dir.path());
        assert_eq!(include.problems.len(), 1, "the line that is not a pattern is reported: {:?}", include.problems);
        assert!(include.problems[0].starts_with(INCLUDE_FILE), "{:?}", include.problems);
        let ignored = [".env", ".venv/", "build/", "config/local.json", "config/other.json", "node_modules/", "packages/a/node_modules/", "apps/web/.dev.vars", "apps/web/.env", "secrets/", "vendor/"];
        let ignored: Vec<String> = ignored.iter().map(|p| p.to_string()).collect();
        assert_eq!(included(&include.rules, &ignored), [".env", "config/local.json", "node_modules", "packages/a/node_modules", "apps/web/.dev.vars"]);

        // A line that names a path inside a folder git ignores as a whole: git lists that
        // folder as one entry, so Codly looks inside it. A folder a rule takes whole needs no
        // look, and a bare name (`.dev.vars`) starts none.
        assert_eq!(include.paths, ["config/local.json", "node_modules/", "secrets/dev.env", "vendor/keys/", "vendor/cache/"]);
        assert_eq!(reached(&include, &ignored), ["secrets/", "vendor/"]);
        let part = |file| named_part(&include.rules, file);
        assert_eq!(part("secrets/dev.env"), Some("secrets/dev.env"));
        assert_eq!(part("secrets/other.env"), None);
        assert_eq!(part("vendor/keys/a.pem"), Some("vendor/keys/a.pem"));
        assert_eq!(part("vendor/keys/a.txt"), None);
        assert_eq!(part("vendor/cache/x/y.bin"), Some("vendor/cache"), "a folder a line names is copied whole");
        assert_eq!(part("vendor/pkg/.dev.vars"), Some("vendor/pkg/.dev.vars"));

        // No include file: the built-in list alone, at the repo root only.
        let bare = tempfile::tempdir().unwrap();
        let include = include_rules(bare.path());
        assert!(include.problems.is_empty(), "{:?}", include.problems);
        assert_eq!(included(&include.rules, &ignored), [".env", ".venv", "node_modules"]);
        assert!(reached(&include, &ignored).is_empty());
    }

    #[test]
    fn a_script_is_started_by_the_program_its_first_line_names() {
        assert_eq!(interpreter("#!/usr/bin/env bash\nset -e\n"), ["/usr/bin/env", "bash"]);
        assert_eq!(interpreter("#! /bin/zsh -e\r\necho\r\n"), ["/bin/zsh", "-e"]);
        assert_eq!(interpreter("pnpm install\n#!/bin/zsh\n"), ["/bin/sh"]);
        assert_eq!(interpreter("#!\npnpm install\n"), ["/bin/sh"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_new_worktree_gets_the_ignored_files_the_rules_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "node_modules/\n.env*\nconfig/local.json\ntarget/\n/secrets/\n");
        write(&repo.join("node_modules/dep/index.js"), "module.exports = 1;\n");
        write(&repo.join("secrets/dev.env"), "TOKEN=dev\n");
        write(&repo.join("secrets/prod.env"), "TOKEN=prod\n");
        write(&repo.join("packages/a/node_modules/x.js"), "x\n");
        write(&repo.join(".env"), "API_URL=http://localhost\n");
        write(&repo.join("config/local.json"), "{\"mine\":true}\n");
        write(&repo.join("target/big.bin"), "not named by a rule\n");
        write(&repo.join("notes.txt"), "untracked, not ignored\n");
        write(&dir.path().join("elsewhere.env"), "SECRET=1\n");
        std::os::unix::fs::symlink(dir.path().join("elsewhere.env"), repo.join(".env.local")).unwrap();
        write(&repo.join(INCLUDE_FILE), "config/local.json\nnode_modules/\nsecrets/dev.env\n");
        let (core, ws) = start(dir.path(), &repo).await;
        let wt = new_worktree(&core, &ws, "t1").await;

        core.inner.prepare_worktree(&ws, &wt, None, None).await.unwrap();
        assert_eq!(std::fs::read_to_string(wt.join("secrets/dev.env")).unwrap(), "TOKEN=dev\n", "a file inside a folder git ignores as a whole");
        assert!(!wt.join("secrets/prod.env").exists(), "the rest of that folder is not named");
        assert_eq!(std::fs::read_to_string(wt.join("node_modules/dep/index.js")).unwrap(), "module.exports = 1;\n");
        assert!(wt.join("packages/a/node_modules/x.js").exists(), "a nested folder the include file names");
        assert!(wt.join(".env").exists());
        assert_eq!(std::fs::read_to_string(wt.join("config/local.json")).unwrap(), "{\"mine\":true}\n");
        assert_eq!(std::fs::read_to_string(wt.join("config/app.json")).unwrap(), "{}\n", "tracked files stay as git checked them out");
        assert!(!wt.join("target").exists(), "ignored, but no rule names it");
        assert!(!wt.join("notes.txt").exists(), "only git-ignored files are copied");
        assert!(std::fs::symlink_metadata(wt.join(".env.local")).is_err(), "a link is not copied");
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "", "nothing copied can end up in a commit");
        let staging = core.inner.data("worktrees").join(".copying");
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0, "no copy is left half-made");
        let said = decisions(&core, &ws);
        assert!(said.iter().any(|s| s.starts_with("copied ") && s.contains("node_modules") && s.contains("config/local.json") && s.contains("secrets/dev.env")), "{said:?}");
        assert!(!said.iter().any(|s| s.starts_with("could not copy")), "{said:?}");

        // Once: a file the agent removed does not come back at the next start.
        std::fs::remove_file(wt.join(".env")).unwrap();
        core.inner.prepare_worktree(&ws, &wt, None, None).await.unwrap();
        assert!(!wt.join(".env").exists());
        core.shutdown().await;
    }

    /// The worktree is cut from the last commit, so an ignore rule you did not commit is not
    /// in it: `git add -A` there would take the copied file into the task's commit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_the_worktrees_branch_does_not_ignore_is_not_copied() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "node_modules/\n");
        // Not committed: the new rule, and a `.gitignore` of its own in a new folder.
        write(&repo.join(".gitignore"), "node_modules/\nconfig/local.json\n");
        write(&repo.join("config/local.json"), "{\"token\":\"mine\"}\n");
        write(&repo.join("keys/.gitignore"), "*\n");
        write(&repo.join("keys/dev.pem"), "private\n");
        write(&repo.join("node_modules/dep/index.js"), "module.exports = 1;\n");
        write(&repo.join(INCLUDE_FILE), "config/local.json\nkeys/dev.pem\n");
        let (core, ws) = start(dir.path(), &repo).await;
        let wt = new_worktree(&core, &ws, "t1").await;

        core.inner.prepare_worktree(&ws, &wt, None, None).await.unwrap();
        assert!(wt.join("node_modules/dep/index.js").exists(), "the committed rule covers it");
        assert!(!wt.join("config/local.json").exists(), "the worktree's .gitignore does not have the new line");
        assert!(!wt.join("keys").exists(), "the folder's own .gitignore is not in the worktree");
        // What a task's snapshot does.
        sh(&wt, &["add", "-A"]);
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "", "nothing copied can end up in a commit");
        let said = decisions(&core, &ws);
        let problem = said.iter().find(|s| s.starts_with("could not copy")).unwrap_or_else(|| panic!("{said:?}"));
        assert!(problem.contains("config/local.json: git does not ignore it on the worktree's branch"), "{problem}");
        assert!(problem.contains("keys/dev.pem: git does not ignore it on the worktree's branch"), "{problem}");
        core.shutdown().await;
    }

    /// A worktree outlives the branch it was made on, and a branch can lose an ignore rule.
    /// What Codly copied in then stays out of Codly's commit, and the record says so.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_copy_stays_out_of_the_commit_on_a_branch_that_does_not_ignore_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "node_modules/\n.env\n/secrets/\n");
        write(&repo.join("node_modules/dep/index.js"), "module.exports = 1;\n");
        write(&repo.join(".env"), "API_URL=http://localhost\n");
        write(&repo.join("secrets/dev.env"), "TOKEN=dev\n");
        write(&repo.join(INCLUDE_FILE), "secrets/dev.env\n");
        let (core, ws) = start(dir.path(), &repo).await;
        let wt = new_worktree(&core, &ws, "t1").await;
        core.inner.prepare_worktree(&ws, &wt, None, None).await.unwrap();
        assert!(wt.join(".env").exists() && wt.join("node_modules/dep/index.js").exists() && wt.join("secrets/dev.env").exists());
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "");

        // The worktree's next branch has older rules: git no longer hides the copies there.
        write(&wt.join(".gitignore"), "");
        write(&wt.join("work.txt"), "work\n");
        let commit = core.inner.commit_noted(Some(&ws.id), None, "t1", &wt, "codly(t1): work").await.unwrap().unwrap();
        assert_eq!(commit.files, [".gitignore", "work.txt"], "the work, and no copy");
        assert_eq!(sh(&wt, &["status", "--porcelain", "-uall"]), "?? .env\n?? node_modules/dep/index.js\n?? secrets/dev.env", "the copies stay, untracked");
        let said = decisions(&core, &ws);
        let left = "t1: Codly did not commit .env (copied in by Codly), node_modules/ (copied in by Codly), secrets/dev.env (copied in by Codly).";
        let record = said.iter().find(|s| s.starts_with(left)).unwrap_or_else(|| panic!("{said:?}"));
        assert!(record.contains("it belongs in .gitignore") && !record.contains("25 MB"), "{record}");
        core.shutdown().await;
    }

    /// A copy that was not made does not stay on the worktree's record of copies: a file the
    /// agent later creates under that name is its work, and Codly commits it like any other.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_copy_that_was_not_made_is_not_on_the_record_of_copies() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), ".env\n/secrets/\n");
        write(&repo.join(".env"), "API_URL=http://localhost\n");
        write(&repo.join("secrets/dev.env"), "TOKEN=dev\n");
        write(&repo.join(INCLUDE_FILE), "secrets/dev.env\n");
        let (core, ws) = start(dir.path(), &repo).await;
        let wt = new_worktree(&core, &ws, "t1").await;
        // In the way of one copy: a file where that copy needs a folder.
        write(&wt.join("secrets"), "not a folder\n");

        core.inner.prepare_worktree(&ws, &wt, None, None).await.unwrap();
        assert!(wt.join(".env").exists(), "the other copy is made");
        assert_eq!(std::fs::read_to_string(wt.join("secrets")).unwrap(), "not a folder\n", "nothing was written over");
        let said = decisions(&core, &ws);
        let problem = said.iter().find(|s| s.starts_with("could not copy")).unwrap_or_else(|| panic!("{said:?}"));
        assert!(problem.contains("secrets/dev.env: "), "{problem}");
        assert!(!problem.contains("git does not ignore it") && !problem.contains("record of copies"), "the copy itself failed, after it was put on record: {problem}");

        // The agent makes room and writes a file of that name itself. On a branch that does
        // not ignore it, that file is work; the copy that was made still stays out.
        std::fs::remove_file(wt.join("secrets")).unwrap();
        write(&wt.join("secrets/dev.env"), "TOKEN=from-the-agent\n");
        write(&wt.join(".gitignore"), "");
        let commit = core.inner.commit_noted(Some(&ws.id), None, "t1", &wt, "codly(t1): work").await.unwrap().unwrap();
        assert_eq!(commit.files, [".gitignore", "secrets/dev.env"]);
        assert_eq!(sh(&wt, &["status", "--porcelain", "-uall"]), "?? .env");
        let said = decisions(&core, &ws);
        let left = said.iter().find(|s| s.starts_with("t1: Codly did not commit")).unwrap_or_else(|| panic!("{said:?}"));
        assert!(left.starts_with("t1: Codly did not commit .env (copied in by Codly).") && !left.contains("secrets"), "{left}");
        core.shutdown().await;
    }

    /// `.codly/.gitignore` as an older Codly wrote it hid the setup script from git.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn opening_a_repo_lets_git_see_the_setup_script() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "");
        write(&repo.join(".codly/.gitignore"), "*\n");
        write(&repo.join(".codly/attachments/a1/shot.png"), "png\n");
        write(&repo.join(SETUP_FILE), "pnpm install\n");
        assert_eq!(sh(&repo, &["status", "--porcelain"]), "", "the old file hides the script");
        let (core, _ws) = start(dir.path(), &repo).await;
        assert_eq!(std::fs::read_to_string(repo.join(".codly/.gitignore")).unwrap(), CODLY_IGNORE);
        assert_eq!(sh(&repo, &["status", "--porcelain", "-uall"]), "?? .codly/setup.sh", "the script, and nothing else of .codly/");
        core.shutdown().await;

        // The file of a Codly that knew the setup script only hid the required checks.
        write(&repo.join(".codly/.gitignore"), "*\n!/setup.sh\n");
        write(&repo.join(".codly/checks"), "pnpm test\n");
        show_setup_file_to_git(&repo);
        assert_eq!(sh(&repo, &["status", "--porcelain", "-uall"]), "?? .codly/checks\n?? .codly/setup.sh");

        // A `.codly/.gitignore` you wrote yourself is left as it is.
        write(&repo.join(".codly/.gitignore"), "*\n!/notes.md\n");
        show_setup_file_to_git(&repo);
        assert_eq!(std::fs::read_to_string(repo.join(".codly/.gitignore")).unwrap(), "*\n!/notes.md\n");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_setup_script_runs_only_for_the_text_you_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "setup.out\n");
        write(&repo.join(SETUP_FILE), "printf '%s %s' \"$CODLY_PORT_BASE\" \"$CODLY_REPO_PATH\" > setup.out\n");
        let (core, ws) = start(dir.path(), &repo).await;

        // Off until you turn it on: the text and its risks are shown, nothing runs.
        let status = core.worktree_setup(None).await.unwrap();
        assert!(!status.on && !status.changed);
        assert!(status.script.as_deref().is_some_and(|s| s.starts_with("printf")));
        assert!(!status.risks.is_empty(), "the rules cannot check a `$` expansion: {:?}", status.risks);
        let t1 = new_worktree(&core, &ws, "t1").await;
        core.inner.prepare_worktree(&ws, &t1, None, None).await.unwrap();
        assert!(!t1.join("setup.out").exists());

        // On, for the text you read only.
        let stale = SetupChoice { allow: Some(fingerprint("another text")) };
        let err = core.worktree_setup(Some(stale)).await.unwrap_err().to_string();
        assert!(err.contains("changed since you read it"), "{err}");
        let on = core.worktree_setup(Some(SetupChoice { allow: status.sha.clone() })).await.unwrap();
        assert!(on.on && !on.changed);
        let t2 = new_worktree(&core, &ws, "t2").await;
        core.inner.prepare_worktree(&ws, &t2, None, None).await.unwrap();
        let base = core.inner.worktree_env(&t2);
        assert_eq!(base.len(), 1, "{base:?}");
        assert_eq!(std::fs::read_to_string(t2.join("setup.out")).unwrap(), format!("{} {}", base[0].1, ws.repo_path));
        let said = decisions(&core, &ws);
        assert!(said.iter().any(|s| s == "running .codly/setup.sh in the new worktree (up to 10 min)"), "said before the wait: {said:?}");
        assert!(said.iter().any(|s| s == "ran .codly/setup.sh in the new worktree"), "{said:?}");
        assert!(!t1.join("setup.out").exists(), "a worktree that was ready before stays as it is");
        core.inner.prepare_worktree(&ws, &t1, None, None).await.unwrap();
        assert!(!t1.join("setup.out").exists());

        // The script changed (a pull, a merged branch): it does not run until you read it again.
        write(&repo.join(SETUP_FILE), "echo broken >&2\nexit 3\n");
        let changed = core.worktree_setup(None).await.unwrap();
        assert!(changed.changed && !changed.on);
        let t3 = new_worktree(&core, &ws, "t3").await;
        let err = core.inner.prepare_worktree(&ws, &t3, None, None).await.unwrap_err().to_string();
        assert!(err.contains("changed since you turned it on"), "{err}");

        // A script that fails: the start says why, the output is kept, the next start tries again.
        core.worktree_setup(Some(SetupChoice { allow: changed.sha.clone() })).await.unwrap();
        let err = core.inner.prepare_worktree(&ws, &t3, None, None).await.unwrap_err().to_string();
        assert!(err.contains("exited with code 3: broken"), "{err}");
        let said = decisions(&core, &ws);
        assert!(said.iter().any(|s| s == ".codly/setup.sh exited with code 3 in the new worktree"), "{said:?}");
        // The program the first line names runs the script (the array is not /bin/sh syntax),
        // from a copy outside the worktree, in the worktree's top folder.
        write(&repo.join(SETUP_FILE), "#!/usr/bin/env bash\nwords=(fixed by bash)\necho \"${words[0]} ${#words[@]} $(basename \"$0\") $(dirname \"$0\")\" > setup.out\n");
        let fixed = core.worktree_setup(None).await.unwrap();
        core.worktree_setup(Some(SetupChoice { allow: fixed.sha.clone() })).await.unwrap();
        core.inner.prepare_worktree(&ws, &t3, None, None).await.unwrap();
        let out = std::fs::read_to_string(t3.join("setup.out")).unwrap();
        let copy = out.trim_end().strip_prefix("fixed 3 setup.sh ").unwrap_or_else(|| panic!("{out}"));
        assert!(!Path::new(copy).starts_with(&t3) && !Path::new(copy).exists(), "the copy is not in the worktree, and is gone: {copy}");

        // The file becomes one Codly does not run (a link here). Settings still loads and says
        // why, a new worktree's start says what to do, and the script can be turned off.
        std::fs::remove_file(repo.join(SETUP_FILE)).unwrap();
        write(&dir.path().join("elsewhere.sh"), "echo elsewhere > setup.out\n");
        std::os::unix::fs::symlink(dir.path().join("elsewhere.sh"), repo.join(SETUP_FILE)).unwrap();
        let odd = core.worktree_setup(None).await.unwrap();
        assert_eq!(odd.problem.as_deref(), Some(".codly/setup.sh is not a regular file"));
        assert_eq!((odd.script, odd.sha, odd.on, odd.changed), (None, None, false, true));
        let t4 = new_worktree(&core, &ws, "t4").await;
        let err = core.inner.prepare_worktree(&ws, &t4, None, None).await.unwrap_err().to_string();
        assert!(err.contains("is not a regular file, so Codly did not run it") && err.contains("turn the script off: Settings"), "{err}");
        let err = core.worktree_setup(Some(SetupChoice { allow: fixed.sha.clone() })).await.unwrap_err().to_string();
        assert!(err.contains("is not a regular file"), "{err}");

        // Off again: nothing runs, and a repo without a script has nothing to turn on.
        let off = core.worktree_setup(Some(SetupChoice { allow: None })).await.unwrap();
        assert!(!off.on && !off.changed && off.problem.is_some());
        std::fs::remove_file(repo.join(SETUP_FILE)).unwrap();
        write(&repo.join(SETUP_FILE), "echo fixed > setup.out\n");
        core.inner.prepare_worktree(&ws, &t4, None, None).await.unwrap();
        assert!(!t4.join("setup.out").exists());
        std::fs::remove_file(repo.join(SETUP_FILE)).unwrap();
        let none = core.worktree_setup(None).await.unwrap();
        assert_eq!((none.script, none.sha, none.on), (None, None, false));
        assert!(core.worktree_setup(Some(SetupChoice { allow: fixed.sha })).await.is_err());
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_setup_is_said_once_in_a_solo_agents_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "");
        write(&repo.join(SETUP_FILE), "echo no network >&2\nexit 1\n");
        let (core, ws) = start(dir.path(), &repo).await;
        let sha = core.worktree_setup(None).await.unwrap().sha;
        core.worktree_setup(Some(SetupChoice { allow: sha })).await.unwrap();
        let agent = core.inner.create_agent(&ws.id, "Ada", Provider::Claude, Role::Solo, 0, None).unwrap();
        // As a hire leaves it: created, prepared at the first message.
        let wt = new_worktree(&core, &ws, "ada").await;

        let said = |core: &Core| -> Vec<String> {
            let chats = core.inner.store.recent_chats(Some(&ws.id), 50).unwrap();
            chats.into_iter().filter(|c| c.thread == format!("agent:{}", agent.id) && c.author == "codly").map(|c| c.text).collect()
        };
        core.inner.prepare_solo_worktree(&ws, &agent, &wt).await;
        let first = said(&core);
        assert_eq!(first.len(), 2, "{first:?}");
        assert!(first.iter().any(|t| t == "Setting up the new worktree: running .codly/setup.sh (up to 10 min)."), "said before the wait: {first:?}");
        assert!(first.iter().any(|t| t.starts_with("Ada's new worktree is not set up: .codly/setup.sh exited with code 1: no network.")), "{first:?}");
        core.inner.prepare_solo_worktree(&ws, &agent, &wt).await;
        assert_eq!(said(&core).len(), 2, "the next message does not run the script again");
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn each_worktree_gets_its_own_ports_and_a_removed_one_gives_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "");
        let (core, ws) = start(dir.path(), &repo).await;
        let base = |wt: &Path| core.inner.worktree_env(wt).into_iter().find(|(k, _)| k == PORT_VAR).map(|(_, v)| v);
        let (a, b) = (new_worktree(&core, &ws, "a").await, new_worktree(&core, &ws, "b").await);
        assert_eq!(base(&a), None, "no block before the worktree is prepared");
        core.inner.prepare_worktree(&ws, &a, None, None).await.unwrap();
        core.inner.prepare_worktree(&ws, &b, None, None).await.unwrap();
        assert_eq!((base(&a).as_deref(), base(&b).as_deref()), (Some("20000"), Some("20010")));
        assert_eq!(base(Path::new(&ws.repo_path)), None, "your checkout has no block");

        std::fs::remove_dir_all(&a).unwrap();
        let c = new_worktree(&core, &ws, "c").await;
        core.inner.prepare_worktree(&ws, &c, None, None).await.unwrap();
        assert_eq!((base(&c).as_deref(), base(&b).as_deref()), (Some("20000"), Some("20010")));
        assert_eq!(base(&a), None);
        core.shutdown().await;
    }
}
