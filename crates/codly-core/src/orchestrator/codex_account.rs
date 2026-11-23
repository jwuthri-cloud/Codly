//! What Codly asks a Codex sign-in's app-server outside a run (`adapters/codex/account.rs`):
//! the models it offers with their thinking efforts, the account's usage windows, and a
//! compaction of a long conversation. Also the model catalog of the other CLIs, which have no
//! such question (`env::list_models`).
//!
//! Pure, unit-tested policy: `codex_catalog` (the CLI's list, or Codly's built-in one with the
//! reason), `limit_targets` (whose usage is read) and `compact_refusal` (when a compaction may
//! not start).

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::json;

use super::{Inner, lock, row};
use crate::adapters::codex::{self, AccountQuery};
use crate::model::*;

/// How long one question to an app-server may take, a server start included.
const ASK_TIMEOUT: Duration = Duration::from_secs(15);

/// The folder under `runs/` whose `raw.ndjson` logs these questions and their answers: one per
/// start of Codly. A server that a question started also logs there what belongs to no agent,
/// for as long as it runs, so the log must end with the app like a run's log ends with its
/// run. The storage policy removes it like a run's: when nothing was written to it for its days.
fn query_log() -> &'static str {
    static FOLDER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FOLDER.get_or_init(|| format!("codex-account-{}", new_id()))
}

/// Codex models offered when its app-server cannot be asked: the listed ones of
/// docs/research/cli-claude-codex.md §3.1, with the efforts Codly offered for Codex before it
/// read them from the server. None is marked as the default: only the CLI knows it.
const CODEX_BUILTIN: [&str; 4] = ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna"];
const CODEX_BUILTIN_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// The Codex catalog to show: the server's own list, or the built-in one with why the server
/// could not be asked.
pub fn codex_catalog(asked: Result<Vec<ModelOption>>) -> ModelCatalog {
    match asked {
        Ok(models) => ModelCatalog { models, source: ModelSource::Cli, problem: None },
        Err(e) => {
            let efforts: Vec<String> = CODEX_BUILTIN_EFFORTS.map(String::from).to_vec();
            let models = CODEX_BUILTIN.iter().map(|id| ModelOption { efforts: efforts.clone(), ..ModelOption::named(*id) }).collect();
            ModelCatalog { models, source: ModelSource::BuiltIn, problem: Some(format!("{e:#}")) }
        }
    }
}

/// A sign-in whose usage was reported this recently is not asked again: its agents report it
/// after every model response, and the agent panel refreshes the accounts each time it opens.
const LIMITS_FRESH_MS: i64 = 60_000;

/// The Codex sign-ins whose usage Codly reads now: those it may run agents on, that are
/// signed in, and that have no reading younger than `LIMITS_FRESH_MS`. None when the agents
/// pay with your own API credentials (`own_key`, BYOK): the windows are the ChatGPT plan's,
/// and the agents do not draw from them.
pub fn limit_targets(accounts: &[AccountInfo], own_key: bool, now: i64) -> Vec<AccountInfo> {
    if own_key {
        return Vec::new();
    }
    let fresh = |a: &AccountInfo| a.limits.iter().any(|w| now - w.seen_at < LIMITS_FRESH_MS);
    accounts.iter().filter(|a| a.provider == Provider::Codex && a.enabled && a.logged_in == Some(true) && !fresh(a)).cloned().collect()
}

/// An agent's live run, as far as a compaction cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunNow {
    /// A turn runs, a prompt waits for one, or the CLI still starts.
    pub busy: bool,
    /// The run works on a task: the gate checks the task when one of its turns ends.
    pub on_task: bool,
}

/// Why the conversation of `name`, a Codex agent, cannot be compacted now, or None. Codex compacts in a turn of its
/// own, which must not run into the agent's work. This is the first look: a turn can start
/// right after it, so the agent's adapter has the last word and keeps that turn out of the
/// agent's own turns (`codex.rs`, `Compaction`). A run on a task is still refused, so that a
/// compaction Codex reports in another way than expected cannot send a worker to the gate.
pub fn compact_refusal(name: &str, run: Option<RunNow>) -> Option<String> {
    match run {
        None => Some(format!("{name} is not running")),
        Some(r) if r.busy => Some(format!("{name} is working. Compact it when its turn ends.")),
        Some(r) if r.on_task => Some(format!("{name} is on a task. Compact it between tasks.")),
        Some(_) => None,
    }
}

