//! Settings → Troubleshooting: the text a user pastes in a bug report. It says which Codly,
//! which system, which agent CLIs and what the log said last. Codly's log lines say what
//! happened, never a prompt or file contents; what can still be private in them and in error
//! texts (a token, a key, an email address, a path under your home folder) is taken out here.
//! Names stay: a repository, an agent, a folder outside the home folder. The report is built on
//! the shell side, so it also answers when the core did not start.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use codly_core::model::{Billing, CliInfo};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::cli::VERSION;
use crate::commands::AppState;
use crate::logging;

const LOG_LINES: usize = 200;

#[derive(Serialize)]
pub struct Diagnostics {
    pub version: &'static str,
    /// The log file, to show in Finder. None when it could not be opened.
    pub log_file: Option<String>,
    /// The report, as plain text.
    pub report: String,
}

#[tauri::command]
pub async fn diagnostics<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> Result<Diagnostics, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    // Not `state.core()`: that waits for the core, and this must answer when it never starts.
    // The CLIs are asked as the core asks them: with your shell's Claude sign-in only if the
    // setting is on (off when the core is not there to say).
    let (core, shell_auth) = match &*state.core.borrow() {
        None => ("starting".to_string(), false),
        Some(Ok(core)) => ("running".to_string(), core.settings().claude_shell_auth),
        Some(Err(e)) => (format!("did not start: {e}"), false),
    };
    let clis = codly_core::env::detect_all(state.shell_env().await, shell_auth).await;
    let log_dir = logging::dir();
    tokio::task::spawn_blocking(move || {
        let facts = Facts {
            system: system(),
            core,
            home: codly_core::skills::home_dir().ok(),
            database: std::fs::metadata(data_dir.join("codly.sqlite")).ok().map(|m| m.len()),
            log: log_dir.as_deref().map(|dir| logging::last_lines(dir, LOG_LINES).map_err(|e| e.to_string())),
            data_dir,
            clis,
        };
        Diagnostics { version: VERSION, log_file: log_dir.map(|dir| dir.join(logging::FILE).to_string_lossy().into_owned()), report: report(&facts) }
    })
    .await
    .map_err(|e| e.to_string())
}

/// What the report is made of.
struct Facts {
    system: String,
    /// "running", "starting" or why the core did not start.
    core: String,
    home: Option<PathBuf>,
    data_dir: PathBuf,
    /// Size of codly.sqlite in bytes.
    database: Option<u64>,
    /// The last log lines; None when there is no log file, Err when it could not be read.
    log: Option<Result<Vec<String>, String>>,
    clis: Vec<CliInfo>,
}

fn report(f: &Facts) -> String {
    let home = f.home.as_deref();
    let scrub = |text: &str| scrub(text, home, &f.data_dir);
    let mut out = format!("Codly {VERSION}\nSystem: {}\nCore: {}\n", f.system, scrub(&f.core));
    let _ = writeln!(out, "Data folder: {}", tildify(&f.data_dir, home));
    let _ = match f.database {
        Some(bytes) => writeln!(out, "Database: {:.1} MB", bytes as f64 / 1e6),
        None => writeln!(out, "Database: not found"),
    };

    out.push_str("\nAgent CLIs\n");
    for c in &f.clis {
        let name = c.provider.as_str();
        let Some(path) = &c.path else {
            let _ = writeln!(out, "- {name}: not installed");
            continue;
        };
        let signed_in = match c.logged_in {
            Some(true) => "signed in",
            Some(false) => "not signed in",
            None => "sign-in unknown",
        };
        let billing = match c.billing {
            Billing::Subscription => "subscription",
            Billing::ApiKey => "API key",
            Billing::ThirdParty => "third party",
            Billing::Unknown => "unknown",
        };
        let version = c.version.as_deref().unwrap_or("version unknown");
        let _ = write!(out, "- {name} {version} at {} ({}): {signed_in}, billing {billing}", tildify(Path::new(path), home), c.transport);
        if let Some(problem) = &c.problem {
            let _ = write!(out, ", problem: {}", scrub(problem));
        }
        out.push('\n');
    }

    match &f.log {
        None => out.push_str("\nNo log file: it could not be opened.\n"),
        Some(Err(e)) => {
            let _ = writeln!(out, "\nThe log could not be read: {}", scrub(e));
        }
        Some(Ok(lines)) => {
            let _ = writeln!(out, "\nLast {} log lines", lines.len());
            for line in lines {
                out.push_str(&scrub(line));
                out.push('\n');
            }
        }
    }
    out
}

