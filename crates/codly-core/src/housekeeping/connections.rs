//! Default MCP servers for new agents: a catalog, what is switched on, where keys live, and
//! how the enabled ones reach an agent's session.
//!
//! - SAFE read-only servers are on by default (sequential-thinking, time, fetch, context7,
//!   filesystem scoped to the agent's working directory, git scoped to its repository).
//! - CREDENTIALED servers (GitHub, a database, Google mail/calendar) are off until you add the
//!   key, and the toggle does nothing while a key is missing.
//! - Keys go to the macOS Keychain (service `codly-mcp`), never to the settings in `kv` or to
//!   any config file of Codly's. Codly still never reads the CLIs' own credentials.
//! - Changes apply to the NEXT spawn only: `resolve` runs when a run starts and the result is
//!   written to `<run dir>/extra-mcp.json` (references to keys, not the keys); each adapter
//!   merges that file beside the `codly` server. A running agent keeps what it started with.
//!
//! Package names follow each project's README at the time of writing; they are fetched with
//! `npx`/`uvx` on first use, so an unreachable registry means that server fails to start (the
//! agent still runs), not that the spawn fails.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Keychain service name of every secret Codly stores.
pub const KEYCHAIN_SERVICE: &str = "codly-mcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Tier {
    /// Reads only; on by default.
    SafeReadonly,
    /// Needs a key and can act on your behalf; off until you add one.
    Credentialed,
}

/// One environment variable a server needs, kept in the Keychain.
pub struct SecretSlot {
    pub env: &'static str,
    pub label: &'static str,
}

pub struct CatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub blurb: &'static str,
    pub tier: Tier,
    pub command: &'static str,
    /// `{cwd}` is replaced by the agent's working directory.
    pub args: &'static [&'static str],
    pub secrets: &'static [SecretSlot],
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "sequential-thinking",
        label: "Sequential thinking",
        blurb: "A scratchpad for step-by-step reasoning.",
        tier: Tier::SafeReadonly,
        command: "npx",
        args: &["-y", "@modelcontextprotocol/server-sequential-thinking"],
        secrets: &[],
    },
    CatalogEntry {
        id: "time",
        label: "Time",
        blurb: "Current time and time-zone conversion.",
        tier: Tier::SafeReadonly,
        command: "uvx",
        args: &["mcp-server-time"],
        secrets: &[],
    },
    CatalogEntry {
        id: "fetch",
        label: "Fetch",
        blurb: "Read a web page as text.",
        tier: Tier::SafeReadonly,
        command: "uvx",
        args: &["mcp-server-fetch"],
        secrets: &[],
    },
    CatalogEntry {
        id: "context7",
        label: "Context7",
        blurb: "Up-to-date library documentation.",
        tier: Tier::SafeReadonly,
        command: "npx",
        args: &["-y", "@upstash/context7-mcp"],
        secrets: &[],
    },
    CatalogEntry {
        id: "filesystem",
        label: "Filesystem",
        blurb: "Reads files, scoped to the agent's own working folder.",
        tier: Tier::SafeReadonly,
        command: "npx",
        args: &["-y", "@modelcontextprotocol/server-filesystem", "{cwd}"],
        secrets: &[],
    },
    CatalogEntry {
        id: "git",
        label: "Git",
        blurb: "Log, diff and blame, scoped to the agent's repository.",
        tier: Tier::SafeReadonly,
        command: "uvx",
        args: &["mcp-server-git", "--repository", "{cwd}"],
        secrets: &[],
    },
    CatalogEntry {
        id: "github",
        label: "GitHub",
        blurb: "Issues, pull requests and code search on GitHub.",
        tier: Tier::Credentialed,
        command: "npx",
        args: &["-y", "@modelcontextprotocol/server-github"],
        secrets: &[SecretSlot { env: "GITHUB_PERSONAL_ACCESS_TOKEN", label: "Personal access token" }],
    },
    CatalogEntry {
        id: "database",
        label: "Database",
        blurb: "Query a Postgres, MySQL or SQLite database.",
        tier: Tier::Credentialed,
        command: "npx",
        args: &["-y", "@bytebase/dbhub", "--transport", "stdio"],
        secrets: &[SecretSlot { env: "DSN", label: "Connection string" }],
    },
    CatalogEntry {
        id: "google-workspace",
        label: "Email and calendar",
        blurb: "Gmail and Google Calendar.",
        tier: Tier::Credentialed,
        command: "uvx",
        args: &["workspace-mcp"],
        secrets: &[
            SecretSlot { env: "GOOGLE_OAUTH_CLIENT_ID", label: "OAuth client ID" },
            SecretSlot { env: "GOOGLE_OAUTH_CLIENT_SECRET", label: "OAuth client secret" },
        ],
    },
];

