//! Child-process plumbing shared by the adapters: process groups, a bounded NDJSON line
//! reader that never stops on a bad line, a stderr ring buffer, raw logs, and the
//! shutdown ladder. See docs/research/tauri-stack.md §5.2.
//!
//! CONTRACT (implemented by the adapters owner).
//!
//! Why a hand-written reader: `FramedRead` + `LinesCodec` yields `None` once after an
//! oversized or non-UTF-8 line, so a `while let Some` pump silently stops reading, the pipe
//! fills and the CLI blocks (CRITIQUE W1). `read_record` works on bytes, splits on `\n` only,
//! skips oversized records and keeps going.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Largest record kept (Claude `init` is ~25 KB, `commands_changed` 187 KB, Read results MBs).
pub const MAX_LINE: usize = 64 << 20;

pub enum Line {
    Complete,
    TooLong(usize),
    Eof,
}

/// Read one '\n'-terminated record into `buf` (newline stripped) without buffering > `max`.
/// An oversized record is consumed and reported as `TooLong(len)`; the next call starts at
/// the following record. A final record without a newline is returned as `Complete`.
pub async fn read_record<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, max: usize) -> std::io::Result<Line> {
    buf.clear();
    // Don't keep a 64 MiB allocation alive because of one huge record.
    if buf.capacity() > 8 << 20 {
        buf.shrink_to(256 << 10);
    }
    let (mut seen, mut overflow) = (0usize, false);
    loop {
        let avail = r.fill_buf().await?;
        if avail.is_empty() {
            return Ok(match (seen, overflow) {
                (0, _) => Line::Eof,
                (n, true) => Line::TooLong(n),
                _ => Line::Complete,
            });
        }
        let (len, used, done) = match memchr::memchr(b'\n', avail) {
            Some(i) => (i, i + 1, true),
            None => (avail.len(), avail.len(), false),
        };
        seen += len;
        if !overflow && buf.len() + len > max {
            overflow = true;
            buf.clear();
        }
        if !overflow {
            buf.extend_from_slice(&avail[..len]);
        }
        r.consume(used);
        if done {
            if !overflow && buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(if overflow { Line::TooLong(seen) } else { Line::Complete });
        }
    }
}

/// One item from [`spawn_reader`].
pub enum Record<'a> {
    Line(&'a [u8]),
    TooLong(usize),
    /// Stdout closed (or failed). Always the last record.
    Eof,
}

/// Drain `r` record by record on its own task, so the child never blocks on a full stdout
/// pipe no matter how slowly the adapter consumes. Empty lines are skipped.
pub fn spawn_reader<R, F>(r: R, mut on: F) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    F: FnMut(Record<'_>) + Send + 'static,
{
    tokio::spawn(async move {
        let mut r = BufReader::with_capacity(256 << 10, r);
        let mut buf = Vec::with_capacity(64 << 10);
        loop {
            match read_record(&mut r, &mut buf, MAX_LINE).await {
                Ok(Line::Complete) => {
                    if !buf.iter().all(u8::is_ascii_whitespace) {
                        on(Record::Line(&buf));
                    }
                }
                Ok(Line::TooLong(n)) => on(Record::TooLong(n)),
                Ok(Line::Eof) | Err(_) => {
                    on(Record::Eof);
                    return;
                }
            }
        }
    })
}

/// A parsed stdout record for adapters that own one process (Claude, ACP agents).
pub enum Inbound {
    Json(Value),
    /// A line that is not JSON (banners, warnings printed on stdout).
    Text(String),
    TooLong(usize),
    Eof,
}

/// [`spawn_reader`] + raw log + JSON parse: every line is appended to `raw` before parsing.
pub fn spawn_json_reader<R: AsyncRead + Unpin + Send + 'static>(r: R, raw: Arc<RawLog>) -> tokio::sync::mpsc::UnboundedReceiver<Inbound> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    spawn_reader(r, move |rec| {
        let item = match rec {
            Record::Line(b) => {
                raw.inbound(b);
                match serde_json::from_slice::<Value>(b) {
                    Ok(v) => Inbound::Json(v),
                    Err(_) => Inbound::Text(String::from_utf8_lossy(b).into_owned()),
                }
            }
            Record::TooLong(n) => {
                raw.note(&format!("dropped a {n}-byte record (over {MAX_LINE} bytes)"));
                Inbound::TooLong(n)
            }
            Record::Eof => Inbound::Eof,
        };
        let _ = tx.send(item);
    });
    rx
}