fn system() -> String {
    let os = if cfg!(target_os = "macos") { format!("macOS {}", mac_version().unwrap_or_else(|| "(version unknown)".into())) } else { std::env::consts::OS.to_string() };
    format!("{os} ({})", std::env::consts::ARCH)
}

/// `sw_vers -productVersion`, for example "15.1".
fn mac_version() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A path that is not private apart from the user's name (the data folder, a CLI): `~/…`.
fn tildify(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Free text (a log line, an error) without what is private: folders under the home folder,
/// email addresses, and anything that looks like a key or a token.
fn scrub(text: &str, home: Option<&Path>, data_dir: &Path) -> String {
    // The data folder first: it is inside the home folder, and its top-level names are Codly's.
    let text = hide_paths(text, &data_dir.to_string_lossy(), "<data>", 1);
    let text = match home {
        Some(home) => hide_paths(&text, &home.to_string_lossy(), "~", 0),
        None => text,
    };
    hide_secrets(&text)
}

/// Every path under `root` becomes `label`, the first `keep` names after it and "…" for the
/// rest: "<data>/worktrees/…". `path_after` says where each path ends.
fn hide_paths(text: &str, root: &str, label: &str, keep: usize) -> String {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    // `text[..done]` is in `out`; the next `root` is looked for from `from`.
    let (mut done, mut from) = (0, 0);
    while let Some(i) = text[from..].find(root).map(|i| from + i) {
        let after = i + root.len();
        let quote = text[..i].chars().next_back().filter(|c| QUOTES.contains(c));
        let path = path_after(&text[after..], root, quote);
        // "/Users/ana" must not match inside "/Users/anabel".
        if !path.is_empty() && !path.starts_with('/') {
            from = after;
            continue;
        }
        out.push_str(&text[done..i]);
        out.push_str(label);
        let names: Vec<&str> = path.split('/').filter(|n| !n.is_empty()).collect();
        for name in names.iter().take(keep) {
            out.push('/');
            out.push_str(name);
        }
        if names.len() > keep {
            out.push_str("/…");
        }
        done = after + path.len();
        from = done;
    }
    out.push_str(&text[done..]);
    out
}

const QUOTES: [char; 3] = ['"', '\'', '`'];

/// The part of `after` (the text behind a `root`) that is still that path. `quote`: the quote
/// sign in front of `root`, if there is one. A folder name can hold spaces, so a space does not
/// end a path. Of a last name with spaces and no quotes around the path, the words after the
/// first stay: "Repo" of "…/My Repo is not a git repository".
fn path_after<'a>(after: &'a str, root: &str, quote: Option<char>) -> &'a str {
    let line = after.lines().next().unwrap_or_default();
    if !line.starts_with('/') {
        // `root` itself ("home is /Users/ana."), or another name ("/Users/anabel").
        let end = line.find(|c: char| c.is_whitespace() || "\"'`()[]<>,;".contains(c)).unwrap_or(line.len());
        return line[..end].trim_end_matches([':', '.']);
    }
    // In quotes, the path is all of what they hold.
    if let Some(end) = quote.and_then(|q| line.find(q)) {
        return &line[..end];
    }
    // The sentence goes on at a quote, at "<data>", at ": " and at the next path under `root`.
    let mut end = line.find(['"', '\'', '`', '<', '>']).unwrap_or(line.len());
    for stop in [": ", root] {
        end = line[1..end].find(stop).map_or(end, |i| i + 1);
    }
    // Before that, the path ends with the last word that holds a `/`.
    let span = &line[..end];
    let last = span.rfind('/').unwrap_or(0);
    let end = span[last..].find(char::is_whitespace).map_or(span.len(), |i| last + i);
    // "(see …/repo)." and "…/repo: not a git repository": the signs belong to the sentence.
    span[..end].trim_end_matches([':', '.', ',', ';', ')', ']'])
}