pub fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// What the user switched, as overrides of the defaults (stored in `kv`; holds no secrets).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPrefs {
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
}

/// Switched on: your choice, else the default for its tier.
pub fn is_on(e: &CatalogEntry, prefs: &McpPrefs) -> bool {
    prefs.enabled.get(e.id).copied().unwrap_or(e.tier == Tier::SafeReadonly)
}

// ───────────────────────────── secrets ─────────────────────────────

/// Where keys live. The app uses the Keychain; tests use memory.
pub trait SecretStore {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, value: &str) -> anyhow::Result<()>;
    fn delete(&self, account: &str) -> anyhow::Result<()>;
}

/// The Keychain entry of one slot: `<server id>:<ENV NAME>`.
pub fn account(entry_id: &str, env: &str) -> String {
    format!("{entry_id}:{env}")
}

/// macOS Keychain through the `security` tool. The value travels on stdin (hex), never argv.
pub struct KeychainStore;

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Option<String> {
        let out = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account, "-w"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8(out.stdout).ok()?;
        let v = v.trim_end_matches(['\n', '\r']);
        (!v.is_empty()).then(|| v.to_string())
    }

    fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
        use std::io::Write;
        anyhow::ensure!(!account.contains(['"', '\'', '\\', ' ', '\n']), "bad account name");
        let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
        let mut child = Command::new("security").arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
        let cmd = format!("add-generic-password -U -s {KEYCHAIN_SERVICE} -a {account} -X {hex}\n");
        child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?.write_all(cmd.as_bytes())?;
        let out = child.wait_with_output()?;
        anyhow::ensure!(out.status.success() && out.stderr.is_empty(), "the Keychain refused the key: {}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(())
    }

    fn delete(&self, account: &str) -> anyhow::Result<()> {
        // A missing entry is fine: the key is gone either way.
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(())
    }
}

/// All slots of an entry have a key.
pub fn has_keys(e: &CatalogEntry, store: &dyn SecretStore) -> bool {
    e.secrets.iter().all(|s| store.get(&account(e.id, s.env)).is_some())
}

// ───────────────────────────── the UI's view ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretState {
    pub env: String,
    pub label: String,
    /// A key is stored (its value never leaves the Keychain).
    pub set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Connection {
    pub id: String,
    pub label: String,
    pub blurb: String,
    pub tier: Tier,
    /// The switch. A credentialed server stays inactive while a key is missing.
    pub enabled: bool,
    pub secrets: Vec<SecretState>,
    /// Will be given to the next agents that start (switched on, and every key present).
    pub active: bool,
    /// What the agent CLI will run, for the curious.
    pub command: String,
}

pub fn connections(prefs: &McpPrefs, store: &dyn SecretStore) -> Vec<Connection> {
    CATALOG
        .iter()
        .map(|e| {
            let enabled = is_on(e, prefs);
            let secrets: Vec<SecretState> =
                e.secrets.iter().map(|s| SecretState { env: s.env.into(), label: s.label.into(), set: store.get(&account(e.id, s.env)).is_some() }).collect();
            let keyed = secrets.iter().all(|s| s.set);
            Connection {
                id: e.id.into(),
                label: e.label.into(),
                blurb: e.blurb.into(),
                tier: e.tier,
                enabled,
                active: enabled && keyed,
                secrets,
                command: std::iter::once(e.command).chain(e.args.iter().copied()).collect::<Vec<_>>().join(" "),
            }
        })
        .collect()
}

// ───────────────────────────── spawn-time merge ─────────────────────────────

/// One extra MCP server for an agent's session. Secrets are named, not carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraMcp {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Plain environment.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Environment variables whose values are in the Keychain: (env name, Keychain account).
    #[serde(default)]
    pub secret_env: Vec<(String, String)>,
    /// Safe to call without asking (a read-only server).
    pub read_only: bool,
}

