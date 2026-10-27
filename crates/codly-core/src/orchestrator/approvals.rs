//! Approval policy, level L0 (CRITIQUE D6, agents-backend §5.2): deterministic rules settle
//! the obvious cases before you are asked. Pure; the broker glue lives in ingest.rs.
//!
//! Order of the rules:
//! 1. questions and permission changes always go to you;
//! 2. any risk — a provider hint, or one Codly detects (outside the worktree, deletes,
//!    network, push, sudo, git writes, unparseable shell) — always goes to you;
//! 3. Codly's own MCP tools are allowed in every mode;
//! 4. then the mode: strict asks for everything else; balanced allows read-only actions,
//!    the task's own verify commands and test runners; autonomous allows anything that stays
//!    inside the agent's worktree.
//!
//! Only `allow_once` is ever chosen automatically, never a session or permanent grant.
//!
//! The shell checks err toward asking: a "read-only command" is a bare program name (no path,
//! no `VAR=x` prefix, no wrapper, no `$` expansion) whose arguments use none of its options
//! that write or run something; risks look through wrappers (`timeout 5 git push`) and nested
//! `sh -c`; a word whose value the shell computes (`$HOME/x`) counts as outside the worktree.

use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use crate::adapters::classify::{self, ShellRun};
use crate::mcp::tools;
use crate::model::*;

pub struct PolicyCtx<'a> {
    pub mode: ApprovalMode,
    /// The agent's working directory: task or solo worktree, repo root for the Lead.
    pub worktree: Option<&'a Path>,
    /// The task's own verify commands (always allowed in balanced mode).
    pub verify: &'a [String],
    /// Lead, reviewers, between-task chats: only reads are ever auto-allowed, in any mode
    /// (the Lead's working directory is the user's own checkout).
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow { choice: ApprovalChoice, rule: String },
    Ask { reason: String },
}

pub fn evaluate(a: &Approval, ctx: &PolicyCtx) -> Verdict {
    let ask = |reason: String| Verdict::Ask { reason };
    match &a.subject {
        ApprovalSubject::Question { .. } => return ask("the agent asked you a question".into()),
        ApprovalSubject::Permissions { .. } => return ask("the agent wants broader permissions".into()),
        _ => {}
    }
    let mut risks = a.risks.clone();
    for r in detect_risks(&a.subject, ctx.worktree) {
        if !risks.contains(&r) {
            risks.push(r);
        }
    }
    if !risks.is_empty() {
        return ask(format!("risk: {}", risks.join(", ")));
    }
    let allow = |rule: String| {
        if a.choices.is_empty() || a.choices.contains(&ApprovalChoice::AllowOnce) {
            Verdict::Allow { choice: ApprovalChoice::AllowOnce, rule }
        } else {
            Verdict::Ask { reason: format!("{rule}, but the agent offered no one-time allow") }
        }
    };
    if let ApprovalSubject::Tool { name, .. } = &a.subject
        && is_codly_tool(name)
    {
        return allow(format!("Codly tool {name}"));
    }
    match ctx.mode {
        ApprovalMode::Strict => ask("strict mode: you approve every action".into()),
        _ if ctx.read_only => match &a.subject {
            ApprovalSubject::Command { command, .. } if classify_command(command, &[]) == Some("read-only command") => {
                allow(format!("read-only command `{}`", short(command)))
            }
            ApprovalSubject::Tool { name, .. } if is_read_only_tool(name) => allow(format!("read-only tool {name}")),
            _ => ask("this agent is read-only: only reads are approved automatically".into()),
        },
        ApprovalMode::Balanced => match &a.subject {
            ApprovalSubject::Command { command, .. } => match classify_command(command, ctx.verify) {
                Some(kind) => allow(format!("{kind} `{}`", short(command))),
                None => ask("not a read-only command, test runner or verify command".into()),
            },
            ApprovalSubject::Tool { name, .. } if is_read_only_tool(name) => allow(format!("read-only tool {name}")),
            ApprovalSubject::Tool { name, .. } => ask(format!("tool {name} is not read-only")),
            _ => ask("balanced mode asks before file changes the CLI did not auto-accept".into()),
        },
        ApprovalMode::Autonomous => match ctx.worktree {
            Some(_) => allow("autonomous: stays inside the worktree".into()),
            None => ask("autonomous mode needs a worktree to contain the agent".into()),
        },
    }
}

fn short(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 { format!("{}…", one.chars().take(59).collect::<String>()) } else { one }
}

/// Codly's own MCP tools, as each CLI names them (`mcp__codly__codly_board` for Claude,
/// `codly_codly_board` for OpenCode…). Only server-qualified names count: a bare
/// `codly_board` could be any MCP server's tool.
pub fn is_codly_tool(name: &str) -> bool {
    let Some(n) = ["mcp__codly__", "codly__", "codly.", "codly/", "codly_"].iter().find_map(|p| name.strip_prefix(p)) else {
        return false;
    };
    [
        tools::GET_TASK,
        tools::BOARD,
        tools::SEARCH,
        tools::MEMORY_SEARCH,
        tools::MEMORY_WRITE,
        tools::KG_QUERY,
        tools::KG_ADD,
        tools::POST_NOTE,
        tools::SUBMIT_PLAN,
        tools::SUBMIT_REPORT,
        tools::ASK_LEAD,
        tools::SUBMIT_REVIEW,
        tools::MESSAGE_MEMBER,
        tools::QUICK_TASK,
        tools::REASSIGN,
    ]
    .contains(&n)
}

fn is_read_only_tool(name: &str) -> bool {
    const RO: &[&str] = &[
        "read",
        "glob",
        "grep",
        "ls",
        "notebookread",
        "tasklist",
        "taskget",
        "taskcreate",
        "taskupdate",
        "todoread",
        "todowrite",
        "read_file",
        "read_many_files",
        "list_directory",
        "search_file_content",
        "list",
        "view",
        "find",
    ];
    RO.contains(&name.to_ascii_lowercase().as_str())
}

// ───────────────────────────── risks ─────────────────────────────