/// Where a secret's value may follow its name: `token=…`, `"password": "…"`, `Bearer …`.
#[derive(Clone, Copy, PartialEq)]
enum Key {
    No,
    /// The last word names a secret.
    Named,
    /// …and `=` or `:` came after it: what follows is the value.
    Assigned,
}

fn hide_secrets(text: &str) -> String {
    let word_char = |c: char| c.is_ascii_alphanumeric() || "_-.@+".contains(c);
    // A value runs to the next space, quote or closing bracket: it may hold `/`, `=` or `:`.
    let ends_value = |c: char| c.is_whitespace() || "\"'`,;)}]".contains(c);
    let mut out = String::with_capacity(text.len());
    let mut key = Key::No;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let value = key == Key::Assigned && !ends_value(c);
        if !value && !word_char(c) {
            key = match (key, c) {
                (Key::Named, '=' | ':') => Key::Assigned,
                (Key::Named, ' ' | '"' | '\'') => Key::Named,
                (Key::Assigned, ' ' | '"' | '\'' | '`') => Key::Assigned,
                _ => Key::No,
            };
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = match value {
            true => rest.find(ends_value),
            false => rest.find(|c: char| !word_char(c)),
        }
        .unwrap_or(rest.len());
        // A full stop that ends a sentence is not part of the word.
        let word = rest[..end].trim_end_matches('.');
        let stops = &rest[word.len()..end];
        let bearer = word.eq_ignore_ascii_case("bearer");
        out.push_str(if bearer {
            word
        } else if !word.is_empty() && (value || is_secret(word)) {
            "<redacted>"
        } else if is_email(word) {
            "<email>"
        } else {
            word
        });
        key = if bearer {
            Key::Assigned
        } else if !value && names_a_secret(word) {
            Key::Named
        } else {
            Key::No
        };
        out.push_str(stops);
        rest = &rest[end..];
    }
    out
}

fn names_a_secret(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    ["token", "secret", "password", "passwd", "_key", "-key", "apikey", "authorization"].iter().any(|name| word.ends_with(name))
}

fn is_email(word: &str) -> bool {
    word.split_once('@').is_some_and(|(name, host)| !name.is_empty() && host.contains('.') && !host.starts_with('.') && !host.contains('@'))
}

/// A vendor's key by its prefix, or any long mix of letters and digits that is not one of
/// Codly's ids (UUIDs).
fn is_secret(word: &str) -> bool {
    const PREFIXES: [&str; 15] = ["sk-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-", "xapp-", "AKIA", "AIza"];
    let mixed = word.bytes().any(|b| b.is_ascii_digit()) && word.bytes().any(|b| b.is_ascii_alphabetic());
    (word.len() >= 20 && PREFIXES.iter().any(|p| word.starts_with(p))) || (word.len() >= 32 && mixed && !is_uuid(word))
}

