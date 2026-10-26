//! Pure classification and text helpers shared by every adapter: what a tool call *is*
//! (activity kind + target + how sure we are), test-run detection and counts, approval risk
//! hints, short activity labels, and output truncation.
//!
//! Order of evidence (CRITIQUE C23, design-world §9.2): the provider's own tool table or item
//! type → ACP `kind` → the input's shape → shell argv rules. Anything decided by the first two
//! is "observed"; heuristics are "inferred".

use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::model::{ActivityKind, Provider, Target};

pub const OBSERVED: &str = "observed";
pub const INFERRED: &str = "inferred";

/// (activity, target, confidence "observed" | "inferred")
pub type Classified = (ActivityKind, Target, &'static str);

/// Classify one tool call.
/// - Claude: `name` is the tool name ("Bash", "mcp__codly__codly_post_note"), `input` its input.
/// - Codex: `name` is the ThreadItem type ("commandExecution", "fileChange"…), `input` the item.
/// - ACP agents: `name` is the ACP `kind` ("read", "execute"…), `input` the merged ToolCall
///   (`title`, `locations`, `rawInput`, `content`, `name`).
pub fn classify_tool(provider: Provider, name: &str, input: &Value) -> Classified {
    match provider {
        Provider::Claude => claude_tool(name, input),
        Provider::Codex => codex_item(name, input),
        Provider::Gemini | Provider::Opencode | Provider::Cursor | Provider::Copilot => acp_tool(name, input),
    }
}

fn s(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn with_path(path: Option<String>) -> Target {
    Target { path, ..Default::default() }
}

fn claude_tool(name: &str, input: &Value) -> Classified {
    use ActivityKind::*;
    if let Some(rest) = name.strip_prefix("mcp__") {
        let (server, tool) = rest.split_once("__").unwrap_or((rest, ""));
        return mcp_tool(server, tool, input);
    }
    let path = s(input, &["file_path", "notebook_path", "path"]);
    match name {
        "Bash" => classify_command(&s(input, &["command"]).unwrap_or_default()),
        "Read" => (Read, with_path(path), OBSERVED),
        "Edit" | "MultiEdit" | "NotebookEdit" => (Edit, with_path(path), OBSERVED),
        // Claude Code steers edits of existing files to Edit; Write is how it creates files.
        "Write" => (Create, with_path(path), OBSERVED),
        "Grep" | "Glob" => (Search, Target { query: s(input, &["pattern"]), path, ..Default::default() }, OBSERVED),
        "WebFetch" => (Fetch, Target { url: s(input, &["url"]), ..Default::default() }, OBSERVED),
        "WebSearch" => (Fetch, Target { query: s(input, &["query"]), ..Default::default() }, OBSERVED),
        "Agent" | "Task" => (
            Delegate,
            Target { query: s(input, &["description", "prompt"]), tool: s(input, &["subagent_type"]), ..Default::default() },
            OBSERVED,
        ),
        "TaskCreate" | "TaskUpdate" | "TaskList" | "TaskGet" | "TodoWrite" => {
            (Plan, Target { query: s(input, &["subject", "taskId"]), ..Default::default() }, OBSERVED)
        }
        "AskUserQuestion" => {
            let q = input["questions"][0]["question"].as_str().map(str::to_owned);
            (AwaitApproval, Target { query: q, ..Default::default() }, OBSERVED)
        }
        "SendMessage" => (Message, Target::default(), OBSERVED),
        "Skill" => (Read, Target { tool: s(input, &["skill", "name", "command"]), ..Default::default() }, INFERRED),
        "LSP" => (Search, Target { path: s(input, &["filePath", "file_path", "path"]), query: s(input, &["operation", "query"]), ..Default::default() }, INFERRED),
        "ToolSearch" => (Search, Target { query: s(input, &["query"]), ..Default::default() }, INFERRED),
        _ => by_shape(name, input),
    }
}

/// Codly's own MCP tools, by name. Agents prefix them differently (`mcp__codly__x`,
/// `codly_x` as a Codex tool, `codly_codly_x` in OpenCode), so match on the suffix.
const CODLY_TOOLS: &[(&str, ActivityKind)] = &[
    ("codly_post_note", ActivityKind::Note),
    ("codly_memory_write", ActivityKind::Memory),
    ("codly_kg_add", ActivityKind::Memory),
    ("codly_submit_plan", ActivityKind::Plan),
    ("codly_memory_search", ActivityKind::Search),
    ("codly_kg_query", ActivityKind::Search),
    ("codly_search", ActivityKind::Search),
    ("codly_ask_lead", ActivityKind::Message),
    ("codly_submit_review", ActivityKind::Review),
    ("codly_submit_report", ActivityKind::Done),
    ("codly_get_task", ActivityKind::Read),
    ("codly_board", ActivityKind::Read),
];

/// Activity of a Codly MCP tool, if `name` (any prefix) is one.
pub fn codly_tool_kind(name: &str) -> Option<ActivityKind> {
    CODLY_TOOLS.iter().find(|(t, _)| name.ends_with(t)).map(|(_, k)| *k)
}

fn mcp_tool(server: &str, tool: &str, input: &Value) -> Classified {
    let target = Target {
        server: Some(server.to_string()).filter(|s| !s.is_empty()),
        tool: Some(tool.to_string()).filter(|s| !s.is_empty()),
        query: s(input, &["query", "text", "question", "summary"]),
        ..Default::default()
    };
    let kind = codly_tool_kind(tool).filter(|_| server == "codly").unwrap_or(ActivityKind::Mcp);
    (kind, target, OBSERVED)
}

fn codex_item(item_type: &str, item: &Value) -> Classified {
    use ActivityKind::*;
    match item_type {
        "commandExecution" => {
            let cmd = item["command"].as_str().unwrap_or("");
            item["commandActions"].as_array().and_then(|a| from_command_actions(cmd, a)).unwrap_or_else(|| classify_command(cmd))
        }
        "fileChange" => {
            let changes = item["changes"].as_array().cloned().unwrap_or_default();
            let paths: Vec<String> = changes.iter().filter_map(|c| c["path"].as_str().map(str::to_owned)).collect();
            let kinds: Vec<&str> = changes.iter().filter_map(|c| c["kind"]["type"].as_str()).collect();
            let kind = if !kinds.is_empty() && kinds.iter().all(|k| *k == "add") {
                Create
            } else if !kinds.is_empty() && kinds.iter().all(|k| *k == "delete") {
                Delete
            } else {
                Edit
            };
            let path = (paths.len() == 1).then(|| paths[0].clone());
            (kind, Target { path, paths, ..Default::default() }, OBSERVED)
        }
        "mcpToolCall" => mcp_tool(item["server"].as_str().unwrap_or(""), item["tool"].as_str().unwrap_or(""), &item["arguments"]),
        "dynamicToolCall" => (Mcp, Target { tool: s(item, &["tool"]), ..Default::default() }, OBSERVED),
        "webSearch" => {
            let action = &item["action"];
            let target = Target { query: s(item, &["query"]).or_else(|| s(action, &["query"])), url: s(action, &["url"]), ..Default::default() };
            (Fetch, target, OBSERVED)
        }
        "collabAgentToolCall" => {
            let kind = match item["tool"].as_str() {
                Some("sendInput" | "sendMessage") => Message,
                _ => Delegate,
            };
            (kind, Target { tool: s(item, &["tool"]), query: s(item, &["prompt"]), ..Default::default() }, OBSERVED)
        }
        "subAgentActivity" => (Delegate, Target { tool: s(item, &["kind"]), ..Default::default() }, OBSERVED),
        "imageView" => (Read, with_path(s(item, &["path"])), OBSERVED),
        "plan" => (Plan, Target::default(), OBSERVED),
        "contextCompaction" => (Compacting, Target::default(), OBSERVED),
        _ => by_shape(item_type, item),
    }
}

/// Codex's own parse of a shell command (`commandActions`): trusted when every action is
/// read / listFiles / search. Anything with an `unknown` part falls back to argv rules,
/// which also catch tests, commits and merges.
fn from_command_actions(cmd: &str, actions: &[Value]) -> Option<Classified> {
    if actions.is_empty() {
        return None;
    }
    let types: Vec<&str> = actions.iter().filter_map(|a| a["type"].as_str()).collect();
    if types.len() != actions.len() || types.iter().any(|t| !matches!(*t, "read" | "listFiles" | "search")) {
        return None;
    }
    let paths: Vec<String> = actions.iter().filter_map(|a| a["path"].as_str().map(str::to_owned)).collect();
    let mut target = Target { command: Some(unwrap_shell(cmd)), path: paths.first().cloned(), ..Default::default() };
    if types.iter().all(|t| *t == "read") {
        if paths.len() > 1 {
            target.paths = paths;
        }
        return Some((ActivityKind::Read, target, OBSERVED));
    }
    target.query = actions.iter().find_map(|a| a["query"].as_str().map(str::to_owned));
    Some((ActivityKind::Search, target, OBSERVED))
}

fn acp_tool(kind: &str, call: &Value) -> Classified {
    use ActivityKind::*;
    let raw = &call["rawInput"];
    let locations: Vec<String> =
        call["locations"].as_array().map(|a| a.iter().filter_map(|l| l["path"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let diff_paths: Vec<String> = call["content"]
        .as_array()
        .map(|a| a.iter().filter(|c| c["type"] == "diff").filter_map(|c| c["path"].as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let path = s(raw, &["filePath", "file_path", "absolute_path", "path"]).or_else(|| locations.first().cloned()).or_else(|| diff_paths.first().cloned());
    let title = s(call, &["title"]);
    let tool_name = s(call, &["name"]);
    // MCP tools surface as kind "other" with the MCP tool's name; Codly's own come first.
    let named = tool_name.clone().or_else(|| title.clone()).unwrap_or_default();
    if let Some(k) = codly_tool_kind(&named) {
        return (k, Target { tool: Some(named), server: Some("codly".into()), query: s(raw, &["query", "text"]), ..Default::default() }, OBSERVED);
    }
    match kind {
        "read" => (Read, Target { path, ..Default::default() }, OBSERVED),
        "edit" => {
            let new_file = call["content"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c["type"] == "diff" && (c["oldText"].is_null() || c["_meta"]["kind"] == "add")));
            (if new_file { Create } else { Edit }, Target { path, ..Default::default() }, OBSERVED)
        }
        "delete" => (Delete, Target { path, ..Default::default() }, OBSERVED),
        "move" => (Edit, Target { path, paths: locations, ..Default::default() }, OBSERVED),
        "search" => (Search, Target { query: s(raw, &["pattern", "query", "glob"]).or(title), path, ..Default::default() }, OBSERVED),
        "execute" => {
            // OpenCode puts the command in rawInput; Gemini only in the title.
            classify_command(&s(raw, &["command", "cmd"]).or(title).unwrap_or_default())
        }
        "fetch" => (Fetch, Target { url: s(raw, &["url"]), query: s(raw, &["query"]).or(title), ..Default::default() }, OBSERVED),
        "think" => (Thinking, Target { query: title, ..Default::default() }, OBSERVED),
        "switch_mode" => (Plan, Target { query: title, ..Default::default() }, OBSERVED),
        _ => {
            let (k, mut t, _) = by_shape(&named, raw);
            if t.path.is_none() {
                t.path = path;
            }
            (k, t, INFERRED)
        }
    }
}

/// Rule 3: guess from the input's shape when the tool is unknown.
fn by_shape(name: &str, input: &Value) -> Classified {
    use ActivityKind::*;
    if let Some(cmd) = s(input, &["command", "cmd"]) {
        let (k, t, _) = classify_command(&cmd);
        return (k, t, INFERRED);
    }
    if let Some(url) = s(input, &["url"]) {
        return (Fetch, Target { url: Some(url), ..Default::default() }, INFERRED);
    }
    if let Some(q) = s(input, &["pattern", "query", "glob", "regex"]) {
        return (Search, Target { query: Some(q), path: s(input, &["path"]), ..Default::default() }, INFERRED);
    }
    if let Some(p) = s(input, &["file_path", "filePath", "notebook_path", "path"]) {
        let writes = ["content", "new_string", "old_string", "edits", "newText", "new_source", "patch", "diff"].iter().any(|k| input.get(*k).is_some());
        return (if writes { Edit } else { Read }, with_path(Some(p)), INFERRED);
    }
    (Execute, Target { tool: Some(name.to_string()).filter(|n| !n.is_empty()), ..Default::default() }, INFERRED)
}

// ───────────────────────────── shell commands ─────────────────────────────

static TEST_RE: LazyLock<Regex> = LazyLock::new(|| {
    // design-world §9.2, plus `/` as a boundary so `vendor/bin/phpunit`, `./node_modules/.bin/jest` match
    Regex::new(r"(^|[\s;&|/])((p?npm|yarn|bun)( run)? test|vitest|jest|pytest|cargo (test|nextest)|go test|swift test|xcodebuild .*test|rspec|mix test|phpunit|deno test|playwright test)\b").unwrap()
});

/// Does this shell command run a test suite? (design-world §9.2)
pub fn is_test_command(command: &str) -> bool {
    TEST_RE.is_match(&unwrap_shell(command))
}

/// One shell word, or an operator that separates commands.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Op(&'static str),
    Redirect(String),
}

/// Small shell tokenizer: quotes, backslashes, `&& || | ; &` and redirections. Not a shell
/// parser; good enough to find argv[0] and the arguments that look like paths.
fn tokenize(cmd: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    // `(` inside a word (`$(date)`, a zsh `*(.)` qualifier) opens a group its `)` closes;
    // anywhere else `(`/`)` delimit a subshell, so `(git push)` still yields `git push`.
    let mut depth = 0u32;
    let mut chars = cmd.chars().peekable();
    let flush = |cur: &mut String, in_word: &mut bool, out: &mut Vec<Tok>| {
        if *in_word {
            out.push(Tok::Word(std::mem::take(cur)));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(&n) = chars.peek()
                                && matches!(n, '"' | '\\' | '$' | '`')
                            {
                                cur.push(n);
                                chars.next();
                                continue;
                            }
                            cur.push('\\');
                        }
                        _ => cur.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next()
                    && n != '\n'
                {
                    cur.push(n);
                }
            }
            ' ' | '\t' if depth == 0 => flush(&mut cur, &mut in_word, &mut out),
            '(' if in_word => {
                depth += 1;
                cur.push(c);
            }
            ')' if depth > 0 => {
                depth -= 1;
                cur.push(c);
            }
            '\n' | ';' | '(' | ')' if depth == 0 => {
                flush(&mut cur, &mut in_word, &mut out);
                out.push(Tok::Op(";"));
            }
            '&' | '|' => {
                flush(&mut cur, &mut in_word, &mut out);
                let op = match (c, chars.peek()) {
                    ('&', Some('&')) => "&&",
                    ('|', Some('|')) => "||",
                    ('&', Some('>')) => {
                        // `&>file` / `&>>file`
                        chars.next();
                        let append = chars.peek() == Some(&'>');
                        if append {
                            chars.next();
                        }
                        out.push(Tok::Redirect(if append { "&>>" } else { "&>" }.into()));
                        continue;
                    }
                    ('&', _) => "&",
                    _ => "|",
                };
                if op.len() == 2 {
                    chars.next();
                }
                out.push(Tok::Op(op));
            }
            '>' | '<' => {
                let fd = if in_word && cur.chars().all(|c| c.is_ascii_digit()) { std::mem::take(&mut cur) } else { String::new() };
                if fd.is_empty() {
                    flush(&mut cur, &mut in_word, &mut out);
                } else {
                    in_word = false;
                }
                let mut op = format!("{fd}{c}");
                while let Some(&n) = chars.peek() {
                    if n == '>' || n == '<' || n == '&' {
                        op.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // `2>&1`: the digits after `&` are part of the redirect, not a file
                if op.ends_with('&') {
                    while let Some(&n) = chars.peek() {
                        if n.is_ascii_digit() || n == '-' {
                            op.push(n);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                out.push(Tok::Redirect(op));
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    flush(&mut cur, &mut in_word, &mut out);
    out
}

/// One simple command: argv, redirect targets, and whether it reads from a pipe.
#[derive(Debug, Default)]
struct Seg {
    argv: Vec<String>,
    redirects: Vec<String>,
    piped_in: bool,
}

fn segments(cmd: &str) -> Vec<Seg> {
    let mut out = vec![Seg::default()];
    let mut pending_redirect = false;
    for t in tokenize(cmd) {
        match t {
            Tok::Op(op) => {
                pending_redirect = false;
                out.push(Seg { piped_in: op == "|", ..Default::default() });
            }
            Tok::Redirect(r) => {
                // fd duplications (`2>&1`) take no file argument
                pending_redirect = !r.contains('&') || r.starts_with('&');
            }
            Tok::Word(w) => {
                let seg = out.last_mut().unwrap();
                if pending_redirect {
                    seg.redirects.push(w);
                    pending_redirect = false;
                } else {
                    seg.argv.push(w);
                }
            }
        }
    }
    out.retain(|s| !s.argv.is_empty() || !s.redirects.is_empty());
    out
}

/// argv without leading `VAR=x` assignments, shell keywords and wrappers (see `command_start`).
fn core_argv(argv: &[String]) -> &[String] {
    &argv[command_start(argv).unwrap_or(argv.len())..]
}

/// `NAME=value` (a shell assignment, not an option like `--x=y`).
pub(crate) fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Programs that run their arguments as a command, with the options that take a value and
/// how many positional words come before the command (`timeout 5 git push`).
const WRAPPERS: &[(&str, &[&str], usize)] = &[
    (
        "sudo",
        &[
            "-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-T", "-U", "--user", "--group", "--chdir", "--host", "--prompt", "--role", "--type",
            "--other-user", "--close-from", "--command-timeout",
        ],
        0,
    ),
    ("doas", &["-u", "-C"], 0),
    ("env", &["-u", "-C", "-P", "--unset", "--chdir"], 0),
    ("time", &["-o", "-f", "--output", "--format"], 0),
    ("nice", &["-n", "--adjustment"], 0),
    ("nohup", &[], 0),
    ("command", &[], 0),
    ("builtin", &[], 0),
    ("exec", &["-a"], 0),
    ("noglob", &[], 0),
    ("nocorrect", &[], 0),
    ("caffeinate", &["-w", "-t"], 0),
    ("stdbuf", &["-i", "-o", "-e", "--input", "--output", "--error"], 0),
    ("timeout", &["-s", "-k", "--signal", "--kill-after"], 1),
    ("gtimeout", &["-s", "-k", "--signal", "--kill-after"], 1),
    (
        "xargs",
        &[
            "-I", "-J", "-L", "-n", "-P", "-R", "-S", "-s", "-E", "-d", "-a", "--max-args", "--max-procs", "--max-lines", "--max-chars",
            "--delimiter", "--arg-file", "--eof", "--process-slot-var",
        ],
        0,
    ),
];

/// Shell words that may precede a command (`if git push; then …`, `! git push`).
const KEYWORDS: &[&str] = &["if", "then", "else", "elif", "do", "while", "until", "!", "{", "}"];

/// A wrapper program or a shell keyword (a word that is not the command itself).
pub(crate) fn is_wrapper(w: &str) -> bool {
    KEYWORDS.contains(&w) || WRAPPERS.iter().any(|(n, ..)| *n == basename(w))
}

/// Index of the command a simple command really runs: past `VAR=x` assignments, shell
/// keywords and wrappers with their options and values (`nice -n 5 git push`,
/// `timeout -k 1 5 git push`, `env FOO=1 git push`, `xargs -n1 git push` → `git`).
/// `argv.len()` when there is no command; None when it is built at run time
/// (`env -S 'git push'`), so nothing can be said about it.
pub(crate) fn command_start(argv: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if is_assignment(a) || KEYWORDS.contains(&a) {
            i += 1;
            continue;
        }
        let name = basename(a);
        let Some(&(_, values, positional)) = WRAPPERS.iter().find(|(n, ..)| *n == name) else { return Some(i) };
        i += 1;
        while let Some(o) = argv.get(i).map(String::as_str) {
            if o == "--" {
                i += 1;
                break;
            }
            if !o.starts_with('-') || o == "-" {
                break;
            }
            i += 1;
            if name == "env" && (o.starts_with("--split-string") || (!o.starts_with("--") && o.contains('S'))) {
                return None;
            }
            if name == "nice" && o[1..].chars().all(|c| c.is_ascii_digit()) {
                continue; // `nice -5 cmd`
            }
            if o.starts_with("--") {
                if !o.contains('=') && values.contains(&o) {
                    i += 1;
                }
                continue;
            }
            // A short option that takes a value takes the rest of its cluster or the next word.
            let cluster = &o[1..];
            if let Some((j, c)) = cluster.char_indices().find(|(_, c)| values.iter().any(|v| v.len() == 2 && v.ends_with(*c)))
                && j + c.len_utf8() == cluster.len()
            {
                i += 1;
            }
        }
        i = (i + positional).min(argv.len());
    }
    Some(i)
}

fn basename(p: &str) -> &str {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)
}

pub(crate) const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

/// What a shell runs, given its arguments (after the program name).
#[derive(Debug, PartialEq)]
pub(crate) enum ShellRun<'a> {
    /// `-c` (in any flag cluster: `-lc`, `-ic`, `-ec`) or fish `--command`: this string.
    Command(&'a str),
    /// No script: commands come from stdin (`curl … | sh`, `sh -s`).
    Stdin,
    /// A script file.
    Script,
}

pub(crate) fn shell_run(args: &[String]) -> ShellRun<'_> {
    let (mut c, mut stdin) = (false, false);
    let mut i = 0;
    while let Some(a) = args.get(i).map(String::as_str) {
        if a == "--" {
            i += 1;
            break;
        }
        if matches!(a, "-o" | "+o" | "-O" | "+O" | "--rcfile" | "--init-file") {
            i += 2;
            continue;
        }
        if a == "--command" {
            return args.get(i + 1).map_or(ShellRun::Stdin, |s| ShellRun::Command(s));
        }
        if let Some(s) = a.strip_prefix("--command=") {
            return ShellRun::Command(s);
        }
        if a.starts_with("--") {
            i += 1;
            continue;
        }
        if a.len() > 1 && (a.starts_with('-') || a.starts_with('+')) {
            if a.starts_with('-') {
                c |= a.contains('c');
                stdin |= a.contains('s');
            }
            i += 1;
            continue;
        }
        break;
    }
    match args.get(i) {
        Some(s) if c => ShellRun::Command(s),
        Some(_) if !stdin => ShellRun::Script,
        _ => ShellRun::Stdin,
    }
}

/// `/bin/zsh -lc 'npm test'` → `npm test` (Codex wraps every command like this).
pub fn unwrap_shell(command: &str) -> String {
    let toks = tokenize(command);
    if let [Tok::Word(sh), Tok::Word(flag), Tok::Word(script)] = toks.as_slice()
        && SHELLS.contains(&basename(sh))
        && flag.starts_with('-')
        && flag.contains('c')
    {
        return script.clone();
    }
    command.trim().to_string()
}

const READ_CMDS: &[&str] = &["cat", "head", "tail", "bat", "less", "more", "nl", "wc", "file", "stat", "xxd", "hexdump", "od"];
const SEARCH_CMDS: &[&str] = &["rg", "grep", "egrep", "fgrep", "ag", "ack", "fd", "find", "ls", "tree", "locate", "mdfind"];

/// Shell argv rules (design-world §9.2 rule 4). Plain commands are "observed" execution;
/// anything refined from argv (read, search, test, commit, merge) is "inferred".
pub fn classify_command(command: &str) -> Classified {
    use ActivityKind::*;
    let cmd = unwrap_shell(command);
    let mut target = Target { command: Some(cmd.clone()), ..Default::default() };
    if TEST_RE.is_match(&cmd) {
        return (Test, target, INFERRED);
    }
    for seg in segments(&cmd) {
        let argv = core_argv(&seg.argv);
        let Some(a0) = argv.first().map(|a| basename(a)) else { continue };
        if a0 == "cd" || a0 == "pushd" || a0 == "export" || a0 == "set" || a0 == "source" || a0 == "." {
            continue;
        }
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect();
        match a0 {
            "git" => {
                return match args.first().copied() {
                    Some("commit") => (Commit, target, INFERRED),
                    Some("merge") => (Merge, target, INFERRED),
                    Some("grep" | "ls-files") => {
                        target.query = args.get(1).map(|q| q.to_string());
                        (Search, target, INFERRED)
                    }
                    Some("diff" | "log" | "show" | "status" | "blame") => (Read, target, INFERRED),
                    _ => (Execute, target, OBSERVED),
                };
            }
            "gh" if args.first() == Some(&"pr") && args.get(1) == Some(&"merge") => return (Merge, target, INFERRED),
            _ if READ_CMDS.contains(&a0) || (a0 == "sed" && argv.iter().any(|a| a == "-n")) => {
                target.path = args.last().filter(|_| a0 != "sed" || args.len() >= 2).map(|p| p.to_string());
                return (Read, target, INFERRED);
            }
            _ if SEARCH_CMDS.contains(&a0) => {
                match a0 {
                    "ls" | "tree" | "find" | "fd" | "locate" | "mdfind" => target.path = args.first().map(|p| p.to_string()),
                    _ => {
                        target.query = args.first().map(|q| q.to_string());
                        target.path = args.get(1).map(|p| p.to_string());
                    }
                }
                return (Search, target, INFERRED);
            }
            _ => return (Execute, target, OBSERVED),
        }
    }
    (Execute, target, OBSERVED)
}

// ───────────────────────────── test counts ─────────────────────────────

static ANSI_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07").unwrap());
static CARGO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"test result: \w+\. (\d+) passed; (\d+) failed").unwrap());
static JEST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*Tests:\s+(.+)$").unwrap());
static VITEST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*Tests\s{2,}(.+)$").unwrap());
static PYTEST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^=+ (.*\d+ (?:passed|failed|errors?).*) in [\d.]+s").unwrap());
static PASSED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+) passed").unwrap());
static FAILED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+) (?:failed|errors?)\b").unwrap());

pub fn strip_ansi(s: &str) -> String {
    ANSI_RE.replace_all(s, "").into_owned()
}

fn count(re: &Regex, s: &str) -> i64 {
    re.captures_iter(s).filter_map(|c| c[1].parse::<i64>().ok()).sum()
}

/// (passed, failed) from a test runner's summary: vitest, jest, pytest, cargo, go.
pub fn parse_test_counts(output: &str) -> Option<(i64, i64)> {
    let text = strip_ansi(output);
    if CARGO_RE.is_match(&text) {
        let (mut p, mut f) = (0, 0);
        for c in CARGO_RE.captures_iter(&text) {
            p += c[1].parse::<i64>().unwrap_or(0);
            f += c[2].parse::<i64>().unwrap_or(0);
        }
        return Some((p, f));
    }
    // Watch modes print several summaries; the last one is current.
    for re in [&*JEST_RE, &*VITEST_RE, &*PYTEST_RE] {
        if let Some(c) = re.captures_iter(&text).last() {
            let line = &c[1];
            return Some((count(&PASSED_RE, line), count(&FAILED_RE, line)));
        }
    }
    let lines = || text.lines().map(str::trim_start);
    let (pass, fail) = (lines().filter(|l| l.starts_with("--- PASS")).count(), lines().filter(|l| l.starts_with("--- FAIL")).count());
    if pass + fail > 0 {
        return Some((pass as i64, fail as i64));
    }
    // `go test` without -v: one line per package
    let (pkg_ok, pkg_fail) = (
        text.lines().filter(|l| l.starts_with("ok  ") || l.starts_with("ok\t")).count(),
        text.lines().filter(|l| l.starts_with("FAIL\t")).count(),
    );
    (pkg_ok + pkg_fail > 0).then_some((pkg_ok as i64, pkg_fail as i64))
}

// ───────────────────────────── approval risk hints ─────────────────────────────

/// Lexically normalize (no filesystem access); `/private/tmp` ≡ `/tmp` on macOS.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    match out.strip_prefix("/private") {
        Ok(rest) if rest.starts_with("tmp") || rest.starts_with("var") || rest.starts_with("etc") => Path::new("/").join(rest),
        _ => out,
    }
}

/// Is `path` (absolute, or relative to `cwd`) outside `root`? `~` paths count as outside.
pub fn outside(path: &str, cwd: &Path, root: &Path) -> bool {
    if path.starts_with('~') {
        return true;
    }
    let p = Path::new(path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    !normalize(&abs).starts_with(normalize(root))
}

const HARMLESS_PATHS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/stdin", "/dev/tty", "/dev/zero", "/dev/random", "/dev/urandom"];

fn looks_like_path(a: &str) -> bool {
    a.starts_with('/') || a.starts_with('~') || a == ".." || a.starts_with("../") || a.contains("/../")
}

/// Inferred risk hints for an approval card: "outside worktree", "deletes files",
/// "network", "git push", "sudo", "pipes to shell".
pub fn risk_hints(command: &str, cwd: Option<&Path>, worktree: &Path) -> Vec<String> {
    let mut hints: Vec<&'static str> = Vec::new();
    hints_into(command, cwd.unwrap_or(worktree), worktree, 0, &mut hints);
    hints.into_iter().map(str::to_owned).collect()
}

/// `--package=x` / `-p x` on npx, bunx or `npm exec`: installs and runs that package.
fn package_flag(args: &[&str]) -> bool {
    args.iter().any(|a| a.starts_with("--package") || (a.starts_with("-p") && !a.starts_with("--")))
}

/// `git -C dir -c k=v push` → `push`.
fn git_subcommand<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while let Some(a) = args.get(i).copied() {
        if !a.starts_with('-') {
            return Some(a);
        }
        i += if matches!(a, "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace") { 2 } else { 1 };
    }
    None
}

fn hints_into(command: &str, cwd: &Path, worktree: &Path, depth: u8, hints: &mut Vec<&'static str>) {
    let cmd = unwrap_shell(command);
    let add = |h: &'static str, hints: &mut Vec<&'static str>| {
        if !hints.contains(&h) {
            hints.push(h);
        }
    };
    if outside(&cwd.to_string_lossy(), worktree, worktree) {
        add("outside worktree", hints);
    }
    // `bash -c "$(curl …)"`, `sh <(wget …)`, `eval "$(curl …)"`: a download run as a script
    let downloads = ["$(curl", "$(wget", "<(curl", "<(wget", "`curl", "`wget"].iter().any(|d| command.contains(d));
    if downloads
        && segments(command)
            .iter()
            .any(|s| core_argv(&s.argv).first().is_some_and(|a| SHELLS.contains(&basename(a)) || a == "eval" || a == "source"))
    {
        add("pipes to shell", hints);
    }
    let mut cur_dir = cwd.to_path_buf();
    for seg in segments(&cmd) {
        let full = &seg.argv;
        let argv = core_argv(full);
        let wrappers = &full[..full.len() - argv.len()];
        if wrappers.iter().any(|a| matches!(basename(a), "sudo" | "doas")) || argv.first().is_some_and(|a| basename(a) == "su") {
            add("sudo", hints);
        }
        let Some(a0) = argv.first().map(|a| basename(a)) else { continue };
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        let sub = args.iter().find(|a| !a.starts_with('-')).copied();
        if seg.piped_in && SHELLS.contains(&a0) {
            add("pipes to shell", hints);
        }
        // `bash -lc 'git push'` inside a longer command: check what it runs.
        if SHELLS.contains(&a0)
            && let ShellRun::Command(script) = shell_run(&argv[1..])
            && depth < 3
        {
            hints_into(script, &cur_dir, worktree, depth + 1, hints);
        }
        match a0 {
            "rm" | "rmdir" | "unlink" | "shred" | "trash" => add("deletes files", hints),
            "find" if args.contains(&"-delete") => add("deletes files", hints),
            "git" => match git_subcommand(&args) {
                Some("push") => add("git push", hints),
                Some("clean" | "rm") => add("deletes files", hints),
                Some("clone" | "fetch" | "pull" | "ls-remote" | "submodule") => add("network", hints),
                _ => {}
            },
            "curl" | "wget" | "ssh" | "scp" | "sftp" | "rsync" | "nc" | "ncat" | "telnet" | "ftp" | "http" | "https" | "xh" | "gh" | "npx" | "bunx" => {
                add("network", hints)
            }
            "npm" | "pnpm" | "yarn" | "bun" if matches!(sub, Some("install" | "i" | "add" | "ci" | "update" | "upgrade" | "dlx" | "x" | "publish")) => {
                add("network", hints)
            }
            "npm" | "pnpm" | "yarn" | "bun" if sub == Some("exec") && package_flag(&args) => add("network", hints),
            "pip" | "pip3" | "gem" | "brew" | "uv" | "poetry" | "cargo" | "go" | "composer" | "bundle" | "docker"
                if matches!(sub, Some("install" | "add" | "get" | "sync" | "update" | "upgrade" | "fetch" | "pull" | "push" | "login" | "require" | "download")) =>
            {
                add("network", hints)
            }
            _ => {}
        }
        if a0 == "cd"
            && let Some(dir) = args.first()
        {
            if outside(dir, &cur_dir, worktree) {
                add("outside worktree", hints);
            }
            let d = Path::new(dir);
            cur_dir = if d.is_absolute() { d.to_path_buf() } else { cur_dir.join(d) };
            continue;
        }
        let path_args = args.iter().map(|a| a.split_once('=').map_or(*a, |(_, v)| v)).chain(seg.redirects.iter().map(String::as_str));
        for a in path_args {
            if looks_like_path(a) && !HARMLESS_PATHS.contains(&a) && outside(a, &cur_dir, worktree) {
                add("outside worktree", hints);
            }
        }
    }
}

// ───────────────────────────── labels and text ─────────────────────────────

/// Short human label for the world and the agent card: the basename of the target file or
/// the first two words of the command, at most 28 characters. Never empty.
pub fn activity_label(kind: ActivityKind, target: &Target) -> String {
    let raw = if let Some(p) = target.path.as_deref().or(target.paths.first().map(String::as_str)) {
        let mut s = basename(p).to_string();
        if target.paths.len() > 1 {
            s.push_str(&format!(" +{}", target.paths.len() - 1));
        }
        s
    } else if let Some(c) = target.command.as_deref() {
        let words: Vec<String> = segments(&unwrap_shell(c))
            .into_iter()
            .map(|s| core_argv(&s.argv).to_vec())
            .find(|a| a.first().is_some_and(|a0| !matches!(basename(a0), "cd" | "export")))
            .unwrap_or_default();
        let words: Vec<String> = words.into_iter().take(2).map(|w| if w.contains('/') && !w.starts_with('-') { basename(&w).to_string() } else { w }).collect();
        if words.is_empty() { c.trim().to_string() } else { words.join(" ") }
    } else if let Some(q) = target.query.as_deref() {
        q.to_string()
    } else if let Some(u) = target.url.as_deref() {
        let rest = u.split_once("://").map_or(u, |(_, r)| r);
        rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
    } else if let Some(t) = target.tool.as_deref() {
        match target.server.as_deref() {
            Some(s) if !t.starts_with(s) => format!("{s} {t}"),
            _ => t.to_string(),
        }
    } else {
        String::new()
    };
    let raw = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let raw = if raw.is_empty() {
        serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    } else {
        raw
    };
    truncate_chars(&raw, 28)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Keep at most `max` bytes: the head and the tail, with a marker in between (tool outputs
/// in `ToolCompleted` are capped at 8 KB; the full text stays in the raw log).
pub fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let mut head_end = half;
    while !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - half;
    while !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!("{}\n… [{} bytes omitted] …\n{}", &s[..head_end], tail_start - head_end, &s[tail_start..])
}

pub const OUTPUT_MAX: usize = 8 << 10;

const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// `"gpt-6-luna:low"` → (`Some("gpt-6-luna")`, `Some("low")`). Only a known effort level
/// counts, so ids like Bedrock's `…-v1:0` pass through unchanged.
///
/// ponytail: stopgap until `SpawnSpec` gets an `effort` field (listed as a contract request).
pub fn split_model_effort(model: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) else { return (None, None) };
    match m.rsplit_once(':') {
        Some((base, e)) if EFFORTS.contains(&e) => (Some(base.to_string()).filter(|b| !b.is_empty()), Some(e.to_string())),
        _ => (Some(m.to_string()), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kind(p: Provider, name: &str, input: Value) -> (ActivityKind, &'static str) {
        let (k, _, c) = classify_tool(p, name, &input);
        (k, c)
    }

    #[test]
    fn claude_tool_table() {
        use ActivityKind::*;
        let c = Provider::Claude;
        assert_eq!(kind(c, "Read", json!({"file_path": "/w/a.rs"})), (Read, OBSERVED));
        assert_eq!(kind(c, "Edit", json!({"file_path": "/w/a.rs"})), (Edit, OBSERVED));
        assert_eq!(kind(c, "MultiEdit", json!({"file_path": "/w/a.rs"})), (Edit, OBSERVED));
        assert_eq!(kind(c, "NotebookEdit", json!({"notebook_path": "/w/a.ipynb"})), (Edit, OBSERVED));
        assert_eq!(kind(c, "Write", json!({"file_path": "/w/new.rs"})), (Create, OBSERVED));
        assert_eq!(kind(c, "Grep", json!({"pattern": "useAuth"})), (Search, OBSERVED));
        assert_eq!(kind(c, "Glob", json!({"pattern": "**/*.ts"})), (Search, OBSERVED));
        assert_eq!(kind(c, "WebFetch", json!({"url": "https://docs.rs"})), (Fetch, OBSERVED));
        assert_eq!(kind(c, "WebSearch", json!({"query": "tokio"})), (Fetch, OBSERVED));
        assert_eq!(kind(c, "Agent", json!({"description": "x", "prompt": "y"})), (Delegate, OBSERVED));
        assert_eq!(kind(c, "Task", json!({})), (Delegate, OBSERVED));
        for t in ["TaskCreate", "TaskUpdate", "TaskList", "TaskGet", "TodoWrite"] {
            assert_eq!(kind(c, t, json!({})), (Plan, OBSERVED), "{t}");
        }
        assert_eq!(kind(c, "AskUserQuestion", json!({"questions": [{"question": "Which?"}]})), (AwaitApproval, OBSERVED));
        assert_eq!(kind(c, "Skill", json!({"skill": "pdf"})), (Read, INFERRED));
        assert_eq!(kind(c, "LSP", json!({"operation": "goToDefinition", "filePath": "/w/a.rs"})), (Search, INFERRED));
        assert_eq!(kind(c, "ToolSearch", json!({"query": "select:x"})), (Search, INFERRED));
        assert_eq!(kind(c, "mcp__linear__create_issue", json!({})), (Mcp, OBSERVED));
        let (_, t, _) = classify_tool(c, "mcp__linear__create_issue", &json!({}));
        assert_eq!((t.server.as_deref(), t.tool.as_deref()), (Some("linear"), Some("create_issue")));
        assert_eq!(kind(c, "Bash", json!({"command": "ls -la"})), (Search, INFERRED));
        assert_eq!(kind(c, "Bash", json!({"command": "make build"})), (Execute, OBSERVED));
        assert_eq!(kind(c, "SomethingNew", json!({"file_path": "/w/a", "content": "x"})), (Edit, INFERRED));
        assert_eq!(kind(c, "SomethingNew", json!({})), (Execute, INFERRED));
    }

    #[test]
    fn codly_mcp_tools_have_their_own_activities() {
        use ActivityKind::*;
        let cases = [
            ("codly_post_note", Note),
            ("codly_memory_write", Memory),
            ("codly_kg_add", Memory),
            ("codly_submit_plan", Plan),
            ("codly_search", Search),
            ("codly_memory_search", Search),
            ("codly_kg_query", Search),
            ("codly_ask_lead", Message),
            ("codly_submit_review", Review),
            ("codly_submit_report", Done),
        ];
        for (tool, want) in cases {
            assert_eq!(kind(Provider::Claude, &format!("mcp__codly__{tool}"), json!({})).0, want, "claude {tool}");
            let item = json!({"server": "codly", "tool": tool, "arguments": {}});
            assert_eq!(kind(Provider::Codex, "mcpToolCall", item).0, want, "codex {tool}");
            let call = json!({"title": format!("codly_{tool}"), "kind": "other", "rawInput": {}});
            assert_eq!(kind(Provider::Opencode, "other", call).0, want, "acp {tool}");
        }
        // a third-party server's tool that happens to share a name is just MCP
        assert_eq!(kind(Provider::Claude, "mcp__other__codly_post_note_x", json!({})).0, Mcp);
    }

    #[test]
    fn codex_items() {
        use ActivityKind::*;
        let cx = Provider::Codex;
        let ls = json!({"command": "/bin/zsh -lc ls", "commandActions": [{"type": "listFiles", "command": "ls", "path": null}]});
        assert_eq!(kind(cx, "commandExecution", ls), (Search, OBSERVED));
        let cat = json!({"command": "/bin/zsh -lc 'cat README.md'", "commandActions": [{"type": "read", "command": "cat README.md", "name": "README.md", "path": "/w/README.md"}]});
        let (k, t, c) = classify_tool(cx, "commandExecution", &cat);
        assert_eq!((k, c, t.path.as_deref(), t.command.as_deref()), (Read, OBSERVED, Some("/w/README.md"), Some("cat README.md")));
        let rg = json!({"command": "rg foo", "commandActions": [{"type": "search", "command": "rg foo", "query": "foo", "path": "src"}]});
        let (k, t, _) = classify_tool(cx, "commandExecution", &rg);
        assert_eq!((k, t.query.as_deref()), (Search, Some("foo")));
        let test = json!({"command": "/bin/zsh -lc 'npm test'", "commandActions": [{"type": "unknown", "command": "npm test"}]});
        assert_eq!(kind(cx, "commandExecution", test), (Test, INFERRED));
        let add = json!({"changes": [{"path": "/w/notes.txt", "kind": {"type": "add"}, "diff": "hello\n"}]});
        let (k, t, _) = classify_tool(cx, "fileChange", &add);
        assert_eq!((k, t.path.as_deref()), (Create, Some("/w/notes.txt")));
        let mixed = json!({"changes": [{"path": "/w/a", "kind": {"type": "add"}}, {"path": "/w/b", "kind": {"type": "update"}}]});
        let (k, t, _) = classify_tool(cx, "fileChange", &mixed);
        assert_eq!((k, t.paths.len(), t.path), (Edit, 2, None));
        let del = json!({"changes": [{"path": "/w/a", "kind": {"type": "delete"}}]});
        assert_eq!(kind(cx, "fileChange", del).0, Delete);
        assert_eq!(kind(cx, "mcpToolCall", json!({"server": "linear", "tool": "x", "arguments": {}})).0, Mcp);
        assert_eq!(kind(cx, "webSearch", json!({"query": "rust", "action": {"type": "search"}})).0, Fetch);
        assert_eq!(kind(cx, "plan", json!({"text": "1. x"})).0, Plan);
    }

    #[test]
    fn acp_kinds() {
        use ActivityKind::*;
        let g = Provider::Gemini;
        let read = json!({"title": "README.md", "kind": "read", "locations": [{"path": "/w/README.md"}]});
        let (k, t, _) = classify_tool(g, "read", &read);
        assert_eq!((k, t.path.as_deref()), (Read, Some("/w/README.md")));
        let write_new = json!({"kind": "edit", "content": [{"type": "diff", "path": "/w/g.txt", "oldText": "", "newText": "hi", "_meta": {"kind": "add"}}], "locations": [{"path": "/w/g.txt"}]});
        assert_eq!(kind(g, "edit", write_new).0, Create);
        let edit = json!({"kind": "edit", "content": [{"type": "diff", "path": "/w/g.txt", "oldText": "a", "newText": "b"}]});
        assert_eq!(kind(g, "edit", edit).0, Edit);
        assert_eq!(kind(g, "execute", json!({"title": "git status --short"})).0, Read);
        assert_eq!(kind(Provider::Opencode, "execute", json!({"title": "bash", "rawInput": {"command": "npm test"}})).0, Test);
        assert_eq!(kind(g, "search", json!({"rawInput": {"pattern": "x"}})).0, Search);
        assert_eq!(kind(g, "delete", json!({})).0, Delete);
        assert_eq!(kind(g, "fetch", json!({"rawInput": {"url": "https://x"}})).0, Fetch);
        assert_eq!(kind(g, "think", json!({"title": "Update tactical intent"})).0, Thinking);
        assert_eq!(kind(g, "other", json!({"title": "mystery"})), (Execute, INFERRED));
    }

    #[test]
    fn shell_argv_rules() {
        use ActivityKind::*;
        let k = |c: &str| classify_command(c).0;
        assert_eq!(k("cat src/main.rs"), Read);
        assert_eq!(k("head -n 20 README.md"), Read);
        assert_eq!(k("tail -f log.txt"), Read);
        assert_eq!(k("sed -n '1,40p' src/lib.rs"), Read);
        assert_eq!(k("sed -i 's/a/b/' x"), Execute);
        assert_eq!(k("bat x.rs"), Read);
        assert_eq!(k("rg -n useAuth src"), Search);
        assert_eq!(k("grep -r TODO ."), Search);
        assert_eq!(k("fd .rs"), Search);
        assert_eq!(k("find . -name '*.ts'"), Search);
        assert_eq!(k("ls -la"), Search);
        assert_eq!(k("tree src"), Search);
        assert_eq!(k("cd src && rg foo"), Search);
        assert_eq!(k("FOO=1 cat x"), Read);
        assert_eq!(k("git commit -m 'feat: x'"), Commit);
        assert_eq!(k("git merge main"), Merge);
        assert_eq!(k("gh pr merge 12"), Merge);
        assert_eq!(k("git status --short"), Read);
        assert_eq!(k("git push origin main"), Execute);
        assert_eq!(k("npm run build"), Execute);
        assert_eq!(k("/bin/zsh -lc 'cat README.md'"), Read);
        let (_, t, _) = classify_command("/bin/zsh -lc \"rg -n 'x y' src\"");
        assert_eq!(t.command.as_deref(), Some("rg -n 'x y' src"));
        assert_eq!(t.query.as_deref(), Some("x y"));
        let (_, t, _) = classify_command("cat a.txt > b.txt");
        assert_eq!(t.path.as_deref(), Some("a.txt"), "redirect targets are not read paths");
    }

    #[test]
    fn test_runner_detection() {
        for c in [
            "npm test",
            "pnpm test",
            "npm run test",
            "yarn test",
            "bun test",
            "npx vitest run",
            "pnpm vitest",
            "jest --ci",
            "pytest -q tests/",
            "python -m pytest",
            "uv run pytest",
            "cargo test -p codly-core",
            "cargo nextest run",
            "go test ./...",
            "swift test",
            "xcodebuild -scheme App test",
            "bundle exec rspec",
            "mix test",
            "vendor/bin/phpunit",
            "deno test",
            "npx playwright test",
            "cd web && npm test",
            "/bin/zsh -lc 'cargo test'",
        ] {
            assert!(is_test_command(c), "{c}");
            assert_eq!(classify_command(c).0, ActivityKind::Test, "{c}");
        }
        for c in ["npm run build", "cargo build", "echo testing", "git status", "contest"] {
            assert!(!is_test_command(c), "{c}");
        }
    }

    #[test]
    fn test_counts() {
        let vitest = "\x1b[32m✓\x1b[39m src/a.test.ts (3)\n\n Test Files  1 failed | 2 passed (3)\n      Tests  1 failed | 10 passed (11)\n   Start at  12:00:00\n";
        assert_eq!(parse_test_counts(vitest), Some((10, 1)));
        let vitest_ok = " Test Files  2 passed (2)\n      Tests  12 passed (12)\n";
        assert_eq!(parse_test_counts(vitest_ok), Some((12, 0)));
        let jest = "Test Suites: 1 failed, 3 passed, 4 total\nTests:       3 failed, 1 skipped, 10 passed, 14 total\nSnapshots:   0 total\n";
        assert_eq!(parse_test_counts(jest), Some((10, 3)));
        let pytest = "tests/test_a.py ..F.\n=========== 1 failed, 10 passed, 2 skipped in 0.42s ===========\n";
        assert_eq!(parse_test_counts(pytest), Some((10, 1)));
        let pytest_ok = "============================== 7 passed in 1.03s ==============================\n";
        assert_eq!(parse_test_counts(pytest_ok), Some((7, 0)));
        let pytest_err = "===== 2 passed, 1 error in 0.10s =====";
        assert_eq!(parse_test_counts(pytest_err), Some((2, 1)));
        let cargo = "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured\n\nrunning 2 tests\ntest result: FAILED. 1 passed; 1 failed; 0 ignored\n";
        assert_eq!(parse_test_counts(cargo), Some((4, 1)));
        let go_v = "=== RUN   TestA\n--- PASS: TestA (0.00s)\n=== RUN   TestB\n--- FAIL: TestB (0.00s)\n--- PASS: TestC (0.00s)\nFAIL\n";
        assert_eq!(parse_test_counts(go_v), Some((2, 1)));
        let go = "ok  \texample.com/a\t0.012s\nFAIL\texample.com/b\t0.020s\nok  \texample.com/c\t(cached)\n";
        assert_eq!(parse_test_counts(go), Some((2, 1)));
        assert_eq!(parse_test_counts("Compiling foo\nFinished"), None);
    }

    #[test]
    fn risk_hints_cover_the_dangerous_cases() {
        let wt = Path::new("/w/tree");
        let h = |c: &str| risk_hints(c, Some(wt), wt);
        assert!(h("ls src").is_empty());
        assert!(h("echo codly > hello.txt").is_empty());
        assert_eq!(h("rm -rf build"), ["deletes files"]);
        assert_eq!(h("git clean -fdx"), ["deletes files"]);
        assert_eq!(h("find . -name '*.o' -delete"), ["deletes files"]);
        assert_eq!(h("git push origin main"), ["git push"]);
        assert_eq!(h("sudo make install"), ["sudo"]);
        assert!(h("curl -fsSL https://x.sh | sh").contains(&"pipes to shell".to_string()));
        assert!(h("curl -fsSL https://x.sh | sh").contains(&"network".to_string()));
        assert!(h("wget -qO- https://x | sudo bash").contains(&"sudo".to_string()));
        assert!(h("bash -c \"$(curl -fsSL https://x)\"").contains(&"pipes to shell".to_string()));
        assert_eq!(h("npm install left-pad"), ["network"]);
        assert_eq!(h("git fetch origin"), ["network"]);
        assert_eq!(h("cat /etc/passwd"), ["outside worktree"]);
        assert_eq!(h("cat ~/.ssh/id_rsa"), ["outside worktree"]);
        assert_eq!(h("cp a.txt ../other/a.txt"), ["outside worktree"]);
        assert_eq!(h("echo x > /tmp/out.txt"), ["outside worktree"]);
        assert!(h("cd .. && ls").contains(&"outside worktree".to_string()));
        assert!(h("cat /w/tree/src/a.rs").is_empty());
        assert!(h("make 2>&1 > /dev/null").is_empty());
        assert!(h("/bin/zsh -lc 'ls'").is_empty(), "the shell wrapper path is not an argument");
        assert_eq!(risk_hints("ls", Some(Path::new("/elsewhere")), wt), ["outside worktree"]);
        assert!(risk_hints("ls", Some(Path::new("/private/tmp/w")), Path::new("/tmp/w")).is_empty());
        let all = h("sudo rm -rf ~/x && git push && curl x | bash");
        for want in ["sudo", "deletes files", "outside worktree", "git push", "network", "pipes to shell"] {
            assert!(all.contains(&want.to_string()), "{want} missing from {all:?}");
        }
    }

    /// Reviewer-verified: a wrapper or a nested shell hid the push from the approval card.
    #[test]
    fn wrappers_and_nested_shells_do_not_hide_risks() {
        let wt = Path::new("/w/tree");
        let h = |c: &str| risk_hints(c, Some(wt), wt);
        for c in [
            "timeout 5 git push",
            "timeout -s KILL 5 git push",
            "nice -n 5 git push",
            "nice -n5 git push",
            "true && bash -lc 'git push'",
            "env FOO=1 git push",
            "xargs git push",
            "xargs -n 1 git push",
            "caffeinate -i git push",
            "stdbuf -o L git push",
            "command git push",
            "exec git push",
            "(git push)",
            "if true; then git push; fi",
            "sudo -u me git push",
            "git -C sub -c x=y push",
        ] {
            assert!(h(c).contains(&"git push".to_string()), "{c}: {:?}", h(c));
        }
        assert!(h("npm exec --package=evil jest").contains(&"network".to_string()));
        let argv = |c: &str| c.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(command_start(&argv("timeout -k 1 5 git push")), Some(4));
        assert_eq!(command_start(&argv("FOO=1 env -u X BAR=2 git")), Some(5));
        assert_eq!(command_start(&argv("env -S x")), None);
        assert_eq!(command_start(&argv("env")), Some(1));
        assert_eq!(shell_run(&argv("-o pipefail -c x")), ShellRun::Command("x"));
        assert_eq!(shell_run(&argv("--norc script.sh")), ShellRun::Script);
        assert_eq!(shell_run(&[]), ShellRun::Stdin);
        let (_, t, _) = classify_command("echo $(date)");
        assert_eq!(t.command.as_deref(), Some("echo $(date)"));
        assert_eq!(activity_label(ActivityKind::Execute, &t), "echo $(date)");
    }

    #[test]
    fn labels() {
        use ActivityKind::*;
        let t = |path: Option<&str>, command: Option<&str>| Target {
            path: path.map(str::to_owned),
            command: command.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(activity_label(Edit, &t(Some("/w/src/app.ts"), None)), "app.ts");
        assert_eq!(activity_label(Test, &t(None, Some("npm test -- --watch=false"))), "npm test");
        assert_eq!(activity_label(Execute, &t(None, Some("/bin/zsh -lc 'cd web && pnpm build'"))), "pnpm build");
        assert_eq!(activity_label(Execute, &t(None, Some("FOO=1 /usr/bin/make all"))), "make all");
        let paths = Target { paths: vec!["/w/a.rs".into(), "/w/b.rs".into(), "/w/c.rs".into()], ..Default::default() };
        assert_eq!(activity_label(Edit, &paths), "a.rs +2");
        let url = Target { url: Some("https://docs.rs/tokio/latest".into()), ..Default::default() };
        assert_eq!(activity_label(Fetch, &url), "docs.rs");
        let mcp = Target { server: Some("linear".into()), tool: Some("create_issue".into()), ..Default::default() };
        assert_eq!(activity_label(Mcp, &mcp), "linear create_issue");
        assert_eq!(activity_label(Thinking, &Target::default()), "thinking");
        assert_eq!(activity_label(AwaitApproval, &Target::default()), "await_approval");
        let long = activity_label(Read, &t(Some("/w/a_really_long_file_name_that_goes_on_forever.rs"), None));
        assert_eq!(long.chars().count(), 28);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn text_helpers() {
        assert_eq!(truncate_output("short", 10), "short");
        let s = "é".repeat(100); // 200 bytes
        let t = truncate_output(&s, 50);
        assert!(t.len() < 120 && t.contains("bytes omitted"));
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m"), "ok");
        assert_eq!(split_model_effort(Some("gpt-6-luna:low")), (Some("gpt-6-luna".into()), Some("low".into())));
        assert_eq!(split_model_effort(Some("haiku")), (Some("haiku".into()), None));
        assert_eq!(split_model_effort(Some("anthropic.claude-v1:0")), (Some("anthropic.claude-v1:0".into()), None));
        assert_eq!(split_model_effort(Some(":high")), (None, Some("high".into())));
        assert_eq!(split_model_effort(None), (None, None));
    }
}