/// Risks Codly can see in a request (in addition to the provider's own hints).
pub fn detect_risks(subject: &ApprovalSubject, worktree: Option<&Path>) -> Vec<String> {
    let mut risks: Vec<String> = vec![];
    let mut add = |r: &str| {
        if !risks.iter().any(|x| x == r) {
            risks.push(r.to_string());
        }
    };
    match subject {
        ApprovalSubject::Command { command, cwd } => {
            let base = cwd.as_deref().map(PathBuf::from).or_else(|| worktree.map(Path::to_path_buf));
            if let (Some(c), Some(w)) = (cwd.as_deref(), worktree)
                && escapes(Path::new(c), w)
            {
                add("outside worktree");
            }
            for r in command_risks(command, base.as_deref(), worktree, 0) {
                add(&r);
            }
        }
        ApprovalSubject::FileChange { paths, diff } => {
            for p in paths {
                if worktree.is_some_and(|w| escapes(&resolve(w, p), w)) {
                    add("outside worktree");
                }
                if p.split('/').any(|s| s == ".git") {
                    add("git internals");
                }
            }
            if diff.as_deref().is_some_and(|d| d.contains("deleted file mode")) {
                add("deletes files");
            }
        }
        ApprovalSubject::Tool { name, input } => {
            let lname = name.to_ascii_lowercase();
            if ["webfetch", "websearch", "web_fetch", "google_web_search", "fetch"].contains(&lname.as_str()) {
                add("network");
            }
            if lname.contains("delete") || lname.contains("remove") {
                add("deletes files");
            }
            if let Some(w) = worktree {
                for p in input_paths(input) {
                    if escapes(&resolve(w, &p), w) {
                        add("outside worktree");
                    }
                }
            }
        }
        ApprovalSubject::Question { .. } | ApprovalSubject::Permissions { .. } => {}
    }
    risks
}

fn input_paths(input: &serde_json::Value) -> Vec<String> {
    let mut out = vec![];
    if let Some(obj) = input.as_object() {
        for (k, v) in obj {
            if !["path", "file_path", "notebook_path", "paths", "directory", "dir", "cwd", "absolute_path"].contains(&k.as_str()) {
                continue;
            }
            match v {
                serde_json::Value::String(s) => out.push(s.clone()),
                serde_json::Value::Array(a) => out.extend(a.iter().filter_map(|x| x.as_str().map(String::from))),
                _ => {}
            }
        }
    }
    out
}

fn resolve(base: &Path, p: &str) -> PathBuf {
    if p.starts_with('~') {
        return PathBuf::from("/~home"); // never inside a worktree
    }
    let p = Path::new(p);
    lexical(&if p.is_absolute() { p.to_path_buf() } else { base.join(p) })
}

/// Normalize `.` and `..` without touching the filesystem.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Directories that are never an agent's, even when its worktree contains them: Codly's
/// data dir (other runs' MCP configs with their tokens, raw logs, the database). Task and solo
/// worktrees live inside it, so only paths outside the agent's own root count.
static PRIVATE_DIRS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Treat `dir` as Codly's own (the Core registers its data dir at start).
pub fn protect_dir(dir: &Path) {
    let mut dirs = PRIVATE_DIRS.write().unwrap_or_else(|e| e.into_inner());
    for d in [lexical(dir), std::fs::canonicalize(dir).map_or_else(|_| lexical(dir), |c| lexical(&c))] {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
}

/// Is `p` outside `root`, or inside one of Codly's private dirs that `root` is not inside?
fn escapes(p: &Path, root: &Path) -> bool {
    let (p, root) = (lexical(p), lexical(root));
    !p.starts_with(&root) || PRIVATE_DIRS.read().unwrap_or_else(|e| e.into_inner()).iter().any(|d| p.starts_with(d) && !root.starts_with(d))
}

/// One shell word, with what the shell would do to it that the text alone does not show.
#[derive(Debug, Clone, Default, PartialEq)]
struct Word {
    text: String,
    /// Holds a `$` expansion (unquoted or in double quotes): its value is unknown.
    var: bool,
    /// Starts with an unquoted glob or brace list (`*`, `?x`, `[ab]*`, `{-o,f}`): it may turn
    /// into several words, options included.
    glob: bool,
}

/// A shell command split into simple commands, with what can be checked statically.
#[derive(Debug, Default)]
struct Parsed {
    segments: Vec<Vec<Word>>,
    /// Files written through `>` / `>>` / `&>`.
    writes: Vec<Word>,
    /// Command or process substitution, here-docs, zsh glob qualifiers and `=(…)`: what runs
    /// cannot be checked.
    complex: bool,
}

/// Does `$` followed by `n` expand (`$HOME`, `${x}`, `$'…'`, zsh `$=x`)? `$?`, `$#`, `$$`, `$!`
/// and `$-` are numbers or flags, never paths; a lone `$` is literal.
fn expands(n: char, in_double_quotes: bool) -> bool {
    n.is_ascii_alphanumeric() || "_{@*[=~^+".contains(n) || (!in_double_quotes && (n == '\'' || n == '"'))
}

#[derive(Default)]
struct Lexer {
    p: Parsed,
    cur: Word,
    in_word: bool,
    redirect: bool,
    /// The current word's first character, when it was not quoted.
    lead: Option<char>,
}

impl Lexer {
    fn push(&mut self, c: char, quoted: bool) {
        if !quoted && self.cur.text.is_empty() && self.lead.is_none() {
            self.lead = Some(c);
        }
        self.in_word = true;
        self.cur.text.push(c);
    }

    fn flush(&mut self) {
        let lead = self.lead.take();
        if !self.in_word {
            return;
        }
        self.in_word = false;
        let mut w = std::mem::take(&mut self.cur);
        let t = w.text.as_str();
        match lead {
            Some('*' | '?') => w.glob = true,
            Some('[') if t.len() > 1 && t.contains(']') => w.glob = true,
            Some('{') if t.contains(',') || t.contains("..") => w.glob = true,
            // zsh `=cmd` expands to the command's path
            Some('=') if t[1..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') => w.var = true,
            _ => {}
        }
        if std::mem::take(&mut self.redirect) {
            self.p.writes.push(w);
        } else if let Some(seg) = self.p.segments.last_mut() {
            seg.push(w);
        }
    }

    fn split(&mut self) {
        self.flush();
        self.p.segments.push(vec![]);
    }
}

fn parse(cmd: &str) -> Parsed {
    let mut lx = Lexer { p: Parsed { segments: vec![vec![]], ..Default::default() }, ..Default::default() };
    let mut it = cmd.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                lx.in_word = true;
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    lx.push(c, true);
                }
            }
            '"' => {
                lx.in_word = true;
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = it.next() {
                                lx.push(n, true);
                            }
                        }
                        '`' => lx.p.complex = true,
                        '$' if it.peek() == Some(&'(') => lx.p.complex = true,
                        '$' => {
                            lx.cur.var |= it.peek().is_some_and(|n| expands(*n, true));
                            lx.push('$', true);
                        }
                        _ => lx.push(c, true),
                    }
                }
            }
            '\\' => {
                lx.in_word = true;
                if let Some(n) = it.next()
                    && n != '\n'
                {
                    lx.push(n, true);
                }
            }
            '`' => lx.p.complex = true,
            '$' if it.peek() == Some(&'(') => lx.p.complex = true,
            '$' => {
                lx.cur.var |= it.peek().is_some_and(|n| expands(*n, false));
                lx.push('$', false);
            }
            ' ' | '\t' => lx.flush(),
            '>' => {
                // "2>file" / "2>&1": a leading fd number is not a word.
                if lx.in_word && !lx.cur.text.is_empty() && lx.cur.text.chars().all(|c| c.is_ascii_digit()) {
                    lx.cur = Word::default();
                    lx.lead = None;
                    lx.in_word = false;
                } else {
                    lx.flush();
                }
                if it.peek() == Some(&'>') {
                    it.next();
                }
                match it.peek() {
                    Some('&') => {
                        it.next();
                        while it.peek().is_some_and(|c| c.is_ascii_digit() || *c == '-') {
                            it.next();
                        }
                    }
                    Some('(') => lx.p.complex = true,
                    _ => lx.redirect = true,
                }
            }
            '<' => {
                lx.flush();
                if matches!(it.peek(), Some('<') | Some('(')) {
                    lx.p.complex = true;
                }
            }
            '&' if it.peek() == Some(&'>') => {
                lx.flush();
                it.next();
                if it.peek() == Some(&'>') {
                    it.next();
                }
                lx.redirect = true;
            }
            '\n' | ';' | '|' | '&' | ')' => lx.split(),
            // Inside a word, `(` starts a zsh glob qualifier (`*(e:cmd:)` runs code), `=(…)`
            // or an extglob: nothing that can be checked.
            '(' if lx.in_word => lx.p.complex = true,
            '(' => lx.split(),
            // `{ …; }` is a group; `{a,b}` and `{}` are words.
            '{' | '}' if !lx.in_word && it.peek().is_none_or(|n| n.is_whitespace() || *n == ';') => lx.split(),
            _ => lx.push(c, false),
        }
    }
    lx.flush();
    let mut p = lx.p;
    p.segments.retain(|s| !s.is_empty());
    p
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn has(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| flags.contains(&a.as_str()) || flags.iter().any(|f| f.starts_with("--") && a.starts_with(&format!("{f}="))))
}