/// The enabled servers for an agent working in `cwd`. Credentialed ones are left out while a
/// key is missing.
pub fn resolve(prefs: &McpPrefs, store: &dyn SecretStore, cwd: &Path) -> Vec<ExtraMcp> {
    let cwd = cwd.to_string_lossy();
    CATALOG
        .iter()
        .filter(|e| is_on(e, prefs) && has_keys(e, store))
        .map(|e| ExtraMcp {
            name: e.id.to_string(),
            command: e.command.to_string(),
            args: e.args.iter().map(|a| a.replace("{cwd}", &cwd)).collect(),
            env: vec![],
            secret_env: e.secrets.iter().map(|s| (s.env.to_string(), account(e.id, s.env))).collect(),
            read_only: e.tier == Tier::SafeReadonly,
        })
        .collect()
}

/// An extra server ready to hand to a CLI: secrets looked up, `env` complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyMcp {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub read_only: bool,
}

/// Look the keys up. A server whose key vanished since `resolve` is dropped.
pub fn materialize(extra: &[ExtraMcp], store: &dyn SecretStore) -> Vec<ReadyMcp> {
    extra
        .iter()
        .filter_map(|x| {
            let mut env = x.env.clone();
            for (name, acct) in &x.secret_env {
                env.push((name.clone(), store.get(acct)?));
            }
            Some(ReadyMcp { name: x.name.clone(), command: x.command.clone(), args: x.args.clone(), env, read_only: x.read_only })
        })
        .collect()
}

/// `<run dir>/extra-mcp.json`, next to the raw log.
pub fn sidecar_path(raw_log: &Path) -> std::path::PathBuf {
    raw_log.with_file_name("extra-mcp.json")
}

pub fn write_sidecar(raw_log: &Path, extra: &[ExtraMcp]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(sidecar_path(raw_log))?;
    f.write_all(serde_json::to_string(extra)?.as_bytes())?;
    Ok(())
}

/// What the adapters merge: the run's extra servers with their keys. Empty without a sidecar
/// (older runs, tests), and never fails a spawn.
pub fn load_ready(raw_log: &Path) -> Vec<ReadyMcp> {
    load_ready_with(raw_log, &KeychainStore)
}

