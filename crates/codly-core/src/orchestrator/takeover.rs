//! "Open in my terminal": an agent's CLI session goes to the user, and comes back.
//!
//! The conversation an agent has belongs to its CLI, so the user can open it in the CLI's own
//! terminal UI with the CLI's own resume command. Two processes on one session corrupt it, so
//! this is a hand-over, enforced here and not in the UI:
//!
//! - **Take over**: only an agent that is between turns and whose task, if it has one, is not
//!   in flight. Codly marks the agent, then stops its own process for the session. The mark
//!   comes first, so no start gets in while the process stops: `take_over` writes the mark and
//!   then reads the live table, `spawn_run` enters the live table and then reads the mark, and
//!   one of the two always sees the other. The agent is in your terminal once the process is
//!   gone; if it does not stop, the mark is taken back.
//! - **While it is there** no run starts for the agent: `spawn_run` refuses, a message to it is
//!   refused before it is recorded (`chat_agent_from`), the scheduler counts it as busy
//!   (`busy_agents`), so a task for it waits and says why, and it is not picked for a review
//!   (`start_review`).
//! - **Give it back** removes the mark. The agent's session id never changed, so its next
//!   message or task resumes the same session.
//!
//! The mark lives in `kv` (`takeover:<agent id>` → the hand-over: where, the command, since
//! when). It outlives a restart of Codly: a terminal of the user's own can still have the
//! session open. It is what was handed over, so it stays readable whatever happens to the
//! agent's row meanwhile, and the agent can always be given back.
//!
//! The command lines are built here and nowhere else (`resume_command`): the CLI's name, its
//! resume form and a session id, never a flag that changes what the CLI may do. A CLI has one
//! only where docs/research shows a form that opens one session by its id, and shows that the
//! session Codly resumes afterwards has what the terminal did.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::{Inner, Live, lock, row, scheduler};
use crate::model::*;
use crate::pty::PtyManager;

/// kv key of an agent that is in your terminal: `takeover:<agent id>` → its `Takeover`.
/// Also the tag of the terminal Codly opened for it.
const MARK: &str = "takeover:";

fn mark(agent_id: &str) -> String {
    format!("{MARK}{agent_id}")
}

/// An agent's session in your terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Takeover {
    pub agent_id: Id,
    /// Where the terminal opens: the agent's worktree (the repository for an agent without one).
    pub cwd: String,
    /// The CLI's own resume command for the agent's session, for a terminal that is in `cwd`.
    pub command: String,
    /// `command` behind a `cd` to `cwd`: one line for a terminal of your own.
    pub copy_command: String,
    /// Since when (Unix ms).
    pub since: i64,
}

/// What the agent sheet shows about "Open in my terminal".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TakeoverState {
    /// The agent is in your terminal: Codly starts no process for it until you give it back.
    pub takeover: Option<Takeover>,
    /// Why it cannot go to your terminal right now (None: it can, or it is there already).
    pub blocked: Option<String>,
}

/// A session id that can stand in a command line as one plain word: letters, digits, `-` and
/// `_`, and it does not start with `-` (a word that starts with one is read as an option).
/// The ids of Claude Code and Codex are UUIDs.
fn plain_id(id: &str) -> bool {
    let word = id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    word && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
}

/// The CLI's own command that opens the session `session_id` in its terminal UI, or why there
/// is none (a sentence for the user).
///
/// - Claude Code: `claude --resume <id>` (docs/research/cli-claude-codex.md §2.6: it finds the
///   session by its id, from any folder). Each run of Codly is a process of its own that reads
///   the session from disk, so the next one has what the terminal did.
/// - Codex has `codex resume <id>` (cli-claude-codex.md §3.1), but its sessions do not go yet.
///   Codly's agents of one sign-in share one `codex app-server`, which stays up after an agent
///   lets go of its thread, and `thread/resume` on a thread the server still holds rejoins it
///   in memory. Nothing on record shows when the server drops a thread nobody listens to, so
///   the turns of a terminal could be missing after the give-back (cli-claude-codex.md §3.5).
/// - Gemini CLI resumes `latest` or a place in its list, not an id (cli-others-acp.md §4.1).
///   OpenCode's `--session <id>` is recorded for `opencode run` only (§5.1). Cursor and Copilot
///   were never installed here (§6, §7). None of them gets a command until a live check shows
///   one that opens the session Codly started.
pub fn resume_command(provider: Provider, session_id: &str) -> Result<String> {
    let resume = match provider {
        Provider::Claude => "claude --resume",
        Provider::Codex => bail!(
            "a Codex agent cannot go to your terminal yet: Codex can keep its session in memory, and what you do in the terminal could be lost when you give it back"
        ),
        Provider::Gemini | Provider::Opencode | Provider::Cursor | Provider::Copilot => {
            bail!("Codly knows no {} command that opens one session by its id", provider.label())
        }
    };
    if !plain_id(session_id) {
        bail!("the session id of {} is not a plain word, so Codly does not put it in a command", provider.label());
    }
    Ok(format!("{resume} {session_id}"))
}