/// Options before `--`.
fn options(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter().map(String::as_str).take_while(|a| *a != "--").filter(|a| a.starts_with('-') && *a != "-")
}

/// A short-option cluster (`-o`, `-ro`, `-ofile`) holds one of `letters`.
fn short_flag(args: &[String], letters: &str) -> bool {
    options(args).filter(|a| !a.starts_with("--")).any(|a| a[1..].contains(|c| letters.contains(c)))
}

/// `--name`, `--name=value` or an abbreviation of it (getopt_long and git accept unambiguous
/// prefixes: `--out=f` is `--output=f`).
fn long_flag(args: &[String], names: &[&str]) -> bool {
    options(args)
        .filter_map(|a| a.strip_prefix("--"))
        .map(|a| a.split('=').next().unwrap_or(a))
        .any(|n| !n.is_empty() && names.iter().any(|x| x.starts_with(n)))
}

/// Operands (non-option arguments), skipping the value of the short options in `valued`.
fn operands<'a>(args: &'a [String], valued: &str) -> Vec<&'a str> {
    let (mut out, mut i, mut done) = (vec![], 0, false);
    while let Some(a) = args.get(i).map(String::as_str) {
        i += 1;
        if done || a == "-" || !a.starts_with('-') {
            out.push(a);
        } else if a == "--" {
            done = true;
        } else if !a.starts_with("--")
            && let Some(j) = a[1..].find(|c| valued.contains(c))
            && j + 2 == a.len()
        {
            i += 1;
        }
    }
    out
}

fn first_non_flag(args: &[String]) -> Option<&str> {
    args.iter().map(String::as_str).find(|a| !a.starts_with('-'))
}

/// `--package=x` / `-p x` on npx, bunx or `npm exec`: installs and runs that registry package.
fn package_flag(args: &[String]) -> bool {
    options(args).any(|a| a.starts_with("--package") || (a.starts_with("-p") && !a.starts_with("--")))
}

/// `npx -c 'cmd'` / `npm exec --call 'cmd'`: runs a shell command string.
fn call_flag(args: &[String]) -> bool {
    options(args).any(|a| a == "-c" || a.starts_with("--call"))
}

/// The git subcommand, skipping global options (`-C dir`, `-c k=v`, `--no-pager`…).
fn git_sub(args: &[String]) -> (Option<&str>, &[String]) {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-C" || a == "-c" || a == "--git-dir" || a == "--work-tree" || a == "--namespace" {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            return (Some(a), &args[i + 1..]);
        }
    }
    (None, &[])
}

