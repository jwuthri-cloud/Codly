//! The pull-request step: what comes after a plan is done, or after a solo agent's work.
//!
//! On the user's click, and after a confirm that names the remote, the branch and the base,
//! Codly pushes one of its own `codly/…` branches (a plan's integration branch, or a solo
//! agent's branch) and opens a pull request with the user's own `gh`. The rules kept here:
//! - only a `codly/…` branch is pushed, never the branch the user works on;
//! - the push is `Git::push`: a plain refspec, never forced;
//! - nothing is sent when the branch, the remote or the base is not what the user confirmed;
//! - `gh` runs with the user's login environment and its own sign-in: Codly stores no token;
//! - without a usable `gh` the branch can still be pushed, and the compare page is offered;
//! - the dialog says how many of the user's own unpushed commits the branch would carry;
//! - a plan whose request started from an issue ends its drafted description with
//!   "Closes #N" (`intake::link_line`); the user can edit it like the rest of the draft;
//! - a pull request's state and checks are read when the user asks (`gh pr view`, `gh pr
//!   checks`): there is no polling.
//!
//! Every `gh` argv comes from a builder (`auth_args`, `create_args`, `state_args`,
//! `checks_args`), so a test can check what would run without running `gh`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use ts_rs::TS;

use super::agent_git::{GitLogEntry, parse_log};
use super::{Core, Inner, intake, prompts, row};
use crate::model::*;
use crate::store;

/// Codly's own branches: the only ones this step pushes.
const OWN_PREFIX: &str = "codly/";
/// `gh auth status` asks the host whether the sign-in still works.
pub(super) const AUTH_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const GH_TIMEOUT: Duration = Duration::from_secs(60);
const TITLE_MAX: usize = 120;
const GOAL_MAX: usize = 4000;
/// GitHub refuses a longer description.
const BODY_MAX: usize = 65_536;
/// The scope of a plan's integration checks in the drafted description.
const ALL_TASKS: &str = "all tasks together";

/// Whose branch the pull request is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[ts(export)]
pub enum PrSource {
    /// A plan: its integration branch.
    Plan(Id),
    /// A solo agent: its own branch.
    Agent(Id),
}

impl PrSource {
    /// kv key of the pull request Codly opened for it.
    fn key(&self) -> String {
        match self {
            PrSource::Plan(id) => format!("pull_request:plan:{id}"),
            PrSource::Agent(id) => format!("pull_request:agent:{id}"),
        }
    }
}

/// What Create pull request would send, for the confirm. Nothing has left this machine yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrDraft {
    pub remote: String,
    /// The Codly branch that is pushed: the head of the pull request.
    pub branch: String,
    /// The branch the pull request goes into. The user can change it.
    pub base: String,
    pub title: String,
    pub body: String,
    /// Why `gh` cannot open the pull request (None: it can). The branch can still be pushed.
    pub gh_problem: Option<String>,
    /// Something to know about `base` before sending: it is not on the remote, or the push
    /// would also carry commits of the user's own branch that the remote does not have.
    pub warning: Option<String>,
    /// Where the pull request on record stands, read just now (None: there is none, or `gh`
    /// could not say). New commits join it only while it is open.
    pub pr_state: Option<PrState>,
}

/// What the user confirmed: nothing is sent when the repository no longer matches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrConfirm {
    pub remote: String,
    pub branch: String,
    pub base: String,
}

/// Where a pull request stands on its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PrState {
    Open,
    Merged,
    /// Closed without a merge.
    Closed,
}

/// A pull request Codly opened, as last read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequest {
    pub url: String,
    pub remote: String,
    pub branch: String,
    pub base: String,
    pub created_at: i64,
    /// As of the last read: when it was opened, on Refresh, and when the dialog opens.
    pub state: PrState,
    /// Its checks as of the last Refresh (None: not read yet).
    pub checks: Option<PrChecks>,
}

/// The checks of a pull request, counted from one `gh pr checks` call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrChecks {
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    pub skipped: u32,
    /// Names of the first failed checks.
    pub failing: Vec<String>,
    pub read_at: i64,
}

/// The branch is on the remote; no pull request was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrPushed {
    pub remote: String,
    pub branch: String,
    /// Where to open the pull request by hand (remotes on GitHub only).
    pub compare_url: Option<String>,
}

// ───────────── gh: what would run ─────────────

/// `host/owner/repository`, as `gh --repo` takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepoSlug {
    pub(super) host: String,
    pub(super) owner: String,
    pub(super) repo: String,
}

impl RepoSlug {
    pub(super) fn arg(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.repo)
    }
}

/// The repository a remote address names: `git@host:owner/repo.git`, `ssh://git@host/owner/repo`
/// or `https://host/owner/repo`. None for a local path, and for an address with other parts.
pub(super) fn parse_remote(url: &str) -> Option<RepoSlug> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((scheme, rest)) if matches!(scheme, "https" | "http" | "ssh" | "git" | "git+ssh") => rest.split_once('/')?,
        Some(_) => return None,
        None => url.split_once(':').filter(|(authority, _)| !authority.contains('/'))?,
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _port)| host);
    let path = path.trim_matches('/');
    let (owner, repo) = path.strip_suffix(".git").unwrap_or(path).split_once('/')?;
    let plain = |s: &str| !s.is_empty() && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    (plain(host) && plain(owner) && plain(repo)).then(|| RepoSlug { host: host.into(), owner: owner.into(), repo: repo.into() })
}

/// The page that opens a pull request by hand, for a repository on GitHub.
fn compare_url(slug: &RepoSlug, base: &str, branch: &str) -> Option<String> {
    let github = slug.host == "github.com" || slug.host.ends_with(".github.com") || slug.host.ends_with(".ghe.com");
    github.then(|| format!("https://{}/{}/{}/compare/{}...{}?expand=1", slug.host, slug.owner, slug.repo, url_path(base), url_path(branch)))
}

/// A branch name inside a URL path: everything but letters, digits and `-._~/` is escaped.
fn url_path(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

pub(super) fn auth_args(host: &str) -> Vec<String> {
    ["auth", "status", "--hostname", host].map(String::from).to_vec()
}

/// `gh pr create` for a branch that is already on the remote (`--head` stops gh from pushing
/// by itself). The description comes on stdin.
fn create_args(slug: &RepoSlug, base: &str, head: &str, title: &str) -> Vec<String> {
    let repo = slug.arg();
    ["pr", "create", "--repo", &repo, "--base", base, "--head", head, "--title", title, "--body-file", "-"].map(String::from).to_vec()
}

fn state_args(url: &str) -> Vec<String> {
    ["pr", "view", url, "--json", "state"].map(String::from).to_vec()
}

pub(super) fn checks_args(url: &str) -> Vec<String> {
    ["pr", "checks", url, "--json", "name,bucket"].map(String::from).to_vec()
}

/// The pull request's address in what `gh pr create` printed: when it opened one, and when
/// it says that one is open for the branch already.
fn pr_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .rev()
        .find(|w| w.starts_with("https://") && w.contains("/pull/") && w.chars().all(|c| c.is_ascii_graphic()))
        .map(String::from)
}

/// The state in what `gh pr view --json state` printed.
fn parse_state(stdout: &str) -> Result<PrState> {
    #[derive(Deserialize)]
    struct View {
        state: String,
    }
    let view: View = serde_json::from_str(stdout.trim())?;
    match view.state.as_str() {
        "OPEN" => Ok(PrState::Open),
        "MERGED" => Ok(PrState::Merged),
        "CLOSED" => Ok(PrState::Closed),
        other => bail!("gh named a pull request state Codly does not know: {other}"),
    }
}

#[derive(Deserialize)]
struct GhCheck {
    name: String,
    /// gh's own grouping of a check's state: pass, fail, pending, skipping, cancel.
    bucket: String,
}

/// Count what `gh pr checks --json name,bucket` printed.
pub(super) fn parse_checks(stdout: &str, now: i64) -> Result<PrChecks> {
    let list: Vec<GhCheck> = serde_json::from_str(stdout.trim())?;
    let mut checks = PrChecks { read_at: now, ..Default::default() };
    for c in list {
        match c.bucket.as_str() {
            "pass" => checks.passed += 1,
            "fail" | "cancel" => {
                checks.failed += 1;
                if checks.failing.len() < 5 {
                    checks.failing.push(c.name);
                }
            }
            "skipping" => checks.skipped += 1,
            // "pending", gh's bucket for everything that has not finished.
            _ => checks.pending += 1,
        }
    }
    Ok(checks)
}

/// `gh` on the login PATH.
pub(super) fn gh_bin(env: &HashMap<String, String>) -> Option<PathBuf> {
    let cwd = env.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    which::which_in("gh", Some(env.get("PATH")?), cwd).ok()
}

pub(super) struct GhOut {
    pub(super) ok: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl GhOut {
    /// What gh said went wrong.
    pub(super) fn said(&self) -> &str {
        let err = self.stderr.trim();
        if err.is_empty() { self.stdout.trim() } else { err }
    }
}

// ───────────── the drafted title and description ─────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    /// With the exit code, if the command ended by itself.
    Failed(Option<i64>),
    NotRun,
}