/// Your own API credentials for Codex (BYOK), as `Inner::byok_for` gives them: a saved key,
/// or the command that mints a token.
type CodexByok = (Option<crate::inbound::secrets::Secret>, Option<String>);

impl Inner {
    /// What a question to the app-server of one Codex sign-in needs: the binary, env and BYOK
    /// command `spawn_run` gives that sign-in's agents. `account` None = no account detected
    /// for Codex (the login shell's sign-in).
    fn codex_query(&self, account: Option<&AccountInfo>, (key, byok_command): &CodexByok) -> Result<AccountQuery> {
        // Tests fix the CLI list and run no real CLI.
        if self.fixed_clis {
            bail!("the CLI list is fixed, so no CLI is asked");
        }
        let cli_path = self.clis().into_iter().find(|c| c.provider == Provider::Codex).and_then(|c| c.path);
        let bin = crate::env::resolve_bin(Provider::Codex, &self.base_env)
            .or_else(|| cli_path.map(std::path::PathBuf::from))
            .ok_or_else(|| anyhow!("Codex is not installed"))?;
        let mut env = crate::env::child_env_with(&self.base_env, Provider::Codex, key.as_ref().map(|k| k.expose()));
        if let Some(a) = account {
            crate::env::apply_account(&mut env, Provider::Codex, a.home.as_deref());
        }
        let raw_log = self.data("runs").join(query_log()).join("raw.ndjson");
        Ok(AccountQuery { bin, env, byok_command: byok_command.clone(), raw_log })
    }

    /// The models Codex's own picker lists, asked from the app-server of the sign-in a new
    /// session would run on.
    async fn codex_models(&self) -> Result<Vec<ModelOption>> {
        let account = self.choose_account(Provider::Codex, self.settings().account_strategy).and_then(|id| self.account(&id));
        let q = self.codex_query(account.as_ref(), &self.byok_for(Provider::Codex).await?)?;
        ask(async move { codex::models(&q).await }).await
    }

    /// The model names a CLI itself lists (empty when it has no list), for the checks that
    /// refuse a name the CLI does not know. Never Codly's built-in fallback.
    pub(super) async fn cli_model_names(&self, provider: Provider) -> Result<Vec<String>> {
        if provider == Provider::Codex {
            return Ok(self.codex_models().await?.into_iter().map(|m| m.id).collect());
        }
        let key = self.byok_for(provider).await.ok().and_then(|(key, _)| key);
        crate::env::list_models(provider, &self.base_env, key.as_ref().map(|k| k.expose())).await.map_err(|e| anyhow!(e))
    }

    /// The models of a CLI for a picker, with where the list comes from. Codex: its
    /// app-server's list with each model's efforts, or Codly's built-in list and the reason
    /// when the server cannot be asked. Claude: its aliases (built in; it has no list).
    /// OpenCode: what `opencode models` prints. The others list nothing.
    async fn model_catalog(&self, provider: Provider) -> Result<ModelCatalog> {
        if provider == Provider::Codex {
            return Ok(codex_catalog(self.codex_models().await));
        }
        let models = self.cli_model_names(provider).await?.into_iter().map(ModelOption::named).collect();
        let source = if provider == Provider::Opencode { ModelSource::Cli } else { ModelSource::BuiltIn };
        Ok(ModelCatalog { models, source, problem: None })
    }

