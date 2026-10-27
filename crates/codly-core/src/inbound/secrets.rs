//! Where inbound secrets live: the webhook endpoint secrets and the Slack tokens.
//!
//! Codly never stores the credentials of the agent CLIs (their own sign-ins are used as they
//! are), so there was no store to reuse. These secrets are Codly's own, and they go to the macOS
//! Keychain, never into `inbound.json`, the SQLite DB, logs or events. The Keychain is driven
//! through `security -i`, which reads its commands from stdin, so a secret is never an argv
//! entry that `ps` could show.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Result, bail};

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    /// Deleting what is not there is not an error.
    fn delete(&self, key: &str) -> Result<()>;
}

/// A secret held in memory. Its `Debug` hides the value, so a stray `{:?}` cannot log it.
#[derive(Clone, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Equal bytes, in time that depends only on the longer length, never on where they differ.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    std::hint::black_box(diff) == 0
}

/// A fresh random token (two v4 UUIDs, 244 random bits from the OS CSPRNG), e.g. `cwh_3f…`.
pub fn random_token(prefix: &str) -> String {
    format!("{prefix}_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

// ───────────────────────────── macOS Keychain ─────────────────────────────

pub struct Keychain {
    service: String,
}

impl Keychain {
    /// `service` must be one plain word (it is a word of the `security -i` line).
    pub fn new(service: &str) -> Self {
        assert!(plain(service), "keychain service must be a plain word");
        Keychain { service: service.into() }
    }
}

/// `security -i` splits its stdin line on spaces and quotes, so only plain words go through it.
fn plain(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/+=@-".contains(&b))
}

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>> {
        if !plain(key) {
            bail!("bad secret name");
        }
        let out = Command::new("security").args(["find-generic-password", "-s", &self.service, "-a", key, "-w"]).stdin(Stdio::null()).output()?;
        match out.status.code() {
            Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())),
            Some(44) => Ok(None), // errSecItemNotFound
            _ => bail!("the Keychain could not be read"),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        if !plain(key) || !plain(value) {
            bail!("the value contains characters the Keychain helper cannot take (use letters, digits and . _ : / + = @ -)");
        }
        let mut child = Command::new("security").arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        let line = format!("add-generic-password -U -s {} -a {key} -w {value}\n", self.service);
        child.stdin.take().expect("piped").write_all(line.as_bytes())?;
        child.wait()?; // `-i` exits 0 even when a command failed, so read it back
        if self.get(key)?.as_deref() != Some(value) {
            bail!("the Keychain refused the secret");
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        if !plain(key) {
            bail!("bad secret name");
        }
        let st = Command::new("security").args(["delete-generic-password", "-s", &self.service, "-a", key]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        match st.code() {
            Some(0 | 44) => Ok(()),
            _ => bail!("the Keychain could not delete the secret"),
        }
    }
}

// ───────────────────────────── in memory ─────────────────────────────

/// Tests and non-macOS builds: secrets live as long as the process.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        Ok(())
    }
}

/// The store the app uses: the Keychain on macOS.
pub fn default_store() -> Box<dyn SecretStore> {
    if cfg!(target_os = "macos") { Box::new(Keychain::new("dev.codly.inbound")) } else { Box::new(MemorySecrets::default()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_is_exact_about_content_and_length() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"abcd", b"abc"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn secrets_do_not_debug_print() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(***)");
        assert!(random_token("cwh").len() > 60);
        assert_ne!(random_token("cwh"), random_token("cwh"));
    }

    /// Touches the real login Keychain (a throwaway service, deleted at the end):
    /// `cargo test -p codly-core keychain_round_trip -- --ignored`
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn keychain_round_trip() {
        let k = Keychain::new("dev.codly.inbound.test");
        let key = random_token("t");
        assert_eq!(k.get(&key).unwrap(), None);
        k.set(&key, "xoxb-1-abc.DEF_ghi").unwrap();
        assert_eq!(k.get(&key).unwrap().as_deref(), Some("xoxb-1-abc.DEF_ghi"));
        k.set(&key, "replaced").unwrap();
        assert_eq!(k.get(&key).unwrap().as_deref(), Some("replaced"));
        k.delete(&key).unwrap();
        k.delete(&key).unwrap();
        assert_eq!(k.get(&key).unwrap(), None);
    }

    #[test]
    fn keychain_refuses_what_security_i_would_misparse() {
        let k = Keychain::new("dev.codly.test");
        assert!(k.set("a", "two words").is_err());
        assert!(k.set("a", "quo\"te").is_err());
        assert!(k.set("a b", "x").is_err());
    }
}
