//! Housekeeping: keeping the board tidy and giving new agents sensible default tools.
//!
//! - `archive`      pure board-archiving policy + the searchable archive file
//! - `connections`  the default MCP server catalog, Keychain keys, and the spawn-time merge
//! - `storage`      what Codly leaves behind (worktrees, `codly/…` branches, run logs) and what
//!   may be removed
//!
//! The Core glue (settings in `kv`, the sweep, Tauri-facing methods) is in
//! `orchestrator/housekeeping.rs` and `orchestrator/housekeeping/storage.rs`.

pub mod archive;
pub mod connections;
pub mod storage;
