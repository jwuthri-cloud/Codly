//! Start-up failures in plain words. A CLI that will not start answers with an option parser's
//! error or a line about tokens; this module reads that text and leads the error with what to
//! run: update the CLI, or sign in. The CLI's own words stay in the error, after the advice.

use std::path::Path;

use anyhow::bail;

use super::SpawnSpec;
use super::claude::{self, SandboxNotOn};
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

/// The start error of a run that was to resume a conversation its CLI no longer has (Claude
/// Code's own words: `session_trouble`). It is the context of the CLI's error, so
/// `Error::is` finds it. After this failure the conversation is known to be gone, and the
/// orchestrator starts a new one in its place (`Inner::resume_or_new`).
#[derive(Debug)]
pub struct SessionGone;

/// What a `SessionGone` error says.
const SESSION_GONE: &str = "Claude Code no longer has this conversation";

impl std::fmt::Display for SessionGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(SESSION_GONE)
    }
}

impl std::error::Error for SessionGone {}

/// What Claude Code says when it cannot open the conversation it was started for, in plain
/// words (its texts: docs/research/cli-claude-stream-json-2.1.311.md §7). With `--resume` of a
/// conversation it no longer has, and with `--session-id` of one it already has, it exits
/// with code 1 before it reads anything.
pub(crate) fn session_trouble(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();
    if text.contains("no conversation found with session id") {
        Some(SESSION_GONE)
    } else if text.contains("session id") && text.contains("is already in use") {
        Some("Claude Code already has a conversation with the id Codly gave this new one")
    } else {
        None
    }
}