pub fn load_ready_with(raw_log: &Path, store: &dyn SecretStore) -> Vec<ReadyMcp> {
    let Ok(text) = std::fs::read_to_string(sidecar_path(raw_log)) else { return vec![] };
    let Ok(extra) = serde_json::from_str::<Vec<ExtraMcp>>(&text) else { return vec![] };
    // The `codly` name is ours; an extra server may never replace it.
    materialize(&extra.into_iter().filter(|x| x.name != "codly").collect::<Vec<_>>(), store)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    pub struct MemoryStore(RefCell<HashMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn get(&self, account: &str) -> Option<String> {
            self.0.borrow().get(account).cloned()
        }
        fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
            self.0.borrow_mut().insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> anyhow::Result<()> {
            self.0.borrow_mut().remove(account);
            Ok(())
        }
    }

    fn names(x: &[ExtraMcp]) -> Vec<&str> {
        x.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn safe_servers_are_on_by_default_and_credentialed_ones_are_off() {
        let x = resolve(&McpPrefs::default(), &MemoryStore::default(), Path::new("/wt/t1"));
        assert_eq!(names(&x), ["sequential-thinking", "time", "fetch", "context7", "filesystem", "git"]);
        assert!(x.iter().all(|e| e.read_only && e.secret_env.is_empty()));
    }

    #[test]
    fn filesystem_and_git_are_scoped_to_the_agents_folder() {
        let x = resolve(&McpPrefs::default(), &MemoryStore::default(), Path::new("/wt/t1"));
        let fs = x.iter().find(|e| e.name == "filesystem").unwrap();
        assert_eq!(fs.args.last().unwrap(), "/wt/t1");
        let git = x.iter().find(|e| e.name == "git").unwrap();
        assert_eq!(git.args, ["mcp-server-git", "--repository", "/wt/t1"]);
        assert!(x.iter().all(|e| e.args.iter().all(|a| !a.contains("{cwd}"))));
    }

    #[test]
    fn a_credentialed_server_needs_the_switch_and_every_key() {
        let store = MemoryStore::default();
        let mut prefs = McpPrefs::default();
        prefs.enabled.insert("github".into(), true);
        // Switched on without a key: still not given to agents.
        assert!(!names(&resolve(&prefs, &store, Path::new("/r"))).contains(&"github"));
        let c = connections(&prefs, &store);
        let gh = c.iter().find(|c| c.id == "github").unwrap();
        assert!(gh.enabled && !gh.active && !gh.secrets[0].set);
        // A key without the switch: also not.
        store.set(&account("github", "GITHUB_PERSONAL_ACCESS_TOKEN"), "ghp_x").unwrap();
        assert!(!names(&resolve(&McpPrefs::default(), &store, Path::new("/r"))).contains(&"github"));
        // Both: given, with the secret named, not carried.
        let x = resolve(&prefs, &store, Path::new("/r"));
        let gh = x.iter().find(|e| e.name == "github").unwrap();
        assert!(!gh.read_only);
        assert_eq!(gh.secret_env, [("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), "github:GITHUB_PERSONAL_ACCESS_TOKEN".to_string())]);
        assert!(!serde_json::to_string(&x).unwrap().contains("ghp_x"), "the sidecar never holds a key");
    }

    #[test]
    fn a_server_with_two_keys_needs_both() {
        let store = MemoryStore::default();
        let mut prefs = McpPrefs::default();
        prefs.enabled.insert("google-workspace".into(), true);
        store.set(&account("google-workspace", "GOOGLE_OAUTH_CLIENT_ID"), "id").unwrap();
        assert!(!names(&resolve(&prefs, &store, Path::new("/r"))).contains(&"google-workspace"));
        store.set(&account("google-workspace", "GOOGLE_OAUTH_CLIENT_SECRET"), "s").unwrap();
        assert!(names(&resolve(&prefs, &store, Path::new("/r"))).contains(&"google-workspace"));
    }

    #[test]
    fn switching_a_safe_server_off_removes_it() {
        let mut prefs = McpPrefs::default();
        prefs.enabled.insert("fetch".into(), false);
        assert!(!names(&resolve(&prefs, &MemoryStore::default(), Path::new("/r"))).contains(&"fetch"));
    }

    #[test]
    fn the_sidecar_round_trips_and_keys_are_looked_up_at_load_time() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.ndjson");
        let store = MemoryStore::default();
        let mut prefs = McpPrefs::default();
        prefs.enabled.insert("github".into(), true);
        store.set("github:GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_x").unwrap();
        write_sidecar(&raw, &resolve(&prefs, &store, Path::new("/r"))).unwrap();
        let ready = load_ready_with(&raw, &store);
        let gh = ready.iter().find(|r| r.name == "github").unwrap();
        assert_eq!(gh.env, [("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), "ghp_x".to_string())]);
        assert_eq!(ready.len(), 7);
        // The key was removed after the run was prepared: that server is dropped, the rest stay.
        store.delete("github:GITHUB_PERSONAL_ACCESS_TOKEN").unwrap();
        assert_eq!(load_ready_with(&raw, &store).len(), 6);
        // No sidecar, no extras.
        assert!(load_ready_with(&dir.path().join("none").join("raw.ndjson"), &store).is_empty());
    }

    #[test]
    fn an_extra_server_can_never_replace_codly() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.ndjson");
        let evil = ExtraMcp { name: "codly".into(), command: "x".into(), args: vec![], env: vec![], secret_env: vec![], read_only: true };
        write_sidecar(&raw, &[evil]).unwrap();
        assert!(load_ready_with(&raw, &MemoryStore::default()).is_empty());
    }

    /// Talks to the real login Keychain: `cargo test -p codly-core keychain -- --ignored`.
    #[test]
    #[ignore]
    fn keychain_round_trip() {
        let s = KeychainStore;
        let a = "selftest:CODLY_SELFTEST";
        s.set(a, "sk-test 123/abc").unwrap();
        assert_eq!(s.get(a).as_deref(), Some("sk-test 123/abc"));
        s.delete(a).unwrap();
        assert_eq!(s.get(a), None);
    }
}
