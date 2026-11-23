//! Agent runs: one CLI process lifetime per run (`run` table), spawned through the
//! `Spawner` seam with a per-run MCP token and a raw log, plus the live-run table the
//! ingest loop and the gates consult.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;

use super::{Inner, Live, guard, lock, read};
use crate::adapters::{AgentCmd, AgentHandle, SessionMode, SessionRequest, SpawnSpec, classify};
use crate::mcp::ToolCtx;
use crate::model::*;
use crate::store::RunRow;

pub(super) struct RunReq {
    pub agent: Agent,
    /// The role this run plays (tools, read-only defaults); an idle worker chatting 1:1
    /// runs with its own role but read-only.
    pub role: Role,
    pub cwd: PathBuf,
    pub read_only: bool,
    pub task_id: Option<Id>,
    pub session: SessionMode,
    pub system_prompt: String,
    pub workspace_id: Option<Id>,
    /// `AccountInfo::id` to run on. None = a resume runs on the agent's recorded account (its
    /// session lives there), a new session on `Inner::choose_account`.
    pub account: Option<String>,
}

/// A new session id for providers that accept a pre-assigned one (Claude `--session-id`).
pub(super) fn new_session(provider: Provider) -> SessionMode {
    SessionMode::New { session_id: (provider == Provider::Claude).then(|| uuid::Uuid::new_v4().to_string()) }
}