const NETWORK: &[&str] =
    &["curl", "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "netcat", "telnet", "ftp", "http", "https", "aria2c", "gh"];
const DELETES: &[&str] = &["rm", "rmdir", "unlink", "shred", "srm", "trash"];
const HARMLESS_PATHS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr"];

/// Values a word may stand for: itself, an option's value glued to its flag (`-f/etc/hosts`,
/// `--file=/etc/hosts`, `FOO=~/x`), the parts of a brace list (`{a,/etc/x}`), and `~` entries
/// of a `:` list (`PATH=x:~/bin`).
fn path_candidates(t: &str) -> Vec<&str> {
    let mut out = vec![t];
    if let Some((_, v)) = t.split_once('=') {
        out.push(v);
    }
    if t.starts_with('-') {
        let v = t.trim_start_matches('-').trim_start_matches(|c: char| c.is_ascii_alphabetic());
        if !v.is_empty() {
            out.push(v);
        }
    }
    if t.contains('{') {
        out.extend(t.split(['{', ',', '}']));
    }
    out.extend(t.split(':').skip(1).filter(|p| p.starts_with('~')));
    out
}

fn path_outside(v: &str, base: Option<&Path>, root: Option<&Path>) -> bool {
    if v.starts_with('~') {
        return true; // `~`, `~/x`, `~user/x`
    }
    if HARMLESS_PATHS.contains(&v) {
        return false;
    }
    match (base, root) {
        (Some(base), Some(root)) => escapes(&resolve(base, v), root),
        _ => v.starts_with('/'),
    }
}

/// Could this argument name something outside the worktree? A `$` expansion is unknown, so
/// it could (`cat $HOME/.profile`).
fn word_outside(w: &Word, base: Option<&Path>, root: Option<&Path>) -> bool {
    w.var || path_candidates(&w.text).into_iter().any(|v| path_outside(v, base, root))
}

/// The same for a command name: `/usr/bin/git` is fine, `~/bin/x`, `../../x.sh` and `$X` are not.
fn program_outside(w: &Word, base: Option<&Path>, root: Option<&Path>) -> bool {
    w.var || w.text.starts_with('~') || (w.text.contains("..") && path_outside(&w.text, base, root))
}

fn command_risks(cmd: &str, cwd: Option<&Path>, worktree: Option<&Path>, depth: u8) -> Vec<String> {
    let p = parse(cmd);
    let mut risks = vec![];
    let mut add = |r: &str| {
        if !risks.iter().any(|x: &String| x == r) {
            risks.push(r.to_string());
        }
    };
    if p.complex {
        add("command substitution or shell expansion (cannot be checked)");
    }
    let base = cwd.or(worktree);
    for w in &p.writes {
        if word_outside(w, base, worktree) {
            add("outside worktree");
        }
    }
    for seg in &p.segments {
        let texts: Vec<String> = seg.iter().map(|w| w.text.clone()).collect();
        let start = classify::command_start(&texts);
        let at = start.unwrap_or(texts.len());
        for (k, w) in seg.iter().enumerate() {
            let command_word = k == at || (k < at && classify::is_wrapper(&w.text));
            if if command_word { program_outside(w, base, worktree) } else { word_outside(w, base, worktree) } {
                add("outside worktree");
            }
        }
        if texts[..at].iter().any(|w| matches!(basename(w), "sudo" | "doas")) {
            add("sudo");
        }
        let Some(start) = start else {
            add("runs dynamic code"); // `env -S 'git push'`
            continue;
        };
        let Some(first) = seg.get(start) else { continue };
        if first.var || first.glob {
            add("runs dynamic code"); // `$CMD args`: the program itself is unknown
            continue;
        }
        let prog = basename(&first.text).to_string();
        let args = &texts[start + 1..];
        // `cd`, `cd -`, `pushd`, `popd` without a directory go home or back: outside.
        if matches!(prog.as_str(), "cd" | "pushd" | "popd") && args.iter().all(|a| a.starts_with('-')) {
            add("outside worktree");
        }
        // Programs that run other programs: check what they run.
        let mut runs: Vec<(String, Vec<String>)> = vec![(prog.clone(), args.to_vec())];
        if prog == "find"
            && let Some(i) = args.iter().position(|a| ["-exec", "-execdir", "-ok", "-okdir"].contains(&a.as_str()))
            && let Some(x) = args.get(i + 1)
        {
            runs.push((basename(x).to_string(), args[i + 2..].to_vec()));
        }
        for (prog, args) in runs {
            let a = args.as_slice();
            match prog.as_str() {
                "sudo" | "doas" | "su" => add("sudo"),
                p if DELETES.contains(&p) => add("deletes files"),
                "find" if has(a, &["-delete"]) => add("deletes files"),
                p if NETWORK.contains(&p) => add("network"),
                "chmod" | "chown" | "chgrp" if has(a, &["-R", "--recursive"]) => add("changes permissions recursively"),
                "dd" | "mkfs" | "diskutil" | "fdisk" => add("disk"),
                "kill" | "pkill" | "killall" | "launchctl" | "systemctl" | "shutdown" | "reboot" => add("system"),
                "docker" | "podman" => add("containers"),
                "eval" | "source" | "." => add("runs dynamic code"),
                p if classify::SHELLS.contains(&p) => match classify::shell_run(a) {
                    ShellRun::Command(inner) if depth < 3 => {
                        for r in command_risks(inner, cwd, worktree, depth + 1) {
                            add(&r);
                        }
                    }
                    ShellRun::Command(_) => add("runs dynamic code"),
                    ShellRun::Stdin => add("pipes into a shell"),
                    ShellRun::Script => {}
                },
                "git" => {
                    let (sub, rest) = git_sub(a);
                    match sub.unwrap_or("") {
                        "push" => add("push"),
                        "fetch" | "pull" | "clone" | "ls-remote" | "submodule" => add("network"),
                        "clean" => add("deletes files"),
                        "reset" if has(rest, &["--hard"]) => add("discards changes"),
                        "restore" | "checkout" if rest.iter().any(|x| x == "--" || x == ".") => add("discards changes"),
                        s if !git_read_only(s, rest) => add("git write (Codly commits and merges for you)"),
                        _ => {}
                    }
                }
                "npm" | "pnpm" | "yarn" | "bun" => match first_non_flag(a).unwrap_or("") {
                    "publish" => add("publish"),
                    "install" | "i" | "add" | "ci" | "update" | "upgrade" | "up" | "remove" | "rm" | "uninstall" | "dlx" | "x"
                    | "create" => add("network (package install)"),
                    "exec" if package_flag(a) => add("network (package install)"),
                    "exec" if call_flag(a) => add("runs dynamic code"),
                    _ => {}
                },
                "npx" | "bunx" if package_flag(a) || !first_non_flag(a).is_some_and(|x| RUNNERS.contains(&x)) => add("network (npx)"),
                "pip" | "pip3" | "poetry" | "uv" | "pipx" | "gem" | "bundle" | "brew" | "apt" | "apt-get" | "yum" | "dnf" | "composer" => {
                    if first_non_flag(a).is_some_and(|x| {
                        ["install", "add", "update", "upgrade", "sync", "lock", "require", "remove", "uninstall", "pip", "tool"]
                            .contains(&x)
                    }) {
                        add("network (package install)");
                    }
                }
                "cargo" if first_non_flag(a).is_some_and(|x| ["install", "add", "publish", "remove", "update", "login"].contains(&x)) => {
                    add("network (package install)")
                }
                "go" if first_non_flag(a).is_some_and(|x| ["get", "install"].contains(&x)) => add("network (package install)"),
                _ => {}
            }
        }
    }
    risks
}

fn git_read_only(sub: &str, rest: &[String]) -> bool {
    match sub {
        "" | "status" | "diff" | "log" | "show" | "blame" | "grep" | "ls-files" | "ls-tree" | "rev-parse" | "describe" | "shortlog"
        | "cat-file" | "version" | "rev-list" | "merge-base" | "name-rev" | "whatchanged" | "show-ref" | "for-each-ref"
        | "count-objects" => true,
        // `git help -w` starts a browser
        "help" => rest.iter().all(|a| !a.starts_with('-')),
        "reflog" => !first_non_flag(rest).is_some_and(|a| ["expire", "delete", "drop", "write"].contains(&a)),
        "branch" | "tag" => rest
            .iter()
            .all(|a| ["-a", "-r", "-l", "--list", "-v", "-vv", "--all", "--show-current", "--contains", "--merged"].contains(&a.as_str())),
        "remote" => rest.iter().all(|a| a == "-v" || a == "show" || a == "get-url"),
        "config" => has(rest, &["--get", "--list", "-l", "--get-all", "--get-regexp"]),
        "stash" => rest.first().is_some_and(|a| a == "list" || a == "show"),
        "worktree" => rest.first().is_some_and(|a| a == "list"),
        _ => false,
    }
}

// ───────────────────────────── balanced mode ─────────────────────────────

/// Test runners and checkers, as the program itself or behind npx/bunx/`pnpm exec`.
const RUNNERS: &[&str] = &[
    "jest",
    "vitest",
    "mocha",
    "ava",
    "tap",
    "playwright",
    "pytest",
    "py.test",
    "tox",
    "nox",
    "rspec",
    "phpunit",
    "ctest",
    "tsc",
    "eslint",
    "svelte-check",
    "vue-tsc",
    "mypy",
    "pyright",
    "ruff",
    "flake8",
    "pylint",
    "black",
    "isort",
    "golangci-lint",
    "staticcheck",
    "swiftlint",
    "shellcheck",
    "stylelint",
    "biome",
    "prettier",
];

/// Script names (`npm run <x>`) that are checks rather than arbitrary code.
fn check_script(s: &str) -> bool {
    ["test", "lint", "typecheck", "type-check", "check", "build", "tsc", "verify", "ci"]
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}:")) || s.starts_with(&format!("{p}-")))
}