/// Claude Code that ended at its start because the shell sandbox cannot start: `text` (its
/// stderr) names the setting that made it end there, `sandbox.failIfUnavailable`, which
/// `claude::settings_json` passes. The same refusal as when the CLI starts and says that its
/// sandbox is off. The stderr text is read in the 2.1.311 bundle ("Set
/// sandbox.failIfUnavailable=false to allow unsandboxed execution"), not recorded
/// (docs/research/cli-claude-stream-json-2.1.311-part2.md, scenario 16): no machine at hand
/// has a sandbox that cannot start.
fn sandbox_trouble(text: &str) -> Option<SandboxNotOn> {
    text.to_lowercase().contains("failifunavailable").then(SandboxNotOn::off)
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

/// The sign-in of the user's shell that this run carries, as the names of its variables
/// ("CLAUDE_CODE_USE_BEDROCK"): `env::apply_shell_auth` left them in the run's env (Settings →
/// Safety, "Use my shell's Claude sign-in"). None for a run that signs in as the CLI does.
pub(crate) fn shell_sign_in(spec: &SpawnSpec) -> Option<String> {
    if spec.provider != Provider::Claude {
        return None;
    }
    env::shell_auth_names(&spec.env)
}

/// "Claude Code is not signed in: the sign-in your shell selects (…) was not accepted", for a
/// run on the shell's sign-in (`shell_sign_in`): the sign-in command would not help there.
fn shell_sign_in_refused(provider: Provider, names: &str) -> String {
    format!("{} is not signed in: {}", provider.label(), env::shell_auth_refused(names))
}

/// What a signed-out run tells the user to do: the sign-in command of its account, or, on the
/// shell's sign-in, to look at that.
pub(crate) fn sign_in_advice(spec: &SpawnSpec) -> String {
    match shell_sign_in(spec) {
        Some(names) => shell_sign_in_refused(spec.provider, &names),
        None => not_signed_in(spec.provider, account_home(spec), None),
    }
}

/// What to do about a start-up failure, read from the CLI's own `text` (stderr or a protocol
/// error); `version` is the CLI's, when the handshake got far enough to learn it.
/// - an option was rejected, or the version is below the minimum: update the CLI;
/// - the text says signed out: the sign-in command. Not with `own_key`: there the credential
///   is the user's API key, and signing in would not help. With `shell` (`shell_sign_in`), the
///   sign-in is the one the user's shell selects: the advice names its variables instead;
/// - Claude Code cannot open the conversation: that, in plain words.
fn advice(provider: Provider, text: &str, version: Option<&str>, home: Option<&str>, own_key: bool, shell: Option<&str>) -> Option<String> {
    let text = text.to_lowercase();
    let says = |patterns: &[&str]| patterns.iter().any(|p| text.contains(p));
    if says(&REJECTED) {
        return env::update_advice(provider).map(|update| format!("this {} does not accept an option Codly needs. {update}", provider.label()));
    }
    if says_signed_out(&text) {
        return (!own_key).then(|| match shell {
            Some(names) => shell_sign_in_refused(provider, names),
            None => not_signed_in(provider, home, None),
        });
    }
    if provider == Provider::Claude
        && let Some(why) = session_trouble(&text)
    {
        return Some(why.to_string());
    }
    let version = version?;
    env::update_to(provider, version)?;
    env::update_advice(provider).map(|update| format!("{} {version} is older than Codly supports. {update}", provider.label()))
}

/// The start-up error to show for `raw`: led by the advice when its text says why it failed,
/// unchanged otherwise (also when it already is that advice). A run with Claude Code's shell
/// sandbox that ended because the sandbox cannot start is led by a `SandboxNotOn`, in the
/// words of Codly's own check (`sandbox_trouble`): the CLI's advice names a setting that is
/// not the user's to change in Codly. A conversation that is gone
/// leads as the typed `SessionGone`.
pub(crate) fn explain(spec: &SpawnSpec, version: Option<&str>, raw: anyhow::Error) -> anyhow::Error {
    let text = format!("{raw:#}");
    if spec.provider == Provider::Claude
        && claude::sandboxed(spec)
        && let Some(not_on) = sandbox_trouble(&text)
    {
        return raw.context(not_on);
    }
    match advice(spec.provider, &text, version, account_home(spec), uses_own_key(spec), shell_sign_in(spec).as_deref()) {
        Some(advice) if text.contains(&advice) => raw,
        Some(advice) if advice == SESSION_GONE => raw.context(SessionGone),
        Some(advice) => raw.context(advice),
        None => raw,
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
            agent_name: "Ada".into(),
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
            local_settings: false,
            raw_log: "/tmp/raw.ndjson".into(),
            byok_command: None,
            sandbox_shell: false,
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
        let update = |p, text| advice(p, text, None, None, false, None).unwrap();
        // commander (Claude Code), clap (Codex), yargs (Gemini CLI)
        let claude = update(Provider::Claude, "error: unknown option '--tools'");
        assert_eq!(claude, "this Claude Code does not accept an option Codly needs. Update Claude Code to 2.1.302 or later: run `claude update` in a terminal");
        assert!(update(Provider::Claude, "error: option '--permission-mode <mode>' argument 'manual' is invalid. Allowed choices are default, plan.").contains("claude update"));
        assert!(update(Provider::Codex, "error: unrecognized subcommand 'app-server'").contains("Update Codex to 0.161.0 or later"));
        assert!(update(Provider::Gemini, "Unknown arguments: acp, skip-trust").contains("Update Gemini CLI to 0.62.0 or later"));
        assert_eq!(advice(Provider::Cursor, "error: unknown option '--acp'", None, None, false, None), None, "no minimum version, no advice");
    }

    #[test]
    fn signed_out_text_becomes_the_sign_in_command() {
        let said = |p, text| advice(p, text, None, None, false, None);
        assert_eq!(said(Provider::Claude, "Invalid API key · Please run /login").as_deref(), Some("Claude Code is not signed in: run `claude auth login` in a terminal"));
        assert_eq!(said(Provider::Codex, "Not logged in").as_deref(), Some("Codex is not signed in: run `codex login` in a terminal"));
        assert!(said(Provider::Gemini, "Please set an Auth method in your settings.json").unwrap().contains("run `gemini` in a terminal"));
        // The account's own home goes into the command.
        let work = advice(Provider::Claude, "OAuth token has expired", None, Some("/Users/x/.claude-work"), false, None).unwrap();
        assert_eq!(work, "Claude Code is not signed in: run `CLAUDE_CONFIG_DIR=/Users/x/.claude-work claude auth login` in a terminal");
        // With your own API key, signing in is not the fix: the CLI's words stand alone.
        assert_eq!(advice(Provider::Claude, "Invalid API key", None, None, true, None), None);
        assert_eq!(said(Provider::Claude, "ENOSPC: no space left on device"), None);

        assert_eq!(not_signed_in(Provider::Opencode, None, Some("Run `opencode auth login` in the terminal")), "OpenCode is not signed in: Run `opencode auth login` in the terminal");
        assert_eq!(not_signed_in(Provider::Cursor, None, None), "Cursor is not signed in: sign in with its CLI in a terminal first");
    }

    /// The CLI's own lines, from the recordings `resume-missing` and `session-id-in-use`
    /// (docs/research/fixtures/claude-2.1.311): what it wrote to stderr before exit code 1.
    #[test]
    fn a_conversation_claude_cannot_open_is_said_in_plain_words() {
        let stderr = |name: &str| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures/claude-2.1.311").join(name);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            lines.iter().find_map(|l| l["_stderr"].as_str().map(str::to_owned)).expect("a stderr line")
        };
        let claude = spec(Provider::Claude, &[]);
        let gone = stderr("resume-missing.ndjson");
        assert_eq!(gone, "No conversation found with session ID: sess-1");
        let e = explain(&claude, None, anyhow!("claude exited during startup (code 1): {gone}"));
        assert_eq!(e.to_string(), "Claude Code no longer has this conversation");
        assert!(format!("{e:#}").ends_with("claude exited during startup (code 1): No conversation found with session ID: sess-1"), "{e:#}");
        // Typed: the orchestrator knows by it that the conversation is gone.
        assert!(e.is::<SessionGone>());
        let again = explain(&claude, None, e);
        assert!(again.is::<SessionGone>() && again.chain().count() == 2, "explained once: {again:#}");

        let taken = stderr("session-id-in-use.ndjson");
        assert_eq!(taken, "Error: Session ID sess-1 is already in use.");
        let e = explain(&claude, None, anyhow!("claude exited during startup (code 1): {taken}"));
        assert_eq!(e.to_string(), "Claude Code already has a conversation with the id Codly gave this new one");
        assert!(!e.is::<SessionGone>(), "that conversation is not gone");
        assert!(!explain(&claude, None, anyhow!("claude did not answer the initialize request within 30 s")).is::<SessionGone>());

        // Claude Code's words only: another CLI's text stands as it is.
        assert_eq!(advice(Provider::Codex, &gone, None, None, false, None), None);
        assert_eq!(session_trouble("ENOSPC: no space left on device"), None);
    }

    /// A run with the shell sandbox whose CLI ended at its start, because the sandbox cannot
    /// start, says what Codly's own check says, with the switch to turn off: not the CLI's
    /// advice about a setting. The stderr line is written from the text in the 2.1.311 bundle
    /// (part 2, scenario 16: not recorded), so only the setting's name in it is relied on.
    #[test]
    fn a_sandbox_that_cannot_start_is_said_in_plain_words() {
        let exited = || anyhow!("claude exited during startup (code 1): Error: sandbox required but unavailable. Set sandbox.failIfUnavailable=false to allow unsandboxed execution");
        let sandboxed = SpawnSpec { sandbox_shell: true, ..spec(Provider::Claude, &[]) };
        let e = explain(&sandboxed, None, exited());
        assert_eq!(
            e.to_string(),
            "Claude Code did not turn its sandbox on, so Codly did not start this agent. \
             Check Claude Code's sandbox on this computer, or turn off \"Sandbox Claude's shell\" in Settings → Safety"
        );
        assert!(e.is::<SandboxNotOn>(), "the callers that resume keep the session: {e:?}");
        assert!(format!("{e:#}").ends_with("Set sandbox.failIfUnavailable=false to allow unsandboxed execution"), "the CLI's words stay: {e:#}");

        // The refusal of Codly's own check is not said twice.
        let checked = explain(&sandboxed, None, SandboxNotOn::off().into());
        assert_eq!(format!("{checked:#}"), SandboxNotOn::off().to_string());

        // A run that did not ask for the sandbox (the switch is off, or an agent without a
        // shell), and another CLI: the switch is not the fix, so the CLI's words stand alone.
        let read_only = SpawnSpec { read_only: true, ..sandboxed.clone() };
        let codex = SpawnSpec { sandbox_shell: true, ..spec(Provider::Codex, &[]) };
        for other in [spec(Provider::Claude, &[]), read_only, codex] {
            let e = explain(&other, None, exited());
            assert!(!e.is::<SandboxNotOn>() && format!("{e:#}") == exited().to_string(), "{e:#}");
        }
    }

    #[test]
    fn a_known_old_version_is_blamed_only_when_nothing_else_explains() {
        let old = advice(Provider::Codex, "invalid params: unknown field `threadSource`", Some("0.150.2"), None, false, None).unwrap();
        assert_eq!(old, "Codex 0.150.2 is older than Codly supports. Update Codex to 0.161.0 or later: run `npm install -g @openai/codex@latest` in a terminal");
        assert_eq!(advice(Provider::Codex, "invalid params", Some("0.161.0"), None, false, None), None, "a supported version: the CLI's words stand alone");
        assert!(advice(Provider::Codex, "Not logged in", Some("0.150.2"), None, false, None).unwrap().contains("codex login"), "what the CLI says comes first");
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

    /// A run on the sign-in of the user's shell (Bedrock, Vertex or a token) is not sent to
    /// `claude auth login`: the advice names the variable that selects the sign-in.
    #[test]
    fn a_run_on_the_shells_sign_in_is_not_sent_to_the_login_command() {
        let bedrock = spec(Provider::Claude, &[("CLAUDE_CODE_USE_BEDROCK", "1"), ("CLAUDE_CONFIG_DIR", "/Users/x/.claude-work")]);
        assert_eq!(shell_sign_in(&bedrock).as_deref(), Some("CLAUDE_CODE_USE_BEDROCK"));
        let says = "Claude Code is not signed in: the sign-in your shell selects (CLAUDE_CODE_USE_BEDROCK) was not accepted. \
                    Run `claude auth status` in a terminal to see why";
        assert_eq!(sign_in_advice(&bedrock), says);
        let e = explain(&bedrock, None, anyhow!("claude exited during startup (code 1): 401 Unauthorized"));
        assert_eq!(e.to_string(), says);
        assert!(!format!("{e:#}").contains("auth login"), "{e:#}");
        assert!(format!("{e:#}").ends_with("401 Unauthorized"), "the CLI's words stay: {e:#}");

        let token = spec(Provider::Claude, &[("CLAUDE_CODE_OAUTH_TOKEN", "test-token"), ("CLAUDE_CODE_USE_VERTEX", "")]);
        assert_eq!(shell_sign_in(&token).as_deref(), Some("CLAUDE_CODE_OAUTH_TOKEN"), "an empty variable selects nothing");
        let e = explain(&token, None, anyhow!("OAuth token has expired"));
        assert!(e.to_string().contains("(CLAUDE_CODE_OAUTH_TOKEN)") && !e.to_string().contains("test-token"), "{e}");

        // Without such a variable, and for another CLI: the sign-in command, as before.
        let plain = spec(Provider::Claude, &[]);
        assert_eq!((shell_sign_in(&plain), sign_in_advice(&plain).as_str()), (None, "Claude Code is not signed in: run `claude auth login` in a terminal"));
        let codex = spec(Provider::Codex, &[("CLAUDE_CODE_USE_BEDROCK", "1")]);
        assert_eq!((shell_sign_in(&codex), sign_in_advice(&codex).as_str()), (None, "Codex is not signed in: run `codex login` in a terminal"));
    }
}