impl Inner {
    /// Start a CLI run. The run row is written before spawning, so events the adapter emits
    /// during its handshake are already attributable to this run.
    pub(super) async fn spawn_run(self: &Arc<Self>, req: RunReq) -> Result<Id> {
        // Its session is open in your terminal (takeover.rs): a second process would corrupt it.
        self.not_in_terminal(&req.agent)?;
        let provider = req.agent.provider;
        let cli = self.clis().into_iter().find(|c| c.provider == provider);
        let Some(cli) = cli.filter(|c| c.path.is_some()) else { bail!("{} is not installed", provider.label()) };
        // What the folder itself would make the CLI load, and whether you trusted it (trust.rs).
        let repo_config = self.repo_config(req.workspace_id.as_deref(), &req.cwd, provider).await?;
        let account = match (&req.account, &req.session) {
            (Some(a), _) => Some(a.clone()),
            (None, SessionMode::Resume { .. }) => self.agent_account(&req.agent),
            (None, SessionMode::New { .. }) => self.choose_account(provider, self.settings().account_strategy),
        };
        let info = self.run_account(provider, account.as_deref(), &cli)?;
        // Re-resolved at every spawn: CLI updaters re-point their symlinks.
        let bin = guard("env::resolve_bin", || Ok(crate::env::resolve_bin(provider, &self.base_env)))
            .ok()
            .flatten()
            .or_else(|| cli.path.as_ref().map(PathBuf::from))
            .ok_or_else(|| anyhow!("{} binary not found", provider.label()))?;
        let (byok_key, byok_command) = self.byok_for(provider).await?;
        // An ACP agent reports its session's cost as a running total. The adapter takes off what
        // the session's earlier runs cost, so a resumed session is not counted a second time.
        let session_cost_usd = match &req.session {
            SessionMode::Resume { session_id } if !matches!(provider, Provider::Claude | Provider::Codex) => {
                self.store.session_cost(&req.agent.id, session_id)?
            }
            _ => 0.0,
        };
        let mut env = guard("env::child_env", || {
            Ok(crate::env::child_env_with(&self.base_env, provider, byok_key.as_ref().map(|k| k.expose())))
        })
        .unwrap_or_else(|e| {
            tracing::debug!("{e}; using the base env");
            self.base_env.clone()
        });
        // Your shell's Bedrock, Vertex or token sign-in for Claude, only if you asked for it.
        if self.settings().claude_shell_auth {
            crate::env::apply_shell_auth(&mut env, &self.base_env, provider);
        }
        // The sign-in: CLAUDE_CONFIG_DIR / CODEX_HOME / GEMINI_CLI_HOME of that account.
        if let Some(acct) = &info {
            crate::env::apply_account(&mut env, provider, acct.home.as_deref());
        }
        env.extend(self.worktree_env(&req.cwd)); // CODLY_PORT_BASE of a Codly worktree
        let mcp = self.mcp.get().ok_or_else(|| anyhow!("the Codly MCP server is not running"))?.clone();
        let run_id = new_id();
        let dir = self.data("runs").join(&run_id);
        // Owner-only: the run dir holds the MCP config with this run's token and the raw log.
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(&dir)?;
        let raw_log = dir.join("raw.ndjson");
        self.prepare_default_mcp(&raw_log, &req.cwd).await;
        let token =
            mcp.register(ToolCtx { agent_id: req.agent.id.clone(), run_id: run_id.clone(), task_id: req.task_id.clone(), role: req.role });
        // One run per agent, and it is live from here: a stop, an exit or a second message
        // during the handshake finds it. Nobody listens to its handle until the CLI is up.
        {
            let mut live = lock(&self.live);
            if live.contains_key(&req.agent.id) {
                drop(live);
                mcp.revoke(&token);
                bail!("{} is already running", req.agent.name);
            }
            live.insert(
                req.agent.id.clone(),
                Live {
                    handle: AgentHandle { agent_id: req.agent.id.clone(), run_id: run_id.clone(), tx: mpsc::unbounded_channel().0 },
                    run_id: run_id.clone(),
                    role: req.role,
                    provider,
                    model: req.agent.model.clone(),
                    account: account.clone(),
                    task_id: req.task_id.clone(),
                    workspace_id: req.workspace_id.clone(),
                    cwd: req.cwd.clone(),
                    read_only: req.read_only,
                    token: token.clone(),
                    turn_active: false,
                    queued: 0,
                    starting: Some(vec![]),
                    waited: vec![],
                    stopping: false,
                    turns: 0,
                    last_text: None,
                    turn_messages: 0,
                    last_prompt: None,
                    answers: vec![],
                    answer_only: false,
                    failed_turns: 0,
                    last_error: None,
                    continue_owed: None,
                    watch: super::watchdog::Watch::new(),
                    interrupt: Default::default(),
                    session_rows: Default::default(),
                },
            );
        }
        // Looked at again now that the run is in the live table: a take-over writes its mark and
        // then reads that table (`take_over`), so one of the two always sees the other.
        if let Err(e) = self.not_in_terminal(&req.agent) {
            return Err(self.spawn_failed(&req.agent.id, &run_id, &token, e));
        }
        let session_id = match &req.session {
            SessionMode::New { session_id } => session_id.clone(),
            SessionMode::Resume { session_id } => Some(session_id.clone()),
        };
        let run = RunRow {
            id: run_id.clone(),
            agent_id: req.agent.id.clone(),
            task_id: req.task_id.clone(),
            role: req.role,
            provider,
            model: req.agent.model.clone(),
            cwd: req.cwd.to_string_lossy().into_owned(),
            session_id,
            account: account.clone(),
            status: "starting".into(),
            raw_log: raw_log.to_string_lossy().into_owned(),
            report: None,
            started_at: now_ms(),
            ended_at: None,
        };
        self.store.insert_run(&run).map_err(|e| self.spawn_failed(&req.agent.id, &run_id, &token, e))?;
        // A 1:1 agent keeps its own worktree on its row while it runs a task in the task's.
        let own = req.agent.role != Role::Solo || req.role == Role::Solo;
        let worktree = (own && matches!(req.role, Role::Worker | Role::Solo)).then(|| req.cwd.to_string_lossy().into_owned());
        self.update_agent(&req.agent.id, |a| {
            a.status = AgentStatus::Starting;
            a.run_id = Some(run_id.clone());
            a.activity = None;
            a.activity_label = None;
            if account.is_some() {
                a.account = account.clone();
            }
            // The billing badge follows the account (a CLI-wide guess until it is known).
            if let Some(b) = info.as_ref().map(|i| i.billing).filter(|b| *b != Billing::Unknown) {
                a.billing = b;
            }
            if req.task_id.is_some() {
                a.task_id = req.task_id.clone();
            }
            if worktree.is_some() {
                a.worktree = worktree.clone();
            }
        })
        .map_err(|e| self.spawn_failed(&req.agent.id, &run_id, &token, e))?;
        let spec = SpawnSpec {
            agent_id: req.agent.id.clone(),
            run_id: run_id.clone(),
            provider,
            role: req.role,
            model: req.agent.model.clone(),
            cwd: req.cwd.clone(),
            bin,
            env,
            session: req.session,
            session_cost_usd,
            system_prompt: req.system_prompt,
            read_only: req.read_only,
            mcp: Some(mcp.launch(&self.cfg.app_exe, &token)),
            // The folder's trust is about Claude's project files; a Codex agent's own servers are the user's.
            use_user_setup: self.settings().use_my_claude_setup && (provider != Provider::Claude || repo_config.project),
            sandbox_network: self.settings().codex_network_access,
            local_settings: repo_config.local,
            raw_log,
            byok_command,
            sandbox_shell: self.settings().claude_sandbox,
        };
        // A panicking adapter (or a stub) must fail this run, not the caller's task.
        let fut = self.spawner.spawn(spec, self.events_tx.clone());
        let handle = match tokio::spawn(fut).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return Err(self.spawn_failed(&req.agent.id, &run_id, &token, e)),
            Err(e) => {
                return Err(self.spawn_failed(&req.agent.id, &run_id, &token, anyhow!("the {} adapter crashed: {e}", provider.label())));
            }
        };
        // Written before the handle goes in: an exit handled right after has the last word on the row.
        if let Err(e) = self.store.set_run_status(&run_id, "running") {
            tracing::warn!("run {run_id} not marked running: {e}");
        }
        // Still ours: the real handle. What was sent meanwhile waits for the caller's own prompt
        // (the one this run was started for) and follows it: see `send_prompt`. Gone: the CLI
        // exited during its handshake and `on_exit` took the entry.
        let ours = {
            let mut live = lock(&self.live);
            match live.get_mut(&req.agent.id).filter(|l| l.run_id == run_id) {
                Some(l) => {
                    l.handle = handle;
                    l.waited = l.starting.take().unwrap_or_default();
                    if l.stopping {
                        l.handle.send(AgentCmd::Shutdown); // stopped while it started
                    }
                    true
                }
                None => false,
            }
        };
        if !ours {
            return Err(self.spawn_failed(&req.agent.id, &run_id, &token, anyhow!("{} exited during startup", provider.label())));
        }
        self.restore_permission_mode(&req.agent, req.role, req.read_only); // before the run's first prompt
        Ok(run_id)
    }

    /// A start that failed: its token, its place in the live table and its row. The agent row
    /// only if this run still owns it (not a newer run's, not what `on_exit` already wrote).
    /// Never call it holding the `live` lock.
    fn spawn_failed(&self, agent_id: &str, run_id: &str, token: &str, e: anyhow::Error) -> anyhow::Error {
        if let Some(m) = self.mcp.get() {
            m.revoke(token);
        }
        // The row first: ingest reads "not live" and "failed" as a dead run, so it must never
        // see the first without the second.
        let _ = self.store.set_run_status(run_id, "failed");
        // ponytail: prompts queued behind this start (`starting`) are dropped with it; the first
        // caller's error is the signal. Carry them to the retry if a lost one is ever missed.
        let removed = {
            let mut live = lock(&self.live);
            live.get(agent_id).is_some_and(|l| l.run_id == run_id) && live.remove(agent_id).is_some()
        };
        if removed {
            self.control_clear(agent_id); // controls do not outlive the run
        }
        let _ = self.update_agent(agent_id, |a| {
            if a.run_id.as_deref() == Some(run_id) {
                a.status = AgentStatus::Failed;
                a.run_id = None;
            }
        });
        e
    }

    /// Start the run that goes on with an agent's conversation: `req` with the session to
    /// resume. A conversation that cannot be resumed gives way to a new one (`req` with a new
    /// session), and its thread says so (`resume_failed`). Every path that resumes a
    /// conversation of its own starts here: a 1:1 chat, a Lead, a team member.
    pub(super) async fn resume_or_new(self: &Arc<Self>, session_id: &str, req: impl Fn(SessionMode) -> RunReq) -> Result<Id> {
        let resume = req(SessionMode::Resume { session_id: session_id.to_string() });
        let (agent, role, ws, task_id) = (resume.agent.clone(), resume.role, resume.workspace_id.clone(), resume.task_id.clone());
        let e = match self.spawn_run(resume).await {
            Ok(run_id) => return Ok(run_id),
            Err(e) => e,
        };
        if !self.gives_up_conversation(&agent, &e) {
            return Err(e);
        }
        tracing::warn!("resuming {} failed ({e}); starting a new session", agent.name);
        // The old session stays on the agent until the new one has started: a start that
        // fails here too (the CLI is signed out, say) loses no conversation.
        let run_id = self.spawn_run(req(new_session(agent.provider))).await?;
        self.resume_failed(&agent, role, ws.as_deref(), task_id.as_deref(), &e);
        Ok(run_id)
    }

    /// A resume did not start. May a new conversation take the place of the old one? Yes when
    /// the CLI said that it no longer has it (`SessionGone`). Not when Codly itself did not
    /// start the CLI: the folder is not trusted, the agent is in your terminal, or another
    /// start of this agent won. That conversation is fine, and it is the one to keep. After
    /// any other failure a new one is tried: no other answer of a CLI to a conversation it
    /// cannot open is recorded, and an agent must not stay without one.
    fn gives_up_conversation(&self, agent: &Agent, e: &anyhow::Error) -> bool {
        if e.is::<crate::adapters::SessionGone>() {
            return true;
        }
        // (a terminal mark that cannot be read counts as "in your terminal": nothing is given up)
        !e.is::<super::trust::NotStarted>() && !self.is_live(&agent.id) && !self.in_terminal(&agent.id).unwrap_or(true)
    }

    /// A resume failed and a new session took its place: the agent no longer knows what was
    /// said before. Say so where its conversation is (a Lead's request thread, the agent's
    /// own thread otherwise), with the reason, and record it.
    pub(super) fn resume_failed(&self, agent: &Agent, role: Role, ws: Option<&str>, task_id: Option<&str>, error: &anyhow::Error) {
        let why = super::prompts::clip(&error.to_string().split_whitespace().collect::<Vec<_>>().join(" "), 300);
        let name = &agent.name;
        let text = if error.is::<crate::adapters::SessionGone>() {
            format!("Claude Code no longer has {name}'s earlier conversation. A new one started: {name} does not know what was said before.")
        } else {
            format!("{name}'s earlier conversation could not be resumed ({why}). A new one started: {name} does not know what was said before.")
        };
        let thread = match role {
            Role::Lead => super::lead::thread_of(agent).to_string(),
            _ => format!("agent:{}", agent.id),
        };
        let _ = self.chat(ws, &thread, "codly", &text, None);
        let _ = self.decide(ws, task_id, "retry", text, serde_json::json!({ "agent": agent.id, "error": why }));
    }

    /// Send a user turn to a live agent. ACP adapters queue it behind a running turn (it gets
    /// its own TurnStarted/TurnCompleted); Codex and Claude steer it into the running turn
    /// (PromptSteered), or give it a turn of its own when that turn ends first. Which of the
    /// two happens is the adapter's call, so every prompt counts as queued until one of those
    /// events says where it went.
    pub(super) fn send_prompt(&self, agent_id: &str, text: String) -> Result<()> {
        // A pending "stop after this step" or a pause holds every prompt until the resume.
        let Some(text) = self.route_prompt(agent_id, text) else { return Ok(()) };
        self.deliver_prompt(agent_id, text)
    }

    /// `send_prompt` past the agent's controls: for what it must read although it is held (the
    /// circuit breaker's note to a worker it pauses).
    pub(super) fn deliver_prompt(&self, agent_id: &str, text: String) -> Result<()> {
        let wrapped = self.with_briefing(agent_id, &text); // a hired agent's standing goal, on every prompt
        let wrapped = self.with_task_rules(agent_id, wrapped); // a worker's task again, after a compaction
        let wrapped = self.with_restore_note(agent_id, wrapped); // files you put back since its last turn (checkpoints.rs)
        let mut live = lock(&self.live);
        let l = live.get_mut(agent_id).ok_or_else(|| anyhow!("that agent is not running"))?;
        // The adapters drop prompts during their shutdown ladder: say so instead.
        if l.stopping {
            bail!("that agent is shutting down; send your message again in a moment");
        }
        l.queued += 1;
        l.last_prompt = text.lines().find(|s| !s.trim().is_empty()).map(|s| s.trim().to_string());
        // Still in its handshake: it goes out once the CLI is up, behind the run's first prompt.
        if let Some(waiting) = &mut l.starting {
            waiting.push(wrapped);
            return Ok(());
        }
        if !l.handle.send(AgentCmd::Prompt(wrapped)) {
            l.queued = l.queued.saturating_sub(1);
            bail!("that agent's process is gone");
        }
        // The first prompt once the CLI is up is the one this run was started for: what was sent
        // meanwhile follows it.
        // ponytail: nothing marks that prompt. Another one landing first (no await in between
        // for a chat) takes its place and leads.
        for prompt in std::mem::take(&mut l.waited) {
            l.handle.send(AgentCmd::Prompt(prompt));
        }
        Ok(())
    }

    /// Ask a live run to shut down. Returns false if the agent had no live run.
    pub(super) fn stop_run(&self, agent_id: &str) -> bool {
        self.stop_run_if(agent_id, |_| true)
    }

    /// `stop_run`, but only a run the caller means (`mine`): the agent may have another by now.
    pub(super) fn stop_run_if(&self, agent_id: &str, mine: impl FnOnce(&Live) -> bool) -> bool {
        let mut live = lock(&self.live);
        match live.get_mut(agent_id).filter(|l| mine(l)) {
            Some(l) => {
                l.stopping = true;
                // A run still in its handshake has nobody to tell yet: `spawn_run` shuts it down.
                l.starting.is_some() || l.handle.send(AgentCmd::Shutdown)
            }
            None => false,
        }
    }

    /// Switch a 1:1 agent's model. A live Claude session takes the new model as it is
    /// (`live_model_switch`). Otherwise the session resumes on the next message with the new
    /// model, so a live process is stopped first; a turn in progress must finish or be stopped.
    pub(super) async fn set_agent_model(&self, agent_id: &str, model: Option<String>) -> Result<Agent> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo {
            bail!("only a 1:1 agent's model can be changed");
        }
        if agent.model == model {
            return Ok(agent);
        }
        if let Some(name) = self.live_model_switch(agent_id, model.as_deref()) {
            let answer = self.ask_session(agent_id, SessionRequest::Model(name))?;
            Self::session_answer(answer).await?;
            if let Some(l) = lock(&self.live).get_mut(agent_id) {
                l.model = model.clone();
            }
            let ws = self.store.agent_workspace(agent_id)?;
            // (Recorded: after a switch the next request reads the conversation with no cached tokens.)
            let text = format!(
                "{} now works with {}, in the same conversation: no restart was needed. Its next turn reads the whole conversation again with the new model.",
                agent.name,
                model.as_deref().unwrap_or_default()
            );
            self.chat(ws.as_deref(), &format!("agent:{agent_id}"), "codly", &text, None)?;
            return self.update_agent(agent_id, |a| a.model = model);
        }
        // Its run of a task is not a chat to restart: stopping it would strand the task.
        if agent.task_id.is_some() {
            bail!("{} is on a task: its model can change when the task is done", agent.name);
        }
        if lock(&self.live).get(agent_id).is_some_and(|l| l.turn_active) {
            bail!("{} is still working: stop its turn first", agent.name);
        }
        // The old process must be gone before the next message, or it would reach the old model.
        if !self.stop_and_wait(agent_id).await {
            bail!("{} did not stop in time; try again", agent.name);
        }
        self.update_agent(agent_id, |a| a.model = model)
    }

    /// The name to give a live Claude session so that it changes its model with no restart
    /// (`SessionRequest::Model`: recorded between turns and during one). None = the switch
    /// needs the restart: another CLI, no live process, the CLI's default model (no recorded
    /// request names it), or another reasoning effort (it is set when the process starts).
    fn live_model_switch(&self, agent_id: &str, model: Option<&str>) -> Option<String> {
        let (name, effort) = classify::split_model_effort(model);
        let live = lock(&self.live);
        let l = live.get(agent_id).filter(|l| l.provider == Provider::Claude && l.starting.is_none() && !l.stopping)?;
        name.filter(|_| classify::split_model_effort(l.model.as_deref()).1 == effort)
    }

    /// Stop an agent's live run and wait (≤ 5 s) for its process to exit. False = still running.
    pub(super) async fn stop_and_wait(&self, agent_id: &str) -> bool {
        // A run that was told to stop before has closed its inbox while its process exits:
        // the request does not reach it, and it is waited for all the same.
        self.stop_run(agent_id);
        for _ in 0..100 {
            if !self.is_live(agent_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        !self.is_live(agent_id)
    }

    /// A live agent that is between turns with nothing queued.
    pub(super) fn is_quiet(&self, agent_id: &str) -> bool {
        lock(&self.live).get(agent_id).is_none_or(|l| !l.turn_active && l.queued == 0)
    }

    /// Turns the agent's live run has completed (None if not running).
    pub(super) fn turns_done(&self, agent_id: &str) -> Option<u64> {
        lock(&self.live).get(agent_id).map(|l| l.turns)
    }

    /// Last assistant message of the agent's current/last turn.
    pub(super) fn last_text(&self, agent_id: &str) -> Option<String> {
        lock(&self.live).get(agent_id).and_then(|l| l.last_text.clone())
    }

    pub(super) fn cli_usable(&self, p: Provider) -> bool {
        self.provider_usable(p, false)
    }

    /// The account a run goes on, checked: Some = apply its home to the child env; None =
    /// no account detected for the CLI (its login shell's sign-in, env untouched).
    fn run_account(&self, provider: Provider, id: Option<&str>, cli: &CliInfo) -> Result<Option<AccountInfo>> {
        let label = provider.label();
        let listed = read(&self.accounts).iter().any(|a| a.provider == provider);
        let Some(id) = id else {
            if cli.logged_in == Some(false) {
                bail!("{label} is not signed in{}", signed_out_fix(cli.problem.as_deref(), None));
            }
            return Ok(None);
        };
        match self.account(id) {
            Some(a) if a.provider != provider => bail!("{id} is not a {label} account"),
            Some(a) if !a.enabled => bail!("the {label} account '{}' is turned off in Settings", a.label),
            Some(a) if a.logged_in == Some(false) => {
                bail!("the {label} account '{}' is not signed in{}", a.label, signed_out_fix(a.problem.as_deref(), a.login_command.as_deref()))
            }
            Some(a) => Ok(Some(a)),
            None if listed => bail!("the {label} account {id} is no longer available"),
            // No account detected for the CLI: the scheduler's implicit one is the login
            // shell's sign-in (env untouched); a recorded one has its home in its id.
            None if id == provider.as_str() => self.run_account(provider, None, cli),
            None => Ok(crate::env::account_home(provider, id).map(|home| AccountInfo {
                id: id.to_string(),
                provider,
                label: crate::env::account_label(provider, home.as_deref()),
                home,
                enabled: true,
                primary: false,
                configured: false,
                logged_in: cli.logged_in,
                billing: Billing::Unknown,
                plan: None,
                identity: None,
                problem: None,
                login_command: None,
                limits: vec![],
            })),
        }
    }
}

/// What follows "… is not signed in" in a start error: what to do about it. `problem` is the
/// sign-in check's own line (env.rs: "not signed in: run `…` in a terminal", or what to do
/// when your shell selects the sign-in), which knows more than the login command alone.
fn signed_out_fix(problem: Option<&str>, login_command: Option<&str>) -> String {
    match (problem, login_command) {
        (Some(p), _) => format!(": {}", p.strip_prefix("not signed in: ").unwrap_or(p)),
        (None, Some(cmd)) => format!(": run `{cmd}` in a terminal"),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::signed_out_fix;

    #[test]
    fn a_signed_out_start_says_what_the_sign_in_check_found() {
        let login = Some("claude auth login");
        assert_eq!(signed_out_fix(Some("not signed in: run `claude auth login` in a terminal"), login), ": run `claude auth login` in a terminal");
        // Your shell selects the sign-in (Bedrock, Vertex or a token): the check's advice, not the login command.
        let shell = "not signed in: your shell sets CLAUDE_CODE_USE_BEDROCK, and Codly takes that out of an agent's environment. \
                     Turn on \"Use my shell's Claude sign-in\" in Settings → Safety, or run `claude auth login` in a terminal";
        assert_eq!(signed_out_fix(Some(shell), login), format!(": {}", &shell["not signed in: ".len()..]));
        assert_eq!(signed_out_fix(Some("the sign-in check timed out"), login), ": the sign-in check timed out");
        assert_eq!(signed_out_fix(None, login), ": run `claude auth login` in a terminal");
        assert_eq!(signed_out_fix(None, None), "");
    }
}
