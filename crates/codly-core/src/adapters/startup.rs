//! Start-up failures in plain words. A CLI that will not start answers with an option parser's
//! error or a line about tokens; this module reads that text and leads the error with what to
//! run: update the CLI, or sign in. The CLI's own words stay in the error, after the advice.

use std::path::Path;

use anyhow::bail;

use super::SpawnSpec;
use crate::env;
use crate::model::Provider;

/// How option parsers reject a flag or a subcommand they do not know (commander: Claude Code;
/// clap: Codex; yargs: Gemini CLI, OpenCode). Codly passes only options the minimum version
/// has, so a rejection nearly always means an older CLI. Lower case.
const REJECTED: [&str; 7] =
    ["unknown option", "unknown argument", "unrecognized option", "unrecognized subcommand", "unexpected argument", "allowed choices are", "invalid choice"];

/// How the CLIs say that nobody is signed in, or that the sign-in expired. Lower case.
const SIGNED_OUT: [&str; 8] = [
    "not logged in",
    "not signed in",
    "please run /login",
    "invalid api key",
    "oauth token has expired",
    "401 unauthorized",
    "authentication required",
    "set an auth method",
];

/// Does `text` (a CLI's own words) say that nobody is signed in, or that the sign-in expired?
pub(crate) fn says_signed_out(text: &str) -> bool {
    let text = text.to_lowercase();
    SIGNED_OUT.iter().any(|p| text.contains(p))
}

/// A process exit as text: "code 1", or "killed by a signal" when there is no code.
pub(crate) fn exit_text(code: Option<i64>) -> String {
    code.map_or_else(|| "killed by a signal".into(), |c| format!("code {c}"))
}

/// A run whose folder is gone (a removed worktree) fails to spawn with "No such file or
/// directory", which reads as a missing CLI. Say which of the two is missing.
pub(crate) fn check_cwd(cwd: &Path) -> anyhow::Result<()> {
    if !cwd.is_dir() {
        bail!("the working folder {} no longer exists", cwd.display());
    }
    Ok(())
}

/// "Codex is not signed in: run `codex login` in a terminal", with the account's `home` in the
/// command. `how` is what the CLI itself says to do (ACP `authMethods`), for a CLI whose
/// sign-in command Codly does not know.
pub(crate) fn not_signed_in(provider: Provider, home: Option<&str>, how: Option<&str>) -> String {
    let label = provider.label();
    match (env::login_command(provider, home), how) {
        (Some(cmd), _) => format!("{label} is not signed in: run `{cmd}` in a terminal"),
        (None, Some(how)) => format!("{label} is not signed in: {how}"),
        (None, None) => format!("{label} is not signed in: sign in with its CLI in a terminal first"),
    }
}

/// The config home of the account this run uses (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`…), if set.
pub(crate) fn account_home(spec: &SpawnSpec) -> Option<&str> {
    spec.env.get(env::account_var(spec.provider)?).map(String::as_str)
}

/// Does this run pay with the user's own API credentials (BYOK) instead of the CLI's sign-in?
/// `env::child_env` removes the vendor's key variable unless BYOK put it there.
pub(crate) fn uses_own_key(spec: &SpawnSpec) -> bool {
    spec.byok_command.is_some() || crate::byok::Vendor::of(spec.provider).is_some_and(|v| spec.env.contains_key(v.env_var()))
}

/// What to do about a start-up failure, read from the CLI's own `text` (stderr or a protocol
/// error); `version` is the CLI's, when the handshake got far enough to learn it.
/// - an option was rejected, or the version is below the minimum: update the CLI;
/// - the text says signed out: the sign-in command. Not with `own_key`: there the credential
///   is the user's API key, and signing in would not help.
fn advice(provider: Provider, text: &str, version: Option<&str>, home: Option<&str>, own_key: bool) -> Option<String> {
    let text = text.to_lowercase();
    let says = |patterns: &[&str]| patterns.iter().any(|p| text.contains(p));
    if says(&REJECTED) {
        return env::update_advice(provider).map(|update| format!("this {} does not accept an option Codly needs. {update}", provider.label()));
    }
    if says_signed_out(&text) {
        return (!own_key).then(|| not_signed_in(provider, home, None));
    }
    let version = version?;
    env::update_to(provider, version)?;
    env::update_advice(provider).map(|update| format!("{} {version} is older than Codly supports. {update}", provider.label()))
}