/// `command` for the sign-in the session lives in. `home`: that sign-in's config home
/// (`Some(None)` = the CLI's default one, its variable unset; None = Codly knows only one
/// sign-in of this CLI). `shell_home`: the one the user's login shell points the CLI at.
/// A session exists only in its own config home, so a terminal that points elsewhere gets the
/// variable set, or taken out, for this one command.
pub fn on_account(command: &str, provider: Provider, home: Option<Option<&str>>, shell_home: Option<&str>) -> String {
    let (Some(var), Some(home)) = (crate::env::account_var(provider), home) else { return command.to_string() };
    match home {
        _ if home == shell_home => command.to_string(),
        Some(h) => format!("{var}={} {command}", crate::env::shell_quote(h)),
        None => format!("env -u {var} {command}"),
    }
}

/// `command` for a terminal that is somewhere else: a `cd` to `cwd` first.
pub fn with_cd(cwd: &str, command: &str) -> String {
    format!("cd {} && {command}", crate::env::shell_quote(cwd))
}

/// Open a terminal in the agent's worktree, with the login env, and type the resume command
/// into it. Typed, not run: Enter is the user's. One terminal per agent: an earlier one is
/// closed first, so the session never has two of them.
pub fn open_terminal(pty: &PtyManager, env: &HashMap<String, String>, takeover: &Takeover) -> Result<String> {
    let tag = mark(&takeover.agent_id);
    pty.close_tagged(&tag);
    let pty_id = pty.open_tagged(Path::new(&takeover.cwd), env, &tag)?;
    // The tty keeps this until the login shell is ready to read it.
    if let Err(e) = pty.write(&pty_id, takeover.command.as_bytes()) {
        pty.close(&pty_id);
        return Err(e);
    }
    Ok(pty_id)
}

/// Close the terminal `open_terminal` opened for the agent, and the CLI in it.
pub fn close_terminal(pty: &PtyManager, agent_id: &str) {
    pty.close_tagged(&mark(agent_id));
}

/// A run that is between turns: no turn runs, none is queued, and it is not starting (a run
/// starts for a prompt).
fn between_turns(l: &Live) -> bool {
    !l.turn_active && l.queued == 0 && l.starting.is_none()
}

impl Inner {
    /// What was handed over when the agent went to your terminal; None = it is not there.
    fn handed_over(&self, agent_id: &str) -> Result<Option<Takeover>> {
        let Some(json) = self.store.kv_get(&mark(agent_id))? else { return Ok(None) };
        Ok(Some(serde_json::from_str(&json).with_context(|| format!("the terminal mark of agent {agent_id} is unreadable"))?))
    }

    pub(super) fn in_terminal(&self, agent_id: &str) -> Result<bool> {
        Ok(self.store.kv_get(&mark(agent_id))?.is_some())
    }

    /// The guard of every start and of every message: an agent in your terminal gets neither.
    pub(super) fn not_in_terminal(&self, agent: &Agent) -> Result<()> {
        if self.in_terminal(&agent.id)? {
            bail!("{} is in your terminal: give it back first", agent.name);
        }
        Ok(())
    }

    /// Every agent that is in your terminal: no task goes to one of them (`busy_agents`), and
    /// none of them is picked for a review.
    pub(super) fn in_terminal_ids(&self) -> Vec<Id> {
        match self.store.kv_prefixed(MARK) {
            Ok(marks) => marks.into_iter().map(|(key, _)| key[MARK.len()..].to_string()).collect(),
            Err(e) => {
                // `spawn_run` still refuses each of them: a start then fails, with the reason.
                tracing::warn!("the agents in your terminal could not be read: {e}");
                vec![]
            }
        }
    }

