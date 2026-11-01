//! Opt-in "bring your own key" for OpenRouter through OpenCode.
//!
//! Codly is subscription-only by default: `env::child_env` scrubs every API-key variable so an
//! agent can never silently bill an API account. This is the one deliberate exception, and it is
//! narrow on purpose:
//! - the user must switch it on (`Settings.byok_openrouter`) AND have saved a key;
//! - the key goes to OpenCode's env only (never Claude, Codex, Gemini, Cursor or Copilot);
//! - the key lives in the macOS Keychain (service `dev.codly.byok`), never in the settings, the
//!   SQLite DB, a log line or an event, and `ByokStatus` has no field that could carry it.

use std::sync::{Mutex, OnceLock};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::inbound::secrets::{Keychain, MemorySecrets, Secret, SecretStore};
use crate::model::{Billing, CliInfo, Provider};

/// Keychain service and account of the OpenRouter key.
pub const SERVICE: &str = "dev.codly.byok";
const ACCOUNT: &str = "openrouter-api-key";
/// The variable OpenCode reads the key from.
pub const ENV_VAR: &str = "OPENROUTER_API_KEY";
/// What the billing badge's plan says while BYOK is active.
pub const PLAN_LABEL: &str = "OpenRouter · your key";
const MAX_KEY_LEN: usize = 256;

/// What the UI sees. There is deliberately no field that can carry the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ByokStatus {
    /// The switch (`Settings.byok_openrouter`).
    pub enabled: bool,
    /// A key is saved in the Keychain; its value never comes back.
    pub key_saved: bool,
}

/// The key store plus a read cache (agents are spawned often; the Keychain is a subprocess).
pub struct Byok {
    store: Box<dyn SecretStore>,
    cache: Mutex<Option<Option<Secret>>>,
}

impl Byok {
    pub fn new(store: Box<dyn SecretStore>) -> Self {
        Byok { store, cache: Mutex::new(None) }
    }

    /// The Keychain on macOS; elsewhere the key lives as long as the process.
    pub fn system() -> Self {
        Self::new(if cfg!(target_os = "macos") { Box::new(Keychain::new(SERVICE)) } else { Box::new(MemorySecrets::default()) })
    }

    /// Blocking (the Keychain is a subprocess): call from `spawn_blocking` in async code.
    pub fn key(&self) -> Option<Secret> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.as_ref() {
            return hit.clone();
        }
        let key = self.store.get(ACCOUNT).ok()?.map(Secret::new); // an unreadable Keychain is not cached
        *cache = Some(key.clone());
        key
    }

    /// The key that may reach OpenCode's env: only when the switch is on and a key exists.
    pub fn active_key(&self, enabled: bool) -> Option<Secret> {
        if enabled { self.key() } else { None }
    }

    pub fn status(&self, enabled: bool) -> ByokStatus {
        ByokStatus { enabled, key_saved: self.key().is_some() }
    }

    pub fn save_key(&self, raw: &str) -> Result<()> {
        let key = validate_key(raw)?;
        self.store.set(ACCOUNT, &key)?;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(Secret::new(key)));
        Ok(())
    }

    pub fn clear_key(&self) -> Result<()> {
        self.store.delete(ACCOUNT)?;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(None);
        Ok(())
    }
}

/// The process-wide store (one Keychain item), shared by the commands and the agent spawner.
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

