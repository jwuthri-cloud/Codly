//! Login-shell environment capture (GUI apps get launchd's minimal PATH), CLI detection
//! with version/auth/billing, and per-agent child environments with secrets scrubbed.
//! See docs/research/tauri-stack.md §5.10 and cli-claude-codex.md §5.9.
//!
//! CONTRACT (implemented by the adapters owner).
//!
//! Codly never reads tokens, keychains or credential files: auth state comes from each CLI's
//! own status command, or (Gemini) from the non-secret `selectedType` setting.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::model::{AccountConfig, AccountInfo, Billing, CliInfo, InstallHint, Provider};

const BEGIN: &[u8] = b"__CODLY_ENV_BEGIN__";
const END: &[u8] = b"__CODLY_ENV_END__";

/// Variables a Terminal login shell inherits from launchd. Nothing else from Codly's own
/// environment reaches the probe shell, so a dev build started inside another tool (a Claude
/// Code session exports 40-odd CLAUDE_CODE_* / ANTHROPIC_* variables) captures the same env as
/// the app launched from the Dock.
const SESSION_VARS: &[&str] =
    &["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "SSH_AUTH_SOCK", "__CF_USER_TEXT_ENCODING"];

/// Shell bookkeeping and our own probe markers, dropped from the captured env.
const PROBE_VARS: &[&str] = &["DISABLE_AUTO_UPDATE", "ZSH_TMUX_AUTOSTART", "CODLY_SHELL_PROBE", "PWD", "OLDPWD", "SHLVL", "_"];

/// `$SHELL -ilc '/usr/bin/env -0'` between markers, with a timeout. Never calls set_var.
pub fn capture_login_env(timeout: Duration) -> Option<HashMap<String, String>> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let home = std::env::var_os("HOME")?;
    let script = "printf '__CODLY_ENV_BEGIN__'; /usr/bin/env -0; printf '__CODLY_ENV_END__'";
    let mut cmd = std::process::Command::new(&shell);
    cmd.args(["-i", "-l", "-c", script]).env_clear();
    for k in SESSION_VARS {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("TERM", "dumb")
        .env("DISABLE_AUTO_UPDATE", "true") // oh-my-zsh updater prompt
        .env("ZSH_TMUX_AUTOSTART", "false") // oh-my-zsh tmux plugin
        .env("CODLY_SHELL_PROBE", "1") // lets users guard slow rc code
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0); // a timeout kills whatever the rc files started, too
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        let _ = tx.send(b);
    });
    let out = match rx.recv_timeout(timeout) {
        Ok(b) => b,
        Err(_) => {
            let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(child.id() as i32), nix::sys::signal::Signal::SIGKILL);
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let _ = child.wait();
    parse_env_block(&out)
}

fn parse_env_block(out: &[u8]) -> Option<HashMap<String, String>> {
    let start = memchr::memmem::find(out, BEGIN)? + BEGIN.len();
    let end = start + memchr::memmem::rfind(&out[start..], END)?;
    let env: HashMap<String, String> = out[start..end]
        .split(|&b| b == 0)
        .filter_map(|kv| {
            let (k, v) = std::str::from_utf8(kv).ok()?.split_once('=')?;
            (!k.is_empty() && !PROBE_VARS.contains(&k)).then(|| (k.to_owned(), v.to_owned()))
        })
        .collect();
    (!env.is_empty()).then_some(env)
}

/// Base environment for every child: captured login env (or the process env as a fallback)
/// with an effective PATH that also includes ~/.local/bin, ~/.bun/bin, every
/// ~/.nvm/versions/node/*/bin (newest first), /opt/homebrew/bin, /usr/local/bin…
pub fn base_env(captured: Option<HashMap<String, String>>) -> HashMap<String, String> {
    let mut env = captured.unwrap_or_else(|| std::env::vars().collect());
    for k in ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR"] {
        if !env.contains_key(k)
            && let Ok(v) = std::env::var(k)
        {
            env.insert(k.into(), v);
        }
    }
    let home = env.get("HOME").map(PathBuf::from).or_else(dirs::home_dir);
    let nvm = env.get("NVM_DIR").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".nvm")));
    let path = effective_path(env.get("PATH").map(String::as_str).unwrap_or(""), home.as_deref(), nvm.as_deref());
    env.insert("PATH".into(), path);
    env
}

fn effective_path(current: &str, home: Option<&Path>, nvm_dir: Option<&Path>) -> String {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(current).filter(|p| !p.as_os_str().is_empty()).collect();
    let mut extra: Vec<PathBuf> = Vec::new();
    if let Some(h) = home {
        for rel in [".local/bin", ".bun/bin", ".volta/bin", ".cargo/bin", "Library/pnpm", ".npm-global/bin", ".opencode/bin", ".claude/local"] {
            extra.push(h.join(rel));
        }
    }
    if let Some(nvm) = nvm_dir {
        // The nvm `codex` is a `#!/usr/bin/env node` shim: some node has to be on PATH.
        extra.extend(nvm_bins(nvm));
    }
    extra.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    for p in extra {
        if !dirs.contains(&p) && p.is_dir() {
            dirs.push(p);
        }
    }
    std::env::join_paths(dirs).ok().and_then(|s| s.into_string().ok()).unwrap_or_else(|| current.to_string())
}

/// `<nvm>/versions/node/v*/bin`, newest version first.
fn nvm_bins(nvm_dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(nvm_dir.join("versions/node")) else { return Vec::new() };
    let mut versions: Vec<(Vec<u64>, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let key: Vec<u64> = name.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect();
            Some((key, e.path().join("bin")))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().map(|(_, p)| p).collect()
}

fn bin_names(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &["claude"],
        Provider::Codex => &["codex"],
        Provider::Gemini => &["gemini"],
        Provider::Opencode => &["opencode"],
        // The ACP registry still launches `cursor-agent`; newer installs also ship `agent`.
        Provider::Cursor => &["cursor-agent", "agent"],
        Provider::Copilot => &["copilot"],
    }
}

/// Resolve a provider's binary on `base["PATH"]` (re-resolved at every spawn: symlinks move).
/// Returns the path as found on PATH, not canonicalized, so `~/.local/bin/claude` keeps
/// following the auto-updater's symlink.
pub fn resolve_bin(provider: Provider, base: &HashMap<String, String>) -> Option<PathBuf> {
    let path = base.get("PATH")?;
    let cwd = base.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    bin_names(provider).iter().find_map(|n| which::which_in(n, Some(path), &cwd).ok())
}

/// Models a CLI can run, for the start-agent picker. Codex and OpenCode print their catalog and
/// Claude takes aliases; the rest return none and take any name typed in. `openrouter_key` is the
/// opt-in OpenRouter key (byok.rs): OpenCode lists the OpenRouter models only when it has it.
pub async fn list_models(
    provider: Provider,
    base: &HashMap<String, String>,
    openrouter_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let args: &[&str] = match provider {
        Provider::Claude => return Ok(["opus", "sonnet", "haiku", "fable"].map(str::to_owned).to_vec()),
        Provider::Codex => &["debug", "models"],
        Provider::Opencode => &["models"],
        _ => return Ok(vec![]),
    };
    let bin = resolve_bin(provider, base).ok_or_else(|| format!("{} is not installed", bin_names(provider)[0]))?;
    let cwd = base.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let out = run(&bin, args, &child_env_with(base, provider, openrouter_key), &cwd).await?;
    if !out.ok {
        return Err(first_line(&out.text()));
    }
    parse_models(provider, &out.stdout)
}

