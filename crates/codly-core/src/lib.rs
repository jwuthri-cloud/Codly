//! Codly core: everything except the window. No Tauri dependency, so it is testable
//! with `cargo test` and reusable by a future daemon.
//!
//! Module map (see docs/ARCHITECTURE.md):
//! - `model`        shared domain + event types (ts-rs → src/lib/gen)
//! - `env`          login-shell env capture, CLI detection, child env scrubbing
//! - `proc`         process spawning, bounded NDJSON reader, shutdown ladder
//! - `adapters`     Claude (stream-json), Codex (app-server), ACP (Gemini, OpenCode…) → EventKind
//! - `store`        SQLite (WAL) schema, migrations, typed queries
//! - `git`          worktrees, commits, diffs, merges (shells out to `git`)
//! - `orchestrator` the Core facade: Lead supervisor, planner, scheduler, gates, approvals
//! - `mcp`          Codly's MCP server (UDS) + stdio bridge (`codly mcp`) + tool specs
//! - `index`        code retrieval: FTS5 over paths/symbols/chunks, RRF, citations, pointer packs
//! - `memory`       shared/private memories, knowledge triples, memory packs
//! - `pty`          interactive terminals (portable-pty)
//! - `dictation`    Free Flow: Groq Whisper transcription, key in the Keychain
//! - `triggers`     schedules, context rules, network mode: pure policy (run by `orchestrator/triggers.rs`)
//! - `skills`       skills manager: scan, catalog, install, uninstall
//! - `ide`          confined file list/read/write/search for the workspace IDE overlay
//! - `inbound`      webhooks and Slack: outside messages for the Lead, secrets in the Keychain
//! - `install`      onboarding: trusted install commands per CLI, run in a visible terminal
//! - `byok`         opt-in: your own API key per vendor, in the Keychain, each for one CLI only
//! - `quota`        opt-in: a Claude sign-in's usage, read with its OAuth token (`read_claude_quota`)
//! - `hire`         hire manifests (the add-agent wizard's import/AI draft): strict validation
//! - `lot`          build mode's lots: the ready-made Studio and Garage, a repo's first lot, checks
//! - `price`        build mode's price table
//! - `repos`        the git repositories under a folder, for the browser's "Open repository" picker
//! - `housekeeping` board archiving, the default MCP servers of new agents (keys in the Keychain),
//!   and storage: what Codly leaves on disk and in a repository, and what may be removed
//! - `blockers`     "ask me" cards: a decision an agent needs from you
//! - `notifications` which events raise a notification of your system: the switches in Settings
//! - `career`       XP, ranks, skills and traits from real events: pure functions

pub mod adapters;
pub mod dictation;
pub mod blockers;
pub mod byok;
pub mod career;
pub mod env;
pub mod git;
pub mod ide;
pub mod inbound;
pub mod hire;
pub mod housekeeping;
pub mod index;
pub mod install;
pub mod lot;
pub mod mcp;
pub mod memory;
pub mod model;
pub mod notifications;
pub mod orchestrator;
pub mod price;
pub mod proc;
pub mod quota;
pub mod pty;
pub mod repos;
pub mod skills;
pub mod store;
pub mod triggers;

pub use orchestrator::{Core, CoreConfig};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
