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
//! checked, until you turn the new text on or turn the checks off.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::worktree_setup::fingerprint;
use super::{Core, Inner, approvals, lock, prompts, row};
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
        let wt = PathBuf::from(agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to check", agent.name))?);
        let ws = match self.store.agent_workspace(agent_id)? {
            Some(id) => self.store.workspace(&id)?.ok_or_else(|| anyhow!("unknown workspace"))?,
            None => self.require_ws()?,
        };
        let repo = Path::new(&ws.repo_path);
        if wt != repo {
            self.git.check_worktree(repo, &wt).await?;
        }
        let cmds = match self.required_checks(&ws.id, repo)? {
            Required::Run(cmds) => cmds,
            Required::Off => bail!("this repo has no required checks turned on. Write your commands in {CHECKS_FILE}, one on each line, then turn them on in {SETTINGS}"),
            Required::Stale(why) => bail!("{}", not_run(&why)),
        };
        let results = self.run_checks(&ws.id, &wt, Some(agent_id), &cmds).await?;
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
        // The whole result: the agent's Git tab shows the last one from this decision
        // (src/lib/util/checks.ts), also after you left the tab.
        self.decide_by(Some(&ws.id), None, "gate", "human", format!("you checked {}'s work: {outcome}", agent.name), json!({ "agent_id": agent_id, "checks": results }))?;
        self.chat(Some(&ws.id), &format!("agent:{agent_id}"), "codly", &format!("Checked {}'s work: {outcome}.", agent.name), None)?;
        Ok(results)
    }
}

impl Core {
    /// The open repo's required checks (`.codly/checks`): the text, its commands with what
    /// the approval rules see in each, on or off. `set` turns them on, for the text you read,
    /// or off.
    pub async fn required_checks(&self, set: Option<ChecksChoice>) -> Result<RequiredChecks> {
        let ws = self.inner.require_ws()?;
        if let Some(choice) = set {
            self.inner.choose_checks(&ws, choice)?;
        }
        self.inner.checks_status(&ws)
    }

    /// Run the repo's required checks in the worktree of an agent you chat with, and return
    /// each command's result. An error when the checks are off or not the text you turned on.
    pub async fn check_agent(&self, agent_id: &str) -> Result<Vec<CheckResult>> {
        self.inner.check_agent(agent_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