fn parse_models(provider: Provider, stdout: &str) -> Result<Vec<String>, String> {
    if provider == Provider::Codex {
        let v = serde_json::from_str::<Value>(stdout.trim()).map_err(|_| "`codex debug models` printed no JSON".to_string())?;
        return Ok(v["models"].as_array().into_iter().flatten().filter_map(|m| m["slug"].as_str().map(str::to_owned)).collect());
    }
    Ok(stdout.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains(char::is_whitespace)).map(str::to_owned).collect())
}

/// Claude Code's aliases with their context windows. Every current model runs with 1M by default
/// (Claude Code 2.1.318 reports 1,000,000 for opus, sonnet, haiku and fable); the `[1m]` names stay
/// because saved settings may use them.
const CLAUDE_WINDOWS: [(&str, i64); 6] =
    [("opus[1m]", 1_000_000), ("sonnet[1m]", 1_000_000), ("opus", 1_000_000), ("sonnet", 1_000_000), ("haiku", 1_000_000), ("fable", 1_000_000)];

/// Models a CLI can run with their context window (tokens), in the CLI's own order: Claude's
/// aliases, Codex's listed catalog (`context_window`). Empty for a CLI that does not say.
pub async fn model_windows(provider: Provider, base: &HashMap<String, String>) -> Result<Vec<(String, i64)>, String> {
    match provider {
        Provider::Claude => Ok(CLAUDE_WINDOWS.iter().map(|(m, w)| (m.to_string(), *w)).collect()),
        Provider::Codex => {
            let bin = resolve_bin(provider, base).ok_or_else(|| "codex is not installed".to_string())?;
            let cwd = base.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
            let out = run(&bin, &["debug", "models"], &child_env(base, provider), &cwd).await?;
            if !out.ok {
                return Err(first_line(&out.text()));
            }
            parse_codex_windows(&out.stdout)
        }
        _ => Ok(vec![]),
    }
}

fn parse_codex_windows(stdout: &str) -> Result<Vec<(String, i64)>, String> {
    let v = serde_json::from_str::<Value>(stdout.trim()).map_err(|_| "`codex debug models` printed no JSON".to_string())?;
    Ok(v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str() != Some("hide"))
        .filter_map(|m| Some((m["slug"].as_str()?.to_owned(), m["context_window"].as_i64().filter(|w| *w > 0)?)))
        .collect())
}

/// The model with the biggest context window of `windows` (the first of equals), when it is
/// bigger than the agent's own: the window its runs reported, else its model's (`model` may
/// carry a ":effort"). None = nothing bigger to offer.
pub fn bigger_window(windows: &[(String, i64)], model: Option<&str>, reported: Option<i64>) -> Option<(String, i64)> {
    let best = windows.iter().fold(None::<&(String, i64)>, |b, x| if b.is_none_or(|b| x.1 > b.1) { Some(x) } else { b })?;
    let name = model.map(|m| m.rsplit_once(':').map_or(m, |(n, _)| n));
    let own = reported.filter(|w| *w > 0).or_else(|| windows.iter().find(|(m, _)| Some(m.as_str()) == name).map(|x| x.1));
    let bigger = match own {
        Some(w) => best.1 > w,
        None => name != Some(best.0.as_str()),
    };
    bigger.then(|| best.clone())
}

/// Integration path Codly uses for a provider (shown in diagnostics).
pub fn transport(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "stream-json",
        Provider::Codex => "app-server",
        _ => "acp",
    }
}

/// Detect every provider in parallel (≤ 5 s each): path, `--version`, auth status, billing.
pub async fn detect_all(base: &HashMap<String, String>) -> Vec<CliInfo> {
    let mut set = tokio::task::JoinSet::new();
    for (i, provider) in Provider::ALL.into_iter().enumerate() {
        let base = base.clone();
        set.spawn(async move { (i, detect(provider, &base).await) });
    }
    let mut out: Vec<(usize, CliInfo)> = set.join_all().await;
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, info)| info).collect()
}

const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

async fn detect(provider: Provider, base: &HashMap<String, String>) -> CliInfo {
    let mut info = CliInfo {
        provider,
        path: None,
        version: None,
        logged_in: None,
        billing: Billing::Unknown,
        plan: None,
        problem: None,
        transport: transport(provider).into(),
        install: install_hint(provider),
        default_model: None,
    };
    let Some(bin) = resolve_bin(provider, base) else {
        info.problem = Some(format!(
            "not installed: `{}` was not found on your login shell PATH",
            bin_names(provider)[0]
        ));
        return info;
    };
    info.path = Some(bin.to_string_lossy().into_owned());
    // Probe with the same scrubbed env the agents get, so billing reflects what they will use.
    let env = child_env(base, provider);
    let cwd = base.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let (version, auth, default_model) = tokio::join!(run(&bin, &["--version"], &env, &cwd), auth_status(provider, &bin, &env, &cwd), default_model(provider, &bin, &env, &cwd));
    info.default_model = default_model;
    match version {
        Ok(out) if out.ok => info.version = parse_version(&out.text()),
        Ok(out) => info.problem = Some(format!("`{} --version` failed: {}", bin_names(provider)[0], first_line(&out.text()))),
        Err(e) => info.problem = Some(format!("`{} --version` {e}", bin_names(provider)[0])),
    }
    let ((logged_in, billing, plan, problem), _) = auth;
    info.logged_in = logged_in;
    info.billing = billing;
    info.plan = plan;
    if info.problem.is_none() {
        info.problem = problem;
    }
    info
}

