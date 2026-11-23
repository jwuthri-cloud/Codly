//! `codly mcp`: a minimal stdio MCP server (JSON-RPC 2.0, newline-delimited) that agent
//! CLIs launch. It only lists and calls tools; every call is forwarded to the running app
//! over the Unix socket in `CODLY_MCP_SOCK` with the token in `CODLY_MCP_TOKEN`.
//!
//! ponytail: hand-rolled instead of an MCP SDK — tools/list + tools/call is ~100 lines and
//! the SDKs churn. Add resources/prompts here if Codly ever needs them.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::request::{self, Line, MAX_LINE};
use super::tools;

const FALLBACK_PROTOCOL: &str = "2025-06-18";

/// Added to the app's own limit for a call (`tools::time_limit`), so the app's answer comes
/// first. The bridge's limit is for an app that does not answer at all.
const GRACE: Duration = Duration::from_secs(5);

/// How long the bridge waits for the app on one call: to connect, send and get the answer.
fn time_limit(tool: &str) -> Duration {
    tools::time_limit(tool) + GRACE
}

/// One call to the app, given up after `limit`: the agent CLI waits for the bridge's answer,
/// so a bridge that waits for ever hangs the agent's turn.
async fn call_app(sock: &str, token: &str, tool: &str, args: &Value, limit: Duration) -> Result<Value, String> {
    match tokio::time::timeout(limit, exchange(sock, token, tool, args)).await {
        Ok(answer) => answer,
        Err(_) => Err(tools::slow_answer(limit)),
    }
}