/// Programs whose options can neither write files nor run commands, so an argument the shell
/// expands (`*`, `{a,b}`) cannot turn them into anything else (a file named `--pre=sh` makes
/// `rg x *` run every file through `sh`).
const GLOB_SAFE: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "stat", "du", "nl", "strings", "cksum", "md5", "md5sum", "sha1sum",
    "sha256sum", "shasum", "echo",
];

/// Why a command is safe to auto-allow in balanced mode, if it is.
pub fn classify_command(cmd: &str, verify: &[String]) -> Option<&'static str> {
    let trimmed = cmd.trim();
    if verify.iter().any(|v| v.trim() == trimmed) {
        return Some("verify command");
    }
    let p = parse(cmd);
    if p.complex || p.writes.iter().any(|w| w.var || w.text != "/dev/null") || p.segments.is_empty() {
        return None;
    }
    let mut kind = "read-only command";
    for seg in &p.segments {
        let (first, rest) = seg.split_first()?;
        // A plain `program args…`: a bare name (`./cat` or `bin/tool` could be anything), no
        // `VAR=x` prefix (GIT_EXTERNAL_DIFF, PAGER, LD_PRELOAD…), no wrapper, no `$` expansion.
        let prog = first.text.as_str();
        if first.var
            || first.glob
            || prog.is_empty()
            || prog.contains('/')
            || classify::is_assignment(prog)
            || classify::is_wrapper(prog)
            || rest.iter().any(|w| w.var)
            || (rest.iter().any(|w| w.glob) && !GLOB_SAFE.contains(&prog))
        {
            return None;
        }
        let args: Vec<String> = rest.iter().map(|w| w.text.clone()).collect();
        if read_only(prog, &args) {
            continue;
        }
        if test_runner(prog, &args) {
            kind = "test runner";
            continue;
        }
        return None;
    }
    Some(kind)
}

fn read_only(prog: &str, args: &[String]) -> bool {
    const RO: &[&str] = &[
        "ls",
        "cat",
        "head",
        "tail",
        "wc",
        "grep",
        "egrep",
        "fgrep",
        "pwd",
        "echo",
        "printf",
        "which",
        "type",
        "whereis",
        "stat",
        "du",
        "df",
        "cut",
        "tr",
        "diff",
        "cmp",
        "comm",
        "jq",
        "basename",
        "dirname",
        "realpath",
        "readlink",
        "true",
        "false",
        "test",
        "[",
        "nl",
        "column",
        "hexdump",
        "od",
        "strings",
        "sha1sum",
        "sha256sum",
        "shasum",
        "md5",
        "md5sum",
        "cksum",
        "uname",
        "whoami",
        "id",
        "printenv",
        "seq",
        "cd",
    ];
    // ponytail: `ack` (a project `.ackrc` can set --pager) and `hostname NAME` are not here;
    // they ask.
    match prog {
        p if RO.contains(&p) => true,
        // Options that write a file or run a program:
        "sort" => !short_flag(args, "oT") && !long_flag(args, &["output", "compress-program", "temporary-directory"]),
        // the second operand is the output file
        "uniq" => operands(args, "fsw").len() <= 1,
        "xxd" => operands(args, "cglosn").len() <= 1,
        // -o writes the listing, -R writes 00Tree.html into every directory
        "tree" => !short_flag(args, "oR"),
        "fd" => !short_flag(args, "xX") && !long_flag(args, &["exec", "exec-batch"]),
        "rg" => !long_flag(args, &["pre", "pre-glob", "hostname-bin"]),
        "ag" => !long_flag(args, &["pager"]),
        "file" => !short_flag(args, "C") && !long_flag(args, &["compile"]),
        "date" => !short_flag(args, "s") && !long_flag(args, &["set"]) && operands(args, "").iter().all(|a| a.starts_with('+')),
        "find" => !has(args, &["-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-fls"]),
        "sed" => sed_read_only(args),
        "git" => git_read_only_cmd(args),
        _ => false,
    }
}

