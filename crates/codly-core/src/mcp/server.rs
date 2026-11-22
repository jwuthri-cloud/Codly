//! In-app side of the MCP bridge: a Unix-socket listener (mode 0600) that maps per-run
//! tokens to callers and dispatches tool calls to the orchestrator.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::{ToolCtx, ToolHandler, tools};
use crate::adapters::McpLaunch;

pub struct McpServer {
    sock_path: PathBuf,
    tokens: Mutex<HashMap<String, ToolCtx>>,
    /// How long a call to a tool may take before its caller is answered anyway.
    time_limit: fn(&str) -> Duration,
}

/// macOS caps `sun_path` at 104 bytes; fall back to /tmp for long data dirs. /tmp is shared:
/// if that name is taken by anything but a socket of ours (another user's file, a symlink),
/// use a fresh random name instead of failing or reusing it.
fn socket_path(preferred: &Path) -> PathBuf {
    if preferred.as_os_str().len() < 100 {
        return preferred.to_path_buf();
    }
    unique_if_taken(PathBuf::from(format!("/tmp/codly-{}.sock", nix::unistd::getuid())))
}

fn unique_if_taken(p: PathBuf) -> PathBuf {
    match std::fs::symlink_metadata(&p) {
        Ok(meta) if !ours(&meta) => {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            p.with_file_name(format!("{stem}-{}.sock", &uuid::Uuid::new_v4().simple().to_string()[..12]))
        }
        _ => p,
    }
}

/// A socket owned by this user (a stale one from a crashed run, safe to replace).
fn ours(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    meta.file_type().is_socket() && meta.uid() == nix::unistd::getuid().as_raw()
}

impl McpServer {
    pub async fn start(preferred_sock: PathBuf, handler: ToolHandler) -> anyhow::Result<Arc<Self>> {
        Self::start_with(preferred_sock, handler, tools::time_limit).await
    }

