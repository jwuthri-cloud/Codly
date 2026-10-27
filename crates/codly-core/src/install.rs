//! Onboarding: how to install each agent CLI, and the one-click "Run it for me".
//!
//! Safety rule, enforced here rather than left to the UI: the only text that ever reaches the
//! terminal is a constant from `ROWS`, picked by (provider, rung). The UI sends two enums, never
//! a command line, so a compromised webview cannot turn `install_cli` into a shell. The command
//! runs in a visible login-shell terminal (the user sees it, can read it and can stop it); Codly
//! never installs anything silently.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{InstallHint, Provider};
use crate::pty::PtyManager;

/// Which installer to run: the npm package, or the vendor's own script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InstallRung {
    Npm,
    Native,
}

/// A started install: the terminal to attach to (`pty_attach`) and the exact line it was given.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstallRun {
    pub pty_id: String,
    pub command: String,
}

struct Row {
    provider: Provider,
    npm: Option<&'static str>,
    /// Posix shell only (`curl … | bash`).
    native: Option<&'static str>,
    docs: &'static str,
}

/// The whole trust surface. Each line is typed by hand from the vendor's own install docs.
const ROWS: [Row; 6] = [
    Row {
        provider: Provider::Claude,
        npm: Some("npm install -g @anthropic-ai/claude-code"),
        native: Some("curl -fsSL https://claude.ai/install.sh | bash"),
        docs: "https://docs.claude.com/en/docs/claude-code/setup",
    },
    Row { provider: Provider::Codex, npm: Some("npm install -g @openai/codex"), native: None, docs: "https://github.com/openai/codex" },
    Row {
        provider: Provider::Gemini,
        npm: Some("npm install -g @google/gemini-cli"),
        native: None,
        docs: "https://github.com/google-gemini/gemini-cli",
    },
    Row {
        provider: Provider::Opencode,
        npm: Some("npm install -g opencode-ai@latest"),
        native: Some("curl -fsSL https://opencode.ai/install | bash"),
        docs: "https://opencode.ai/docs",
    },
    Row { provider: Provider::Cursor, npm: None, native: Some("curl https://cursor.com/install -fsS | bash"), docs: "https://cursor.com/cli" },
    Row {
        provider: Provider::Copilot,
        npm: Some("npm install -g @github/copilot"),
        native: None,
        docs: "https://github.com/github/copilot-cli",
    },
];

fn row(provider: Provider) -> &'static Row {
    ROWS.iter().find(|r| r.provider == provider).expect("every provider has a row")
}

/// What to show for `provider`: the install lines, the sign-in command and the docs link.
pub fn hint(provider: Provider) -> InstallHint {
    let r = row(provider);
    InstallHint {
        npm: r.npm.map(str::to_owned),
        native: r.native.map(str::to_owned),
        login: crate::env::login_command(provider, None),
        docs: r.docs.to_owned(),
    }
}

/// The constant for (provider, rung), or why there is none. `posix` = the native rungs are
/// shell scripts, so they only run where a POSIX shell is the login shell.
pub fn command_for(provider: Provider, rung: InstallRung, posix: bool) -> Result<&'static str> {
    let r = row(provider);
    match rung {
        InstallRung::Npm => r.npm.ok_or_else(|| anyhow::anyhow!("{} has no npm package; use its own installer", provider.label())),
        InstallRung::Native if !posix => bail!("the {} installer is a shell script and runs on macOS and Linux only", provider.label()),
        InstallRung::Native => r.native.ok_or_else(|| anyhow::anyhow!("{} has no installer script; use npm", provider.label())),
    }
}

/// Open a visible login-shell terminal in `cwd` (with the login env, so `npm` and `curl` are
/// found) and type the constant for (provider, rung) into it.
pub fn start(pty: &PtyManager, cwd: &Path, env: &HashMap<String, String>, provider: Provider, rung: InstallRung) -> Result<InstallRun> {
    let command = command_for(provider, rung, cfg!(unix))?;
    start_line(pty, cwd, env, command)
}