/// The start-up error to show for `raw`: led by the advice when its text says why it failed,
/// unchanged otherwise (also when it already is that advice).
pub(crate) fn explain(spec: &SpawnSpec, version: Option<&str>, raw: anyhow::Error) -> anyhow::Error {
    let text = format!("{raw:#}");
    match advice(spec.provider, &text, version, account_home(spec), uses_own_key(spec)) {
        Some(advice) if !text.contains(&advice) => raw.context(advice),
        _ => raw,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anyhow::anyhow;

    use super::*;
    use crate::adapters::SessionMode;
    use crate::model::Role;

    fn spec(provider: Provider, env: &[(&str, &str)]) -> SpawnSpec {
        SpawnSpec {
            agent_id: "a".into(),
            run_id: "r".into(),
            provider,
            role: Role::Worker,
            model: None,
            cwd: "/tmp".into(),
            bin: "/bin/cli".into(),
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(),
            session: SessionMode::New { session_id: None },
            session_cost_usd: 0.0,
            system_prompt: String::new(),
            read_only: false,
            mcp: None,
            use_user_setup: false,
            sandbox_network: false,
            raw_log: "/tmp/raw.ndjson".into(),
            byok_command: None,
        }
    }

    #[test]
    fn exit_codes_read_as_numbers() {
        assert_eq!(exit_text(Some(1)), "code 1");
        assert_eq!(exit_text(None), "killed by a signal");
    }

    #[test]
    fn a_missing_working_folder_is_named() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_cwd(dir.path()).is_ok());
        let gone = dir.path().join("worktree");
        let e = check_cwd(&gone).unwrap_err().to_string();
        assert_eq!(e, format!("the working folder {} no longer exists", gone.display()));
    }

    #[test]
    fn a_rejected_option_means_update_the_cli() {
        let update = |p, text| advice(p, text, None, None, false).unwrap();
        // commander (Claude Code), clap (Codex), yargs (Gemini CLI)
        let claude = update(Provider::Claude, "error: unknown option '--tools'");
        assert_eq!(claude, "this Claude Code does not accept an option Codly needs. Update Claude Code to 2.1.302 or later: run `claude update` in a terminal");
        assert!(update(Provider::Claude, "error: option '--permission-mode <mode>' argument 'manual' is invalid. Allowed choices are default, plan.").contains("claude update"));
        assert!(update(Provider::Codex, "error: unrecognized subcommand 'app-server'").contains("Update Codex to 0.161.0 or later"));
        assert!(update(Provider::Gemini, "Unknown arguments: acp, skip-trust").contains("Update Gemini CLI to 0.62.0 or later"));
        assert_eq!(advice(Provider::Cursor, "error: unknown option '--acp'", None, None, false), None, "no minimum version, no advice");
    }

    #[test]
    fn signed_out_text_becomes_the_sign_in_command() {
        let said = |p, text| advice(p, text, None, None, false);
        assert_eq!(said(Provider::Claude, "Invalid API key · Please run /login").as_deref(), Some("Claude Code is not signed in: run `claude auth login` in a terminal"));
        assert_eq!(said(Provider::Codex, "Not logged in").as_deref(), Some("Codex is not signed in: run `codex login` in a terminal"));
        assert!(said(Provider::Gemini, "Please set an Auth method in your settings.json").unwrap().contains("run `gemini` in a terminal"));
        // The account's own home goes into the command.
        let work = advice(Provider::Claude, "OAuth token has expired", None, Some("/Users/x/.claude-work"), false).unwrap();
        assert_eq!(work, "Claude Code is not signed in: run `CLAUDE_CONFIG_DIR=/Users/x/.claude-work claude auth login` in a terminal");
        // With your own API key, signing in is not the fix: the CLI's words stand alone.
        assert_eq!(advice(Provider::Claude, "Invalid API key", None, None, true), None);
        assert_eq!(said(Provider::Claude, "ENOSPC: no space left on device"), None);

        assert_eq!(not_signed_in(Provider::Opencode, None, Some("Run `opencode auth login` in the terminal")), "OpenCode is not signed in: Run `opencode auth login` in the terminal");
        assert_eq!(not_signed_in(Provider::Cursor, None, None), "Cursor is not signed in: sign in with its CLI in a terminal first");
    }

    #[test]
    fn a_known_old_version_is_blamed_only_when_nothing_else_explains() {
        let old = advice(Provider::Codex, "invalid params: unknown field `threadSource`", Some("0.150.2"), None, false).unwrap();
        assert_eq!(old, "Codex 0.150.2 is older than Codly supports. Update Codex to 0.161.0 or later: run `npm install -g @openai/codex@latest` in a terminal");
        assert_eq!(advice(Provider::Codex, "invalid params", Some("0.161.0"), None, false), None, "a supported version: the CLI's words stand alone");
        assert!(advice(Provider::Codex, "Not logged in", Some("0.150.2"), None, false).unwrap().contains("codex login"), "what the CLI says comes first");
    }

    #[test]
    fn the_advice_leads_and_the_clis_words_stay() {
        let claude = spec(Provider::Claude, &[("CLAUDE_CONFIG_DIR", "/Users/x/.claude-work")]);
        assert_eq!(account_home(&claude), Some("/Users/x/.claude-work"));
        let e = explain(&claude, None, anyhow!("claude exited during startup (code 1): Not logged in · Please run /login"));
        assert_eq!(e.to_string(), "Claude Code is not signed in: run `CLAUDE_CONFIG_DIR=/Users/x/.claude-work claude auth login` in a terminal");
        assert!(format!("{e:#}").ends_with("claude exited during startup (code 1): Not logged in · Please run /login"), "{e:#}");

        let plain = explain(&claude, None, anyhow!("claude did not answer the initialize request within 30 s"));
        assert_eq!(format!("{plain:#}"), "claude did not answer the initialize request within 30 s");
        // An error that already is the advice (Codex's signed-out start) is not said twice.
        let said = not_signed_in(Provider::Codex, None, None);
        assert_eq!(format!("{:#}", explain(&spec(Provider::Codex, &[]), None, anyhow!(said.clone()))), said);

        // BYOK: the saved key is in the env, or a token command is set.
        let keyed = spec(Provider::Claude, &[("ANTHROPIC_API_KEY", "sk-ant-test")]);
        assert!(uses_own_key(&keyed) && !uses_own_key(&claude));
        assert_eq!(format!("{:#}", explain(&keyed, None, anyhow!("Invalid API key"))), "Invalid API key");
        let command = SpawnSpec { byok_command: Some("op read key".into()), ..spec(Provider::Codex, &[]) };
        assert!(uses_own_key(&command));
        assert!(!uses_own_key(&spec(Provider::Gemini, &[("GEMINI_API_KEY", "x")])), "no BYOK for Gemini");
    }
}
