//! `codly mcp`: a minimal stdio MCP server (JSON-RPC 2.0, newline-delimited) that agent
//! CLIs launch. It only lists and calls tools; every call is forwarded to the running app
//! over the Unix socket in `CODLY_MCP_SOCK` with the token in `CODLY_MCP_TOKEN`.
//!
//! ponytail: hand-rolled instead of an MCP SDK — tools/list + tools/call is ~100 lines and
//! the SDKs churn. Add resources/prompts here if Codly ever needs them.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const FALLBACK_PROTOCOL: &str = "2025-06-18";

async fn call_app(sock: &str, token: &str, tool: &str, args: &Value) -> Result<Value, String> {
    let mut s = UnixStream::connect(sock)
        .await
        .map_err(|e| format!("Codly is not running or unreachable ({e})"))?;
    let req = json!({ "token": token, "tool": tool, "args": args });
    s.write_all(format!("{req}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(s).lines();
    let line = lines
        .next_line()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Codly closed the connection".to_string())?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply["result"].clone())
    } else {
        Err(reply["error"].as_str().unwrap_or("unknown error").to_string())
    }
}

async fn handle(sock: &str, token: &str, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?; // notifications get no reply
    let method = msg["method"].as_str().unwrap_or_default();
    let result: Result<Value, Value> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(FALLBACK_PROTOCOL),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "codly", "title": "Codly", "version": crate::version() },
            "instructions": "Codly coordinates this agent team. Use codly_get_task to see your assignment, \
                             codly_board for the team's board, codly_search to find code by concept, and the \
                             memory/knowledge tools to read and record what the team knows."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => call_app(sock, token, "__list", &json!({})).await.map(|tools| json!({ "tools": tools })).map_err(Value::String),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_app(sock, token, name, &args).await {
                Ok(v) => {
                    let text = if v.is_string() { v.as_str().unwrap().to_string() } else { serde_json::to_string_pretty(&v).unwrap_or_default() };
                    let mut r = json!({ "content": [{ "type": "text", "text": text }], "isError": false });
                    if v.is_object() {
                        r["structuredContent"] = v;
                    }
                    r
                }
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) if e.is_object() => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": e } }),
    })
}

/// Entry point for `codly mcp`. Runs until stdin closes.
pub async fn run_stdio() -> anyhow::Result<()> {
    let sock = std::env::var("CODLY_MCP_SOCK").map_err(|_| anyhow::anyhow!("CODLY_MCP_SOCK not set"))?;
    let token = std::env::var("CODLY_MCP_TOKEN").map_err(|_| anyhow::anyhow!("CODLY_MCP_TOKEN not set"))?;
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = stdin.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&sock, &token, &msg).await,
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            stdout.write_all(format!("{r}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