    /// `start` with the time limit of each tool given (tests: a tool that never answers).
    async fn start_with(preferred_sock: PathBuf, handler: ToolHandler, time_limit: fn(&str) -> Duration) -> anyhow::Result<Arc<Self>> {
        let sock_path = socket_path(&preferred_sock);
        // A stale socket from a crashed run would make bind fail. Only remove our own sockets,
        // never files.
        if let Ok(meta) = std::fs::symlink_metadata(&sock_path)
            && ours(&meta)
        {
            std::fs::remove_file(&sock_path)?;
        }
        if let Some(dir) = sock_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let listener = UnixListener::bind(&sock_path)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))?;
        }
        let server = Arc::new(McpServer { sock_path, tokens: Mutex::new(HashMap::new()), time_limit });
        let accept = server.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let srv = accept.clone();
                        let h = handler.clone();
                        tokio::spawn(async move {
                            if let Err(e) = srv.serve_conn(stream, h).await {
                                tracing::debug!("mcp conn ended: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("mcp accept failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
        });
        Ok(server)
    }

    pub fn sock_path(&self) -> &Path {
        &self.sock_path
    }

    /// Issue a token for one agent run. Revoke it when the run ends.
    pub fn register(&self, ctx: ToolCtx) -> String {
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        self.tokens.lock().unwrap().insert(token.clone(), ctx);
        token
    }

    pub fn revoke(&self, token: &str) {
        self.tokens.lock().unwrap().remove(token);
    }

    /// How an agent CLI should launch the stdio bridge for this token.
    pub fn launch(&self, exe: &Path, token: &str) -> McpLaunch {
        McpLaunch {
            command: exe.to_string_lossy().into_owned(),
            args: vec!["mcp".into()],
            env: vec![
                ("CODLY_MCP_SOCK".into(), self.sock_path.to_string_lossy().into_owned()),
                ("CODLY_MCP_TOKEN".into(), token.into()),
            ],
        }
    }

    async fn serve_conn(&self, stream: UnixStream, handler: ToolHandler) -> anyhow::Result<()> {
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        while let Some(line) = lines.next_line().await? {
            let reply = self.handle_line(&line, &handler).await;
            let mut out = serde_json::to_vec(&reply)?;
            out.push(b'\n');
            w.write_all(&out).await?;
        }
        Ok(())
    }

    async fn handle_line(&self, line: &str, handler: &ToolHandler) -> Value {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return json!({ "ok": false, "error": format!("bad request: {e}") }),
        };
        let token = req["token"].as_str().unwrap_or_default();
        let Some(ctx) = self.tokens.lock().unwrap().get(token).cloned() else {
            return json!({ "ok": false, "error": "unknown or expired Codly token" });
        };
        let tool = req["tool"].as_str().unwrap_or_default().to_string();
        if tool == "__list" {
            return json!({ "ok": true, "result": tools::tool_specs(ctx.role) });
        }
        let allowed = tools::tool_specs(ctx.role).iter().any(|t| t["name"] == tool.as_str());
        if !allowed {
            return json!({ "ok": false, "error": format!("tool {tool} is not available to this agent") });
        }
        let args = req.get("args").cloned().unwrap_or(json!({}));
        let limit = (self.time_limit)(&tool);
        // The call runs as a task of its own: when the limit passes, the caller is answered
        // and the call still runs to its end. Cutting it in the middle could leave half of
        // its work behind (a plan without its tasks).
        let call = tokio::spawn(handler(ctx, tool.clone(), args));
        match tokio::time::timeout(limit, call).await {
            Ok(Ok(Ok(result))) => json!({ "ok": true, "result": result }),
            Ok(Ok(Err(error))) => json!({ "ok": false, "error": error }),
            Ok(Err(e)) => {
                tracing::error!("the MCP tool {tool} stopped: {e}");
                json!({ "ok": false, "error": format!("{tool} failed inside Codly; read codly_board before you try it again") })
            }
            Err(_) => {
                tracing::warn!("the MCP tool {tool} did not answer within {limit:?}");
                json!({ "ok": false, "error": tools::slow_answer(limit) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Role;

    #[tokio::test]
    async fn a_taken_socket_path_gets_a_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        let free = dir.path().join("codly-1.sock");
        assert_eq!(unique_if_taken(free.clone()), free);
        let _l = UnixListener::bind(&free).unwrap();
        assert_eq!(unique_if_taken(free.clone()), free, "our own stale socket is replaced");
        // Not a socket of ours (a file, a symlink, another user's socket): never reused.
        let taken = dir.path().join("codly-2.sock");
        std::fs::write(&taken, "x").unwrap();
        let other = unique_if_taken(taken.clone());
        assert_ne!(other, taken);
        assert!(other.file_name().unwrap().to_string_lossy().starts_with("codly-2-") && other.parent() == taken.parent());
        let link = dir.path().join("codly-3.sock");
        std::os::unix::fs::symlink(&free, &link).unwrap();
        assert_ne!(unique_if_taken(link.clone()), link);
    }

    #[tokio::test]
    async fn token_gates_and_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let handler: ToolHandler = Arc::new(|ctx, tool, args| {
            Box::pin(async move { Ok(json!({ "agent": ctx.agent_id, "tool": tool, "args": args })) })
        });
        let srv = McpServer::start(dir.path().join("t.sock"), handler).await.unwrap();
        let token = srv.register(ToolCtx { agent_id: "a1".into(), run_id: "r1".into(), task_id: None, role: Role::Worker });

        let call = |body: Value| {
            let path = srv.sock_path().to_path_buf();
            async move {
                let mut s = UnixStream::connect(path).await.unwrap();
                s.write_all(format!("{body}\n").as_bytes()).await.unwrap();
                let mut lines = BufReader::new(s).lines();
                serde_json::from_str::<Value>(&lines.next_line().await.unwrap().unwrap()).unwrap()
            }
        };
        let ok = call(json!({ "token": token, "tool": "codly_board", "args": {} })).await;
        assert_eq!(ok["result"]["agent"], "a1");
        let denied = call(json!({ "token": token, "tool": "codly_submit_plan", "args": {} })).await;
        assert_eq!(denied["ok"], false);
        let bad = call(json!({ "token": "nope", "tool": "codly_board" })).await;
        assert_eq!(bad["ok"], false);
        let list = call(json!({ "token": token, "tool": "__list" })).await;
        assert!(list["result"].as_array().unwrap().len() >= 9);
    }

    #[tokio::test]
    async fn a_tool_that_never_answers_does_not_hang_its_caller() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let handler: ToolHandler = Arc::new(move |_ctx, tool, _args| {
            let done = done.clone();
            Box::pin(async move {
                match tool.as_str() {
                    "codly_board" => std::future::pending().await,
                    // Slower than the limit, but it ends.
                    "codly_post_note" => {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        done.store(true, Ordering::SeqCst);
                        Ok(json!("noted"))
                    }
                    _ => Ok(json!("here")),
                }
            })
        });
        let srv = McpServer::start_with(dir.path().join("t.sock"), handler, |_| Duration::from_millis(50)).await.unwrap();
        let token = srv.register(ToolCtx { agent_id: "a1".into(), run_id: "r1".into(), task_id: None, role: Role::Worker });

        // One connection, as the bridge of one agent uses it, call after call.
        let stream = UnixStream::connect(srv.sock_path()).await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        let mut answers = vec![];
        for tool in ["codly_board", "codly_get_task", "codly_post_note"] {
            w.write_all(format!("{}\n", json!({ "token": token, "tool": tool, "args": {} })).as_bytes()).await.unwrap();
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await.expect("the server answers").unwrap().unwrap();
            answers.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        let slow = json!({ "ok": false, "error": tools::slow_answer(Duration::from_millis(50)) });
        assert_eq!(answers[0], slow, "the caller of a tool that hangs is answered");
        assert_eq!(answers[1], json!({ "ok": true, "result": "here" }), "the same connection serves the next call");
        // A call that is only slow is not cut: its work is done after its caller was answered.
        assert_eq!(answers[2], slow);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(finished.load(Ordering::SeqCst), "the slow call ran to its end");
    }
}