impl Outcome {
    /// From a recorded `Verified` event. A check Codly refused to run (it needed the user's
    /// OK) is recorded as not ok, with this note as its output.
    fn of(ok: bool, exit_code: Option<i64>, tail: &str) -> Outcome {
        match (ok, exit_code) {
            (true, _) => Outcome::Passed,
            (false, None) if tail.starts_with("[codly] not run") => Outcome::NotRun,
            (false, code) => Outcome::Failed(code),
        }
    }

    fn words(self) -> String {
        match self {
            Outcome::Passed => "passed".into(),
            Outcome::Failed(Some(code)) => format!("failed (exit {code})"),
            Outcome::Failed(None) => "failed (it did not finish)".into(),
            Outcome::NotRun => "not run".into(),
        }
    }
}

/// One verify command and how its last run ended.
struct Check {
    /// A task's key, or `ALL_TASKS`.
    scope: String,
    command: String,
    outcome: Outcome,
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

fn plan_title(plan: &Plan) -> String {
    prompts::clip(first_line(&plan.summary).unwrap_or("Codly plan"), TITLE_MAX)
}

/// The goal in the user's words, the tasks, and the checks Codly ran with their results.
fn plan_body(plan: &Plan, tasks: &[Task], checks: &[Check]) -> String {
    let goal = if plan.request.trim().is_empty() { plan.summary.trim() } else { plan.request.trim() };
    let mut out = format!("## Goal\n\n{}\n\n## Tasks\n\n", prompts::clip(goal, GOAL_MAX));
    for t in tasks {
        out.push_str(&format!("- **{}** {} ({})\n", t.key, t.title.trim(), store::enum_str(&t.status).replace('_', " ")));
    }
    out.push_str("\n## Checks\n\n");
    if checks.is_empty() {
        out.push_str("No checks ran for this plan.\n");
    }
    for c in checks {
        out.push_str(&format!("- {}: `{}` {}\n", c.scope, c.command, c.outcome.words()));
    }
    out
}

fn agent_title(name: &str, request: &str) -> String {
    first_line(request).map_or_else(|| format!("{name}'s work"), |l| prompts::clip(l, TITLE_MAX))
}

/// A solo agent has no plan: its last request, its commits, and that nothing was checked.
fn agent_body(name: &str, request: &str, commits: &[GitLogEntry]) -> String {
    let mut out = String::new();
    if !request.trim().is_empty() {
        out.push_str(&format!("## Goal\n\n{}\n\n", prompts::clip(request.trim(), GOAL_MAX)));
    }
    if !commits.is_empty() {
        out.push_str("## Commits\n\n");
        for c in commits {
            out.push_str(&format!("- {} ({})\n", c.subject, c.sha));
        }
        out.push('\n');
    }
    out.push_str(&format!("## Checks\n\nNo checks ran in Codly: {name} worked alone, outside a plan.\n"));
    out
}

/// The push would carry `count` commits that are not the branch's own work.
fn unpushed_warning(count: u32, remote: &str, base: &str) -> String {
    let commits = if count == 1 { "1 commit".to_string() } else { format!("{count} commits") };
    format!("This also sends {commits} from your own branch that {remote}/{base} does not have yet. The pull request will show them too.")
}

/// A branch name the user typed as the base. It becomes a `gh` argument and part of a ref.
fn check_base(base: &str, head: &str) -> Result<()> {
    let odd = |c: char| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c);
    if base.is_empty() || base.starts_with('-') || base.contains("..") || base.chars().any(odd) {
        bail!("'{base}' is not a branch name; name the branch the pull request goes into");
    }
    if base == head {
        bail!("the pull request cannot go into its own branch ({head}); name another base");
    }
    Ok(())
}

// ───────────── the step ─────────────

/// The branch a pull request is opened from, and where it lives.
struct Head {
    ws: Workspace,
    repo: PathBuf,
    branch: String,
    owner: Owner,
}

enum Owner {
    Plan(Plan),
    /// A solo agent and its worktree: what its last turn left uncommitted is committed before
    /// the push, as before a merge.
    Agent(Agent, PathBuf),
}

