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
//! - `byok`         opt-in OpenRouter key for OpenCode only, in the Keychain
//! - `hire`         hire manifests (the add-agent wizard's import/AI draft): strict validation

pub mod adapters;
pub mod dictation;
pub mod blockers;
pub mod byok;
pub mod env;
pub mod git;
pub mod ide;
pub mod inbound;
pub mod hire;
pub mod housekeeping;
pub mod index;
pub mod install;
pub mod mcp;
pub mod memory;
pub mod model;
pub mod orchestrator;
pub mod proc;
pub mod pty;
pub mod skills;
pub mod store;
pub mod triggers;

pub use orchestrator::{Core, CoreConfig};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