/// A tokio Command with env_clear().envs(env), cwd, piped stdio, own process group, kill_on_drop.
pub fn command(bin: &Path, args: &[String], cwd: &Path, env: &HashMap<String, String>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Own pgid: killpg() then also reaches the shells and test runners the agent started.
        .process_group(0)
        .kill_on_drop(true);
    cmd
}

/// SIGTERM the process group, wait up to `grace`, then SIGKILL.
///
/// Returns early once the group is gone. The caller must keep reaping the group leader
/// (await `Child::wait` on another task): an unreaped zombie still counts as a member.
pub async fn kill_group(pgid: i32, grace: Duration) {
    if pgid <= 1 {
        return; // 0/1 would signal our own group or init
    }
    let pg = Pid::from_raw(pgid);
    if killpg(pg, Signal::SIGTERM).is_err() {
        return; // ESRCH: already gone
    }
    let deadline = tokio::time::Instant::now() + grace;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if killpg(pg, None).is_err() {
            return;
        }
    }
    let _ = killpg(pg, Signal::SIGKILL);
}

/// Note a run's process group and when its leader started in `pgid`, next to the run's raw
/// log, so that the next start can find the group if Codly dies before the CLI does.
pub fn record_group(raw_log: &Path, pgid: i32, started_ms: i64) {
    if let Err(e) = std::fs::write(raw_log.with_file_name("pgid"), format!("{pgid} {started_ms}")) {
        tracing::warn!("cannot record the process group of a run: {e}");
    }
}

/// What [`record_group`] noted for the run with this raw log: (pgid, started_ms).
pub fn recorded_group(raw_log: &Path) -> Option<(i32, i64)> {
    let text = std::fs::read_to_string(raw_log.with_file_name("pgid")).ok()?;
    let (pgid, started_ms) = text.trim().split_once(' ')?;
    Some((pgid.parse().ok()?, started_ms.parse().ok()?))
}

/// Exit code of a finished child; signals map to 128 + signo like a shell reports them.
pub fn exit_code(status: std::process::ExitStatus) -> Option<i64> {
    use std::os::unix::process::ExitStatusExt;
    status.code().or_else(|| status.signal().map(|s| 128 + s)).map(i64::from)
}

/// Append-only log of every protocol line of one run (`runs/<run_id>/raw.ndjson`), written
/// before parsing so a run can be replayed and re-normalized after mapping fixes.
/// Inbound lines are stored verbatim; lines Codly wrote to the CLI are stored as
/// `{"codly_sent":<line>}` and Codly's own notes as `{"codly_note":"..."}`, so the file stays NDJSON.
/// The file is owner-only, and the keys in what Codly sends to a CLI are left out of it
/// (`write_json`).
pub struct RawLog {
    file: Mutex<File>,
}

const SENT_PREFIX: &[u8] = b"{\"codly_sent\":";

impl RawLog {
    pub fn open(path: &Path) -> std::io::Result<RawLog> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
        Ok(RawLog { file: Mutex::new(file) })
    }

    pub fn inbound(&self, line: &[u8]) {
        self.write(&[line, b"\n"]);
    }

    /// `line` must be one JSON value (it is embedded as-is).
    pub fn outbound(&self, line: &[u8]) {
        self.write(&[SENT_PREFIX, line, b"}\n"]);
    }

    pub fn note(&self, text: &str) {
        let line = serde_json::json!({ "codly_note": text }).to_string();
        self.write(&[line.as_bytes(), b"\n"]);
    }

    fn write(&self, parts: &[&[u8]]) {
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        for p in parts {
            if let Err(e) = f.write_all(p) {
                tracing::warn!("raw log write failed: {e}");
                return;
            }
        }
    }
}

/// Serialize `msg`, log it as outbound, and write it as one NDJSON line. The CLI gets the
/// message as it is; the log gets it without the keys in it (`without_mcp_secrets`).
pub async fn write_json<W: AsyncWrite + Unpin>(w: &mut W, raw: &RawLog, msg: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    match without_mcp_secrets(msg) {
        Some(safe) => raw.outbound(&serde_json::to_vec(&safe)?),
        None => raw.outbound(&line),
    }
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await
}

