//! Opt-in "bring your own key": your own API credentials for one vendor, handed to the one CLI
//! that speaks to it (Anthropic → Claude Code, OpenAI → Codex, OpenRouter → OpenCode).
//!
//! Codly is subscription-only by default: `env::child_env` scrubs every API-key variable so an
//! agent can never silently bill an API account. This is the deliberate exception, and it is
//! narrow on purpose:
//! - each vendor is off until the user picks a source for it (`Settings.byok`);
//! - a source is a saved key, or a command that prints a short-lived token (workload identity
//!   federation and other token brokers); the CLI re-runs the command when its token expires
//!   (Claude `apiKeyHelper`, Codex `[model_providers.<id>.auth]`);
//! - a vendor's credential reaches its own CLI only, never another one;
//! - a key lives in the macOS Keychain (service `dev.codly.byok`) or, on Linux, an owner-only
//!   file; never in the settings, the SQLite DB, a log line or an event, and `ByokStatus` has no
//!   field that could carry it. A command is not a secret: it lives in the settings.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::inbound::secrets::{Keychain, MemorySecrets, Secret, SecretStore};
use crate::model::{Billing, CliInfo, Provider};

/// Keychain service (macOS) and file name (Linux) of the keys.
pub const SERVICE: &str = "dev.codly.byok";
const MAX_KEY_LEN: usize = 256;
const MAX_COMMAND_LEN: usize = 2048;
/// How long a token command may run before the spawn gives up on it.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// The API vendors a user can bring credentials for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Vendor {
    Anthropic,
    Openai,
    Openrouter,
}

impl Vendor {
    pub const ALL: [Vendor; 3] = [Vendor::Anthropic, Vendor::Openai, Vendor::Openrouter];

    /// The one CLI that gets this vendor's credential.
    pub fn cli(self) -> Provider {
        match self {
            Vendor::Anthropic => Provider::Claude,
            Vendor::Openai => Provider::Codex,
            Vendor::Openrouter => Provider::Opencode,
        }
    }

    /// The vendor whose credential a CLI may get, if any.
    pub fn of(provider: Provider) -> Option<Vendor> {
        Vendor::ALL.into_iter().find(|v| v.cli() == provider)
    }

    /// The variable the CLI reads the key from.
    pub fn env_var(self) -> &'static str {
        match self {
            Vendor::Anthropic => "ANTHROPIC_API_KEY",
            Vendor::Openai => "OPENAI_API_KEY",
            Vendor::Openrouter => "OPENROUTER_API_KEY",
        }
    }

    /// Keychain account / file entry of the key. OpenRouter's predates the others.
    fn account(self) -> &'static str {
        match self {
            Vendor::Anthropic => "anthropic-api-key",
            Vendor::Openai => "openai-api-key",
            Vendor::Openrouter => "openrouter-api-key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Vendor::Anthropic => "Anthropic",
            Vendor::Openai => "OpenAI",
            Vendor::Openrouter => "OpenRouter",
        }
    }
}

/// Where a vendor's credential comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ByokSource {
    /// Subscription only: the CLI uses its own sign-in.
    #[default]
    Off,
    /// The key saved for this vendor.
    Key,
    /// A shell command that prints a token on stdout (e.g. a workload identity exchange).
    Command { command: String },
}

/// `Settings.byok`: one source per vendor, all off by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct ByokSettings {
    pub anthropic: ByokSource,
    pub openai: ByokSource,
    pub openrouter: ByokSource,
}

impl ByokSettings {
    pub fn get(&self, v: Vendor) -> &ByokSource {
        match v {
            Vendor::Anthropic => &self.anthropic,
            Vendor::Openai => &self.openai,
            Vendor::Openrouter => &self.openrouter,
        }
    }

    pub fn set(&mut self, v: Vendor, source: ByokSource) {
        match v {
            Vendor::Anthropic => self.anthropic = source,
            Vendor::Openai => self.openai = source,
            Vendor::Openrouter => self.openrouter = source,
        }
    }
}

/// What the UI sees for one vendor. There is deliberately no field that can carry the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VendorStatus {
    pub vendor: Vendor,
    pub source: ByokSource,
    /// A key is saved; its value never comes back.
    pub key_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ByokStatus {
    pub vendors: Vec<VendorStatus>,
}

/// What a CLI gets for one spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// Goes into the vendor's env variable.
    Key(Secret),
    /// The CLI runs it itself and re-runs it when the token expires (Claude, Codex). For a CLI
    /// that cannot (OpenCode), the spawner runs it once with `mint` and passes a `Key`.
    Command(String),
}