    /// The config home the agent's session lives in, as `on_account` takes it.
    fn session_home(&self, a: &Agent) -> Option<Option<String>> {
        let id = self.agent_account(a)?;
        self.account(&id).map(|account| account.home).or_else(|| crate::env::account_home(a.provider, &id))
    }

    /// Where the agent's CLI runs: its worktree, or its repository when it has none (anymore).
    fn terminal_cwd(&self, a: &Agent) -> Result<String> {
        if let Some(worktree) = a.worktree.as_deref().filter(|w| Path::new(w).is_dir()) {
            return Ok(worktree.to_string());
        }
        let ws = match self.store.agent_workspace(&a.id)? {
            Some(id) => self.store.workspace(&id)?,
            None => None,
        };
        ws.map(|w| w.repo_path).ok_or_else(|| anyhow!("the repository of {} is not known", a.name))
    }

    /// The resume command for the agent's session, on the sign-in the session lives in.
    fn terminal_command(&self, a: &Agent) -> Result<String> {
        let Some(session) = &a.session_id else { bail!("{} has no session yet: send it a message first", a.name) };
        let command = resume_command(a.provider, session)?;
        let home = self.session_home(a);
        let shell_home = crate::env::primary_home(&self.base_env, a.provider);
        Ok(on_account(&command, a.provider, home.as_ref().map(|h| h.as_deref()), shell_home.as_deref()))
    }

    /// May the agent go to your terminal now, and what is handed over then? The error is the
    /// reason it may not, as the sheet shows it.
    fn hand_over(&self, a: &Agent) -> Result<Takeover> {
        if a.archived {
            bail!("{} no longer works here", a.name);
        }
        if a.role == Role::Lead {
            bail!("the Lead works through Codly's own tools, and a terminal does not have them");
        }
        let command = self.terminal_command(a)?;
        // A task in flight is Codly's to run: its worker's turns, its checks, its merge.
        if let Some(ws) = self.store.agent_workspace(&a.id)? {
            let mine = |t: &Task| a.task_id.as_ref() == Some(&t.id) || t.assignee.as_ref() == Some(&a.id);
            if let Some(t) = self.store.tasks(&ws)?.into_iter().find(|t| scheduler::is_active(t.status) && mine(t)) {
                bail!("{} is on {}, which is still running: stop {} first, or wait for the task", a.name, t.key, a.name);
            }
        }
        if lock(&self.live).get(&a.id).is_some_and(|l| !between_turns(l)) {
            bail!("{} is in the middle of a turn: wait for it, or stop it first", a.name);
        }
        let cwd = self.terminal_cwd(a)?;
        Ok(Takeover { agent_id: a.id.clone(), copy_command: with_cd(&cwd, &command), cwd, command, since: now_ms() })
    }

    fn takeover_state(&self, a: &Agent) -> Result<TakeoverState> {
        Ok(match self.handed_over(&a.id)? {
            // Marked, and its process is not gone yet (`take_over`): not yours until it is.
            Some(_) if self.is_live(&a.id) => TakeoverState { takeover: None, blocked: Some(format!("{} is stopping: one moment", a.name)) },
            Some(takeover) => TakeoverState { takeover: Some(takeover), blocked: None },
            None => TakeoverState { takeover: None, blocked: self.hand_over(a).err().map(|e| format!("{e:#}")) },
        })
    }

    /// Every agent that is in your terminal, the one that went first at the top.
    fn takeovers(&self) -> Result<Vec<Takeover>> {
        let mut all = vec![];
        for (key, json) in self.store.kv_prefixed(MARK)? {
            let takeover: Takeover = serde_json::from_str(&json).with_context(|| format!("the terminal mark {key} is unreadable"))?;
            // Marked while its process still stops (`take_over`): not there yet.
            if !self.is_live(&takeover.agent_id) {
                all.push(takeover);
            }
        }
        all.sort_by_key(|t| t.since);
        Ok(all)
    }

    /// Record the hand-over where the agent's history shows it.
    fn takeover_decision(&self, a: &Agent, summary: String) {
        let ws = self.store.agent_workspace(&a.id).ok().flatten();
        let kind = EventKind::Decision {
            id: new_id(),
            decision_type: "agent_control".into(),
            method: "human".into(),
            summary,
            detail: json!({ "agent_id": a.id }),
        };
        let _ = self.emit(ws.as_deref(), row(Some(a.id.as_str()), a.task_id.as_deref(), None, kind));
    }