/// How long a CLI may take to read one line Codly writes to its stdin. A CLI that takes
/// longer has stopped reading (its pipe is full): it is hung.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// [`write_json`] with a deadline: `TimedOut` when the CLI does not take the line within
/// `limit`. A part of the line may be in the pipe by then, so the caller must give the
/// process up: nothing more can be written to it.
pub async fn write_json_within<W: AsyncWrite + Unpin>(w: &mut W, raw: &RawLog, msg: &Value, limit: Duration) -> std::io::Result<()> {
    match tokio::time::timeout(limit, write_json(w, raw, msg)).await {
        Ok(written) => written,
        Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("the CLI did not read its input within {} s", limit.as_secs()))),
    }
}

/// What the raw log shows in place of a key.
const REDACTED: &str = "***";

/// Where an outbound message lists the MCP servers of a session, with their environment: ACP
/// `session/new|load|resume` (a list of servers, env as `[{name, value}]`) and Codex
/// `thread/start|resume` (a map of servers, env as a map).
const MCP_SERVER_LISTS: [&str; 2] = ["/params/mcpServers", "/params/config/mcp_servers"];

/// A copy of `msg` for the raw log, with every MCP server's environment values replaced:
/// they hold this run's Codly token and the user's Connection keys (a GitHub token, a database
/// URL), which live in the Keychain and must not be copied to disk. None when `msg` lists no
/// MCP server (almost every message), so nothing is copied.
pub fn without_mcp_secrets(msg: &Value) -> Option<Value> {
    if MCP_SERVER_LISTS.iter().all(|at| msg.pointer(at).is_none()) {
        return None;
    }
    let mut safe = msg.clone();
    for at in MCP_SERVER_LISTS {
        let servers: Vec<&mut Value> = match safe.pointer_mut(at) {
            Some(Value::Array(list)) => list.iter_mut().collect(),
            Some(Value::Object(map)) => map.values_mut().collect(),
            _ => continue,
        };
        for server in servers {
            match server.get_mut("env") {
                Some(Value::Object(env)) => env.values_mut().for_each(|v| *v = REDACTED.into()),
                Some(Value::Array(env)) => env.iter_mut().filter_map(|e| e.get_mut("value")).for_each(|v| *v = REDACTED.into()),
                _ => {}
            }
        }
    }
    Some(safe)
}

/// The last `cap` bytes a child wrote to stderr. The drain task keeps reading forever, so a
/// chatty child (Codex logs every MCP failure there) never blocks on a full stderr pipe.
#[derive(Clone, Default)]
pub struct StderrTail(Arc<Mutex<VecDeque<u8>>>);

