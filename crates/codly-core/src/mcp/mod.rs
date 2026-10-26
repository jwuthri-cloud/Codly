//! Codly's MCP server.
//!
//! Agent CLIs launch `<codly exe> mcp` as a stdio MCP server (configured per run via
//! `--mcp-config`, `-c mcp_servers.codly.*` or ACP `mcpServers`). That bridge process
//! speaks MCP JSON-RPC on stdio and forwards `tools/call` over a Unix socket to the
//! running app, authenticated by a per-run token (env `CODLY_MCP_TOKEN`). The app
//! resolves the token to the calling agent/run/task and dispatches to the orchestrator.
//!
//! Wire format on the socket: one JSON line per request
//! `{"token": "...", "tool": "codly_board", "args": {...}}` and one JSON line back
//! `{"ok": true, "result": <json>}` or `{"ok": false, "error": "..."}`. `tool: "__list"`
//! returns the role-filtered tool specs.

pub mod bridge;
pub mod server;
pub mod tools;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::model::{Id, Role};

/// Identity of the agent calling a tool (resolved from its token).
#[derive(Debug, Clone)]
pub struct ToolCtx {
    pub agent_id: Id,
    pub run_id: Id,
    pub task_id: Option<Id>,
    pub role: Role,
}

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
/// Implemented by the orchestrator: (caller, tool name, arguments) → result JSON.
pub type ToolHandler = Arc<dyn Fn(ToolCtx, String, Value) -> ToolFuture + Send + Sync>;

pub use server::McpServer;