/// The badge OpenCode shows while its own OpenRouter key is in use. Other CLIs are untouched.
pub fn decorate(clis: &mut [CliInfo], active: bool) {
    if !active {
        return;
    }
    for c in clis.iter_mut().filter(|c| c.provider == Provider::Opencode) {
        c.billing = Billing::ThirdParty;
        c.plan = Some(PLAN_LABEL.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::child_env_with;
    use std::collections::HashMap;

    const KEY: &str = "sk-or-v1-0123456789abcdef";

    fn byok(saved: bool) -> Byok {
        let b = Byok::new(Box::new(MemorySecrets::default()));
        if saved {
            b.save_key(KEY).unwrap();
        }
        b
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
    fn save_status_clear_round_trip_never_exposes_the_key() {
        let b = byok(false);
        assert_eq!(b.status(true), ByokStatus { enabled: true, key_saved: false });
        b.save_key(&format!(" {KEY} ")).unwrap();
        assert_eq!(b.status(false), ByokStatus { enabled: false, key_saved: true });
        // The status is two booleans: nothing in its serialized form is the key.
        let json = serde_json::to_string(&b.status(true)).unwrap();
        assert_eq!(json, r#"{"enabled":true,"key_saved":true}"#);
        assert!(b.save_key("bad key").is_err());
        assert_eq!(b.key().unwrap().expose(), KEY, "a refused key leaves the saved one alone");
        b.clear_key().unwrap();
        b.clear_key().unwrap();
        assert!(!b.status(true).key_saved);
        assert!(format!("{:?}", Secret::new(KEY)).find("sk-or").is_none());
    }

    /// (switch, key saved) x provider: the saved key reaches OpenCode when both are on, and
    /// nothing else ever; with either off, OpenCode's OPENROUTER_API_KEY is scrubbed as before.
    #[test]
    fn child_env_matrix() {
        let base: HashMap<String, String> =
            [("PATH", "/usr/bin"), (ENV_VAR, "ambient-openrouter"), ("OPENAI_API_KEY", "ambient-openai")].map(|(k, v)| (k.into(), v.into())).into();
        for enabled in [false, true] {
            for saved in [false, true] {
                let b = byok(saved);
                let active = b.active_key(enabled);
                assert_eq!(active.is_some(), enabled && saved);
                for p in Provider::ALL {
                    let env = child_env_with(&base, p, active.as_ref().map(Secret::expose));
                    let got = env.get(ENV_VAR).map(String::as_str);
                    let ctx = format!("enabled={enabled} saved={saved} {p:?}");
                    match (p, enabled && saved) {
                        (Provider::Opencode, true) => assert_eq!(got, Some(KEY), "{ctx}"),
                        (Provider::Opencode, false) => assert_eq!(got, None, "{ctx}: default scrubbing"),
                        // Everyone else keeps exactly what the login shell had, never our key.
                        _ => assert_eq!(got, Some("ambient-openrouter"), "{ctx}"),
                    }
                    assert!(env.values().all(|v| p == Provider::Opencode || v != KEY), "{ctx}: key leaked");
                    // Other API-key scrubbing is unchanged by BYOK.
                    assert_eq!(env.contains_key("OPENAI_API_KEY"), !matches!(p, Provider::Codex | Provider::Opencode), "{ctx}");
                }
            }
        }
        // Default call path (no BYOK) is the old behaviour.
        assert_eq!(crate::env::child_env(&base, Provider::Opencode).get(ENV_VAR), None);
    }

    #[test]
    fn badge_says_openrouter_your_key_for_opencode_only() {
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
        let mut off = vec![cli(Provider::Opencode), cli(Provider::Claude)];
        decorate(&mut off, false);
        assert_eq!(off[0].plan.as_deref(), Some("Team"));
        let mut on = vec![cli(Provider::Opencode), cli(Provider::Claude)];
        decorate(&mut on, true);
        assert_eq!((on[0].billing, on[0].plan.as_deref()), (Billing::ThirdParty, Some("OpenRouter · your key")));
        assert_eq!((on[1].billing, on[1].plan.as_deref()), (Billing::Subscription, Some("Team")));
    }

    /// Touches the real login Keychain (service `dev.codly.byok`, deleted at the end):
    /// `cargo test -p codly-core byok_keychain_round_trip -- --ignored`
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn byok_keychain_round_trip() {
        let b = Byok::system();
        b.clear_key().unwrap();
        assert!(!b.status(true).key_saved);
        b.save_key(KEY).unwrap();
        assert_eq!(Byok::system().key().unwrap().expose(), KEY, "a fresh handle reads it from the Keychain");
        b.clear_key().unwrap();
        assert!(Byok::system().key().is_none());
    }
}