fn is_uuid(word: &str) -> bool {
    word.len() == 36 && word.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

#[cfg(test)]
mod tests {
    use codly_core::model::{InstallHint, Provider};

    use super::*;

    const HOME: &str = "/Users/ana";
    const DATA: &str = "/Users/ana/Library/Application Support/dev.codly.desktop";

    fn clean(text: &str) -> String {
        scrub(text, Some(Path::new(HOME)), Path::new(DATA))
    }

    #[test]
    fn folders_of_yours_are_hidden() {
        assert_eq!(clean("could not reopen /Users/ana/work/acme-secret/repo: not a git repository"), "could not reopen ~/…: not a git repository");
        assert_eq!(clean("indexed 12 files in /Users/ana/work/acme"), "indexed 12 files in ~/…");
        assert_eq!(clean("home is /Users/ana."), "home is ~.");
        // Codly's own folders keep their first name: it says which part failed.
        assert_eq!(clean(&format!("keeping {DATA}/worktrees/acme-t12/src: it has changes")), "keeping <data>/worktrees/…: it has changes");
        assert_eq!(clean(&format!("cannot open `{DATA}/codly.sqlite`")), "cannot open `<data>/codly.sqlite`");
        assert_eq!(clean("/Users/anabel/notes is not yours"), "/Users/anabel/notes is not yours");
        assert_eq!(clean("(see /Users/ana/work/repo), then /Users/ana and /Users/ana-old."), "(see ~/…), then ~ and /Users/ana-old.");
    }

    #[test]
    fn a_space_in_a_folder_name_does_not_end_the_path() {
        assert_eq!(clean("/Users/ana/Client Work/acme-secret/repo: not a git repository"), "~/…: not a git repository");
        assert_eq!(clean("indexed 12 files in /Users/ana/Client Work/acme-secret/repo"), "indexed 12 files in ~/…");
        assert_eq!(clean("renamed /Users/ana/My Docs/a.txt to /Users/ana/My Docs/b.txt (2 files)"), "renamed ~/… to ~/… (2 files)");
        assert_eq!(clean(&format!("copied /Users/ana/My Docs/a.txt to {DATA}/worktrees/acme-t12/a.txt")), "copied ~/… to <data>/worktrees/…");
        // In quotes, the path is all of what they hold.
        assert_eq!(clean(r#"skipped path="/Users/ana/My Projects/Client Work" kind=repo"#), r#"skipped path="~/…" kind=repo"#);
        assert_eq!(clean("fatal: '/Users/ana/Client Work' does not appear to be a git repository"), "fatal: '~/…' does not appear to be a git repository");
        // A word with a `/` later in the sentence is taken for a part of the path: more is hidden, never less.
        assert_eq!(clean("/Users/ana/My Repo has no branch codly/t12: skipped"), "~/…: skipped");
        // What stays: the last words of a last name with spaces, when no quotes hold the path.
        assert_eq!(clean("/Users/ana/work/My Repo is not a git repository"), "~/… Repo is not a git repository");
        assert_eq!(clean("one /Users/ana/a b\ntwo /Users/ana/c d/e"), "one ~/… b\ntwo ~/…");
    }

    #[test]
    fn emails_keys_and_tokens_are_hidden() {
        assert_eq!(clean("signed in as ana@example.com."), "signed in as <email>.");
        assert_eq!(clean("npm i @anthropic-ai/claude-code"), "npm i @anthropic-ai/claude-code");
        // The shape of `codly serve`'s token (built here: no key-like text in the source).
        let hex: String = (0..48).map(|i| char::from_digit(i % 16, 16).unwrap()).collect();
        assert_eq!(clean(&format!("http://127.0.0.1:4280/?token={hex}")), "http://127.0.0.1:4280/?token=<redacted>");
        assert_eq!(clean(&format!("the link {hex} was shared")), "the link <redacted> was shared");
        let prefixed = ["sk", "ant", "api03", "abcdefghij"].join("-");
        assert_eq!(clean(&format!("key {prefixed} rejected")), "key <redacted> rejected");
        assert_eq!(clean("AKIAIOSFODNN7EXAMPLE"), "<redacted>");
        assert_eq!(clean(r#"{"password": "hunter2", "user": "ana"}"#), r#"{"password": "<redacted>", "user": "ana"}"#);
        assert_eq!(clean("API_KEY=abc123 codex"), "API_KEY=<redacted> codex");
        // A value is hidden whole, whatever it is made of.
        assert_eq!(clean("aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY (from env)"), "aws_secret_access_key = <redacted> (from env)");
        assert_eq!(clean("password=/p:w+d== and more."), "password=<redacted> and more.");
        assert_eq!(clean("token=, secret: `hunter2`"), "token=, secret: `<redacted>`");
        assert_eq!(clean("Authorization: Bearer abc.def"), "Authorization: Bearer <redacted>");
        assert_eq!(clean("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc"), "<redacted>");
    }

    #[test]
    fn what_helps_a_bug_report_stays() {
        for line in [
            "2026-11-22T10:15:30.123456Z  WARN codly_core::orchestrator::plans: could not start task 01890a5d-ac96-774b-bcce-b302099a8057: no agent is free",
            "the agent used 1200 tokens",
            "claude: no pending approval 7",
            "panic in thread 'tokio-runtime-worker' at crates/codly-core/src/orchestrator/ingest.rs:23: boom",
        ] {
            assert_eq!(clean(line), line);
        }
    }

    fn cli(provider: Provider, path: Option<&str>) -> CliInfo {
        CliInfo {
            provider,
            path: path.map(String::from),
            version: path.map(|_| "2.1.0".into()),
            logged_in: path.map(|_| true),
            billing: Billing::Subscription,
            plan: Some("Acme Inc team".into()),
            problem: None,
            transport: "stream-json".into(),
            install: InstallHint { npm: None, native: None, login: None, docs: String::new(), update: None, npm_available: false, update_to: None },
            default_model: None,
        }
    }

    #[test]
    fn the_report_names_the_app_the_system_the_clis_and_the_log() {
        let mut codex = cli(Provider::Codex, Some("/opt/homebrew/bin/codex"));
        codex.logged_in = Some(false);
        codex.problem = Some("not signed in as ana@example.com: run `codex login`".into());
        let mut facts = Facts {
            system: "macOS 15.1 (aarch64)".into(),
            core: format!("did not start: cannot open {DATA}/codly.sqlite"),
            home: Some(HOME.into()),
            data_dir: DATA.into(),
            database: Some(3_200_000),
            log: Some(Ok(vec!["INFO indexed 12 files in /Users/ana/work/acme".into(), format!("WARN token={}", "0f".repeat(16))])),
            clis: vec![cli(Provider::Claude, Some("/Users/ana/.local/bin/claude")), codex, cli(Provider::Gemini, None)],
        };
        assert_eq!(
            report(&facts),
            format!(
                "Codly {VERSION}\n\
                 System: macOS 15.1 (aarch64)\n\
                 Core: did not start: cannot open <data>/codly.sqlite\n\
                 Data folder: ~/Library/Application Support/dev.codly.desktop\n\
                 Database: 3.2 MB\n\
                 \n\
                 Agent CLIs\n\
                 - claude 2.1.0 at ~/.local/bin/claude (stream-json): signed in, billing subscription\n\
                 - codex 2.1.0 at /opt/homebrew/bin/codex (stream-json): not signed in, billing subscription, problem: not signed in as <email>: run `codex login`\n\
                 - gemini: not installed\n\
                 \n\
                 Last 2 log lines\n\
                 INFO indexed 12 files in ~/…\n\
                 WARN token=<redacted>\n"
            )
        );
        // The plan label can name your company: it is never in the report.
        assert!(!report(&facts).contains("Acme"));

        facts.log = None;
        facts.database = None;
        let text = report(&facts);
        assert!(text.contains("Database: not found\n") && text.ends_with("\nNo log file: it could not be opened.\n"), "{text}");
        facts.log = Some(Err("permission denied".into()));
        assert!(report(&facts).ends_with("\nThe log could not be read: permission denied\n"));
    }
}