/// `git …` that only reads: no `-c core.pager=…` / `--config-env` / `--exec-path` (they make
/// any subcommand run a program), no `--output=<file>`, no `git grep -O<pager>`.
fn git_read_only_cmd(args: &[String]) -> bool {
    let mut i = 0;
    while let Some(a) = args.get(i).map(String::as_str).filter(|a| a.starts_with('-')) {
        if a.starts_with("-c") || a.starts_with("--config-env") || a.starts_with("--exec-path") {
            return false;
        }
        i += if matches!(a, "-C" | "--git-dir" | "--work-tree" | "--namespace") { 2 } else { 1 };
    }
    let (sub, rest) = git_sub(args);
    let sub = sub.unwrap_or("");
    if long_flag(rest, &["output"]) || (sub == "grep" && (short_flag(rest, "O") || long_flag(rest, &["open-files-in-pager"]))) {
        return false;
    }
    git_read_only(sub, rest)
}

/// sed without in-place edits (`-i`, `-ni`, `--in-place`), script files (`-f`) or script
/// commands that write, read or run (`w`, `W`, `r`, `R`, `e`, `s///w`, `s///e`).
fn sed_read_only(args: &[String]) -> bool {
    let mut scripts: Vec<&str> = vec![];
    let mut files: Vec<&str> = vec![];
    let (mut i, mut done) = (0, false);
    while let Some(a) = args.get(i).map(String::as_str) {
        i += 1;
        if done || a == "-" || !a.starts_with('-') {
            files.push(a);
            continue;
        }
        if a == "--" {
            done = true;
            continue;
        }
        if let Some(long) = a.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            match (name, value) {
                ("expression", Some(v)) => scripts.push(v),
                ("expression", None) => match args.get(i) {
                    Some(v) => {
                        scripts.push(v);
                        i += 1;
                    }
                    None => return false,
                },
                ("quiet" | "silent" | "regexp-extended" | "separate" | "sandbox" | "debug" | "posix" | "null-data" | "zero-terminated"
                | "unbuffered", None) => {}
                // --in-place, --file, abbreviations of either, anything unknown
                _ => return false,
            }
            continue;
        }
        for (j, c) in a[1..].char_indices() {
            match c {
                'n' | 's' | 'E' | 'r' | 'u' | 'z' => {}
                'e' => {
                    let rest = &a[2 + j..];
                    if !rest.is_empty() {
                        scripts.push(rest);
                    } else if let Some(v) = args.get(i) {
                        scripts.push(v);
                        i += 1;
                    } else {
                        return false;
                    }
                    break;
                }
                // i: in place; f: script file; l: a flag in BSD sed but takes a value in GNU sed
                _ => return false,
            }
        }
    }
    if scripts.is_empty() {
        let Some(s) = files.first() else { return false };
        scripts.push(s);
    }
    scripts.iter().all(|s| sed_script_read_only(s))
}