    /// Hand the agent's session to the user: mark the agent, then stop Codly's process for it.
    /// Asked again for an agent that is there already, it answers with the same hand-over.
    async fn take_over(&self, agent_id: &str) -> Result<TakeoverState> {
        let a = self.agent(agent_id)?;
        if self.in_terminal(agent_id)? {
            return self.takeover_state(&a);
        }
        // No scheduling pass from this check until the mark is set: while its idle chat shuts
        // down (up to 5 s) it looks free, and a pass would hand it a task.
        let _g = self.sched_lock.lock().await;
        let takeover = self.hand_over(&a)?;
        // The mark first: from here a message for it is refused, and a start that is on its way
        // finds the mark once it is in the live table (`spawn_run`) and gives up.
        self.store.kv_set(&mark(agent_id), &serde_json::to_string(&takeover)?)?;
        // Looked at again under the mark, and told to stop in the same step: a prompt or a start
        // that got in before the mark keeps its turn, and nothing gets in after the look.
        let mut in_turn = false;
        self.stop_run_if(agent_id, |l| {
            in_turn = !between_turns(l);
            !in_turn
        });
        let refused = if in_turn {
            Some(anyhow!("{} is in the middle of a turn: wait for it, or stop it first", a.name))
        } else if !self.stop_and_wait(agent_id).await {
            Some(anyhow!("{} did not stop in time; try again", a.name))
        } else {
            None
        };
        if let Some(e) = refused {
            self.store.kv_write(&[(mark(agent_id), None)])?;
            return Err(e);
        }
        self.takeover_decision(&a, format!("{} is in your terminal: Codly starts no process for it until you give it back", a.name));
        Ok(TakeoverState { takeover: Some(takeover), blocked: None })
    }

    /// The user is done in the terminal: Codly may run the agent again, on the same session.
    fn give_back(&self, agent_id: &str) -> Result<TakeoverState> {
        let a = self.agent(agent_id)?;
        if self.in_terminal(agent_id)? {
            self.store.kv_write(&[(mark(agent_id), None)])?;
            self.takeover_decision(&a, format!("{} is back with Codly: its next message or task resumes the same session", a.name));
            self.kick(); // a task that waited for it can start
        }
        self.takeover_state(&a)
    }
}

impl super::Core {
    /// Is the agent in your terminal, and if not, may it go there now?
    pub async fn agent_takeover(&self, agent_id: &str) -> Result<TakeoverState> {
        let a = self.inner.agent(agent_id)?;
        self.inner.takeover_state(&a)
    }

    /// Every agent that is in your terminal: what the window marks them with.
    pub async fn takeovers(&self) -> Result<Vec<Takeover>> {
        self.inner.takeovers()
    }

    /// "Open in my terminal": stop Codly's process for the agent's session and keep Codly away
    /// from it until `give_back_agent`. Refused for an agent in a turn or on a task in flight.
    pub async fn take_over_agent(&self, agent_id: &str) -> Result<TakeoverState> {
        self.inner.take_over(agent_id).await
    }