struct RunOut {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl RunOut {
    fn text(&self) -> String {
        if self.stdout.trim().is_empty() { self.stderr.clone() } else { self.stdout.clone() }
    }
}

async fn run(bin: &Path, args: &[&str], env: &HashMap<String, String>, cwd: &Path) -> Result<RunOut, String> {
    let fut = tokio::process::Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(DETECT_TIMEOUT, fut).await {
        Ok(Ok(o)) => Ok(RunOut {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }),
        Ok(Err(e)) => Err(format!("could not run: {e}")),
        Err(_) => Err(format!("timed out after {} s", DETECT_TIMEOUT.as_secs())),
    }
}

type Auth = (Option<bool>, Billing, Option<String>, Option<String>);

/// The sign-in state of the account `env` points at, and who is signed in when the CLI says.
async fn auth_status(provider: Provider, bin: &Path, env: &HashMap<String, String>, cwd: &Path) -> (Auth, Option<String>) {
    match provider {
        Provider::Claude => match run(bin, &["auth", "status"], env, cwd).await {
            Ok(out) => match serde_json::from_str::<Value>(out.stdout.trim()) {
                Ok(v) => (claude_auth(&v), claude_identity(&v)),
                Err(_) => {
                    ((None, Billing::Unknown, None, Some(format!("`claude auth status` printed no JSON: {}", first_line(&out.text())))), None)
                }
            },
            Err(e) => ((None, Billing::Unknown, None, Some(format!("`claude auth status` {e}"))), None),
        },
        Provider::Codex => match run(bin, &["login", "status"], env, cwd).await {
            Ok(out) => (codex_auth(out.ok, &out.text()), None),
            Err(e) => ((None, Billing::Unknown, None, Some(format!("`codex login status` {e}"))), None),
        },
        Provider::Gemini => (gemini_auth(gemini_selected_type(env).as_deref()), gemini_identity(env)),
        // OpenCode routes to whichever provider the chosen model belongs to (its free
        // `opencode/*` models need no login), so there is no single sign-in to check.
        Provider::Opencode => ((None, Billing::ThirdParty, None, None), None),
        // ponytail: Cursor/Copilot are docs-only (not installed here); add `agent status
        // --format json` / a Copilot probe once one can be verified.
        Provider::Cursor | Provider::Copilot => ((None, Billing::Unknown, None, None), None),
    }
}

/// The model a CLI runs when Codly passes none, as its own config sets it: Claude Code's
/// `ANTHROPIC_MODEL` or `settings.json` `model`; Codex's `config.toml` `model`, else the first
/// model of its catalog; Gemini's and OpenCode's settings. None = the CLI decides at run time.
/// ponytail: the default sign-in's config; an agent on another account (its own CODEX_HOME…)
/// may differ until its first run reports the real one.
async fn default_model(provider: Provider, bin: &Path, env: &HashMap<String, String>, cwd: &Path) -> Option<String> {
    let home = env.get("HOME").map(PathBuf::from);
    let json = |p: PathBuf| serde_json::from_str::<Value>(&std::fs::read_to_string(p).ok()?).ok();
    match provider {
        Provider::Claude => env.get("ANTHROPIC_MODEL").cloned().filter(|m| !m.is_empty()).or_else(|| {
            let dir = env.get("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| Some(home?.join(".claude")))?;
            json(dir.join("settings.json"))?["model"].as_str().map(str::to_owned)
        }),
        Provider::Codex => {
            let dir = env.get("CODEX_HOME").map(PathBuf::from).or_else(|| Some(home.clone()?.join(".codex")));
            if let Some(m) = dir.and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok()).and_then(|t| toml_model(&t)) {
                return Some(m);
            }
            let out = run(bin, &["debug", "models"], env, cwd).await.ok().filter(|o| o.ok)?;
            codex_default(&out.stdout)
        }
        Provider::Gemini => {
            let v = json(gemini_dir(env)?.join("settings.json"))?;
            v["model"]["name"].as_str().or(v["model"].as_str()).map(str::to_owned)
        }
        Provider::Opencode => json(home?.join(".config/opencode/opencode.json"))?["model"].as_str().map(str::to_owned),
        Provider::Cursor | Provider::Copilot => None,
    }
}

/// The top-level `model = "…"` of a Codex `config.toml` (before any `[table]`).
/// ponytail: a line scan, not a TOML parser; profiles are ignored.
fn toml_model(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .find_map(|l| {
            let v = l.strip_prefix("model")?.trim_start().strip_prefix('=')?;
            let v = v.split('#').next()?.trim().trim_matches(|c| c == '"' || c == '\'');
            (!v.is_empty()).then(|| v.to_owned())
        })
}

/// Codex's built-in default: the listed model it ranks first (`priority`).
fn codex_default(stdout: &str) -> Option<String> {
    let v = serde_json::from_str::<Value>(stdout.trim()).ok()?;
    v["models"]
        .as_array()?
        .iter()
        .filter(|m| m["visibility"].as_str() == Some("list"))
        .min_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX))?["slug"]
        .as_str()
        .map(str::to_owned)
}

/// `<GEMINI_CLI_HOME or HOME>/.gemini`.
fn gemini_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    env.get("GEMINI_CLI_HOME").or_else(|| env.get("HOME")).map(|h| PathBuf::from(h).join(".gemini"))
}