/// The key store plus a read cache (agents are spawned often; the Keychain is a subprocess).
pub struct Byok {
    store: Box<dyn SecretStore>,
    cache: Mutex<HashMap<Vendor, Option<Secret>>>,
}

impl Byok {
    pub fn new(store: Box<dyn SecretStore>) -> Self {
        Byok { store, cache: Mutex::new(HashMap::new()) }
    }

    /// The Keychain on macOS, an owner-only file elsewhere.
    pub fn system() -> Self {
        Self::new(if cfg!(target_os = "macos") {
            Box::new(Keychain::new(SERVICE))
        } else if cfg!(unix) {
            Box::new(crate::inbound::secrets::file_store(SERVICE))
        } else {
            Box::new(MemorySecrets::default())
        })
    }

    /// Blocking (the Keychain is a subprocess): call from `spawn_blocking` in async code.
    pub fn key(&self, v: Vendor) -> Option<Secret> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.get(&v) {
            return hit.clone();
        }
        let key = self.store.get(v.account()).ok()?.map(Secret::new); // an unreadable store is not cached
        cache.insert(v, key.clone());
        key
    }

    /// What the vendor's CLI gets under `settings`: None while off, or when `Key` has no key.
    pub fn credential(&self, settings: &ByokSettings, v: Vendor) -> Option<Credential> {
        match settings.get(v) {
            ByokSource::Off => None,
            ByokSource::Key => self.key(v).map(Credential::Key),
            ByokSource::Command { command } => Some(Credential::Command(command.clone())),
        }
    }

    /// The vendors whose credential is in use (the billing badges).
    pub fn active(&self, settings: &ByokSettings) -> Vec<Vendor> {
        Vendor::ALL.into_iter().filter(|v| self.credential(settings, *v).is_some()).collect()
    }

    pub fn status(&self, settings: &ByokSettings) -> ByokStatus {
        ByokStatus {
            vendors: Vendor::ALL
                .into_iter()
                .map(|v| VendorStatus { vendor: v, source: settings.get(v).clone(), key_saved: self.key(v).is_some() })
                .collect(),
        }
    }

    pub fn save_key(&self, v: Vendor, raw: &str) -> Result<()> {
        let key = validate_key(raw)?;
        self.store.set(v.account(), &key)?;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(v, Some(Secret::new(key)));
        Ok(())
    }

    pub fn clear_key(&self, v: Vendor) -> Result<()> {
        self.store.delete(v.account())?;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(v, None);
        Ok(())
    }
}

/// The process-wide store, shared by the commands and the agent spawner.
pub fn global() -> &'static Byok {
    static G: OnceLock<Byok> = OnceLock::new();
    G.get_or_init(Byok::system)
}

/// A pasted key: trimmed, non-empty, at most 256 chars, plain characters only (the Keychain
/// helper reads its commands as space-separated words, and a key has no business with quotes).
pub fn validate_key(raw: &str) -> Result<String> {
    let key = raw.trim();
    if key.is_empty() {
        bail!("the key is empty");
    }
    if key.len() > MAX_KEY_LEN {
        bail!("the key is longer than {MAX_KEY_LEN} characters");
    }
    if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
        bail!("the key may only contain letters, digits, - _ and .");
    }
    Ok(key.to_owned())
}

/// A source the user picked: `Key` needs a saved key; a command is one non-empty line.
pub fn validate_source(source: ByokSource, key_saved: bool) -> Result<ByokSource> {
    Ok(match source {
        ByokSource::Key if !key_saved => bail!("save a key first"),
        ByokSource::Command { command } => {
            let command = command.trim().to_owned();
            if command.is_empty() {
                bail!("the command is empty");
            }
            if command.len() > MAX_COMMAND_LEN {
                bail!("the command is longer than {MAX_COMMAND_LEN} characters");
            }
            if command.contains(['\n', '\r', '\0']) {
                bail!("the command must be one line");
            }
            ByokSource::Command { command }
        }
        other => other,
    })
}