    /// "Give it back": the agent's next message or task resumes the same session in Codly. The
    /// caller closes the terminal it opened first (`close_terminal`).
    pub async fn give_back_agent(&self, agent_id: &str) -> Result<TakeoverState> {
        self.inner.give_back(agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b9f1c5e-7a2d-4c1b-9e61-3f2a8d4c5b6a";

    #[test]
    fn each_cli_gets_its_own_resume_command_and_nothing_else() {
        assert_eq!(resume_command(Provider::Claude, ID).unwrap(), format!("claude --resume {ID}"));
        // Codex has a resume command, but nothing on record shows that the session Codly
        // resumes afterwards has what the terminal did: its sessions stay with Codly.
        let e = resume_command(Provider::Codex, ID).unwrap_err().to_string();
        assert!(e.starts_with("a Codex agent cannot go to your terminal yet"), "{e}");
        // No form that opens one session by its id is on record for the others.
        for p in [Provider::Gemini, Provider::Opencode, Provider::Cursor, Provider::Copilot] {
            let e = resume_command(p, ID).unwrap_err().to_string();
            assert_eq!(e, format!("Codly knows no {} command that opens one session by its id", p.label()));
        }
        // Three or four plain words, whatever the CLI: no flag that changes what it may do.
        for p in Provider::ALL {
            if let Ok(line) = resume_command(p, ID) {
                assert!(!line.contains("danger") && !line.contains("permission") && !line.contains("yolo"), "{line}");
                assert_eq!(line.split(' ').last(), Some(ID), "{line}");
            }
        }
    }

    #[test]
    fn a_session_id_that_is_not_one_plain_word_gets_no_command() {
        for bad in ["", "--dangerously-skip-permissions", "-c", "a b", "a;rm -rf ~", "$(id)", "a\nb", "a'b", "é", "a".repeat(129).as_str()] {
            let e = resume_command(Provider::Claude, bad).unwrap_err().to_string();
            assert!(e.contains("not a plain word"), "{bad:?}: {e}");
        }
        assert!(resume_command(Provider::Claude, "ses_01J_x-2").is_ok());
    }

    #[test]
    fn the_command_names_the_sign_in_only_when_the_terminal_points_elsewhere() {
        let line = "claude --resume abc";
        let claude = |home, shell| on_account(line, Provider::Claude, home, shell);
        // The terminal's own sign-in is the session's: nothing to add.
        assert_eq!(claude(Some(None), None), line);
        assert_eq!(claude(Some(Some("/Users/me/.claude-work")), Some("/Users/me/.claude-work")), line);
        // Codly knows only one sign-in of the CLI: the terminal's.
        assert_eq!(claude(None, Some("/Users/me/.claude-work")), line);
        // Another sign-in: its config home, for this one command.
        assert_eq!(claude(Some(Some("/Users/me/.claude-work")), None), format!("CLAUDE_CONFIG_DIR=/Users/me/.claude-work {line}"));
        assert_eq!(claude(Some(Some("/Users/me/my homes/.claude")), None), format!("CLAUDE_CONFIG_DIR='/Users/me/my homes/.claude' {line}"));
        // The default sign-in, in a shell that points the CLI at another one.
        assert_eq!(claude(Some(None), Some("/Users/me/.claude-work")), format!("env -u CLAUDE_CONFIG_DIR {line}"));
    }

    #[test]
    fn the_line_to_copy_goes_to_the_worktree_first() {
        assert_eq!(with_cd("/data/worktrees/web/bo", "claude --resume abc"), "cd /data/worktrees/web/bo && claude --resume abc");
        assert_eq!(with_cd("/data/my repo/it's", "claude --resume abc"), r"cd '/data/my repo/it'\''s' && claude --resume abc");
    }

    /// What reached the terminal's screen until `done` is true of it, or for 5 s.
    fn screen(rx: &std::sync::mpsc::Receiver<Vec<u8>>, seen: &mut String, done: impl Fn(&str) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !done(seen) {
            if let Ok(bytes) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
                seen.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }

    #[test]
    fn the_terminal_has_the_command_typed_and_enter_is_yours() {
        let pty = PtyManager::default();
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.insert("SHELL".into(), "/bin/sh".into());
        let takeover = Takeover {
            agent_id: "a1".into(),
            cwd: "/tmp".into(),
            command: "echo took-$((40+2))".into(),
            copy_command: String::new(),
            since: 0,
        };
        let id = open_terminal(&pty, &env, &takeover).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let tx = std::sync::Mutex::new(tx);
        pty.attach(&id, Box::new(move |b| { let _ = tx.lock().unwrap().send(b); })).unwrap();
        // The line is there, as typed, and it has not run.
        let mut seen = String::new();
        screen(&rx, &mut seen, |s| s.contains("echo took-$((40+2))"));
        assert!(seen.contains("echo took-$((40+2))"), "got: {seen}");
        std::thread::sleep(std::time::Duration::from_millis(300));
        while let Ok(bytes) = rx.try_recv() {
            seen.push_str(&String::from_utf8_lossy(&bytes));
        }
        assert!(!seen.contains("took-42"), "the command ran without Enter: {seen}");
        // Enter runs it.
        pty.write(&id, b"\n").unwrap();
        screen(&rx, &mut seen, |s| s.contains("took-42"));
        assert!(seen.contains("took-42"), "got: {seen}");

        // A second terminal for the same agent takes the place of the first.
        let again = open_terminal(&pty, &env, &takeover).unwrap();
        assert!(pty.write(&id, b"\n").is_err(), "the first terminal is closed");
        // Giving the agent back closes it, and no other agent's terminal.
        let other = open_terminal(&pty, &env, &Takeover { agent_id: "a2".into(), ..takeover.clone() }).unwrap();
        close_terminal(&pty, "a1");
        assert!(pty.write(&again, b"\n").is_err());
        assert!(pty.write(&other, b"\n").is_ok());
        pty.close_all();
    }
}