async fn exchange(sock: &str, token: &str, tool: &str, args: &Value) -> Result<Value, String> {
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
        "tools/list" => call_app(sock, token, "__list", &json!({}), time_limit("__list")).await.map(|tools| json!({ "tools": tools })).map_err(Value::String),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let answer = match request::arguments(msg["params"].get("arguments")) {
                Ok(args) => call_app(sock, token, name, &args, time_limit(name)).await,
                Err(refused) => Err(refused),
            };
            Ok(match answer {
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
    serve(&sock, &token, BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

/// Answer the JSON-RPC messages on `input`, one per line, until it ends: one at a time, in
/// the order they came.
async fn serve<R, W>(sock: &str, token: &str, mut input: R, mut output: W) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    while let Some(line) = request::read_line(&mut input, MAX_LINE).await? {
        let reply = match line {
            Line::Bytes(line) if line.iter().all(u8::is_ascii_whitespace) => continue,
            Line::Bytes(line) => match serde_json::from_slice::<Value>(&line) {
                Ok(msg) => handle(sock, token, &msg).await,
                Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
            },
            // The id of the request went with the line.
            Line::TooLong => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": request::too_long() } })),
        };
        if let Some(r) = reply {
            output.write_all(format!("{r}\n").as_bytes()).await?;
            output.flush().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::net::UnixListener;

    use super::*;
    use crate::mcp::{McpServer, ToolCtx, ToolHandler};
    use crate::model::Role;

    fn worker() -> ToolCtx {
        ToolCtx { agent_id: "a1".into(), run_id: "r1".into(), task_id: None, role: Role::Worker }
    }

    /// The app's side with one worker signed in: `codly_get_task` answers with text, every
    /// other tool with what it was called with. Returns the folder of the socket (keep it),
    /// the server and the worker's token.
    async fn app() -> (tempfile::TempDir, Arc<McpServer>, String) {
        let dir = tempfile::tempdir().unwrap();
        let handler: ToolHandler = Arc::new(|ctx, tool, args| {
            Box::pin(async move {
                match tool.as_str() {
                    tools::GET_TASK => Ok(json!("You have no task.")),
                    _ => Ok(json!({ "agent": ctx.agent_id, "tool": tool, "args": args })),
                }
            })
        });
        let srv = McpServer::start(dir.path().join("app.sock"), handler).await.unwrap();
        let token = srv.register(worker());
        (dir, srv, token)
    }

    /// What the bridge writes to its output when `lines` arrive on its input, with `token` in
    /// its environment: one JSON-RPC message per line.
    async fn answers(srv: &McpServer, token: &str, lines: &[String]) -> Vec<Value> {
        let input = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        let mut output: Vec<u8> = vec![];
        let served = serve(srv.sock_path().to_str().unwrap(), token, input.as_bytes(), &mut output);
        tokio::time::timeout(Duration::from_secs(30), served).await.expect("the bridge ends with its input").unwrap();
        String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn ask(id: u64, method: &str, params: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
    }

    fn call(id: u64, tool: &str, arguments: Value) -> String {
        ask(id, "tools/call", json!({ "name": tool, "arguments": arguments }))
    }

    #[tokio::test]
    async fn the_bridge_speaks_json_rpc_for_the_tools_of_the_app() {
        let (_dir, srv, token) = app().await;
        let lines = [
            ask(1, "initialize", json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } })),
            // A notification has no id and gets no answer; neither does an empty line.
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            String::new(),
            ask(2, "ping", json!({})),
            ask(3, "tools/list", json!({})),
            call(4, tools::BOARD, json!({ "plan": "p1" })),
            call(5, tools::GET_TASK, json!({})),
            // A call without arguments.
            ask(6, "tools/call", json!({ "name": tools::BOARD })),
            // Not a worker's tool: the app refuses it, and the agent reads why.
            call(7, tools::SUBMIT_PLAN, json!({})),
            ask(8, "resources/list", json!({})),
        ];
        let got = answers(&srv, &token, &lines).await;
        assert_eq!(got.len(), 8, "one answer per request with an id: {got:?}");
        assert!(got.iter().all(|a| a["jsonrpc"] == "2.0"), "{got:?}");
        assert_eq!(got.iter().map(|a| a["id"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8], "in the order they were asked");

        let init = &got[0]["result"];
        assert_eq!((&init["protocolVersion"], &init["serverInfo"]["name"]), (&json!("2025-03-26"), &json!("codly")), "the client's protocol version is taken");
        assert!(init["capabilities"]["tools"].is_object(), "{init}");
        assert_eq!(got[1]["result"], json!({}));

        // The list is the caller's own: a worker has the board and no plan to submit.
        let listed = got[2]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&tools::BOARD) && !names.contains(&tools::SUBMIT_PLAN), "{names:?}");
        assert!(listed.iter().all(|t| t["inputSchema"].is_object()), "each tool says what it takes");

        // An object comes back as text and as structured content; text comes back as it is.
        let board = json!({ "agent": "a1", "tool": tools::BOARD, "args": { "plan": "p1" } });
        assert_eq!((&got[3]["result"]["isError"], &got[3]["result"]["structuredContent"]), (&json!(false), &board));
        assert_eq!(serde_json::from_str::<Value>(got[3]["result"]["content"][0]["text"].as_str().unwrap()).unwrap(), board);
        assert_eq!(got[4]["result"], json!({ "content": [{ "type": "text", "text": "You have no task." }], "isError": false }));
        assert_eq!(got[5]["result"]["structuredContent"]["args"], json!({}));

        // A refusal of the app is a tool error with the app's words, not a protocol error.
        let refusal = format!("tool {} is not available to this agent", tools::SUBMIT_PLAN);
        assert_eq!(got[6]["result"], json!({ "content": [{ "type": "text", "text": refusal }], "isError": true }));
        assert!(got[6].get("error").is_none());
        assert_eq!(got[7]["error"]["code"], -32601);
        assert!(got[7].get("result").is_none());
    }

    #[tokio::test]
    async fn a_request_the_bridge_cannot_take_is_answered_and_the_next_one_is_served() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let handler: ToolHandler = Arc::new(move |_ctx, _tool, _args| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(json!("done")) })
        });
        let srv = McpServer::start(dir.path().join("app.sock"), handler).await.unwrap();
        let token = srv.register(worker());
        let lines = [
            "{ not json".to_string(),
            call(1, tools::BOARD, json!({ "text": "x".repeat(MAX_LINE) })),
            call(2, tools::BOARD, json!("all of it")),
            call(3, tools::BOARD, json!(["all of it"])),
            ask(4, "ping", json!({})),
        ];
        let got = answers(&srv, &token, &lines).await;
        assert_eq!(got.len(), 5, "{got:?}");
        assert_eq!((&got[0]["id"], &got[0]["error"]["code"]), (&Value::Null, &json!(-32700)));
        assert_eq!(got[1], json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": request::too_long() } }));
        for (at, id) in [(2, 2), (3, 3)] {
            assert_eq!((&got[at]["id"], &got[at]["result"]["isError"]), (&json!(id), &json!(true)));
            assert!(got[at]["result"]["content"][0]["text"].as_str().unwrap().contains("must be a JSON object"), "{}", got[at]);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0, "none of them reached a tool of the app");
        assert_eq!(got[4], json!({ "jsonrpc": "2.0", "id": 4, "result": {} }));
    }

    #[tokio::test]
    async fn a_token_the_app_does_not_know_gets_no_tools() {
        let (_dir, srv, token) = app().await;
        let lines = [ask(1, "tools/list", json!({})), call(2, tools::BOARD, json!({}))];
        let unknown = answers(&srv, "not-a-token", &lines).await;
        assert_eq!(unknown[0], json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "unknown or expired Codly token" } }));
        assert_eq!(unknown[1]["result"], json!({ "content": [{ "type": "text", "text": "unknown or expired Codly token" }], "isError": true }));
        // The run ends: its token is revoked, and its bridge is refused from then on.
        assert_eq!(answers(&srv, &token, &lines).await[1]["result"]["isError"], false);
        srv.revoke(&token);
        assert_eq!(answers(&srv, &token, &lines).await[1]["result"]["isError"], true);
    }

    #[tokio::test]
    async fn an_app_that_does_not_answer_does_not_hang_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("app.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        // The app takes the connection and the request, and never answers.
        let silent = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(stream);
        });
        let limit = Duration::from_millis(50);
        let answer = tokio::time::timeout(Duration::from_secs(5), call_app(sock.to_str().unwrap(), "token", tools::BOARD, &json!({}), limit)).await;
        assert_eq!(answer.expect("the bridge gives up by itself"), Err(tools::slow_answer(limit)));
        silent.abort();
    }

    #[tokio::test]
    async fn the_bridge_waits_longer_than_the_app_and_says_when_there_is_no_app() {
        for tool in [tools::BOARD, tools::SUBMIT_PLAN, "__list"] {
            assert_eq!(time_limit(tool), tools::time_limit(tool) + Duration::from_secs(5));
        }
        // No app at the socket: said at once, as a tool error the agent can read.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.sock");
        let msg = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": tools::BOARD, "arguments": {} } });
        let reply = handle(missing.to_str().unwrap(), "token", &msg).await.unwrap();
        assert_eq!((&reply["id"], &reply["result"]["isError"]), (&json!(7), &json!(true)));
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().starts_with("Codly is not running or unreachable"), "{reply}");
    }
}