/// Types `line` (one of `ROWS`, by construction) and Enter. The shell stays open afterwards so
/// the user can read the output; the UI re-detects the CLI when they are done.
fn start_line(pty: &PtyManager, cwd: &Path, env: &HashMap<String, String>, line: &'static str) -> Result<InstallRun> {
    let pty_id = pty.open(cwd, env)?;
    // The tty buffers this until the login shell is ready to read it.
    if let Err(e) = pty.write(&pty_id, format!("{line}\n").as_bytes()) {
        pty.close(&pty_id);
        return Err(e);
    }
    Ok(InstallRun { pty_id, command: line.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_lines() -> Vec<&'static str> {
        ROWS.iter().flat_map(|r| [r.npm, r.native]).flatten().collect()
    }

    #[test]
    fn hint_table_matches_the_vendor_commands() {
        let h = |p| hint(p);
        assert_eq!(h(Provider::Claude).npm.as_deref(), Some("npm install -g @anthropic-ai/claude-code"));
        assert_eq!(h(Provider::Claude).native.as_deref(), Some("curl -fsSL https://claude.ai/install.sh | bash"));
        assert_eq!(h(Provider::Codex).npm.as_deref(), Some("npm install -g @openai/codex"));
        assert_eq!(h(Provider::Gemini).npm.as_deref(), Some("npm install -g @google/gemini-cli"));
        assert_eq!(h(Provider::Opencode).npm.as_deref(), Some("npm install -g opencode-ai@latest"));
        assert_eq!(h(Provider::Opencode).native.as_deref(), Some("curl -fsSL https://opencode.ai/install | bash"));
        assert_eq!(h(Provider::Copilot).npm.as_deref(), Some("npm install -g @github/copilot"));
        assert_eq!((h(Provider::Cursor).npm, h(Provider::Cursor).native.as_deref()), (None, Some("curl https://cursor.com/install -fsS | bash")));
        // Sign-in comes from the existing login_command.
        assert_eq!(h(Provider::Claude).login.as_deref(), Some("claude auth login"));
        assert_eq!(h(Provider::Cursor).login, None);
        for p in Provider::ALL {
            assert!(h(p).docs.starts_with("https://"), "{p:?}");
        }
    }

    /// Nothing in the table can chain, substitute, redirect or background: the only
    /// metacharacter is the `|` of the two `curl … | bash` lines (and the dot-less package names).
    #[test]
    fn hint_table_has_no_shell_metacharacters() {
        for line in all_lines() {
            for bad in [';', '&', '`', '$', '>', '<', '(', ')', '{', '}', '\\', '"', '\'', '\n', '*', '!', '#'] {
                assert!(!line.contains(bad), "{line:?} contains {bad:?}");
            }
            let pipes = line.matches('|').count();
            if line.starts_with("curl ") {
                assert_eq!(pipes, 1, "{line:?}");
                assert!(line.ends_with(" | bash"), "{line:?}");
            } else {
                assert_eq!(pipes, 0, "{line:?}");
                assert!(line.starts_with("npm install -g "), "{line:?}");
            }
            assert!(line.chars().all(|c| c.is_ascii() && !c.is_control()), "{line:?}");
        }
    }

    #[test]
    fn only_https_installers_and_known_hosts() {
        for line in all_lines().into_iter().filter(|l| l.starts_with("curl ")) {
            let url = line.split(' ').find(|w| w.starts_with("http")).unwrap();
            assert!(["https://claude.ai/", "https://opencode.ai/", "https://cursor.com/"].iter().any(|h| url.starts_with(h)), "{url}");
        }
    }

    #[test]
    fn rungs_resolve_to_constants_and_reject_the_rest() {
        assert_eq!(command_for(Provider::Codex, InstallRung::Npm, true).unwrap(), "npm install -g @openai/codex");
        assert!(command_for(Provider::Codex, InstallRung::Native, true).unwrap_err().to_string().contains("use npm"));
        assert!(command_for(Provider::Cursor, InstallRung::Npm, true).unwrap_err().to_string().contains("no npm package"));
        assert!(command_for(Provider::Claude, InstallRung::Native, false).unwrap_err().to_string().contains("macOS and Linux"));
        // npm works wherever npm does.
        assert!(command_for(Provider::Claude, InstallRung::Npm, false).is_ok());
    }

    /// The UI picks a rung by name; anything else fails to deserialize, so no text can ride along.
    #[test]
    fn unknown_rung_is_rejected() {
        assert_eq!(serde_json::from_str::<InstallRung>("\"npm\"").unwrap(), InstallRung::Npm);
        assert_eq!(serde_json::from_str::<InstallRung>("\"native\"").unwrap(), InstallRung::Native);
        for bad in ["\"brew\"", "\"npm; rm -rf ~\"", "\"\"", "\"NPM\"", "1"] {
            assert!(serde_json::from_str::<InstallRung>(bad).is_err(), "{bad}");
        }
    }

    /// Whatever (provider, rung) the UI sends, the line typed into the terminal is a table entry.
    #[test]
    fn only_constants_reach_the_pty() {
        let table = all_lines();
        for p in Provider::ALL {
            for rung in [InstallRung::Npm, InstallRung::Native] {
                if let Ok(cmd) = command_for(p, rung, true) {
                    assert!(table.contains(&cmd), "{cmd}");
                }
            }
        }
    }

    #[test]
    fn the_line_is_typed_into_a_visible_terminal() {
        let pty = PtyManager::default();
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.insert("SHELL".into(), "/bin/sh".into());
        let run = start_line(&pty, Path::new("/tmp"), &env, "echo codly-install-ok").unwrap();
        assert_eq!(run.command, "echo codly-install-ok");
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let tx = std::sync::Mutex::new(tx);
        pty.attach(&run.pty_id, Box::new(move |b| { let _ = tx.lock().unwrap().send(b); })).unwrap();
        let mut seen = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && seen.matches("codly-install-ok").count() < 2 {
            if let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                seen.push_str(&String::from_utf8_lossy(&b));
            }
        }
        pty.close(&run.pty_id);
        // Once as the echoed input, once as the command's output.
        assert!(seen.matches("codly-install-ok").count() >= 2, "got: {seen}");
    }
}