/// Gemini's chosen sign-in method (`security.auth.selectedType`): a setting, not a secret.
fn gemini_selected_type(env: &HashMap<String, String>) -> Option<String> {
    let text = std::fs::read_to_string(gemini_dir(env)?.join("settings.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["security"]["auth"]["selectedType"].as_str().map(str::to_owned)
}

/// The Google account Gemini signed in with (`google_accounts.json` holds emails, no tokens).
fn gemini_identity(env: &HashMap<String, String>) -> Option<String> {
    let text = std::fs::read_to_string(gemini_dir(env)?.join("google_accounts.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["active"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Billing badge for an agent whose protocol does not say (ACP): Gemini from its sign-in
/// setting, OpenCode is whatever provider its model belongs to.
pub fn billing_hint(provider: Provider, base: &HashMap<String, String>) -> Billing {
    match provider {
        Provider::Gemini => gemini_auth(gemini_selected_type(base).as_deref()).1,
        Provider::Opencode => Billing::ThirdParty,
        _ => Billing::Unknown,
    }
}

fn claude_auth(v: &Value) -> Auth {
    if v["loggedIn"].as_bool() != Some(true) {
        return (Some(false), Billing::Unknown, None, Some("not signed in: run `claude auth login` in a terminal".into()));
    }
    let provider = v["apiProvider"].as_str().unwrap_or("firstParty");
    let method = v["authMethod"].as_str().unwrap_or("");
    let (billing, plan) = if provider != "firstParty" {
        (Billing::ThirdParty, Some(provider.to_string()))
    } else if method == "claude.ai" {
        (Billing::Subscription, v["subscriptionType"].as_str().map(str::to_owned))
    } else if method.contains("api") || method.contains("console") {
        (Billing::ApiKey, Some("api key".into()))
    } else {
        (Billing::Unknown, None)
    };
    (Some(true), billing, plan, None)
}

/// "you@x.com · Org" from `claude auth status` (no tokens in it).
fn claude_identity(v: &Value) -> Option<String> {
    let parts: Vec<&str> = [v["email"].as_str(), v["orgName"].as_str()].into_iter().flatten().filter(|s| !s.is_empty()).collect();
    (!parts.is_empty() && v["loggedIn"].as_bool() == Some(true)).then(|| parts.join(" · "))
}

fn codex_auth(ok: bool, text: &str) -> Auth {
    let t = text.to_lowercase();
    if !ok || t.contains("not logged in") {
        return (Some(false), Billing::Unknown, None, Some("not signed in: run `codex login` in a terminal".into()));
    }
    if t.contains("chatgpt") {
        (Some(true), Billing::Subscription, Some("chatgpt".into()), None)
    } else if t.contains("api key") {
        (Some(true), Billing::ApiKey, Some("api key".into()), None)
    } else {
        (Some(true), Billing::Unknown, None, None)
    }
}

fn gemini_auth(selected: Option<&str>) -> Auth {
    match selected {
        Some("oauth-personal") => (None, Billing::Subscription, Some("google account".into()), None),
        Some("gemini-api-key") => (None, Billing::ApiKey, Some("api key".into()), None),
        Some("vertex-ai") => (None, Billing::ApiKey, Some("vertex ai".into()), None),
        Some(other) => (None, Billing::Unknown, Some(other.to_string()), None),
        None => (None, Billing::Unknown, None, Some("sign-in not configured: run `gemini` once in a terminal to choose how to sign in".into())),
    }
}

/// First "1.2.3"-looking token: "2.1.309 (Claude Code)", "codex-cli 0.161.0", "v1.18.31".
fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|t| t.trim_start_matches('v').trim_end_matches([',', ')', '(']))
        .find(|t| t.contains('.') && t.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_owned)
}

fn first_line(s: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no output").chars().take(200).collect()
}

/// Is `key` removed from `provider`'s child env? Codly-internal markers of a parent Claude
/// session always go (they hide sessions from `--resume`, change the entrypoint, or force
/// `--bare`-like behaviour); in subscription mode, so do the keys that would make *this*
/// provider silently bill an API account instead of the user's plan.
///
/// ponytail: v1 is subscription mode only; a BYOK mode keeps the provider's own key (and
/// Bedrock/Vertex users' CLAUDE_CODE_USE_* switches) and passes it explicitly.
fn scrubbed(key: &str, provider: Provider) -> bool {
    if key == "CLAUDECODE" || key.starts_with("CLAUDE_CODE_") || key.starts_with("CLAUDE_AGENT_SDK_") {
        return true;
    }
    // Base URLs and Vertex/ADC switches move billing as surely as a key does.
    // GOOGLE_CLOUD_PROJECT stays: Gemini Code Assist Standard/Enterprise sign-ins need it.
    const GOOGLE_BILLING: &[&str] = &["GOOGLE_GENAI_USE_VERTEXAI", "GOOGLE_APPLICATION_CREDENTIALS"];
    let keys: &[&str] = match provider {
        Provider::Claude => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"],
        Provider::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"],
        Provider::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        // OpenCode picks up any provider key it finds; its free models need none.
        Provider::Opencode => {
            &["OPENROUTER_API_KEY", "OPENAI_API_KEY", "OPENAI_BASE_URL", "ANTHROPIC_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"]
        }
        Provider::Cursor => &["CURSOR_API_KEY"],
        Provider::Copilot => &[],
    };
    keys.contains(&key) || (matches!(provider, Provider::Gemini | Provider::Opencode) && GOOGLE_BILLING.contains(&key))
}

/// Child env for one agent: `base` minus CLAUDECODE / CLAUDE_CODE_* / CLAUDE_AGENT_SDK_* and,
/// in subscription mode, API-key variables that would silently switch billing; plus
/// TERM=dumb, NO_COLOR=1 (Codex), DISABLE_AUTOUPDATER=1 (Claude).
///
/// The user's CLAUDE_CONFIG_DIR / CODEX_HOME pass through untouched (multi-account setups).
/// Claude agents also get CLAUDE_CODE_DISABLE_AUTO_MEMORY=1: auto memory is one directory
/// per repository, shared by every worktree, so parallel workers would race on one
/// MEMORY.md (memory-knowledge §5.6); Codly's own memory replaces it.
pub fn child_env(base: &HashMap<String, String>, provider: Provider) -> HashMap<String, String> {
    child_env_with(base, provider, None)
}

/// `child_env` plus the opt-in OpenRouter key (`byok.rs`): injected for OpenCode only, whatever
/// the provider the caller asks about; `None` is the subscription-only default.
pub fn child_env_with(base: &HashMap<String, String>, provider: Provider, openrouter_key: Option<&str>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> =
        base.iter().filter(|(k, _)| !scrubbed(k, provider)).map(|(k, v)| (k.clone(), v.clone())).collect();
    env.insert("TERM".into(), "dumb".into());
    match provider {
        Provider::Claude => {
            env.insert("DISABLE_AUTOUPDATER".into(), "1".into());
            env.insert("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into());
        }
        Provider::Codex => {
            env.insert("NO_COLOR".into(), "1".into());
        }
        Provider::Opencode => {
            if let Some(key) = openrouter_key {
                env.insert(crate::byok::ENV_VAR.into(), key.into());
            }
        }
        _ => {}
    }
    env
}

// ───────────── accounts (several sign-ins of one CLI) ─────────────

/// The variable that selects a CLI's sign-in (its config home), for CLIs whose credentials
/// it isolates. Verified against the installed CLIs:
/// - Claude Code: `CLAUDE_CONFIG_DIR`. On macOS the Keychain entry is keyed by its value, so
///   the variable unset and set to `~/.claude` are two different sign-ins.
/// - Codex: `CODEX_HOME` (auth.json, sessions and the app-server's state live there).
/// - Gemini CLI: `GEMINI_CLI_HOME`, the directory that holds `.gemini/` (OAuth creds are a file
///   there, unless GEMINI_FORCE_ENCRYPTED_FILE_STORAGE moves them to one shared Keychain entry).
///
/// ponytail: OpenCode signs in per model provider (not one subscription); Cursor and Copilot
/// are not installed here, so their switch is unverified. They get one account each.
pub fn account_var(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some("CLAUDE_CONFIG_DIR"),
        Provider::Codex => Some("CODEX_HOME"),
        Provider::Gemini => Some("GEMINI_CLI_HOME"),
        Provider::Opencode | Provider::Cursor | Provider::Copilot => None,
    }
}

/// Stable account id: "claude" for the default sign-in (variable unset), "claude:<home>".
pub fn account_id(provider: Provider, home: Option<&str>) -> String {
    match home {
        Some(h) => format!("{}:{h}", provider.as_str()),
        None => provider.as_str().to_string(),
    }
}

/// The home an account id stands for (the inverse of `account_id`): `Some(None)` for the
/// default sign-in, None if the id is not one of `provider`'s.
pub fn account_home(provider: Provider, id: &str) -> Option<Option<String>> {
    match id.strip_prefix(provider.as_str())? {
        "" => Some(None),
        rest => rest.strip_prefix(':').filter(|h| !h.is_empty()).map(|h| Some(h.to_string())),
    }
}

/// Point a child env at one sign-in: set the provider's account variable, or remove it.
pub fn apply_account(env: &mut HashMap<String, String>, provider: Provider, home: Option<&str>) {
    let Some(var) = account_var(provider) else { return };
    match home {
        Some(h) => env.insert(var.into(), h.into()),
        None => env.remove(var),
    };
}

/// The login shell's sign-in: the account variable's value in `base` (None = unset).
pub fn primary_home(base: &HashMap<String, String>, provider: Provider) -> Option<String> {
    let v = base.get(account_var(provider)?)?;
    normalize_home(v, base)
}

/// An absolute home path without a trailing slash; `~` expands to $HOME.
pub fn normalize_home(home: &str, base: &HashMap<String, String>) -> Option<String> {
    let h = home.trim();
    let h = match h.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{}{rest}", base.get("HOME")?),
        _ => h.to_string(),
    };
    let h = h.trim_end_matches('/');
    Path::new(h).is_absolute().then(|| h.to_string())
}

/// Folders in $HOME that look like extra sign-ins: `~/.claude-*` / `~/.claude_*` and
/// `~/.codex-*` (config homes themselves), `~/.gemini-*` holding a `.gemini/` (a Gemini home).
/// The CLI's own default folder is not one: it belongs to the unset variable.
pub fn discover_homes(base: &HashMap<String, String>, provider: Provider) -> Vec<String> {
    let prefix = match provider {
        Provider::Claude => ".claude",
        Provider::Codex => ".codex",
        Provider::Gemini => ".gemini",
        _ => return vec![],
    };
    let Some(home) = base.get("HOME").map(PathBuf::from) else { return vec![] };
    let Ok(rd) = std::fs::read_dir(&home) else { return vec![] };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let rest = name.strip_prefix(prefix)?;
            (rest.len() > 1 && rest.starts_with(['-', '_'])).then_some(e.path())
        })
        .filter(|p| provider != Provider::Gemini || p.join(".gemini").is_dir())
        .filter_map(|p| p.to_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

/// Display name of a sign-in: "default" when the variable is unset, else the home's folder
/// name without the CLI's prefix: `~/.claude-work` → "work", `~/gemini-perso` → "perso".
pub fn account_label(provider: Provider, home: Option<&str>) -> String {
    let Some(h) = home else { return "default".into() };
    let name = Path::new(h).file_name().and_then(|n| n.to_str()).unwrap_or(h);
    let bare = name.trim_start_matches('.');
    let short = bare
        .strip_prefix(provider.as_str())
        .map(|r| r.trim_start_matches(['-', '_', '.']))
        .filter(|r| !r.is_empty())
        .unwrap_or(bare);
    if short.is_empty() { name.to_string() } else { short.to_string() }
}

/// How to install and sign in `provider`: trusted constants (`install.rs`), never user input.
pub fn install_hint(provider: Provider) -> InstallHint {
    crate::install::hint(provider)
}

/// The command that signs an account in, for the user to run in a terminal.
pub fn login_command(provider: Provider, home: Option<&str>) -> Option<String> {
    let cmd = match provider {
        Provider::Claude => "claude auth login",
        Provider::Codex => "codex login",
        // First interactive run asks how to sign in.
        Provider::Gemini => "gemini",
        _ => return None,
    };
    Some(match (account_var(provider), home) {
        (Some(var), Some(h)) => format!("{var}={} {cmd}", shell_quote(h)),
        _ => cmd.to_string(),
    })
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Every account of every installed CLI: the ones in `configs` (in that order), the login
/// shell's, and — when `probe` — the CLI's default sign-in and homes found on disk. Each is
/// probed with its own env (≤ 5 s, in parallel); found homes that are not signed in are left
/// out. Without `probe` (fixed CLIs in tests) each provider has its configured accounts and
/// its primary one, with the CLI's detected state. `limits` are left empty (the orchestrator
/// fills them). Configured homes are normalized (`~` expanded, no trailing slash).
pub async fn detect_accounts(base: &HashMap<String, String>, clis: &[CliInfo], configs: &[AccountConfig], probe: bool) -> Vec<AccountInfo> {
    accounts_with(base, clis, configs, probe, None).await
}

/// `detect_accounts` after a settings change (labels, order, enabled, a new home): the
/// sign-in state of accounts in `known` (the previous result) is reused, so only configured
/// homes it lacks are probed; homes found on disk that `known` left out stay out unprobed.
pub async fn reconcile_accounts(
    base: &HashMap<String, String>,
    clis: &[CliInfo],
    configs: &[AccountConfig],
    probe: bool,
    known: &[AccountInfo],
) -> Vec<AccountInfo> {
    accounts_with(base, clis, configs, probe, Some(known)).await
}

async fn accounts_with(
    base: &HashMap<String, String>,
    clis: &[CliInfo],
    configs: &[AccountConfig],
    probe: bool,
    known: Option<&[AccountInfo]>,
) -> Vec<AccountInfo> {
    struct Candidate {
        provider: Provider,
        home: Option<String>,
        config: Option<AccountConfig>,
        primary: bool,
    }
    let norm = |h: &str| normalize_home(h, base).unwrap_or_else(|| h.to_string());
    let mut candidates: Vec<Candidate> = vec![];
    for cli in clis.iter().filter(|c| c.path.is_some()) {
        let p = cli.provider;
        if account_var(p).is_none() {
            candidates.push(Candidate { provider: p, home: None, config: None, primary: true });
            continue;
        }
        let primary = primary_home(base, p);
        let mut homes: Vec<(Option<String>, Option<AccountConfig>)> = vec![];
        for c in configs.iter().filter(|c| c.provider == p) {
            let home = c.home.as_deref().map(norm);
            if !homes.iter().any(|(h, _)| *h == home) {
                homes.push((home.clone(), Some(AccountConfig { home, ..c.clone() })));
            }
        }
        let mut extra = vec![primary.clone()];
        if probe {
            extra.push(None);
            extra.extend(discover_homes(base, p).into_iter().map(Some));
        }
        for h in extra {
            if !homes.iter().any(|(x, _)| *x == h) {
                homes.push((h, None));
            }
        }
        for (home, config) in homes {
            candidates.push(Candidate { provider: p, primary: home == primary, home, config });
        }
    }

    let cwd = base.get("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let known_state = |c: &Candidate| {
        let id = account_id(c.provider, c.home.as_deref());
        known?.iter().find(|a| a.id == id).map(|a| ((a.logged_in, a.billing, a.plan.clone(), a.problem.clone()), a.identity.clone()))
    };
    // With `known`, a found folder it left out was not signed in: drop it without a probe.
    let dropped = |c: &Candidate| known.is_some() && c.config.is_none() && !c.primary && known_state(c).is_none();
    let mut set = tokio::task::JoinSet::new();
    for (i, c) in candidates.iter().enumerate() {
        let cli = clis.iter().find(|x| x.provider == c.provider).cloned();
        let Some(cli) = cli else { continue };
        if !probe || account_var(c.provider).is_none() || known_state(c).is_some() || dropped(c) {
            continue;
        }
        let Some(bin) = cli.path.as_ref().map(PathBuf::from) else { continue };
        let mut env = child_env(base, c.provider);
        apply_account(&mut env, c.provider, c.home.as_deref());
        let (p, cwd) = (c.provider, cwd.clone());
        set.spawn(async move { (i, auth_status(p, &bin, &env, &cwd).await) });
    }
    let probed: HashMap<usize, (Auth, Option<String>)> = set.join_all().await.into_iter().collect();

    let mut out = vec![];
    for (i, c) in candidates.into_iter().enumerate() {
        if dropped(&c) {
            continue;
        }
        let cli = clis.iter().find(|x| x.provider == c.provider);
        let ((logged_in, billing, plan, problem), identity) = match probed.get(&i).cloned().or_else(|| known_state(&c)) {
            Some(a) => a,
            None => (cli.map_or((None, Billing::Unknown, None, None), |x| (x.logged_in, x.billing, x.plan.clone(), x.problem.clone())), None),
        };
        // A found folder nobody is signed in to is noise. Gemini has no sign-in check: the
        // Google account it remembers (identity) is the sign-in.
        if c.config.is_none() && !c.primary && logged_in != Some(true) && identity.is_none() {
            continue;
        }
        let has_configs = configs.iter().any(|x| x.provider == c.provider);
        let enabled = match &c.config {
            Some(cfg) => cfg.enabled,
            None => c.primary && !has_configs,
        };
        let label = c.config.as_ref().and_then(|x| x.label.clone()).filter(|l| !l.trim().is_empty());
        out.push(AccountInfo {
            id: account_id(c.provider, c.home.as_deref()),
            provider: c.provider,
            label: label.unwrap_or_else(|| account_label(c.provider, c.home.as_deref())),
            enabled,
            primary: c.primary,
            configured: c.config.is_some(),
            logged_in,
            billing,
            plan,
            identity,
            problem: problem.map(|p| match login_command(c.provider, c.home.as_deref()) {
                Some(cmd) if logged_in == Some(false) => format!("not signed in: run `{cmd}` in a terminal"),
                _ => p,
            }),
            login_command: login_command(c.provider, c.home.as_deref()),
            home: c.home,
            limits: vec![],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_model_from_codex_config_or_catalog() {
        let cfg = "# mine\nmodel_reasoning_effort = \"xhigh\"\nmodel = \"gpt-6-astra\" # the big one\n[profiles.fast]\nmodel = \"gpt-6-luna\"\n";
        assert_eq!(toml_model(cfg).as_deref(), Some("gpt-6-astra"));
        assert_eq!(toml_model("[profiles.fast]\nmodel = \"gpt-6-luna\"\n"), None);
        let cat = r#"{"models":[{"slug":"hidden","visibility":"hide","priority":0},{"slug":"b","visibility":"list","priority":2},{"slug":"a","visibility":"list","priority":1}]}"#;
        assert_eq!(codex_default(cat).as_deref(), Some("a"));
    }

    use super::*;

    #[test]
    fn parses_model_catalogs() {
        assert_eq!(parse_models(Provider::Opencode, "opencode/big-pickle\n\nopenrouter/x/y\n").unwrap(), ["opencode/big-pickle", "openrouter/x/y"]);
        let codex = r#"{"models":[{"slug":"gpt-6-luna"},{"display_name":"no slug"}]}"#;
        assert_eq!(parse_models(Provider::Codex, codex).unwrap(), ["gpt-6-luna"]);
        assert!(parse_models(Provider::Codex, "not json").is_err());
    }

    #[test]
    fn a_legend_is_offered_the_biggest_context_window() {
        let codex = r#"{"models":[{"slug":"gpt-6-sol","context_window":272000,"visibility":"list"},{"slug":"gpt-6.1-sol","context_window":500000,"visibility":"list"},
            {"slug":"secret","context_window":900000,"visibility":"hide"},{"slug":"gpt-6.1-terra","context_window":500000},{"slug":"no-window"}]}"#;
        let w = parse_codex_windows(codex).unwrap();
        assert_eq!(w, [("gpt-6-sol".into(), 272_000), ("gpt-6.1-sol".into(), 500_000), ("gpt-6.1-terra".into(), 500_000)], "hidden or windowless models are not offered");
        let best = Some(("gpt-6.1-sol".to_string(), 500_000));
        assert_eq!(bigger_window(&w, Some("gpt-6-sol:high"), None), best, "its model's window, effort aside; the first of equals");
        assert_eq!(bigger_window(&w, Some("gpt-6-sol"), Some(272_000)), best);
        assert_eq!(bigger_window(&w, Some("gpt-6.1-terra"), None), None, "already as big");
        assert_eq!(bigger_window(&w, None, Some(500_000)), None, "its runs report as big already");
        assert_eq!(bigger_window(&w, None, None), best, "the CLI default: unknown, so offered");
        assert_eq!(bigger_window(&w, Some("gpt-6.1-sol"), None), None);
        assert_eq!(bigger_window(&[], Some("x"), None), None, "a CLI that says nothing offers nothing");

        let claude: Vec<(String, i64)> = CLAUDE_WINDOWS.iter().map(|(m, w)| (m.to_string(), *w)).collect();
        assert_eq!(bigger_window(&claude, Some("sonnet"), Some(200_000)), Some(("opus[1m]".into(), 1_000_000)));
        assert_eq!(bigger_window(&claude, Some("sonnet[1m]:max"), None), None);
        assert_eq!(bigger_window(&claude, Some("opus"), Some(1_000_000)), None, "a 1M window reported");
        assert_eq!(bigger_window(&claude, Some("fable"), None), None, "Fable already has the biggest window");
        assert_eq!(bigger_window(&claude, Some("haiku"), None), None, "every alias runs with 1M: nothing bigger");
    }

    #[test]
    fn parses_env_block_with_multiline_values() {
        let out = b"motd noise\n__CODLY_ENV_BEGIN__PATH=/usr/bin:/bin\0MULTI=a\nb\0PWD=/Users/x\0SHLVL=2\0EMPTY=\0__CODLY_ENV_END__";
        let env = parse_env_block(out).unwrap();
        assert_eq!(env["PATH"], "/usr/bin:/bin");
        assert_eq!(env["MULTI"], "a\nb");
        assert_eq!(env["EMPTY"], "");
        assert!(!env.contains_key("PWD") && !env.contains_key("SHLVL"));
        assert!(parse_env_block(b"no markers").is_none());
    }

    #[test]
    fn effective_path_appends_existing_fallbacks_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for v in ["v20.15.1", "v22.23.3", "v26.6.0", "v22.9.0"] {
            std::fs::create_dir_all(home.join(".nvm/versions/node").join(v).join("bin")).unwrap();
        }
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let current = format!("/usr/bin:{}", home.join(".local/bin").display());
        let path = effective_path(&current, Some(home), Some(&home.join(".nvm")));
        let parts: Vec<&str> = path.split(':').collect();
        assert_eq!(parts[0], "/usr/bin");
        assert_eq!(parts.iter().filter(|p| p.ends_with(".local/bin")).count(), 1);
        let nvm: Vec<&&str> = parts.iter().filter(|p| p.contains(".nvm")).collect();
        assert!(nvm[0].contains("v26.6.0") && nvm[1].contains("v22.23.3") && nvm[2].contains("v22.9.0") && nvm[3].contains("v20.15.1"));
        assert!(!parts.iter().any(|p| p.contains(".bun")), "missing dirs are not added");
    }

    #[test]
    fn child_env_scrubs_per_provider() {
        let base: HashMap<String, String> = [
            ("PATH", "/usr/bin"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "claude-desktop"),
            ("CLAUDE_AGENT_SDK_VERSION", "0.3"),
            ("CLAUDE_CONFIG_DIR", "/Users/x/.claude-work"),
            ("CODEX_HOME", "/Users/x/.codex-work"),
            ("ANTHROPIC_API_KEY", "sk-ant"),
            ("ANTHROPIC_BASE_URL", "https://proxy"),
            ("OPENAI_API_KEY", "sk-oa"),
            ("GEMINI_API_KEY", "g"),
            ("TERM", "xterm-256color"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let claude = child_env(&base, Provider::Claude);
        for k in ["CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_AGENT_SDK_VERSION", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"] {
            assert!(!claude.contains_key(k), "{k} leaked");
        }
        assert_eq!(claude["CLAUDE_CONFIG_DIR"], "/Users/x/.claude-work");
        assert_eq!(claude["OPENAI_API_KEY"], "sk-oa", "other vendors' keys stay for the agent's own commands");
        assert_eq!(claude["DISABLE_AUTOUPDATER"], "1");
        assert_eq!(claude["CLAUDE_CODE_DISABLE_AUTO_MEMORY"], "1");
        assert_eq!(claude["TERM"], "dumb");

        let codex = child_env(&base, Provider::Codex);
        assert!(!codex.contains_key("OPENAI_API_KEY") && !codex.contains_key("CLAUDECODE"));
        assert_eq!(codex["CODEX_HOME"], "/Users/x/.codex-work");
        assert_eq!(codex["NO_COLOR"], "1");
        assert!(!codex.contains_key("DISABLE_AUTOUPDATER"));

        let gemini = child_env(&base, Provider::Gemini);
        assert!(!gemini.contains_key("GEMINI_API_KEY"));
        let opencode = child_env(&base, Provider::Opencode);
        assert!(!opencode.contains_key("OPENAI_API_KEY") && !opencode.contains_key("ANTHROPIC_API_KEY"));
    }

    /// Base URLs and Vertex/ADC switches move billing off the user's plan like a key does.
    #[test]
    fn child_env_scrubs_billing_switches() {
        let base: HashMap<String, String> = [
            "OPENAI_BASE_URL",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
        ]
        .into_iter()
        .map(|k| (k.to_string(), "x".to_string()))
        .collect();
        assert!(!child_env(&base, Provider::Codex).contains_key("OPENAI_BASE_URL"));
        for p in [Provider::Gemini, Provider::Opencode] {
            let env = child_env(&base, p);
            for k in ["GOOGLE_GENAI_USE_VERTEXAI", "GOOGLE_APPLICATION_CREDENTIALS"] {
                assert!(!env.contains_key(k), "{p:?} kept {k}");
            }
            assert_eq!(env["GOOGLE_CLOUD_PROJECT"], "x", "Code Assist sign-ins need the project");
        }
        assert!(!child_env(&base, Provider::Opencode).contains_key("OPENAI_BASE_URL"));
        assert_eq!(child_env(&base, Provider::Claude)["GOOGLE_CLOUD_PROJECT"], "x", "other vendors' settings stay");
    }

    #[test]
    fn versions_and_auth_parsing() {
        assert_eq!(parse_version("2.1.309 (Claude Code)").as_deref(), Some("2.1.309"));
        assert_eq!(parse_version("codex-cli 0.161.0\n").as_deref(), Some("0.161.0"));
        assert_eq!(parse_version("v1.18.31").as_deref(), Some("1.18.31"));
        assert_eq!(parse_version("no version here"), None);

        let sub = claude_auth(&serde_json::json!({"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","subscriptionType":"team"}));
        assert_eq!((sub.0, sub.1, sub.2.as_deref()), (Some(true), Billing::Subscription, Some("team")));
        let out = claude_auth(&serde_json::json!({"loggedIn":false}));
        assert_eq!(out.0, Some(false));
        assert!(out.3.unwrap().contains("claude auth login"));
        assert_eq!(claude_auth(&serde_json::json!({"loggedIn":true,"apiProvider":"bedrock"})).1, Billing::ThirdParty);

        assert_eq!(codex_auth(true, "Logged in using ChatGPT").1, Billing::Subscription);
        assert_eq!(codex_auth(true, "Logged in using an API key - sk-...").1, Billing::ApiKey);
        assert_eq!(codex_auth(false, "Not logged in").0, Some(false));

        assert_eq!(gemini_auth(Some("gemini-api-key")).1, Billing::ApiKey);
        assert_eq!(gemini_auth(Some("oauth-personal")).1, Billing::Subscription);
        assert!(gemini_auth(None).3.is_some());
    }

    #[tokio::test]
    async fn opencode_lists_the_openrouter_models_only_with_the_byok_key() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("opencode");
        std::fs::write(&bin, "#!/bin/sh\necho opencode/free\n[ -n \"$OPENROUTER_API_KEY\" ] && echo openrouter/a-model\nexit 0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.path().display());
        // A key in the login shell's env never counts: only the opt-in key reaches OpenCode.
        let base: HashMap<String, String> =
            [("PATH", path.as_str()), ("HOME", dir.path().to_str().unwrap()), ("OPENROUTER_API_KEY", "from-the-shell")]
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect();
        assert_eq!(list_models(Provider::Opencode, &base, None).await.unwrap(), ["opencode/free"]);
        assert_eq!(
            list_models(Provider::Opencode, &base, Some("sk-or-test")).await.unwrap(),
            ["opencode/free", "openrouter/a-model"]
        );
    }

    // ───────────── accounts ─────────────

    /// A temp $HOME with sign-in folders, and a fake `claude` whose `auth status` answers per
    /// CLAUDE_CONFIG_DIR (default and ~/.claude-work signed in) and counts its calls.
    struct Home {
        dir: tempfile::TempDir,
        base: HashMap<String, String>,
        clis: Vec<CliInfo>,
    }

    impl Home {
        fn new() -> Home {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let h = dir.path().canonicalize().unwrap();
            for d in [".claude", ".claude-work", ".claude-old", ".claudeish", ".gemini-perso/.gemini", ".gemini-empty"] {
                std::fs::create_dir_all(h.join(d)).unwrap();
            }
            std::fs::write(h.join(".gemini-perso/.gemini/settings.json"), r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#)
                .unwrap();
            std::fs::write(h.join(".gemini-perso/.gemini/google_accounts.json"), r#"{"active":"me@gmail.com","old":[]}"#).unwrap();
            let script = h.join("fake-claude");
            std::fs::write(
                &script,
                r#"#!/bin/sh
echo "$CLAUDE_CONFIG_DIR" >> "$HOME/probes"
case "$CLAUDE_CONFIG_DIR" in
  "") echo '{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"me@home.com","subscriptionType":"pro"}' ;;
  */.claude-work) echo '{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"me@work.com","orgName":"Work","subscriptionType":"max"}' ;;
  *) echo '{"loggedIn":false}' ;;
esac
"#,
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let base: HashMap<String, String> =
                [("HOME", h.to_str().unwrap()), ("PATH", "/usr/bin:/bin")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
            let cli = |provider, path: &str| CliInfo {
                provider,
                path: Some(path.into()),
                version: None,
                logged_in: Some(true),
                billing: Billing::Subscription,
                plan: None,
                problem: None,
                transport: "test".into(),
                install: install_hint(provider),
                default_model: None,
            };
            let clis = vec![cli(Provider::Claude, script.to_str().unwrap()), cli(Provider::Gemini, "/usr/bin/true"), cli(Provider::Opencode, "/usr/bin/true")];
            Home { dir, base, clis }
        }
        fn path(&self, rel: &str) -> String {
            self.dir.path().canonicalize().unwrap().join(rel).to_string_lossy().into_owned()
        }
        fn probes(&self) -> usize {
            std::fs::read_to_string(self.path("probes")).map(|s| s.lines().count()).unwrap_or(0)
        }
    }

    fn config(provider: Provider, home: Option<&str>, label: Option<&str>, enabled: bool) -> AccountConfig {
        AccountConfig { provider, home: home.map(String::from), label: label.map(String::from), enabled }
    }

    #[test]
    fn account_ids_round_trip() {
        assert_eq!(account_id(Provider::Claude, None), "claude");
        assert_eq!(account_home(Provider::Claude, "claude"), Some(None));
        assert_eq!(account_home(Provider::Claude, "claude:/Users/x/.claude-work"), Some(Some("/Users/x/.claude-work".into())));
        assert_eq!(account_home(Provider::Claude, "codex:/x"), None);
        assert_eq!(account_home(Provider::Claude, "claudex"), None);
        assert_eq!(account_home(Provider::Claude, "claude:"), None);
        let base: HashMap<String, String> = [("HOME".to_string(), "/Users/x".to_string())].into();
        assert_eq!(normalize_home("~/.claude-work/", &base).as_deref(), Some("/Users/x/.claude-work"));
        assert_eq!(normalize_home("relative", &base), None);
        assert_eq!(account_label(Provider::Gemini, Some("/Users/x/.gemini-perso")), "perso");
        assert_eq!(login_command(Provider::Claude, Some("/Users/x/My Claude")).unwrap(), "CLAUDE_CONFIG_DIR='/Users/x/My Claude' claude auth login");
    }

    #[tokio::test]
    async fn detects_signed_in_homes_and_keeps_only_the_primary_enabled() {
        let home = Home::new();
        let accounts = detect_accounts(&home.base, &home.clis, &[], true).await;
        let ids: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
        let work = format!("claude:{}", home.path(".claude-work"));
        let perso = format!("gemini:{}", home.path(".gemini-perso"));
        assert_eq!(ids, ["claude", work.as_str(), "gemini", perso.as_str(), "opencode"], "signed-out and look-alike folders are left out");

        let default = &accounts[0];
        assert!(default.primary && default.enabled && !default.configured);
        assert_eq!((default.label.as_str(), default.logged_in, default.plan.as_deref()), ("default", Some(true), Some("pro")));
        assert_eq!(default.identity.as_deref(), Some("me@home.com"));
        assert_eq!(default.login_command.as_deref(), Some("claude auth login"));

        let w = &accounts[1];
        assert!(!w.primary && !w.enabled, "a found sign-in is offered, not used, until you enable it");
        assert_eq!((w.label.as_str(), w.plan.as_deref(), w.identity.as_deref()), ("work", Some("max"), Some("me@work.com · Work")));
        assert_eq!(w.home.as_deref(), Some(home.path(".claude-work").as_str()));
        assert!(w.login_command.as_deref().unwrap().starts_with("CLAUDE_CONFIG_DIR="));

        let g = &accounts[3];
        assert_eq!((g.label.as_str(), g.billing, g.identity.as_deref()), ("perso", Billing::Subscription, Some("me@gmail.com")));
        assert!(accounts[2].primary && accounts[2].problem.as_deref().unwrap().contains("sign-in not configured"));
        assert!(accounts.iter().all(|a| a.limits.is_empty()));
        assert_eq!(home.probes(), 3, "default, work and old were probed");
    }

    #[tokio::test]
    async fn configured_accounts_come_first_in_your_order_with_your_labels() {
        let home = Home::new();
        let configs = [
            config(Provider::Claude, Some("~/.claude-work/"), Some("Work"), true),
            config(Provider::Claude, Some(&home.path(".claude-old")), None, true),
            config(Provider::Claude, None, None, false),
        ];
        let accounts = detect_accounts(&home.base, &home.clis, &configs, true).await;
        let claude: Vec<&AccountInfo> = accounts.iter().filter(|a| a.provider == Provider::Claude).collect();
        assert_eq!(claude.len(), 3);
        assert_eq!(claude[0].id, format!("claude:{}", home.path(".claude-work")), "~ expanded, trailing slash dropped");
        assert_eq!((claude[0].label.as_str(), claude[0].enabled, claude[0].configured), ("Work", true, true));
        let old = claude[1];
        assert_eq!((old.label.as_str(), old.enabled, old.logged_in), ("old", true, Some(false)), "a configured home stays, signed out");
        let cmd = old.login_command.clone().unwrap();
        assert_eq!(old.problem.as_deref(), Some(format!("not signed in: run `{cmd}` in a terminal").as_str()));
        assert!(claude[2].primary && !claude[2].enabled, "you disabled the default sign-in");

        // The login shell's own CLAUDE_CONFIG_DIR is the primary account.
        let mut base = home.base.clone();
        base.insert("CLAUDE_CONFIG_DIR".into(), "~/.claude-work".into());
        let accounts = detect_accounts(&base, &home.clis, &[], true).await;
        let primary: Vec<&str> = accounts.iter().filter(|a| a.primary).map(|a| a.id.as_str()).collect();
        assert_eq!(primary[0], format!("claude:{}", home.path(".claude-work")));
        let default = accounts.iter().find(|a| a.id == "claude").unwrap();
        assert!(!default.primary && !default.enabled, "the unset variable is another sign-in");
    }

    #[tokio::test]
    async fn reconciling_after_a_settings_change_reuses_the_probes() {
        let home = Home::new();
        let known = detect_accounts(&home.base, &home.clis, &[], true).await;
        let probes = home.probes();
        let work = home.path(".claude-work");
        let configs = [config(Provider::Claude, Some(&work), Some("job"), true), config(Provider::Claude, None, None, true)];
        let accounts = reconcile_accounts(&home.base, &home.clis, &configs, true, &known).await;
        assert_eq!(home.probes(), probes, "nothing new to probe");
        assert_eq!(accounts[0].label, "job");
        assert_eq!(accounts[0].identity.as_deref(), Some("me@work.com · Work"), "state carried over");
        assert!(accounts[0].enabled && accounts[1].enabled);
        assert!(!accounts.iter().any(|a| a.id.ends_with(".claude-old")));

        // A newly configured home is probed, once.
        let configs = [config(Provider::Claude, Some(&home.path(".claude-new")), None, true)];
        let accounts = reconcile_accounts(&home.base, &home.clis, &configs, true, &known).await;
        assert_eq!(home.probes(), probes + 1);
        assert_eq!(accounts[0].logged_in, Some(false));

        // Without probing (fixed CLIs), every account takes the CLI's state.
        let accounts = detect_accounts(&home.base, &home.clis, &configs, false).await;
        let ids: Vec<&str> = accounts.iter().filter(|a| a.provider == Provider::Claude).map(|a| a.id.as_str()).collect();
        assert_eq!(ids, [format!("claude:{}", home.path(".claude-new")).as_str(), "claude"]);
        assert!(accounts.iter().all(|a| a.logged_in == Some(true)));
        assert_eq!(home.probes(), probes + 1);
    }
}