/// Runs a token command once (`sh -c`, in the agents' base env, no stdin) and returns its
/// output, trimmed. For CLIs that cannot run the command themselves. The token is never logged;
/// a failure reports the exit code and the start of stderr only.
pub async fn mint(command: &str, env: &HashMap<String, String>) -> Result<Secret> {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.args(["-c", command])
        .env_clear()
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(COMMAND_TIMEOUT, cmd.output())
        .await
        .with_context(|| format!("the token command did not finish within {} s", COMMAND_TIMEOUT.as_secs()))?
        .context("the token command could not start")?;
    if !out.status.success() {
        let err: String = String::from_utf8_lossy(&out.stderr).trim().chars().take(200).collect();
        bail!("the token command failed ({}): {err}", out.status);
    }
    let token = String::from_utf8(out.stdout).context("the token command printed non-UTF-8 output")?.trim().to_owned();
    if token.is_empty() || token.contains(char::is_whitespace) {
        bail!("the token command must print one token and nothing else");
    }
    Ok(Secret::new(token))
}

/// The billing badge of each CLI whose vendor credential is in use. Other CLIs are untouched.
pub fn decorate(clis: &mut [CliInfo], settings: &ByokSettings, active: &[Vendor]) {
    for v in active {
        let how = match settings.get(*v) {
            ByokSource::Command { .. } => "token command",
            _ => "your key",
        };
        for c in clis.iter_mut().filter(|c| c.provider == v.cli()) {
            c.billing = if *v == Vendor::Openrouter { Billing::ThirdParty } else { Billing::ApiKey };
            c.plan = Some(format!("{} API · {how}", v.label()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::child_env_with;

    const KEY: &str = "fake-test-key-0123456789abcdef";

    fn byok() -> Byok {
        Byok::new(Box::new(MemorySecrets::default()))
    }

    fn cmd(c: &str) -> ByokSource {
        ByokSource::Command { command: c.into() }
    }

    #[test]
    fn key_validation() {
        assert_eq!(validate_key("  sk-or-v1-abc_DEF.1 \n").unwrap(), "sk-or-v1-abc_DEF.1");
        assert!(validate_key("").is_err());
        assert!(validate_key("   ").is_err());
        assert!(validate_key(&"a".repeat(257)).is_err());
        assert!(validate_key(&"a".repeat(256)).is_ok());
        for bad in ["two words", "quo\"te", "semi;colon", "new\nline", "ünï", "a$b", "a/b"] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn source_validation() {
        assert!(validate_source(ByokSource::Key, false).is_err());
        assert_eq!(validate_source(ByokSource::Key, true).unwrap(), ByokSource::Key);
        assert_eq!(validate_source(ByokSource::Off, false).unwrap(), ByokSource::Off);
        assert_eq!(validate_source(cmd("  wif-token anthropic \n"), false).unwrap(), cmd("wif-token anthropic"));
        assert!(validate_source(cmd("  "), false).is_err());
        assert!(validate_source(cmd("a\nb"), false).is_err());
        assert!(validate_source(cmd(&"a".repeat(2049)), false).is_err());
    }

    #[test]
    fn keys_are_per_vendor_and_never_in_the_status() {
        let b = byok();
        b.save_key(Vendor::Anthropic, &format!(" {KEY} ")).unwrap();
        assert_eq!(b.key(Vendor::Anthropic).unwrap().expose(), KEY);
        assert!(b.key(Vendor::Openai).is_none() && b.key(Vendor::Openrouter).is_none());
        let s = ByokSettings { anthropic: ByokSource::Key, openai: cmd("mint"), ..Default::default() };
        let json = serde_json::to_string(&b.status(&s)).unwrap();
        assert!(!json.contains(KEY), "{json}");
        assert!(json.contains(r#"{"vendor":"anthropic","source":{"kind":"key"},"key_saved":true}"#), "{json}");
        assert!(json.contains(r#"{"vendor":"openai","source":{"kind":"command","command":"mint"},"key_saved":false}"#), "{json}");
        assert!(b.save_key(Vendor::Anthropic, "bad key").is_err());
        assert_eq!(b.key(Vendor::Anthropic).unwrap().expose(), KEY, "a refused key leaves the saved one alone");
        b.clear_key(Vendor::Anthropic).unwrap();
        b.clear_key(Vendor::Anthropic).unwrap();
        assert!(b.key(Vendor::Anthropic).is_none());
    }

    #[test]
    fn credential_follows_the_source() {
        let b = byok();
        let mut s = ByokSettings::default();
        assert!(Vendor::ALL.iter().all(|v| b.credential(&s, *v).is_none()), "off by default");
        s.anthropic = ByokSource::Key;
        assert_eq!(b.credential(&s, Vendor::Anthropic), None, "Key without a saved key is nothing");
        b.save_key(Vendor::Anthropic, KEY).unwrap();
        assert_eq!(b.credential(&s, Vendor::Anthropic), Some(Credential::Key(Secret::new(KEY))));
        s.openai = cmd("mint-openai");
        assert_eq!(b.credential(&s, Vendor::Openai), Some(Credential::Command("mint-openai".into())));
        assert_eq!(b.active(&s), vec![Vendor::Anthropic, Vendor::Openai]);
        s.anthropic = ByokSource::Off;
        assert_eq!(b.credential(&s, Vendor::Anthropic), None, "off keeps the key but hands it to nobody");
    }

    /// A vendor's key reaches its own CLI only; without it, every API key is scrubbed as before.
    #[test]
    fn child_env_matrix() {
        let base: HashMap<String, String> = [
            ("PATH", "/usr/bin"),
            ("ANTHROPIC_API_KEY", "ambient-anthropic"),
            ("OPENAI_API_KEY", "ambient-openai"),
            ("OPENROUTER_API_KEY", "ambient-openrouter"),
        ]
        .map(|(k, v)| (k.into(), v.into()))
        .into();
        for v in Vendor::ALL {
            for p in Provider::ALL {
                let mine = Vendor::of(p) == Some(v);
                let env = child_env_with(&base, p, mine.then_some(KEY));
                let ctx = format!("{v:?} → {p:?}");
                if mine {
                    assert_eq!(env.get(v.env_var()).map(String::as_str), Some(KEY), "{ctx}");
                }
                assert!(env.iter().all(|(k, val)| val != KEY || Some(k.as_str()) == Vendor::of(p).map(Vendor::env_var)), "{ctx}: key leaked");
            }
        }
        // Without BYOK each CLI's own vendor variable is scrubbed.
        for p in [Provider::Claude, Provider::Codex, Provider::Opencode] {
            let var = Vendor::of(p).unwrap().env_var();
            assert_eq!(child_env_with(&base, p, None).get(var), None, "{p:?}");
        }
    }

    #[tokio::test]
    async fn mint_runs_the_command_and_checks_its_output() {
        let env: HashMap<String, String> = [("PATH", "/usr/bin:/bin"), ("WHO", "tok-123")].map(|(k, v)| (k.into(), v.into())).into();
        assert_eq!(mint("printf '  %s\\n' \"$WHO\"", &env).await.unwrap().expose(), "tok-123");
        let e = mint("echo nope >&2; exit 3", &env).await.unwrap_err().to_string();
        assert!(e.contains("failed") && e.contains("nope"), "{e}");
        assert!(mint("true", &env).await.is_err(), "empty output");
        assert!(mint("echo two words", &env).await.is_err());
    }

    #[test]
    fn badges_follow_the_active_vendors() {
        let cli = |provider| CliInfo {
            provider,
            path: Some("/bin/x".into()),
            version: None,
            logged_in: None,
            billing: Billing::Subscription,
            plan: Some("Team".into()),
            problem: None,
            transport: "acp".into(),
            install: crate::install::hint(provider),
            default_model: None,
        };
        let s = ByokSettings { anthropic: ByokSource::Key, openai: cmd("mint"), ..Default::default() };
        let mut clis = vec![cli(Provider::Claude), cli(Provider::Codex), cli(Provider::Opencode)];
        decorate(&mut clis, &s, &[Vendor::Anthropic, Vendor::Openai]);
        assert_eq!((clis[0].billing, clis[0].plan.as_deref()), (Billing::ApiKey, Some("Anthropic API · your key")));
        assert_eq!((clis[1].billing, clis[1].plan.as_deref()), (Billing::ApiKey, Some("OpenAI API · token command")));
        assert_eq!((clis[2].billing, clis[2].plan.as_deref()), (Billing::Subscription, Some("Team")));
    }

    /// Touches the real login Keychain (service `dev.codly.byok`, deleted at the end):
    /// `cargo test -p codly-core byok_keychain_round_trip -- --ignored`
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn byok_keychain_round_trip() {
        let b = Byok::system();
        b.clear_key(Vendor::Openai).unwrap();
        b.save_key(Vendor::Openai, KEY).unwrap();
        assert_eq!(Byok::system().key(Vendor::Openai).unwrap().expose(), KEY, "a fresh handle reads it from the Keychain");
        b.clear_key(Vendor::Openai).unwrap();
        assert!(Byok::system().key(Vendor::Openai).is_none());
    }
}
