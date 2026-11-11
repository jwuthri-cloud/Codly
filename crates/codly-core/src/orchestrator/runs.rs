//! Agent runs: one CLI process lifetime per run (`run` table), spawned through the
//! `Spawner` seam with a per-run MCP token and a raw log, plus the live-run table the
//! ingest loop and the gates consult.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;

use super::{Inner, Live, guard, lock, read};
use crate::adapters::{AgentCmd, AgentHandle, SessionMode, SpawnSpec};
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
        let provider = req.agent.provider;
        let cli = self.clis().into_iter().find(|c| c.provider == provider);
        let Some(cli) = cli.filter(|c| c.path.is_some()) else { bail!("{} is not installed", provider.label()) };
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
        let byok_key = self.byok_key_for(provider).await;
        let mut env = guard("env::child_env", || {
            Ok(crate::env::child_env_with(&self.base_env, provider, byok_key.as_ref().map(|k| k.expose())))
        })
        .unwrap_or_else(|e| {
            tracing::debug!("{e}; using the base env");
            self.base_env.clone()
        });
        // The sign-in: CLAUDE_CONFIG_DIR / CODEX_HOME / GEMINI_CLI_HOME of that account.
        if let Some(acct) = &info {
            crate::env::apply_account(&mut env, provider, acct.home.as_deref());
        }
        let mcp = self.mcp.get().ok_or_else(|| anyhow!("the Codly MCP server is not running"))?.clone();
        let run_id = new_id();
        let dir = self.data("runs").join(&run_id);
        // Owner-only: the run dir holds the MCP config with this run's token and the raw log.
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(&dir)?;
        let raw_log = dir.join("raw.ndjson");
        self.prepare_default_mcp(&raw_log, &req.cwd);
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
                },
            );
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
            system_prompt: req.system_prompt,
            read_only: req.read_only,
            mcp: Some(mcp.launch(&self.cfg.app_exe, &token)),
            use_user_setup: self.settings().use_my_claude_setup,
            raw_log,
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

    /// Send a user turn to a live agent. Claude and ACP adapters queue it behind a running
    /// turn (it gets its own TurnStarted/TurnCompleted); Codex steers it into the running turn.
    pub(super) fn send_prompt(&self, agent_id: &str, text: String) -> Result<()> {
        // A pending "stop after this step" or a pause holds every prompt until the resume.
        let Some(text) = self.route_prompt(agent_id, text) else { return Ok(()) };
        let wrapped = self.with_briefing(agent_id, &text); // a hired agent's standing goal, on every prompt
        let mut live = lock(&self.live);
        let l = live.get_mut(agent_id).ok_or_else(|| anyhow!("that agent is not running"))?;
        // The adapters drop prompts during their shutdown ladder: say so instead.
        if l.stopping {
            bail!("that agent is shutting down; send your message again in a moment");
        }
        let steer = l.provider == Provider::Codex && l.turn_active;
        if !steer {
            l.queued += 1;
        }
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

    /// Would a prompt sent now be steered into the running turn (no turn of its own)?
    pub(super) fn would_steer(&self, agent_id: &str) -> bool {
        lock(&self.live).get(agent_id).is_some_and(|l| l.provider == Provider::Codex && l.turn_active)
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

    /// Switch a 1:1 agent's model. Its session resumes on the next message with the new model,
    /// so a live process is stopped first; a turn in progress must finish or be stopped.
    pub(super) async fn set_agent_model(&self, agent_id: &str, model: Option<String>) -> Result<Agent> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo {
            bail!("only a 1:1 agent's model can be changed");
        }
        if agent.model == model {
            return Ok(agent);
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

    /// Stop an agent's live run and wait (≤ 5 s) for its process to exit. False = still running.
    pub(super) async fn stop_and_wait(&self, agent_id: &str) -> bool {
        if self.stop_run(agent_id) {
            for _ in 0..100 {
                if !self.is_live(agent_id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
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
                bail!("{label} is not signed in{}", cli.problem.as_deref().map(|p| format!(": {p}")).unwrap_or_default());
            }
            return Ok(None);
        };
        match self.account(id) {
            Some(a) if a.provider != provider => bail!("{id} is not a {label} account"),
            Some(a) if !a.enabled => bail!("the {label} account '{}' is turned off in Settings", a.label),
            Some(a) if a.logged_in == Some(false) => {
                let cmd = a.login_command.as_deref().map(|c| format!(": run `{c}` in a terminal")).unwrap_or_default();
                bail!("the {label} account '{}' is not signed in{cmd}", a.label)
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