impl StderrTail {
    pub fn spawn<R: AsyncRead + Unpin + Send + 'static>(mut r: R, cap: usize) -> StderrTail {
        let tail = StderrTail::default();
        let ring = tail.0.clone();
        tokio::spawn(async move {
            let mut chunk = vec![0u8; 8 << 10];
            loop {
                match r.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        let mut q = ring.lock().unwrap_or_else(|e| e.into_inner());
                        q.extend(&chunk[..n]);
                        let excess = q.len().saturating_sub(cap);
                        q.drain(..excess);
                    }
                }
            }
        });
        tail
    }

    /// Up to the last `max` bytes, lossy UTF-8, trimmed.
    pub fn tail(&self, max: usize) -> String {
        let q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let skip = q.len().saturating_sub(max);
        let bytes: Vec<u8> = q.iter().skip(skip).copied().collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn collect(input: Vec<u8>, max: usize) -> Vec<Result<Vec<u8>, usize>> {
        let mut r = BufReader::with_capacity(4096, &input[..]);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        loop {
            match read_record(&mut r, &mut buf, max).await.unwrap() {
                Line::Complete => out.push(Ok(buf.clone())),
                Line::TooLong(n) => out.push(Err(n)),
                Line::Eof => return out,
            }
        }
    }

    #[tokio::test]
    async fn splits_on_newline_only_and_strips_cr() {
        let out = collect(b"a\r\n{\"x\":\"1\xe2\x80\xa82\"}\nlast".to_vec(), 1 << 20).await;
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].as_ref().unwrap(), b"a");
        // U+2028 inside a record is not a separator
        assert_eq!(out[1].as_ref().unwrap(), "{\"x\":\"1\u{2028}2\"}".as_bytes());
        assert_eq!(out[2].as_ref().unwrap(), b"last");
    }

    #[tokio::test]
    async fn invalid_utf8_does_not_stop_reading() {
        let out = collect(b"\xff\xfe garbage\n{\"ok\":1}\n".to_vec(), 1 << 20).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_ref().unwrap(), b"\xff\xfe garbage");
        assert_eq!(out[1].as_ref().unwrap(), b"{\"ok\":1}");
    }

    #[tokio::test]
    async fn oversized_record_is_skipped_and_reading_continues() {
        let mut input = vec![b'x'; 5000];
        input.extend_from_slice(b"\nnext\n");
        let out = collect(input, 1000).await;
        assert_eq!(out, vec![Err(5000), Ok(b"next".to_vec())]);
    }

    /// CRITIQUE W1: a 70 MiB line through a real pipe must be skipped without buffering it,
    /// and the records after it must still arrive.
    #[tokio::test]
    async fn seventy_mib_line_through_a_pipe() {
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let writer = tokio::spawn(async move {
            w.write_all(b"{\"first\":true}\n").await.unwrap();
            let chunk = vec![b'a'; 1 << 20];
            for _ in 0..70 {
                w.write_all(&chunk).await.unwrap();
            }
            w.write_all(b"\n\xc3\x28 bad utf8\n{\"after\":true}\n").await.unwrap();
        });
        let mut got = Vec::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reader(r, move |rec| {
            let _ = tx.send(match rec {
                Record::Line(b) => Ok(b.to_vec()),
                Record::TooLong(n) => Err(n),
                Record::Eof => Err(0),
            });
        });
        writer.await.unwrap();
        while let Some(item) = rx.recv().await {
            if item == Err(0) {
                break;
            }
            got.push(item);
        }
        assert_eq!(got.len(), 4, "{:?}", got.iter().map(|g| g.as_ref().map(|b| b.len())).collect::<Vec<_>>());
        assert_eq!(got[0].as_ref().unwrap(), b"{\"first\":true}");
        assert_eq!(got[1], Err(70 << 20));
        assert_eq!(got[2].as_ref().unwrap(), b"\xc3\x28 bad utf8");
        assert_eq!(got[3].as_ref().unwrap(), b"{\"after\":true}");
    }

    #[test]
    fn raw_log_wraps_outbound_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs/r1/raw.ndjson");
        let log = RawLog::open(&path).unwrap();
        log.inbound(b"{\"type\":\"system\"}");
        log.outbound(b"{\"type\":\"user\"}");
        log.note("dropped 5 bytes");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["type"], "system");
        assert_eq!(lines[1]["codly_sent"]["type"], "user");
        assert_eq!(lines[2]["codly_note"], "dropped 5 bytes");
    }

    /// A CLI that stopped reading its stdin: the write gives up at the deadline instead of
    /// waiting for ever, and a write that is read goes through.
    #[tokio::test]
    async fn a_write_nobody_reads_gives_up_at_its_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let log = RawLog::open(&dir.path().join("raw.ndjson")).unwrap();
        // 16 bytes of pipe, and nobody reads the other end.
        let (mut w, _unread) = tokio::io::duplex(16);
        let msg = serde_json::json!({ "type": "user", "text": "x".repeat(4096) });
        let started = std::time::Instant::now();
        let e = write_json_within(&mut w, &log, &msg, Duration::from_millis(100)).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut, "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let (mut w, mut r) = tokio::io::duplex(16);
        let reader = tokio::spawn(async move {
            let mut all = Vec::new();
            r.read_to_end(&mut all).await.unwrap();
            all
        });
        write_json_within(&mut w, &log, &msg, Duration::from_secs(5)).await.unwrap();
        drop(w);
        let line = reader.await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&line).unwrap(), msg);
    }

    /// The spec of a run whose codly server carries a token and nothing else.
    fn spec_with_token(provider: crate::model::Provider, token: &str) -> crate::adapters::SpawnSpec {
        use crate::adapters::{McpLaunch, SessionMode, SpawnSpec};
        SpawnSpec {
            agent_id: "a".into(),
            run_id: "r".into(),
            provider,
            role: crate::model::Role::Worker,
            model: None,
            cwd: "/w".into(),
            bin: "cli".into(),
            env: HashMap::new(),
            session: SessionMode::New { session_id: None },
            session_cost_usd: 0.0,
            system_prompt: String::new(),
            read_only: false,
            mcp: Some(McpLaunch { command: "/x/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), token.into())] }),
            use_user_setup: false,
            local_settings: false,
            raw_log: "/nonexistent/r/raw.ndjson".into(),
            byok_command: None,
        }
    }

    /// What each adapter really sends at session start carries the keys; the raw log must not.
    #[tokio::test]
    async fn keys_reach_the_cli_but_not_the_raw_log() {
        use crate::model::Provider;
        const TOKEN: &str = "tok-5f1e2d3c4b5a";
        const PAT: &str = "ghp_connectionSecret0123456789";
        // ACP `session/new`, with a Connection server beside codly's (as `acp::mcp_servers` builds them).
        let mut servers = crate::adapters::acp::mcp_servers(&spec_with_token(Provider::Gemini, TOKEN));
        servers.as_array_mut().unwrap().push(serde_json::json!({
            "name": "github", "command": "npx", "args": ["-y", "server-github"],
            "env": [{ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "value": PAT }],
        }));
        let acp = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "session/new", "params": { "cwd": "/w", "mcpServers": servers } });
        // Codex `thread/start`, the same way (as `codex::thread_request` builds it).
        let (method, mut params) = crate::adapters::codex::thread_request(&spec_with_token(Provider::Codex, TOKEN), None, None);
        params["config"]["mcp_servers"]["github"] = serde_json::json!({ "command": "npx", "args": [], "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": PAT } });
        let codex = serde_json::json!({ "id": 2, "method": method, "params": params });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs/r1/raw.ndjson");
        let log = RawLog::open(&path).unwrap();
        let mut sent = Vec::new();
        for msg in [&acp, &codex] {
            assert!(msg.to_string().contains(TOKEN) && msg.to_string().contains(PAT), "the test message holds both keys: {msg}");
            write_json(&mut sent, &log, msg).await.unwrap();
        }
        // The CLI got every byte.
        let sent = String::from_utf8(sent).unwrap();
        let lines: Vec<Value> = sent.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines, [acp.clone(), codex.clone()]);
        // The log has the same messages with the values hidden, and nothing else changed.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(TOKEN) && !text.contains(PAT), "{text}");
        let logged: Vec<Value> = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["codly_sent"].clone()).collect();
        assert_eq!(logged[0]["params"]["mcpServers"][0]["env"], serde_json::json!([{ "name": "CODLY_MCP_TOKEN", "value": "***" }]));
        assert_eq!(logged[0]["params"]["mcpServers"][1]["env"][0], serde_json::json!({ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "value": "***" }));
        assert_eq!(logged[0]["params"]["mcpServers"][1]["args"], acp["params"]["mcpServers"][1]["args"]);
        assert_eq!(logged[1]["params"]["config"]["mcp_servers"]["codly"]["env"], serde_json::json!({ "CODLY_MCP_TOKEN": "***" }));
        assert_eq!(logged[1]["params"]["config"]["mcp_servers"]["github"]["env"], serde_json::json!({ "GITHUB_PERSONAL_ACCESS_TOKEN": "***" }));
        assert_eq!(logged[1]["params"]["cwd"], codex["params"]["cwd"]);
        // A message without servers is logged as it is, with no copy made.
        let prompt = serde_json::json!({ "id": 3, "method": "session/prompt", "params": { "prompt": [{ "type": "text", "text": "env: {}" }] } });
        assert_eq!(without_mcp_secrets(&prompt), None);
        // Only the owner reads the log.
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn stderr_tail_keeps_the_end() {
        let (mut w, r) = tokio::io::duplex(1024);
        let tail = StderrTail::spawn(r, 10);
        w.write_all(b"0123456789abcdef").await.unwrap();
        drop(w);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(tail.tail(100), "6789abcdef");
        assert_eq!(tail.tail(3), "def");
    }

    #[tokio::test]
    async fn kill_group_terminates_children() {
        let env: HashMap<String, String> = [("PATH".to_string(), "/usr/bin:/bin".to_string())].into();
        let mut child = command(Path::new("/bin/sh"), &["-c".into(), "sleep 30 & sleep 30".into()], Path::new("/"), &env)
            .spawn()
            .unwrap();
        let pgid = child.id().unwrap() as i32;
        let waiter = tokio::spawn(async move { child.wait().await.unwrap() });
        let started = std::time::Instant::now();
        kill_group(pgid, Duration::from_secs(2)).await;
        let status = waiter.await.unwrap();
        assert_eq!(exit_code(status), Some(128 + 15));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(killpg(Pid::from_raw(pgid), None).is_err(), "group still alive");
    }
}