impl Inner {
    fn pull_request(&self, source: &PrSource) -> Result<Option<PullRequest>> {
        Ok(self.store.kv_get(&source.key())?.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    async fn pr_head(&self, source: &PrSource) -> Result<Head> {
        let head = match source {
            PrSource::Plan(id) => {
                let plan = self.plan(id)?;
                if matches!(plan.status, PlanStatus::Running | PlanStatus::Integrating) {
                    bail!("the plan is still running; wait until it is done (or cancel it)");
                }
                let branch = plan.integration_branch.clone().ok_or_else(|| anyhow!("the plan never ran, so it has no branch to push"))?;
                let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
                let repo = PathBuf::from(&ws.repo_path);
                if self.git.rev_parse(&repo, &branch).await?.is_none() {
                    bail!("the plan's branch {branch} was deleted, so there is nothing to push");
                }
                Head { ws, repo, branch, owner: Owner::Plan(plan) }
            }
            PrSource::Agent(id) => {
                let agent = self.agent(id)?;
                if agent.role != Role::Solo {
                    bail!("only a solo agent has a branch of its own; a task's work goes into its plan's pull request");
                }
                let wt = PathBuf::from(agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to push", agent.name))?);
                let ws_id = self.store.agent_workspace(id)?.ok_or_else(|| anyhow!("{} is in no office", agent.name))?;
                let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
                let repo = PathBuf::from(&ws.repo_path);
                if wt == repo {
                    bail!("{} works in your project folder itself, so it has no branch of its own to push", agent.name);
                }
                self.git.check_worktree(&repo, &wt).await?;
                let branch = self.git.current_branch(&wt).await?.ok_or_else(|| anyhow!("{}'s worktree is on a detached HEAD", agent.name))?;
                Head { ws, repo, branch, owner: Owner::Agent(agent, wt) }
            }
        };
        if !head.branch.starts_with(OWN_PREFIX) {
            bail!("{} is not one of Codly's own codly/… branches, and Codly pushes only those from here; push it yourself", head.branch);
        }
        Ok(head)
    }

    async fn remotes(&self, repo: &Path) -> Result<Vec<String>> {
        Ok(self.git.read(repo, &["remote"]).await?.lines().map(str::trim).filter(|r| !r.is_empty()).map(String::from).collect())
    }

    /// The remote a pull request into `base` goes to: the one `base` tracks, else `origin`,
    /// else the only one there is.
    pub(super) async fn pr_remote(&self, repo: &Path, base: &str) -> Result<String> {
        let remotes = self.remotes(repo).await?;
        // `git config --get` fails for a key that is not set: the branch tracks nothing then.
        let tracked = self.git.read(repo, &["config", "--get", &format!("branch.{base}.remote")]).await.ok().map(|r| r.trim().to_string());
        if let Some(remote) = tracked.filter(|r| remotes.contains(r)) {
            return Ok(remote);
        }
        if remotes.iter().any(|r| r == "origin") {
            return Ok("origin".into());
        }
        match remotes.as_slice() {
            [] => bail!("this repository has no remote yet; add one (git remote add origin <address>) first"),
            [only] => Ok(only.clone()),
            many => bail!(
                "this repository has {} remotes ({}) and none is named origin; set the upstream of {base} so Codly knows which one to use",
                many.len(),
                many.join(", ")
            ),
        }
    }

    /// None when the remote's address is not `host/owner/repository` (a local path, a host
    /// with groups): `gh` cannot be pointed at it.
    pub(super) async fn remote_slug(&self, repo: &Path, remote: &str) -> Option<RepoSlug> {
        parse_remote(&self.git.read(repo, &["remote", "get-url", remote]).await.ok()?)
    }

    pub(super) async fn run_gh(&self, gh: &Path, cwd: &Path, args: &[String], stdin: Option<&str>, limit: Duration) -> Result<GhOut> {
        let mut cmd = tokio::process::Command::new(gh);
        cmd.args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(&self.base_env)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| anyhow!("cannot run gh ({}): {e}", gh.display()))?;
        let pipe = child.stdin.take();
        let feed = async {
            if let (Some(text), Some(mut pipe)) = (stdin, pipe) {
                // A gh that ends early closes the pipe: its own message is the error to show.
                let _ = pipe.write_all(text.as_bytes()).await;
            }
        };
        let (_, out) = tokio::join!(feed, tokio::time::timeout(limit, child.wait_with_output()));
        let what = args.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
        let out = out.map_err(|_| anyhow!("gh {what} did not answer within {} s", limit.as_secs()))??;
        Ok(GhOut {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Why `gh` cannot open a pull request on `remote` right now, in words for the dialog.
    async fn gh_problem(&self, repo: &Path, remote: &str, slug: Option<&RepoSlug>) -> Option<String> {
        let Some(slug) = slug else {
            return Some(format!("The address of {remote} is not a host/owner/repository that gh can use."));
        };
        let Some(gh) = gh_bin(&self.base_env) else {
            return Some("The GitHub CLI (gh) is not installed. To open pull requests from here, install it (brew install gh) and sign in (gh auth login).".into());
        };
        match self.run_gh(&gh, repo, &auth_args(&slug.host), None, AUTH_TIMEOUT).await {
            Ok(out) if out.ok => None,
            Ok(_) => Some(format!("gh is not signed in to {}. Run gh auth login in a terminal, then try again.", slug.host)),
            Err(e) => Some(format!("Codly could not ask gh whether it is signed in: {e}.")),
        }
    }

    /// Where the pull request stands, from one `gh pr view` call.
    async fn read_state(&self, gh: &Path, repo: &Path, url: &str) -> Result<PrState> {
        let out = self.run_gh(gh, repo, &state_args(url), None, GH_TIMEOUT).await?;
        if !out.ok {
            bail!("gh could not read the pull request: {}", out.said().trim_end_matches('.'));
        }
        parse_state(&out.stdout)
    }

    /// `git rev-list --count` of a set of revisions.
    async fn count_commits(&self, repo: &Path, revs: &[&str]) -> Result<u32> {
        let mut args = vec!["rev-list", "--count"];
        args.extend_from_slice(revs);
        Ok(self.git.read(repo, &args).await?.trim().parse()?)
    }

    /// How many commits the head's branch has that `base` lacks on its remote, as last
    /// fetched. None: the remote has no such branch.
    async fn ahead_of(&self, head: &Head, base: &str) -> Result<Option<u32>> {
        let remote = self.pr_remote(&head.repo, base).await?;
        let on_remote = format!("refs/remotes/{remote}/{base}");
        if self.git.rev_parse(&head.repo, &on_remote).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.count_commits(&head.repo, &[head.branch.as_str(), "--not", on_remote.as_str()]).await?))
    }

    /// The base drafted for a solo agent's pull request. Its branch started from what the user
    /// had checked out, and Merge goes there too: so the checked-out branch when the remote
    /// has it, unless the default branch is nearer to the agent's branch (fewer commits apart).
    async fn agent_base(&self, head: &Head) -> Result<String> {
        let default = head.ws.default_branch.clone();
        let current = self.git.current_branch(&head.repo).await?.filter(|c| *c != default && !c.starts_with(OWN_PREFIX));
        let Some(current) = current else { return Ok(default) };
        let Some(from_current) = self.ahead_of(head, &current).await? else { return Ok(default) };
        Ok(match self.ahead_of(head, &default).await? {
            Some(from_default) if from_default < from_current => default,
            _ => current,
        })
    }

    /// How many commits the push would send that are not this branch's own work: the commits
    /// of the user's branch it started from that `remote_base` (the base on the remote, as
    /// last fetched) does not have. A plan knows the commit it started from. A solo agent's
    /// branch shares them with the local base, or with the branch that is checked out.
    async fn unpushed_below(&self, head: &Head, base: &str, remote_base: &str) -> Result<u32> {
        let starts = match &head.owner {
            Owner::Plan(Plan { base_commit: Some(start), .. }) => vec![start.clone()],
            _ => {
                let mut shared = vec![];
                for theirs in [format!("refs/heads/{base}"), "HEAD".to_string()] {
                    shared.extend(self.git.merge_base(&head.repo, &theirs, &head.branch).await?);
                }
                shared
            }
        };
        if starts.is_empty() {
            return Ok(0);
        }
        let mut revs: Vec<&str> = starts.iter().map(String::as_str).collect();
        revs.extend(["--not", remote_base]);
        self.count_commits(&head.repo, &revs).await
    }

    /// The checks Codly ran for a plan, each with the result of its last run: every finished
    /// task's verify commands, then the plan's integration checks.
    fn plan_checks(&self, plan: &Plan, tasks: &[Task]) -> Result<Vec<Check>> {
        let outcome = |e: &EventRow, cmd: &str| match &e.kind {
            EventKind::Verified { command, ok, exit_code, tail } if command == cmd => Some(Outcome::of(*ok, *exit_code, tail)),
            _ => None,
        };
        let mut checks = vec![];
        for t in tasks.iter().filter(|t| t.status == TaskStatus::Done) {
            let runs = self.store.task_events(&t.id, "verified")?;
            for cmd in &t.verify {
                let last = runs.iter().rev().find_map(|e| outcome(e, cmd)).unwrap_or(Outcome::NotRun);
                checks.push(Check { scope: t.key.clone(), command: cmd.clone(), outcome: last });
            }
        }
        let integration: Vec<String> = self.store.plan_json(&plan.id)?["integration_checks"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if !integration.is_empty() {
            // Their events name no plan. This plan's run of them is the first one after its
            // last task was checked and merged; another plan's run can come later.
            let work = self.store.work_since(&plan.workspace_id, plan.created_at)?;
            let own = |e: &EventRow| e.task_id.as_deref().is_some_and(|id| tasks.iter().any(|t| t.id == id));
            let after = work.iter().filter(|e| own(e)).map(|e| e.id).max().unwrap_or(0);
            for cmd in integration {
                let first = work.iter().filter(|e| e.id > after && e.task_id.is_none()).find_map(|e| outcome(e, &cmd)).unwrap_or(Outcome::NotRun);
                checks.push(Check { scope: ALL_TASKS.into(), command: cmd, outcome: first });
            }
        }
        Ok(checks)
    }

    /// What Create pull request would send. Reads the repository, and asks `gh` whether it is
    /// signed in and where a pull request on record stands. Nothing is sent and the repository
    /// is not changed; only Codly's note of that pull request's state is brought up to date.
    async fn draft_pull_request(&self, source: &PrSource) -> Result<PrDraft> {
        let head = self.pr_head(source).await?;
        let base = match &head.owner {
            Owner::Plan(plan) => plan.base_branch.clone().unwrap_or_else(|| head.ws.default_branch.clone()),
            Owner::Agent(..) => self.agent_base(&head).await?,
        };
        let remote = self.pr_remote(&head.repo, &base).await?;
        let remote_base = format!("refs/remotes/{remote}/{base}");
        let on_remote = self.git.rev_parse(&head.repo, &remote_base).await?.is_some();
        let (title, body) = match &head.owner {
            Owner::Plan(plan) => {
                let tasks = self.store.plan_tasks(&plan.id)?;
                (plan_title(plan), plan_body(plan, &tasks, &self.plan_checks(plan, &tasks)?))
            }
            Owner::Agent(agent, _) => {
                let request = self.store.last_chat_by(Some(&head.ws.id), &format!("agent:{}", agent.id), "user")?.unwrap_or_default();
                // Its commits that the base does not have, when the base is known here.
                let from = if on_remote { Some(remote_base.clone()) } else { self.git.rev_parse(&head.repo, &base).await? };
                let commits = match from {
                    Some(from) => parse_log(&self.git.read(&head.repo, &["log", "-n30", "--format=%h%x1f%an%x1f%ct%x1f%s", &format!("{from}..{}", head.branch)]).await?),
                    None => vec![],
                };
                (agent_title(&agent.name, &request), agent_body(&agent.name, &request, &commits))
            }
        };
        let slug = self.remote_slug(&head.repo, &remote).await;
        // A plan whose request started from an issue or a pull request names it at the end.
        let body = match &head.owner {
            Owner::Plan(plan) => match self.plan_intake(plan)? {
                Some(started) => format!("{body}\n{}\n", intake::link_line(&started, slug.as_ref())),
                None => body,
            },
            Owner::Agent(..) => body,
        };
        let mut gh_problem = self.gh_problem(&head.repo, &remote, slug.as_ref()).await;
        // A pull request on record may be merged or closed by now. Ask, so the dialog does not
        // say that new commits join it when they would not.
        let mut pr_state = None;
        if gh_problem.is_none()
            && let Some(mut pr) = self.pull_request(source)?
            && let Some(gh) = gh_bin(&self.base_env)
        {
            match self.read_state(&gh, &head.repo, &pr.url).await {
                Ok(state) => {
                    pr.state = state;
                    self.store.kv_set(&source.key(), &serde_json::to_string(&pr)?)?;
                    pr_state = Some(state);
                }
                Err(e) => gh_problem = Some(format!("Codly does not know whether the pull request it opened is still open: {e}.")),
            }
        }
        let warning = if !on_remote {
            Some(format!("{base} is not on {remote} (as last fetched). Push it first, or name another base."))
        } else {
            match self.unpushed_below(&head, &base, &remote_base).await? {
                0 => None,
                count => Some(unpushed_warning(count, &remote, &base)),
            }
        };
        Ok(PrDraft { remote, branch: head.branch, base, title, body, gh_problem, warning, pr_state })
    }

    /// Nothing is sent unless the repository still is what the user confirmed.
    async fn check_confirm(&self, head: &Head, confirm: &PrConfirm) -> Result<()> {
        if confirm.branch != head.branch {
            bail!("its branch is {} now, not {}; nothing was pushed", head.branch, confirm.branch);
        }
        if confirm.remote.starts_with('-') || !self.remotes(&head.repo).await?.contains(&confirm.remote) {
            bail!("this repository has no remote named {}; nothing was pushed", confirm.remote);
        }
        check_base(&confirm.base, &head.branch)?;
        if let Owner::Agent(agent, _) = &head.owner {
            // A stopped agent counts as working until its process is gone: it may still write files.
            let resting = matches!(agent.status, AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed | AgentStatus::Offline);
            if (agent.run_id.is_some() && !resting) || !self.is_quiet(&agent.id) {
                bail!("{} is still working; push when its turn ends (Codly commits its work then)", agent.name);
            }
        }
        Ok(())
    }

    /// Push the confirmed branch to the confirmed remote. A solo agent's uncommitted work is
    /// committed first. Never forced: a remote branch with other commits stays as it is.
    async fn push_head(&self, source: &PrSource, head: &Head, confirm: &PrConfirm) -> Result<()> {
        let ws_id = Some(head.ws.id.as_str());
        {
            let _g = self.repo_lock.lock().await;
            if let Owner::Agent(agent, wt) = &head.owner {
                self.git.check_worktree(&head.repo, wt).await?;
                self.check_resolved(agent, wt).await?;
                let message = format!("codly(solo/{}): before pull request", agent.name);
                if let Some(c) = self.git.commit_all(wt, &message).await? {
                    self.emit(ws_id, row(Some(&agent.id), None, None, EventKind::Committed { sha: c.sha, files: c.files, message }))?;
                }
            }
            let base = format!("refs/remotes/{}/{}", confirm.remote, confirm.base);
            if self.git.rev_parse(&head.repo, &base).await?.is_some() && self.git.is_ancestor(&head.repo, &head.branch, &base).await? {
                bail!("{} on {} already has everything on {}, so there is nothing to push", confirm.base, confirm.remote, head.branch);
            }
            self.git.push(&head.repo, &confirm.remote, &head.branch).await.map_err(|e| {
                if e.to_string().contains("the push was refused") {
                    anyhow!(
                        "{} already has a branch {} with commits this one does not have. Codly never force-pushes, so nothing was sent; delete that branch on {} if it is not needed, then try again",
                        confirm.remote,
                        head.branch,
                        confirm.remote
                    )
                } else {
                    e
                }
            })?;
        }
        let summary = format!("you pushed {} to {}", head.branch, confirm.remote);
        self.decide_by(ws_id, None, "push", "human", summary, json!({ "branch": head.branch, "remote": confirm.remote, "source": source }))?;
        Ok(())
    }

    /// Push the branch and open the pull request with the user's `gh`. Without a `gh`, or
    /// with a remote `gh` cannot be pointed at, nothing is pushed.
    async fn create_pull_request(&self, source: &PrSource, confirm: &PrConfirm, title: &str, body: &str) -> Result<PullRequest> {
        let title = title.trim();
        if title.is_empty() || title.contains('\n') {
            bail!("write a title of one line first");
        }
        if body.chars().count() > BODY_MAX {
            bail!("the description is longer than GitHub takes ({BODY_MAX} characters); shorten it");
        }
        let head = self.pr_head(source).await?;
        self.check_confirm(&head, confirm).await?;
        let slug = self
            .remote_slug(&head.repo, &confirm.remote)
            .await
            .ok_or_else(|| anyhow!("the address of {} is not a host/owner/repository that gh can use; nothing was pushed", confirm.remote))?;
        let gh = gh_bin(&self.base_env).ok_or_else(|| anyhow!("the GitHub CLI (gh) is not installed; nothing was pushed"))?;
        self.push_head(source, &head, confirm).await?;

        let pushed = format!("{} is on {} now, but gh did not open the pull request", head.branch, confirm.remote);
        let by_hand = compare_url(&slug, &confirm.base, &head.branch).map(|u| format!(" You can open it in your browser: {u}")).unwrap_or_default();
        let args = create_args(&slug, &confirm.base, &head.branch, title);
        let out = self.run_gh(&gh, &head.repo, &args, Some(body), GH_TIMEOUT).await.map_err(|e| anyhow!("{pushed}: {e}.{by_hand}"))?;
        // gh refuses a second pull request for a branch and names the open one: that is the one.
        let url = if out.ok { pr_url(&out.stdout) } else { pr_url(&out.stderr).filter(|_| out.stderr.contains("already exists")) };
        let Some(url) = url else { bail!("{pushed}: {}.{by_hand}", out.said().trim_end_matches('.')) };

        // gh opened it just now, or named the one that is open for this branch.
        let pr = PullRequest {
            url,
            remote: confirm.remote.clone(),
            branch: head.branch.clone(),
            base: confirm.base.clone(),
            created_at: now_ms(),
            state: PrState::Open,
            checks: None,
        };
        self.store.kv_set(&source.key(), &serde_json::to_string(&pr)?)?;
        let ws_id = Some(head.ws.id.as_str());
        let summary = format!("you opened a pull request for {} into {}: {}", pr.branch, pr.base, pr.url);
        self.decide_by(ws_id, None, "pull_request", "human", summary, json!({ "url": pr.url, "branch": pr.branch, "base": pr.base, "source": source }))?;
        match &head.owner {
            Owner::Plan(plan) => {
                let text = format!("Opened a pull request for \"{}\" into {}: {}", plan.summary, pr.base, pr.url);
                self.chat(ws_id, &plan.thread, "codly", &text, Some(json!({ "plan_id": plan.id })))?;
            }
            Owner::Agent(agent, _) => {
                let text = format!("Opened a pull request for {}'s work into {}: {}", agent.name, pr.base, pr.url);
                self.chat(ws_id, &format!("agent:{}", agent.id), "codly", &text, None)?;
            }
        }
        Ok(pr)
    }

    /// Push the branch without opening a pull request: the way without `gh`, and the way to
    /// add new commits to an open pull request.
    async fn push_pr_branch(&self, source: &PrSource, confirm: &PrConfirm) -> Result<PrPushed> {
        let head = self.pr_head(source).await?;
        self.check_confirm(&head, confirm).await?;
        self.push_head(source, &head, confirm).await?;
        let compare_url = self.remote_slug(&head.repo, &confirm.remote).await.and_then(|s| compare_url(&s, &confirm.base, &head.branch));
        Ok(PrPushed { remote: confirm.remote.clone(), branch: head.branch, compare_url })
    }

    /// Read the pull request's state and its checks once: `gh pr view`, then `gh pr checks`.
    async fn refresh_pull_request(&self, source: &PrSource) -> Result<PullRequest> {
        let mut pr = self.pull_request(source)?.ok_or_else(|| anyhow!("Codly has not opened a pull request from here yet"))?;
        let ws_id = match source {
            PrSource::Plan(id) => Some(self.plan(id)?.workspace_id),
            PrSource::Agent(id) => self.store.agent_workspace(id)?,
        };
        let ws = match ws_id {
            Some(id) => self.store.workspace(&id)?,
            None => None,
        }
        .ok_or_else(|| anyhow!("unknown workspace"))?;
        let gh = gh_bin(&self.base_env).ok_or_else(|| anyhow!("the GitHub CLI (gh) is not installed, so the pull request cannot be read"))?;
        let repo = Path::new(&ws.repo_path);
        pr.state = self.read_state(&gh, repo, &pr.url).await?;
        let out = self.run_gh(&gh, repo, &checks_args(&pr.url), None, GH_TIMEOUT).await?;
        let checks = match parse_checks(&out.stdout, now_ms()) {
            Ok(checks) => checks,
            // gh's answer for a pull request that has no check yet.
            Err(_) if out.stderr.contains("no checks reported") => PrChecks { read_at: now_ms(), ..Default::default() },
            Err(_) => bail!("gh could not read the checks: {}", out.said()),
        };
        pr.checks = Some(checks);
        self.store.kv_set(&source.key(), &serde_json::to_string(&pr)?)?;
        Ok(pr)
    }
}

impl Core {
    /// The pull request Codly opened for a plan or a solo agent, as last read. No network.
    pub async fn pull_request(&self, source: &PrSource) -> Result<Option<PullRequest>> {
        self.inner.pull_request(source)
    }

    /// What Create pull request would send, for the confirm: remote, branch, base, and a
    /// title and description drafted from the plan, and the state of a pull request that is
    /// on record. Nothing is pushed.
    pub async fn draft_pull_request(&self, source: &PrSource) -> Result<PrDraft> {
        self.inner.draft_pull_request(source).await
    }

    /// Push the Codly branch the user confirmed and open a pull request with their `gh`.
    pub async fn create_pull_request(&self, source: &PrSource, confirm: &PrConfirm, title: &str, body: &str) -> Result<PullRequest> {
        self.inner.create_pull_request(source, confirm, title, body).await
    }

    /// Push the Codly branch the user confirmed, without a pull request.
    pub async fn push_pr_branch(&self, source: &PrSource, confirm: &PrConfirm) -> Result<PrPushed> {
        self.inner.push_pr_branch(source, confirm).await
    }

    /// Read the pull request's state and checks again (one `gh pr view`, one `gh pr checks`).
    pub async fn refresh_pull_request(&self, source: &PrSource) -> Result<PullRequest> {
        self.inner.refresh_pull_request(source).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{CoreConfig, StartOptions};
    use std::sync::Arc;

    fn slug() -> RepoSlug {
        RepoSlug { host: "github.com".into(), owner: "acme".into(), repo: "web".into() }
    }

    #[test]
    fn remote_addresses_are_read_as_host_owner_repository() {
        for url in [
            "git@github.com:acme/web.git",
            "https://github.com/acme/web.git\n",
            "https://someone@github.com/acme/web",
            "ssh://git@github.com:22/acme/web.git",
            "ssh://git@github.com/acme/web/",
        ] {
            assert_eq!(parse_remote(url), Some(slug()), "{url}");
        }
        assert_eq!(parse_remote("git@git.corp.example:team/api.git").unwrap().arg(), "git.corp.example/team/api");
        // A local path, a host with groups, another scheme, a name that would read as an option.
        let not_for_gh = ["/srv/git/web.git", "../web.git", "https://gitlab.com/group/sub/web.git", "file:///srv/git/web.git", "git@github.com:acme", "git@github.com:-o/web", ""];
        for url in not_for_gh {
            assert_eq!(parse_remote(url), None, "{url}");
        }
    }

    #[test]
    fn gh_commands_are_built_without_running_gh() {
        assert_eq!(auth_args("github.com"), ["auth", "status", "--hostname", "github.com"]);
        assert_eq!(
            create_args(&slug(), "main", "codly/add-hello-ab12/integration", "Add a hello file"),
            ["pr", "create", "--repo", "github.com/acme/web", "--base", "main", "--head", "codly/add-hello-ab12/integration", "--title", "Add a hello file", "--body-file", "-"]
        );
        assert_eq!(state_args("https://github.com/acme/web/pull/7"), ["pr", "view", "https://github.com/acme/web/pull/7", "--json", "state"]);
        assert_eq!(checks_args("https://github.com/acme/web/pull/7"), ["pr", "checks", "https://github.com/acme/web/pull/7", "--json", "name,bucket"]);
        // The page to open a pull request by hand: on GitHub only, branch names escaped.
        assert_eq!(
            compare_url(&slug(), "release/1.x", "codly/solo/ada #1").as_deref(),
            Some("https://github.com/acme/web/compare/release/1.x...codly/solo/ada%20%231?expand=1")
        );
        assert_eq!(compare_url(&RepoSlug { host: "git.corp.example".into(), ..slug() }, "main", "codly/x"), None);
    }

    #[test]
    fn gh_answers_are_read() {
        let created = "Creating pull request for codly/x into main in acme/web\n\nhttps://github.com/acme/web/pull/7\n";
        assert_eq!(pr_url(created).as_deref(), Some("https://github.com/acme/web/pull/7"));
        let exists = "a pull request for branch \"codly/x\" into branch \"main\" already exists:\nhttps://github.com/acme/web/pull/7";
        assert_eq!(pr_url(exists).as_deref(), Some("https://github.com/acme/web/pull/7"));
        assert_eq!(pr_url("pull request create failed: GraphQL: No commits between main and codly/x"), None);
        assert_eq!(pr_url("see https://github.com/acme/web/issues/3"), None);

        let json = r#"[{"bucket":"pass","name":"build"},{"bucket":"fail","name":"lint"},{"bucket":"cancel","name":"deploy"},
            {"bucket":"pending","name":"e2e"},{"bucket":"skipping","name":"docs"}]"#;
        let checks = parse_checks(json, 42).unwrap();
        assert_eq!(checks, PrChecks { passed: 1, failed: 2, pending: 1, skipped: 1, failing: vec!["lint".into(), "deploy".into()], read_at: 42 });
        assert_eq!(parse_checks("[]", 1).unwrap(), PrChecks { read_at: 1, ..Default::default() });
        assert!(parse_checks("no checks reported on the 'codly/x' branch", 1).is_err());

        for (state, read) in [("OPEN", PrState::Open), ("MERGED", PrState::Merged), ("CLOSED", PrState::Closed)] {
            assert_eq!(parse_state(&format!("{{\"state\":\"{state}\"}}\n")).unwrap(), read);
        }
        assert!(parse_state(r#"{"state":"DRAFT"}"#).unwrap_err().to_string().contains("does not know: DRAFT"));
        assert!(parse_state("no pull requests found").is_err());
    }

    #[test]
    fn the_warning_counts_the_commits_that_go_with_the_branch() {
        assert_eq!(
            unpushed_warning(1, "origin", "main"),
            "This also sends 1 commit from your own branch that origin/main does not have yet. The pull request will show them too."
        );
        assert!(unpushed_warning(12, "upstream", "release/1.x").starts_with("This also sends 12 commits from your own branch that upstream/release/1.x does not"));
    }

    #[test]
    fn a_base_is_a_plain_branch_name() {
        for base in ["main", "release/1.x", "feature-x"] {
            check_base(base, "codly/x/integration").unwrap();
        }
        for base in ["", "--repo", "main branch", "main..dev", "a:b", "main\n"] {
            assert!(check_base(base, "codly/x/integration").unwrap_err().to_string().contains("is not a branch name"), "{base:?}");
        }
        assert!(check_base("codly/x/integration", "codly/x/integration").unwrap_err().to_string().contains("its own branch"));
    }

    #[test]
    fn a_solo_agent_draft_says_that_nothing_was_checked() {
        assert_eq!(agent_title("Ada", "\n  Add release notes  \nwith the details"), "Add release notes");
        assert_eq!(agent_title("Ada", "  "), "Ada's work");
        let commits = [GitLogEntry { sha: "a1b2c3d".into(), subject: "codly(solo/Ada): Add release notes".into(), author: "Codly".into(), ts: 0 }];
        assert_eq!(
            agent_body("Ada", "Add release notes", &commits),
            "## Goal\n\nAdd release notes\n\n## Commits\n\n- codly(solo/Ada): Add release notes (a1b2c3d)\n\n## Checks\n\nNo checks ran in Codly: Ada worked alone, outside a plan.\n"
        );
        assert_eq!(agent_body("Ada", "", &[]), "## Checks\n\nNo checks ran in Codly: Ada worked alone, outside a plan.\n");
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// What stands in for `gh`: it writes down each call (its arguments, one per line, then
    /// `--`) and what `pr create` got on stdin, and answers as gh does. `auth status` fails
    /// while a `signed-out` file is next to it, `pr create` finds a pull request open already
    /// while an `exists` file is, and `pr view` answers with the state written in a `state`
    /// file (OPEN without one; it fails on the state GONE).
    const FAKE_GH: &str = r#"#!/bin/sh
dir=${0%/*}
printf '%s\n' "$@" -- >> "$dir/gh.calls"
case "$1 $2" in
  "auth status")
    if [ -e "$dir/signed-out" ]; then echo "You are not logged into any GitHub hosts." >&2; exit 1; fi ;;
  "pr create")
    /bin/cat > "$dir/gh.body"
    if [ -e "$dir/exists" ]; then
      echo 'a pull request for branch "x" into branch "main" already exists:' >&2
      echo "https://github.com/acme/web/pull/7" >&2
      exit 1
    fi
    echo "https://github.com/acme/web/pull/7" ;;
  "pr view")
    state=OPEN
    if [ -e "$dir/state" ]; then read -r state < "$dir/state"; fi
    if [ "$state" = GONE ]; then echo "GraphQL: Could not resolve to a PullRequest." >&2; exit 1; fi
    echo "{\"state\":\"$state\"}" ;;
  "pr checks")
    echo '[{"name":"build","bucket":"pass"},{"name":"lint","bucket":"fail"},{"name":"e2e","bucket":"pending"}]' ;;
esac
"#;

    struct Bench {
        _root: tempfile::TempDir,
        core: Arc<Core>,
        repo: PathBuf,
        /// The bare repository pushes go to.
        remote: PathBuf,
        /// The only folder on the core's PATH: git, and the stand-in `gh` when there is one.
        bin: PathBuf,
        /// The commit the user's checkout is at, where plans and agents start: `main` as the
        /// remote has it, until `local_commit` adds to it.
        base: String,
    }

    impl Bench {
        /// A repository on `main` whose `origin` is named like a GitHub repository and pushes
        /// to a bare repository beside it, opened in a core that finds a `gh` only if `gh` is set.
        async fn new(gh: bool) -> Bench {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            std::os::unix::fs::symlink(which::which("git").unwrap(), bin.join("git")).unwrap();
            if gh {
                std::fs::write(bin.join("gh"), FAKE_GH).unwrap();
                std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let (repo, remote) = (root.path().join("repo"), root.path().join("remote.git"));
            std::fs::create_dir(&repo).unwrap();
            sh(&repo, &["init", "-q", "-b", "main"]);
            sh(&repo, &["config", "user.name", "Test"]);
            sh(&repo, &["config", "user.email", "test@example.com"]);
            std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
            sh(&repo, &["add", "-A"]);
            sh(&repo, &["commit", "-qm", "init"]);
            sh(root.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
            sh(&repo, &["remote", "add", "origin", "https://github.com/acme/web.git"]);
            sh(&repo, &["config", "remote.origin.pushurl", &remote.to_string_lossy()]);
            sh(&repo, &["push", "-q", "origin", "main"]);
            let base = sh(&repo, &["rev-parse", "HEAD"]);

            let mut env: HashMap<String, String> = std::env::vars().collect();
            env.insert("PATH".into(), bin.to_string_lossy().into_owned());
            let opts = StartOptions { env: Some(env), clis: Some(vec![]), ..Default::default() };
            let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
            core.open_workspace(repo.to_str().unwrap()).await.unwrap();
            Bench { _root: root, core, repo, remote, bin, base }
        }

        /// A commit of the user's own on the checked-out branch. It is not pushed.
        fn local_commit(&mut self, file: &str) {
            std::fs::write(self.repo.join(file), "mine\n").unwrap();
            sh(&self.repo, &["add", file]);
            sh(&self.repo, &["commit", "-qm", file]);
            self.base = sh(&self.repo, &["rev-parse", "HEAD"]);
        }

        /// The calls the stand-in `gh` got, each as its argument list.
        fn gh_calls(&self) -> Vec<Vec<String>> {
            let text = std::fs::read_to_string(self.bin.join("gh.calls")).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            lines.split(|l| *l == "--").filter(|call| !call.is_empty()).map(|call| call.iter().map(|a| a.to_string()).collect()).collect()
        }

        /// The branches of the remote.
        fn remote_branches(&self) -> Vec<String> {
            sh(&self.remote, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).lines().map(String::from).collect()
        }

        /// A finished plan as the gate leaves it: one done task whose work is on the plan's
        /// integration branch, and the checks that ran on record.
        async fn done_plan(&self) -> Plan {
            let i = &self.core.inner;
            let ws = i.require_ws().unwrap();
            let branch = "codly/add-hello-ab12/integration";
            let wt = self.bin.with_file_name("integration");
            i.git.worktree_add(&self.repo, &wt, branch, &self.base).await.unwrap();
            std::fs::write(wt.join("hello.txt"), "hello\n").unwrap();
            i.git.commit_all(&wt, "codly(t1): hello").await.unwrap().unwrap();
            let plan: Plan = serde_json::from_value(json!({ "id": "plan-1", "workspace_id": ws.id, "thread": "lead", "request": "Please add a hello file",
                "summary": "Add a hello file", "assumptions": [], "open_questions": [], "status": "done", "base_commit": self.base, "base_branch": "main",
                "integration_branch": branch, "integration_worktree": wt, "created_at": now_ms() - 1000, "updated_at": now_ms() }))
            .unwrap();
            let task = |id: &str, key: &str, title: &str, status: &str, verify: &[&str]| -> Task {
                serde_json::from_value(json!({ "id": id, "plan_id": "plan-1", "key": key, "title": title, "goal": "g", "acceptance": [], "write_scope": [],
                    "verify": verify, "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
                    "attempt": 1, "status_reason": null, "summary": null, "created_at": 0, "updated_at": 0 }))
                .unwrap()
            };
            let emit = |task_id: Option<&str>, kind: EventKind| {
                i.emit(Some(&ws.id), row(None, task_id, None, kind)).unwrap();
            };
            emit(None, EventKind::PlanChanged { plan: plan.clone() });
            i.store.with(|db| store::set_plan_json(db, &plan.id, &json!({ "integration_checks": ["make check", "./deploy.sh --dry-run"] }))).unwrap();
            emit(Some("task-1"), EventKind::TaskChanged { task: task("task-1", "t1", "Write hello.txt", "done", &["grep -q hello hello.txt", "make lint"]) });
            emit(Some("task-2"), EventKind::TaskChanged { task: task("task-2", "t2", "Write the docs", "cancelled", &["make docs"]) });
            let verified = |task_id: Option<&str>, command: &str, ok: bool, exit_code: Option<i64>, tail: &str| {
                emit(task_id, EventKind::Verified { command: command.into(), ok, exit_code, tail: tail.into() });
            };
            // The task's check failed once, then passed; its second check ran out of time.
            verified(Some("task-1"), "grep -q hello hello.txt", false, Some(1), "");
            verified(Some("task-1"), "grep -q hello hello.txt", true, Some(0), "");
            verified(Some("task-1"), "make lint", false, None, "[codly] killed after 10 min");
            // The plan's own checks: one passed, one Codly did not run without an OK.
            verified(None, "make check", true, Some(0), "");
            verified(None, "./deploy.sh --dry-run", false, None, "[codly] not run: this command needs your OK (risk: runs a script).");
            // A later plan in the same office ran the same check, and it failed there.
            verified(None, "make check", false, Some(2), "");
            plan
        }

        /// A solo agent in its own worktree, on its own `codly/solo/…` branch, started from
        /// the user's checkout.
        async fn solo(&self, name: &str) -> (Agent, PathBuf) {
            let i = &self.core.inner;
            let ws = i.require_ws().unwrap();
            let agent = i.create_agent(&ws.id, name, Provider::Claude, Role::Solo, 0, None).unwrap();
            let (branch, wt) = i.solo_worktree(&ws, &agent);
            i.git.worktree_add(&self.repo, &wt, &branch, &self.base).await.unwrap();
            let agent = i.update_agent(&agent.id, |a| a.worktree = Some(wt.to_string_lossy().into_owned())).unwrap();
            (agent, wt)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_plan_gets_a_pull_request_through_the_users_gh() {
        let b = Bench::new(true).await;
        let plan = b.done_plan().await;
        let source = PrSource::Plan(plan.id.clone());
        let branch = plan.integration_branch.clone().unwrap();
        assert_eq!(b.core.pull_request(&source).await.unwrap(), None);

        // The draft: where it would go, and a description from the plan and its checks.
        std::fs::write(b.bin.join("signed-out"), "").unwrap();
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!(draft.gh_problem.as_deref(), Some("gh is not signed in to github.com. Run gh auth login in a terminal, then try again."));
        std::fs::remove_file(b.bin.join("signed-out")).unwrap();
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!((draft.remote.as_str(), draft.branch.as_str(), draft.base.as_str()), ("origin", branch.as_str(), "main"));
        assert_eq!((draft.gh_problem.as_deref(), draft.warning.as_deref()), (None, None));
        assert_eq!(draft.title, "Add a hello file");
        assert_eq!(
            draft.body,
            "## Goal\n\nPlease add a hello file\n\n## Tasks\n\n- **t1** Write hello.txt (done)\n- **t2** Write the docs (cancelled)\n\n## Checks\n\n\
             - t1: `grep -q hello hello.txt` passed\n- t1: `make lint` failed (it did not finish)\n\
             - all tasks together: `make check` passed\n- all tasks together: `./deploy.sh --dry-run` not run\n"
        );
        assert_eq!(b.gh_calls(), [auth_args("github.com"), auth_args("github.com")], "a draft only asks gh whether it is signed in");
        assert_eq!(b.remote_branches(), ["main"], "a draft sends nothing");

        // Nothing is sent when the repository is not what the confirm named.
        let confirm = PrConfirm { remote: draft.remote.clone(), branch: draft.branch.clone(), base: draft.base.clone() };
        let stale = PrConfirm { branch: "codly/other/integration".into(), ..confirm.clone() };
        let e = b.core.create_pull_request(&source, &stale, &draft.title, &draft.body).await.unwrap_err().to_string();
        assert!(e.contains("not codly/other/integration; nothing was pushed"), "{e}");
        let elsewhere = PrConfirm { remote: "upstream".into(), ..confirm.clone() };
        let e = b.core.create_pull_request(&source, &elsewhere, &draft.title, &draft.body).await.unwrap_err().to_string();
        assert!(e.contains("no remote named upstream"), "{e}");
        let e = b.core.create_pull_request(&source, &confirm, " ", &draft.body).await.unwrap_err().to_string();
        assert!(e.contains("write a title"), "{e}");
        assert_eq!(b.remote_branches(), ["main"]);

        // Create: the plan's branch is pushed, and gh opens the pull request with the edited text.
        let pr = b.core.create_pull_request(&source, &confirm, "Add hello", "My words.\n").await.unwrap();
        assert_eq!(pr.url, "https://github.com/acme/web/pull/7");
        assert_eq!((pr.remote.as_str(), pr.branch.as_str(), pr.base.as_str(), &pr.checks), ("origin", branch.as_str(), "main", &None));
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(sh(&b.remote, &["rev-parse", &branch]), sh(&b.repo, &["rev-parse", &branch]));
        assert_eq!(sh(&b.remote, &["rev-parse", "main"]), b.base, "the user's branch is not pushed");
        assert_eq!(b.gh_calls().last().unwrap(), &create_args(&slug(), "main", &branch, "Add hello"));
        assert_eq!(std::fs::read_to_string(b.bin.join("gh.body")).unwrap(), "My words.\n");
        assert_eq!(b.core.pull_request(&source).await.unwrap().as_ref(), Some(&pr), "it is on record");
        let ws = b.core.inner.require_ws().unwrap();
        let chats = b.core.inner.store.recent_chats(Some(&ws.id), 50).unwrap();
        let said = "Opened a pull request for \"Add a hello file\" into main: https://github.com/acme/web/pull/7";
        assert!(chats.iter().any(|c| c.thread == "lead" && c.author == "codly" && c.text == said), "{chats:?}");

        // Refresh reads the state and the checks once.
        let before = b.gh_calls().len();
        let read = b.core.refresh_pull_request(&source).await.unwrap();
        let checks = read.checks.clone().unwrap();
        assert_eq!((checks.passed, checks.failed, checks.pending, checks.skipped), (1, 1, 1, 0));
        assert_eq!(checks.failing, ["lint"]);
        assert_eq!(read.state, PrState::Open);
        assert_eq!(b.gh_calls()[before..], [state_args(&pr.url), checks_args(&pr.url)]);
        assert_eq!(b.core.pull_request(&source).await.unwrap(), Some(read));

        // A branch that has its pull request already: gh names it, and that is the answer.
        std::fs::write(b.bin.join("exists"), "").unwrap();
        let again = b.core.create_pull_request(&source, &confirm, "Add hello", "").await.unwrap();
        assert_eq!(again.url, pr.url);
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_gh_a_solo_agents_branch_is_pushed_and_never_forced() {
        let b = Bench::new(false).await;
        let (agent, wt) = b.solo("Ada").await;
        let i = &b.core.inner;
        let ws = i.require_ws().unwrap();
        i.chat(Some(&ws.id), &format!("agent:{}", agent.id), "user", "Add notes for the release\nKeep them short.", None).unwrap();
        std::fs::write(wt.join("notes.txt"), "from the agent\n").unwrap();
        let source = PrSource::Agent(agent.id.clone());

        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert!(draft.branch.starts_with("codly/solo/ada-"), "{}", draft.branch);
        assert_eq!((draft.remote.as_str(), draft.base.as_str(), draft.title.as_str()), ("origin", "main", "Add notes for the release"));
        assert!(draft.gh_problem.as_deref().unwrap().starts_with("The GitHub CLI (gh) is not installed."), "{:?}", draft.gh_problem);
        assert_eq!(draft.body, "## Goal\n\nAdd notes for the release\nKeep them short.\n\n## Checks\n\nNo checks ran in Codly: Ada worked alone, outside a plan.\n");

        // Without gh, Create sends nothing; Push branch does, and says where to open it by hand.
        let confirm = PrConfirm { remote: draft.remote.clone(), branch: draft.branch.clone(), base: draft.base.clone() };
        let e = b.core.create_pull_request(&source, &confirm, &draft.title, &draft.body).await.unwrap_err().to_string();
        assert!(e.contains("gh) is not installed; nothing was pushed"), "{e}");
        assert_eq!(b.remote_branches(), ["main"]);
        let odd = PrConfirm { base: "main --force".into(), ..confirm.clone() };
        assert!(b.core.push_pr_branch(&source, &odd).await.unwrap_err().to_string().contains("is not a branch name"));

        let pushed = b.core.push_pr_branch(&source, &confirm).await.unwrap();
        assert_eq!(pushed.compare_url, Some(format!("https://github.com/acme/web/compare/main...{}?expand=1", draft.branch)));
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "", "Codly committed the agent's loose work first");
        assert_eq!(sh(&b.remote, &["rev-parse", &draft.branch]), sh(&wt, &["rev-parse", "HEAD"]));
        assert_eq!(sh(&b.remote, &["rev-parse", "main"]), b.base, "the user's branch is not pushed");
        assert_eq!(b.core.pull_request(&source).await.unwrap(), None, "a push alone opens no pull request");

        // The draft now lists what the branch adds.
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert!(draft.body.contains("## Commits\n\n- codly(solo/Ada): before pull request ("), "{}", draft.body);

        // Someone else put other commits on that branch of the remote: no force, nothing sent.
        let other = b.bin.with_file_name("other");
        sh(&b.repo, &["clone", "-q", "--branch", &draft.branch, &b.remote.to_string_lossy(), &other.to_string_lossy()]);
        std::fs::write(other.join("theirs.txt"), "theirs\n").unwrap();
        sh(&other, &["add", "-A"]);
        sh(&other, &["-c", "user.name=O", "-c", "user.email=o@o", "commit", "-qm", "theirs"]);
        sh(&other, &["push", "-q", "origin", &draft.branch]);
        let theirs = sh(&b.remote, &["rev-parse", &draft.branch]);
        std::fs::write(wt.join("notes.txt"), "more from the agent\n").unwrap();
        let e = b.core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
        assert!(e.contains("Codly never force-pushes, so nothing was sent"), "{e}");
        assert_eq!(sh(&b.remote, &["rev-parse", &draft.branch]), theirs);

        // Only Codly's own branches: on any other branch the agent's work is not pushed from here.
        sh(&wt, &["switch", "-q", "-c", "feature"]);
        let e = b.core.draft_pull_request(&source).await.unwrap_err().to_string();
        assert!(e.contains("feature is not one of Codly's own codly/… branches"), "{e}");
        let e = b.core.push_pr_branch(&source, &PrConfirm { branch: "feature".into(), ..confirm }).await.unwrap_err().to_string();
        assert!(e.contains("pushes only those"), "{e}");
        assert!(!b.remote_branches().contains(&"feature".to_string()));
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_merged_or_closed_pull_request_makes_room_for_a_new_one() {
        let b = Bench::new(true).await;
        let plan = b.done_plan().await;
        let source = PrSource::Plan(plan.id.clone());
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!(draft.pr_state, None, "no pull request is on record yet");
        let confirm = PrConfirm { remote: draft.remote.clone(), branch: draft.branch.clone(), base: draft.base.clone() };
        let pr = b.core.create_pull_request(&source, &confirm, "Add hello", "").await.unwrap();

        // While it is open, the dialog can say that new commits join it.
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!((draft.pr_state, draft.gh_problem.as_deref()), (Some(PrState::Open), None));
        assert_eq!(b.gh_calls().last().unwrap(), &state_args(&pr.url), "one gh pr view call says where it stands");

        // Merged or closed on GitHub: the draft finds out, and the record says so.
        for (said, state) in [("MERGED", PrState::Merged), ("CLOSED", PrState::Closed)] {
            std::fs::write(b.bin.join("state"), said).unwrap();
            let draft = b.core.draft_pull_request(&source).await.unwrap();
            assert_eq!((draft.pr_state, draft.gh_problem.as_deref()), (Some(state), None));
            assert_eq!(b.core.pull_request(&source).await.unwrap().unwrap().state, state);
        }
        // Refresh reads the state too.
        std::fs::write(b.bin.join("state"), "MERGED").unwrap();
        assert_eq!(b.core.refresh_pull_request(&source).await.unwrap().state, PrState::Merged);

        // gh cannot say where it stands: nothing is claimed, and the record keeps the last answer.
        std::fs::write(b.bin.join("state"), "GONE").unwrap();
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!(draft.pr_state, None);
        assert_eq!(
            draft.gh_problem.as_deref(),
            Some("Codly does not know whether the pull request it opened is still open: gh could not read the pull request: GraphQL: Could not resolve to a PullRequest.")
        );
        assert_eq!(b.core.pull_request(&source).await.unwrap().unwrap().state, PrState::Merged);
        let e = b.core.refresh_pull_request(&source).await.unwrap_err().to_string();
        assert!(e.starts_with("gh could not read the pull request"), "{e}");

        // More work on the branch, then Create again: the new pull request takes the place of
        // the merged one on record.
        let wt = PathBuf::from(plan.integration_worktree.clone().unwrap());
        std::fs::write(wt.join("more.txt"), "more\n").unwrap();
        b.core.inner.git.commit_all(&wt, "codly(t3): more").await.unwrap().unwrap();
        let next = b.core.create_pull_request(&source, &confirm, "Add more", "").await.unwrap();
        assert_eq!(next.state, PrState::Open);
        assert!(next.created_at >= pr.created_at);
        assert_eq!(b.core.pull_request(&source).await.unwrap(), Some(next));
        assert_eq!(sh(&b.remote, &["rev-parse", &confirm.branch]), sh(&wt, &["rev-parse", "HEAD"]));
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_draft_says_how_many_of_the_users_unpushed_commits_go_too() {
        let mut b = Bench::new(false).await;
        // An earlier plan was merged into main and not pushed; the next plan starts there.
        b.local_commit("mine-1.txt");
        let plan = b.done_plan().await;
        let plan = PrSource::Plan(plan.id);
        let one = "This also sends 1 commit from your own branch that origin/main does not have yet. The pull request will show them too.";
        assert_eq!(b.core.draft_pull_request(&plan).await.unwrap().warning.as_deref(), Some(one));

        // A solo agent that starts on a branch of the user's that was never pushed.
        sh(&b.repo, &["switch", "-q", "-c", "topic"]);
        b.local_commit("mine-2.txt");
        let (agent, _wt) = b.solo("Ada").await;
        let agent = PrSource::Agent(agent.id);
        let draft = b.core.draft_pull_request(&agent).await.unwrap();
        assert_eq!(draft.base, "main", "topic is not on the remote, so it is no base");
        let two = "This also sends 2 commits from your own branch that origin/main does not have yet. The pull request will show them too.";
        assert_eq!(draft.warning.as_deref(), Some(two));

        // Once main is pushed, the plan's branch carries only its own work; the agent's still
        // carries the commit on topic.
        sh(&b.repo, &["push", "-q", "origin", "main"]);
        assert_eq!(b.core.draft_pull_request(&plan).await.unwrap().warning, None);
        assert_eq!(b.core.draft_pull_request(&agent).await.unwrap().warning.as_deref(), Some(one));
        assert_eq!(b.remote_branches(), ["main"], "a draft sends nothing");
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_solo_agents_draft_goes_into_the_branch_it_started_from() {
        let mut b = Bench::new(false).await;
        // The remote has release (old), main (one commit later) and develop (one more).
        sh(&b.repo, &["branch", "release"]);
        b.local_commit("main.txt");
        sh(&b.repo, &["switch", "-q", "-c", "develop"]);
        b.local_commit("develop.txt");
        for branch in ["release", "main", "develop"] {
            sh(&b.repo, &["push", "-q", "origin", branch]);
        }
        let (agent, wt) = b.solo("Ada").await;
        std::fs::write(wt.join("notes.txt"), "from the agent\n").unwrap();
        b.core.inner.git.commit_all(&wt, "codly(solo/Ada): notes").await.unwrap().unwrap();
        let source = PrSource::Agent(agent.id);

        // The checkout is on develop, where the agent started: that is the base, and the
        // description lists the agent's commit only.
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!((draft.remote.as_str(), draft.base.as_str(), draft.warning.as_deref()), ("origin", "develop", None));
        assert!(draft.body.contains("## Commits\n\n- codly(solo/Ada): notes ("), "{}", draft.body);
        assert!(!draft.body.contains("develop.txt"), "{}", draft.body);

        // A checked-out branch that is farther from the agent's work than the default branch is
        // not the base, and neither is one the remote does not have.
        sh(&b.repo, &["switch", "-q", "release"]);
        assert_eq!(b.core.draft_pull_request(&source).await.unwrap().base, "main");
        sh(&b.repo, &["switch", "-q", "-c", "topic", "develop"]);
        assert_eq!(b.core.draft_pull_request(&source).await.unwrap().base, "main");
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_is_pushed_for_a_busy_agent_a_running_plan_or_an_empty_branch() {
        let b = Bench::new(true).await;
        let i = &b.core.inner;
        let ws = i.require_ws().unwrap();
        let (agent, wt) = b.solo("Ada").await;
        let source = PrSource::Agent(agent.id.clone());
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        let confirm = PrConfirm { remote: draft.remote.clone(), branch: draft.branch.clone(), base: draft.base.clone() };

        // A branch with nothing the base lacks: there is nothing to push.
        let e = b.core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
        assert!(e.contains(&format!("main on origin already has everything on {}, so there is nothing to push", draft.branch)), "{e}");
        assert_eq!(b.remote_branches(), ["main"]);

        // An agent in the middle of a turn: nothing is committed for it and nothing is sent.
        std::fs::write(wt.join("notes.txt"), "half written\n").unwrap();
        i.update_agent(&agent.id, |a| {
            a.status = AgentStatus::Working;
            a.run_id = Some("run-1".into());
        })
        .unwrap();
        let e = b.core.create_pull_request(&source, &confirm, "Notes", "").await.unwrap_err().to_string();
        assert!(e.contains("Ada is still working"), "{e}");
        let e = b.core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
        assert!(e.contains("Ada is still working"), "{e}");
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "?? notes.txt");
        assert_eq!(b.remote_branches(), ["main"]);
        i.update_agent(&agent.id, |a| {
            a.status = AgentStatus::Idle;
            a.run_id = None;
        })
        .unwrap();

        // A remote whose address names no host/owner/repository: gh cannot be pointed at it, so
        // Create sends nothing, also with gh installed and signed in.
        sh(&b.repo, &["remote", "set-url", "origin", &b.remote.to_string_lossy()]);
        let local = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!(local.gh_problem.as_deref(), Some("The address of origin is not a host/owner/repository that gh can use."));
        let e = b.core.create_pull_request(&source, &confirm, "Notes", "").await.unwrap_err().to_string();
        assert!(e.contains("is not a host/owner/repository that gh can use; nothing was pushed"), "{e}");
        assert_eq!(b.remote_branches(), ["main"]);
        assert!(b.gh_calls().iter().all(|call| call[..2] == ["auth", "status"]), "{:?}", b.gh_calls());
        sh(&b.repo, &["remote", "set-url", "origin", "https://github.com/acme/web.git"]);

        // The agent's work, pushed and then merged into main on the remote (as a fetch would
        // show it): a second push has nothing to send.
        b.core.push_pr_branch(&source, &confirm).await.unwrap();
        let tip = sh(&wt, &["rev-parse", "HEAD"]);
        assert_eq!(sh(&b.remote, &["rev-parse", &draft.branch]), tip);
        sh(&b.repo, &["update-ref", "refs/remotes/origin/main", &tip]);
        let e = b.core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
        assert!(e.contains("so there is nothing to push"), "{e}");

        // A plan that is not at rest (its tasks are still being merged and checked).
        let mut plan = b.done_plan().await;
        plan.status = PlanStatus::Integrating;
        let source = PrSource::Plan(plan.id.clone());
        let confirm = PrConfirm { remote: "origin".into(), branch: plan.integration_branch.clone().unwrap(), base: "main".into() };
        i.emit(Some(&ws.id), row(None, None, None, EventKind::PlanChanged { plan })).unwrap();
        let e = b.core.draft_pull_request(&source).await.unwrap_err().to_string();
        assert!(e.contains("the plan is still running"), "{e}");
        let e = b.core.push_pr_branch(&source, &confirm).await.unwrap_err().to_string();
        assert!(e.contains("the plan is still running"), "{e}");
        assert!(!b.remote_branches().contains(&confirm.branch));
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_remote_is_the_one_the_base_tracks_else_origin_else_the_only_one() {
        let b = Bench::new(false).await;
        let plan = b.done_plan().await;
        let source = PrSource::Plan(plan.id);
        assert_eq!(b.core.draft_pull_request(&source).await.unwrap().remote, "origin");

        // The remote the base tracks comes before origin.
        sh(&b.repo, &["remote", "add", "upstream", "https://github.com/acme/upstream.git"]);
        sh(&b.repo, &["config", "branch.main.remote", "upstream"]);
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!(draft.remote, "upstream");
        assert_eq!(draft.warning.as_deref(), Some("main is not on upstream (as last fetched). Push it first, or name another base."));

        // Several remotes, none tracked and none named origin: Codly does not guess.
        sh(&b.repo, &["config", "--unset", "branch.main.remote"]);
        sh(&b.repo, &["remote", "rename", "origin", "first"]);
        let e = b.core.draft_pull_request(&source).await.unwrap_err().to_string();
        assert!(e.contains("has 2 remotes") && e.contains("none is named origin; set the upstream of main"), "{e}");

        // The only remote there is, whatever its name.
        sh(&b.repo, &["remote", "remove", "upstream"]);
        let draft = b.core.draft_pull_request(&source).await.unwrap();
        assert_eq!((draft.remote.as_str(), draft.warning.as_deref()), ("first", None));
        b.core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_plan_started_from_an_issue_names_it_in_the_description() {
        use super::intake::{IntakeKind, IntakeSource};
        let b = Bench::new(false).await;
        let plan = b.done_plan().await;
        let source = PrSource::Plan(plan.id.clone());
        let i = &b.core.inner;
        let ws = i.require_ws().unwrap();
        let plain = b.core.draft_pull_request(&source).await.unwrap().body;
        assert!(!plain.contains("Closes") && !plain.contains("Related to"), "{plain}");

        // The user's "From issue…" message in the plan's request, as `Core::start_from_github` leaves it.
        let from = |kind: IntakeKind, repo: &str, number: u32, ts: i64| {
            let path = if kind == IntakeKind::Issue { "issues" } else { "pull" };
            let url = format!("https://github.com/acme/{repo}/{path}/{number}");
            let started = IntakeSource { kind, host: "github.com".into(), owner: "acme".into(), repo: repo.into(), number, url, title: "Dark mode resets".into() };
            let message = ChatMessage { id: new_id(), thread: "lead".into(), author: "user".into(), text: "Work on it.".into(), attachment: Some(json!({ "intake": started })), ts };
            i.emit(Some(&ws.id), row(None, None, None, EventKind::Chat { message })).unwrap();
        };
        let body = async || b.core.draft_pull_request(&source).await.unwrap().body;

        // A message that came after the plan was made is not what the plan answers.
        from(IntakeKind::Issue, "web", 99, plan.created_at + 1);
        assert_eq!(body().await, plain);
        // The request started from an issue of the repository the pull request goes to.
        from(IntakeKind::Issue, "web", 12, plan.created_at - 500);
        assert_eq!(body().await, format!("{plain}\nCloses #12\n"));
        assert!(!body().await.contains("Dark mode resets"), "the title on GitHub is not copied into the description");
        // The newest one before the plan counts: here an issue of another repository, then a pull request.
        from(IntakeKind::Issue, "api", 3, plan.created_at - 400);
        assert_eq!(body().await, format!("{plain}\nCloses acme/api#3\n"));
        from(IntakeKind::PullRequest, "web", 7, plan.created_at - 300);
        assert_eq!(body().await, format!("{plain}\nRelated to #7\n"));

        // An earlier plan of the request that is done delivered what was asked: this plan is new work.
        let earlier = Plan { id: "plan-0".into(), created_at: plan.created_at - 100, ..plan.clone() };
        i.emit(Some(&ws.id), row(None, None, None, EventKind::PlanChanged { plan: earlier })).unwrap();
        assert_eq!(body().await, plain);
        b.core.shutdown().await;
    }
}
