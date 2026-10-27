//! Agent runs: one CLI process lifetime per run (`run` table), spawned through the
//! `Spawner` seam with a per-run MCP token and a raw log, plus the live-run table the
//! ingest loop and the gates consult.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::{Inner, Live, guard, lock, read};
use crate::adapters::{AgentCmd, SessionMode, SpawnSpec};
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
        let mut env = guard("env::child_env", || Ok(crate::env::child_env(&self.base_env, provider))).unwrap_or_else(|e| {
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
        let token =
            mcp.register(ToolCtx { agent_id: req.agent.id.clone(), run_id: run_id.clone(), task_id: req.task_id.clone(), role: req.role });
        let session_id = match &req.session {
            SessionMode::New { session_id } => session_id.clone(),
            SessionMode::Resume { session_id } => Some(session_id.clone()),
        };
        self.store.insert_run(&RunRow {
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
        })?;
        let worktree = matches!(req.role, Role::Worker | Role::Solo).then(|| req.cwd.to_string_lossy().into_owned());
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
        })?;
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
        lock(&self.live).insert(
            req.agent.id.clone(),
            Live {
                handle,
                run_id: run_id.clone(),
                role: req.role,
                provider,
                model: req.agent.model.clone(),
                account,
                task_id: req.task_id,
                workspace_id: req.workspace_id,
                cwd: req.cwd,
                read_only: req.read_only,
                token,
                turn_active: false,
                queued: 0,
                stopping: false,
                turns: 0,
                last_text: None,
                turn_messages: 0,
                last_prompt: None,
                answers: vec![],
                answer_only: false,
            },
        );
        self.store.set_run_status(&run_id, "running")?;
        Ok(run_id)
    }

    fn spawn_failed(&self, agent_id: &str, run_id: &str, token: &str, e: anyhow::Error) -> anyhow::Error {
        if let Some(m) = self.mcp.get() {
            m.revoke(token);
        }
        let _ = self.store.set_run_status(run_id, "failed");
        let _ = self.update_agent(agent_id, |a| {
            a.status = AgentStatus::Failed;
            a.run_id = None;
        });
        e
    }

    /// Send a user turn to a live agent. Claude and ACP adapters queue it behind a running
    /// turn (it gets its own TurnStarted/TurnCompleted); Codex steers it into the running turn.
    pub(super) fn send_prompt(&self, agent_id: &str, text: String) -> Result<()> {
        let wrapped = self.with_briefing(agent_id, &text); // a hired agent's standing goal, on every prompt
        let mut live = lock(&self.live);
        let l = live.get_mut(agent_id).ok_or_else(|| anyhow!("that agent is not running"))?;
        let steer = l.provider == Provider::Codex && l.turn_active;
        if !steer {
            l.queued += 1;
        }
        l.last_prompt = text.lines().find(|s| !s.trim().is_empty()).map(|s| s.trim().to_string());
        if !l.handle.send(AgentCmd::Prompt(wrapped)) {
            l.queued = l.queued.saturating_sub(1);
            bail!("that agent's process is gone");
        }
        Ok(())
    }

    /// Would a prompt sent now be steered into the running turn (no turn of its own)?
    pub(super) fn would_steer(&self, agent_id: &str) -> bool {
        lock(&self.live).get(agent_id).is_some_and(|l| l.provider == Provider::Codex && l.turn_active)
    }

    /// Ask a live run to shut down. Returns false if the agent had no live run.
    pub(super) fn stop_run(&self, agent_id: &str) -> bool {
        let mut live = lock(&self.live);
        match live.get_mut(agent_id) {
            Some(l) => {
                l.stopping = true;
                l.handle.send(AgentCmd::Shutdown)
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