    /// Ask each Codex sign-in's app-server for the account's usage windows (what Codex's
    /// `/status` shows), so a sign-in no agent ran on since Codly started has its real usage,
    /// and one that recovered is not left parked on an old reading. Codex reads them with its
    /// own sign-in: Codly sees no credential. A sign-in whose server does not run gets one
    /// (it stays for its next agent). Whose usage is read: `limit_targets`.
    pub(super) async fn pull_codex_limits(&self) {
        if self.fixed_clis || !self.codex_usage_reads.load(Ordering::Relaxed) {
            return;
        }
        let byok = match self.byok_for(Provider::Codex).await {
            Ok(byok) => byok,
            Err(e) => {
                tracing::warn!("Codex usage not read: {e:#}");
                return;
            }
        };
        let own_key = byok.0.is_some() || byok.1.is_some();
        let mut set = tokio::task::JoinSet::new();
        for account in limit_targets(&self.accounts(), own_key, now_ms()) {
            let q = match self.codex_query(Some(&account), &byok) {
                Ok(q) => q,
                Err(e) => {
                    tracing::warn!("Codex usage not read: {e:#}");
                    return;
                }
            };
            set.spawn(async move { (account.id, ask(async move { codex::rate_limits(&q).await }).await) });
        }
        for (id, readings) in set.join_all().await {
            match readings {
                Ok(readings) => {
                    for r in readings {
                        self.record_usage(&id, &r.window, Some(r.used_fraction), r.resets_at, r.status.as_deref());
                    }
                }
                Err(e) => tracing::warn!("usage of the Codex sign-in {id} not read: {e:#}"),
            }
        }
        self.kick();
    }

    /// Ask Codex to compact an agent's conversation now (`thread/compact/start`). Only between
    /// turns and off a task (`compact_refusal`, then the agent's adapter: a prompt can reach
    /// the agent while this runs). Ok = Codex took the request; the compaction shows in the
    /// agent's history as Codex reports it.
    pub(super) async fn compact_codex(&self, agent_id: &str) -> Result<()> {
        let agent = self.agent(agent_id).map_err(|_| anyhow!("unknown agent"))?;
        let run = lock(&self.live).get(agent_id).map(|l| {
            let now = RunNow { busy: l.turn_active || l.queued > 0 || l.starting.is_some(), on_task: l.task_id.is_some() };
            (now, l.account.clone(), l.workspace_id.clone())
        });
        if let Some(why) = compact_refusal(&agent.name, run.as_ref().map(|r| r.0)) {
            bail!(why);
        }
        let (_, account, ws) = run.ok_or_else(|| anyhow!("{} is not running", agent.name))?;
        let Some(thread) = agent.session_id.clone() else { bail!("{} has no conversation to compact yet", agent.name) };
        let account = account.and_then(|id| self.account(&id));
        let q = self.codex_query(account.as_ref(), &self.byok_for(Provider::Codex).await?)?;
        ask(async move { codex::compact_thread(&q, &thread).await }).await.map_err(|e| anyhow!("{} was not compacted: {e:#}", agent.name))?;
        // The History shows who asked; what Codex then does is in its own events.
        let kind = EventKind::Decision {
            id: new_id(),
            decision_type: "agent_control".into(),
            method: "human".into(),
            summary: format!("you asked Codex to compact {}’s conversation", agent.name),
            detail: json!({ "agent_id": agent_id }),
        };
        self.emit(ws.as_deref(), row(Some(agent_id), None, None, kind))?;
        Ok(())
    }
}

/// Run one question in a task of its own and wait `ASK_TIMEOUT` for it. A server start that
/// outlasts the wait still ends in its task, so the next question finds the server instead of
/// a half-started one.
async fn ask<T: Send + 'static>(question: impl std::future::Future<Output = Result<T>> + Send + 'static) -> Result<T> {
    match tokio::time::timeout(ASK_TIMEOUT, tokio::spawn(question)).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(e)) => Err(anyhow!("the question to Codex stopped: {e}")),
        Err(_) => Err(anyhow!("Codex did not answer within {} s", ASK_TIMEOUT.as_secs())),
    }
}

impl super::Core {
    /// The models of a CLI for the hire wizard and the model dialog, with each model's thinking
    /// efforts where the CLI gives them and where the list comes from (`ModelCatalog::source`).
    pub async fn model_catalog(&self, provider: Provider) -> Result<ModelCatalog> {
        self.inner.model_catalog(provider).await
    }