/// Walk a sed script command by command. Anything not understood counts as not read-only.
fn sed_script_read_only(script: &str) -> bool {
    let c: Vec<char> = script.chars().collect();
    let n = c.len();
    let mut i = 0;
    let blank = |i: &mut usize| {
        while *i < n && (c[*i] == ' ' || c[*i] == '\t') {
            *i += 1;
        }
    };
    // `/re/`, `\%re%`: index after the closing delimiter
    let regex = |mut i: usize, delim: char| -> Option<usize> {
        while i < n && c[i] != delim {
            if c[i] == '\\' {
                i += 1;
            }
            i += 1;
        }
        (i < n).then_some(i + 1)
    };
    // One address: a line number, `first~step`, `$`, `/re/` or `\cREc` (+ GNU `I`/`M`), or
    // after a comma GNU `+N` / `~N`.
    let address = |mut i: usize| -> Option<usize> {
        match c.get(i) {
            Some(d) if d.is_ascii_digit() || *d == '+' || *d == '~' => {
                i += 1;
                while i < n && (c[i].is_ascii_digit() || c[i] == '~') {
                    i += 1;
                }
                Some(i)
            }
            Some('$') => Some(i + 1),
            Some('/') => regex(i + 1, '/').map(|mut j| {
                while j < n && (c[j] == 'I' || c[j] == 'M') {
                    j += 1;
                }
                j
            }),
            Some('\\') => {
                let d = *c.get(i + 1)?;
                regex(i + 2, d).map(|mut j| {
                    while j < n && (c[j] == 'I' || c[j] == 'M') {
                        j += 1;
                    }
                    j
                })
            }
            _ => Some(i),
        }
    };
    loop {
        while i < n && (c[i].is_whitespace() || c[i] == ';') {
            i += 1;
        }
        if i >= n {
            return true;
        }
        let Some(j) = address(i) else { return false };
        i = j;
        blank(&mut i);
        if i < n && c[i] == ',' {
            i += 1;
            blank(&mut i);
            let Some(j) = address(i) else { return false };
            i = j;
        }
        while i < n && (c[i] == '!' || c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        let Some(&cmd) = c.get(i) else { return false };
        i += 1;
        match cmd {
            '{' | '}' | '=' | 'd' | 'D' | 'g' | 'G' | 'h' | 'H' | 'n' | 'N' | 'p' | 'P' | 'x' | 'z' | 'F' => {}
            // optional numeric argument
            'l' | 'L' | 'q' | 'Q' => {
                blank(&mut i);
                while i < n && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // a label, up to `;` or the end of the line
            ':' | 'b' | 't' | 'T' | 'v' => {
                while i < n && c[i] != ';' && c[i] != '\n' {
                    i += 1;
                }
            }
            '#' => {
                while i < n && c[i] != '\n' {
                    i += 1;
                }
            }
            // text to the end of the line, continued by a trailing backslash
            'a' | 'i' | 'c' => {
                while i < n && !(c[i] == '\n' && c[i - 1] != '\\') {
                    i += 1;
                }
            }
            's' | 'y' => {
                let Some(&delim) = c.get(i) else { return false };
                if delim == '\\' || delim == '\n' {
                    return false;
                }
                let Some(j) = regex(i + 1, delim).and_then(|j| regex(j, delim)) else { return false };
                i = j;
                if cmd == 's' {
                    while i < n && !matches!(c[i], ';' | '\n' | '}') {
                        if !(c[i].is_ascii_digit() || matches!(c[i], 'g' | 'p' | 'i' | 'I' | 'm' | 'M' | ' ' | '\t')) {
                            return false; // `w file`, `e`, anything unknown
                        }
                        i += 1;
                    }
                }
            }
            // w W r R e, anything else
            _ => return false,
        }
    }
}

fn test_runner(prog: &str, args: &[String]) -> bool {
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect();
    let writes = has(args, &["--fix", "--write", "-w", "--watch", "--update-snapshots", "-u"]);
    if writes {
        return false;
    }
    match prog {
        "npm" | "pnpm" | "yarn" | "bun" => {
            if let Some(i) = words.iter().position(|w| *w == "run" || *w == "run-script") {
                return words.get(i + 1).is_some_and(|s| check_script(s));
            }
            // `dlx` / `x` always fetch the package: never a plain test run
            if words.contains(&"dlx") || words.first() == Some(&"x") {
                return false;
            }
            if let Some(i) = words.iter().position(|w| *w == "exec") {
                return !package_flag(args) && !call_flag(args) && words.get(i + 1).is_some_and(|r| RUNNERS.contains(r));
            }
            words.iter().any(|w| ["test", "t", "lint", "typecheck", "check"].contains(w) || RUNNERS.contains(w))
        }
        "npx" | "bunx" => !package_flag(args) && !call_flag(args) && words.first().is_some_and(|r| RUNNERS.contains(r)),
        // `node --test [files]`: Node's built-in runner (seen live from Claude workers).
        "node" => has(args, &["--test"]) && !has(args, &["--eval", "-e", "--print", "-p"]),
        "black" | "isort" | "prettier" => has(args, &["--check", "--diff", "-c"]),
        "ruff" => words.first().is_some_and(|w| *w == "check") || has(args, &["--check"]),
        p if RUNNERS.contains(&p) => true,
        "python" | "python3" | "uv" | "poetry" => {
            let w: Vec<&str> = if prog == "uv" || prog == "poetry" { words.iter().skip(1).copied().collect() } else { words.clone() };
            w.first().is_some_and(|x| RUNNERS.contains(x))
                || has(args, &["-m"]) && w.first().is_some_and(|m| ["pytest", "unittest", "mypy", "pyright", "ruff"].contains(m))
        }
        "go" => words.first().is_some_and(|w| ["test", "vet", "build"].contains(w)),
        "cargo" => match words.first().copied() {
            Some("test" | "check" | "clippy" | "build" | "nextest" | "bench" | "doc" | "metadata" | "tree") => true,
            Some("fmt") => has(args, &["--check"]),
            _ => false,
        },
        "make" | "just" | "task" | "ninja" => words.iter().any(|w| ["test", "tests", "check", "lint", "typecheck", "build"].contains(w)),
        "gradle" | "gradlew" | "mvn" | "mvnw" => words.iter().any(|w| ["test", "check", "build", "verify", "compile"].contains(w)),
        "dotnet" | "swift" | "deno" | "mix" | "zig" | "bazel" | "bazelisk" | "xcodebuild" => {
            words.iter().any(|w| ["test", "build", "check", "lint", "vet"].contains(w))
        }
        _ => false,
    }
}

#[cfg(test)]
mod node_test_runner {
    use super::*;

    #[test]
    fn node_test_pipelines_are_test_runs() {
        assert_eq!(classify_command("node --test test/strings.test.js 2>&1 | tail -8", &[]), Some("test runner"));
        assert_eq!(classify_command("node --test", &[]), Some("test runner"));
        assert_eq!(classify_command("node script.js", &[]), None);
        assert_eq!(classify_command("node --test -e 'require(\"fs\").rmSync(\"x\")'", &[]), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(segs: &[Vec<Word>]) -> Vec<Vec<&str>> {
        segs.iter().map(|s| s.iter().map(|w| w.text.as_str()).collect()).collect()
    }

    #[test]
    fn tokenizer_finds_segments_and_writes() {
        let p = parse("cd src && grep -rn 'a b' . | head -5 > out.txt 2>&1");
        assert_eq!(texts(&p.segments), vec![vec!["cd", "src"], vec!["grep", "-rn", "a b", "."], vec!["head", "-5"]]);
        assert_eq!(p.writes.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), vec!["out.txt"]);
        assert!(!p.complex);
        assert!(parse("echo $(whoami)").complex);
        assert!(parse("cat <<EOF\nx\nEOF").complex);
        assert_eq!(parse("ls 2>/dev/null").writes[0].text, "/dev/null");
        // what the shell expands is marked; quoted text is not
        let p = parse("cat $HOME/x \"${A}\" $'b' '$c' \\$d 'x$' * a* ''{-o,f} {} \"$?\"");
        let flags: Vec<(bool, bool)> = p.segments[0].iter().map(|w| (w.var, w.glob)).collect();
        let (v, g, none) = ((true, false), (false, true), (false, false));
        assert_eq!(flags, vec![none, v, v, v, none, none, none, g, none, g, none, none]);
        assert_eq!(texts(&parse("(git push) && { ls; }").segments), vec![vec!["git", "push"], vec!["ls"]]);
    }

    /// Reviewer-verified bypasses: each of these was auto-allowed as a read.
    #[test]
    fn writes_and_execs_are_not_read_only() {
        for c in [
            "sort -o out.txt in.txt",
            "sort -ro out.txt in.txt",
            "sort --output=out.txt in.txt",
            "sort --out=out.txt in.txt",
            "sort --compress-program=sh in.txt",
            "uniq a b",
            "xxd a b",
            "tree -o f",
            "tree -R -H . src",
            "sed -n 'w out.txt' in",
            "sed -n '1,5w out.txt' in",
            "sed 's/a/b/w out.txt' in",
            "sed 's/a/b/e' in",
            "sed -n '/x/{p;r /etc/hosts\n}' in",
            "sed '1e touch x' in",
            "sed -ni p in",
            "sed -i.bak s/a/b/ in",
            "sed --in-place s/a/b/ in",
            "sed -f script.sed in",
            "git diff --output=f",
            "git log -p --output f",
            "git grep -O foo",
            "git grep --open-files-in-pager=vim foo",
            "git reflog expire --all",
            "git help -w log",
            "fd . -x true",
            "fd -HX rm",
            "fd . --exec true",
            "rg --pre ./x.sh foo",
            "rg --pre=./x.sh foo",
            "rg --pre-glob '*.pdf' --pre x foo",
            "rg --hostname-bin ./x.sh foo",
            "ag --pager=./x.sh foo",
            "ack foo",
            "file -C -m magic",
            "date -s 12:00",
            "date 0101000026",
            "hostname evil",
            "git -c diff.external=true diff",
            "git -c core.pager=./x.sh log",
            "git --config-env=core.pager=X log",
            "git --exec-path=. status",
            "GIT_EXTERNAL_DIFF=true git diff",
            "PAGER=./x.sh git log",
            "env git diff",
            "nice cat x",
            "./cat x",
            "./tool.sh --help",
            "bin/tool --version",
            "/tmp/x/ls",
            "cargo --version",
            "cat $HOME/.profile",
            "sort $X",
            "$CMD foo",
            "rg foo *",
            "sort ''{-o,out} in",
            "ls *(e:true:)",
        ] {
            assert_eq!(classify_command(c, &[]), None, "{c}");
        }
        for c in [
            "sort in.txt",
            "sort -rn -k2 in.txt",
            "uniq -c in.txt",
            "xxd -l 64 in.bin",
            "tree -L 2 src",
            "sed -n '1,40p' src/lib.rs",
            "sed -n '/start/,/end/p;$p' x",
            "sed -E 's/(a|b)/x/g' x",
            "sed -n -e 's/x/y/p' -e '2{p;q}' x",
            "sed '1a hello; w out' x",
            "git diff --stat HEAD~1",
            "git log --oneline -5",
            "git grep -n foo",
            "git reflog -5",
            "fd -e rs src",
            "rg -n --pretty 'fn main$' src",
            "date +%s",
            "ls *.ts",
            "wc -l src/*.rs",
            "grep -n foo *",
            "[ -f x ]",
            "echo \"exit $?\"",
        ] {
            assert_eq!(classify_command(c, &[]), Some("read-only command"), "{c}");
        }
    }

    fn risks(c: &str) -> Vec<String> {
        command_risks(c, Some(Path::new("/w/t1")), Some(Path::new("/w/t1")), 0)
    }

    #[test]
    fn hidden_paths_are_outside_the_worktree() {
        for c in [
            "cat $HOME/.profile",
            "cat \"${HOME}/.profile\"",
            "cat $'\\x2fetc/hosts'",
            "grep -f/etc/hosts x",
            "grep --file=/etc/hosts x",
            "tar -xzf~/a.tgz",
            "cat ~root/.profile",
            "PATH=bin:~/bin make",
            "cat {a,/etc/hosts}",
            "cat =ls",
            "cd && cat .profile",
            "cd - && ls",
            "~/bin/tool",
            "../../run.sh",
            "echo x > $OUT",
        ] {
            assert!(risks(c).iter().any(|r| r == "outside worktree"), "{c}: {:?}", risks(c));
        }
        for c in ["cat src/a.rs", "rg 'x$' src", "git diff HEAD~1..HEAD", "echo \"$?\"", "/usr/bin/git status", "find . -exec grep -l x {} +"] {
            assert!(!risks(c).iter().any(|r| r == "outside worktree"), "{c}: {:?}", risks(c));
        }
    }

    #[test]
    fn codly_data_dir_is_outside_even_inside_the_root() {
        protect_dir(Path::new("/home/me/Library/Codly"));
        let r = |c: &str| command_risks(c, Some(Path::new("/home/me")), Some(Path::new("/home/me")), 0);
        assert!(r("cat /home/me/Library/Codly/runs/r1/claude-mcp.json").iter().any(|x| x == "outside worktree"));
        assert!(r("cat Library/Codly/runs/r1/claude-mcp.json").iter().any(|x| x == "outside worktree"));
        assert!(r("cat notes.txt").is_empty());
        let wt = Path::new("/home/me/Library/Codly/worktrees/repo/p/t1");
        assert!(command_risks("cat src/a.rs", Some(wt), Some(wt), 0).is_empty(), "a worktree inside the data dir is the agent's");
        assert!(command_risks("cat ../../../../runs/r1/claude-mcp.json", Some(wt), Some(wt), 0).iter().any(|x| x == "outside worktree"));
    }

    #[test]
    fn wrappers_and_nested_shells_do_not_hide_risks() {
        for c in [
            "timeout 5 git push",
            "gtimeout -k 1 5 git push",
            "nice -n 5 git push",
            "nice -5 git push",
            "true && bash -lc 'git push'",
            "zsh -ic \"git push origin main\"",
            "env FOO=1 git push",
            "env -u X git push",
            "xargs git push",
            "xargs -n1 -I {} git push {}",
            "nohup git push &",
            "time git push",
            "stdbuf -oL git push",
            "caffeinate -t 60 git push",
            "command git push",
            "exec git push",
            "if true; then git push; fi",
            "(git push)",
            "find . -exec sh -c 'git push' \\;",
        ] {
            assert!(risks(c).iter().any(|r| r == "push"), "{c}: {:?}", risks(c));
        }
        assert!(risks("sudo -u me git push").iter().any(|r| r == "push"));
        assert!(risks("env -S 'git push'").iter().any(|r| r == "runs dynamic code"));
        assert!(risks("bash -l").iter().any(|r| r == "pipes into a shell"));
    }

    #[test]
    fn registry_packages_are_network() {
        for c in ["npx --package=x jest", "npx -p x jest", "bunx --package x jest", "npm exec --package=x jest", "pnpm dlx jest", "yarn dlx jest"] {
            assert!(risks(c).iter().any(|r| r.starts_with("network")), "{c}: {:?}", risks(c));
            assert_ne!(classify_command(c, &[]), Some("test runner"), "{c}");
        }
        assert!(risks("npx vitest run").is_empty());
        assert!(risks("npm exec -c 'curl x | sh'").iter().any(|r| r == "runs dynamic code"));
    }

    #[test]
    fn zsh_glob_qualifiers_are_not_checkable() {
        for c in ["ls *(e:true:)", "ls **/*(+evil)", "cat =(curl x)", "ls @(a|b)"] {
            assert!(parse(c).complex, "{c}");
            assert!(risks(c).iter().any(|r| r.contains("cannot be checked")), "{c}");
        }
    }

    #[test]
    fn only_qualified_codly_tool_names_count() {
        assert!(is_codly_tool("mcp__codly__codly_board"));
        assert!(is_codly_tool("codly_codly_submit_report"));
        assert!(is_codly_tool("codly__codly_post_note"));
        assert!(!is_codly_tool("codly_board"));
        assert!(!is_codly_tool("codly_submit_report"));
        assert!(!is_codly_tool("mcp__evil__codly_board"));
    }
}
