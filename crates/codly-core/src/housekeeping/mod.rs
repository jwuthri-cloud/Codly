//! Housekeeping: keeping the board tidy and giving new agents sensible default tools.
//!
//! - `archive`      pure board-archiving policy + the searchable archive file
//! - `connections`  the default MCP server catalog, Keychain keys, and the spawn-time merge
//!
//! The Core glue (settings in `kv`, the sweep, Tauri-facing methods) is in
//! `orchestrator/housekeeping.rs`.

pub mod archive;
pub mod connections;