    /// Model names for a picker that shows no source (empty when the CLI takes any name): the
    /// CLI's own list, or the error. Never Codly's built-in Codex list: only `model_catalog`
    /// offers it, with the reason.
    pub async fn list_models(&self, provider: Provider) -> Result<Vec<String>> {
        self.inner.cli_model_names(provider).await
    }

    /// Switch the usage read of `accounts(refresh)` on or off (on at start). The read starts
    /// each Codex sign-in's app-server, which asks OpenAI with that sign-in: a test that runs
    /// on a machine with a real Codex switches it off.
    pub fn set_codex_usage_reads(&self, on: bool) {
        self.inner.codex_usage_reads.store(on, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_says_whether_the_cli_or_codly_listed_the_models() {
        let sol = ModelOption { id: "gpt-6.1-sol".into(), name: "GPT-6.1 Sol".into(), is_default: true, efforts: vec!["low".into()], default_effort: Some("low".into()) };
        let asked = codex_catalog(Ok(vec![sol.clone()]));
        assert_eq!(asked, ModelCatalog { models: vec![sol], source: ModelSource::Cli, problem: None });

        let fallback = codex_catalog(Err(anyhow!("Codex is not signed in").context("model/list")));
        assert_eq!(fallback.source, ModelSource::BuiltIn);
        assert_eq!(fallback.problem.as_deref(), Some("model/list: Codex is not signed in"), "the reason is shown, not hidden");
        assert_eq!(fallback.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), CODEX_BUILTIN);
        assert!(fallback.models.iter().all(|m| m.efforts == CODEX_BUILTIN_EFFORTS && !m.is_default && m.default_effort.is_none()));
    }

    fn account(id: &str, provider: Provider, enabled: bool, logged_in: Option<bool>) -> AccountInfo {
        AccountInfo {
            id: id.into(),
            provider,
            home: None,
            label: id.into(),
            enabled,
            primary: false,
            configured: true,
            logged_in,
            billing: Billing::Subscription,
            plan: None,
            identity: None,
            problem: None,
            login_command: None,
            limits: vec![],
        }
    }

    #[test]
    fn usage_is_read_for_codex_sign_ins_codly_may_use() {
        let accounts = [
            account("codex", Provider::Codex, true, Some(true)),
            account("codex:off", Provider::Codex, false, Some(true)),
            account("codex:out", Provider::Codex, true, Some(false)),
            account("codex:unknown", Provider::Codex, true, None),
            account("claude", Provider::Claude, true, Some(true)),
        ];
        let now = 1_800_000_000_000;
        let ids = |accounts: &[AccountInfo]| -> Vec<String> { limit_targets(accounts, false, now).into_iter().map(|a| a.id).collect() };
        assert_eq!(ids(&accounts), ["codex"], "never a sign-in you switched off, one that is signed out or unchecked, or another CLI's");
        // your own API key pays (BYOK): the plan's windows are not the agents', and nothing is read
        assert!(limit_targets(&accounts, true, now).is_empty());

        // a reading from the last minute is not asked for again; an older one is
        let seen = |ago_ms: i64| UsageWindow { window: "five_hour".into(), used_fraction: 0.2, resets_at: None, status: None, seen_at: now - ago_ms };
        let mut read = account("codex", Provider::Codex, true, Some(true));
        read.limits = vec![seen(59_000)];
        assert!(ids(&[read.clone()]).is_empty());
        read.limits = vec![seen(61_000)];
        assert_eq!(ids(&[read]), ["codex"]);
    }

    #[test]
    fn a_compaction_starts_only_between_turns_and_off_a_task() {
        let idle = RunNow { busy: false, on_task: false };
        assert_eq!(compact_refusal("Ada", Some(idle)), None);
        assert_eq!(compact_refusal("Ada", None).as_deref(), Some("Ada is not running"));
        assert_eq!(
            compact_refusal("Ada", Some(RunNow { busy: true, on_task: false })).as_deref(),
            Some("Ada is working. Compact it when its turn ends.")
        );
        // the turn Codex compacts in would end like a worker's turn, and the gate would check it
        assert_eq!(compact_refusal("Ada", Some(RunNow { busy: false, on_task: true })).as_deref(), Some("Ada is on a task. Compact it between tasks."));
    }
}
