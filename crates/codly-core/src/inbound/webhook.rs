//! The webhook listener: `POST /hook/<endpoint id>` with the endpoint's secret in an
//! `Authorization: Bearer …` (or `X-Codly-Secret`) header and a text or JSON body.
//!
//! Hardening, in the order a request meets it:
//! - bound to 127.0.0.1 only (reach it from elsewhere through a tunnel you run yourself);
//! - at most 32 connections at once, 10 s to send the request, so a slow client cannot pile up;
//! - only `POST`, and a browser preflight (`OPTIONS`) is refused with no CORS headers, so a web
//!   page cannot post here: its custom headers would need one;
//! - the secret is checked before the body is read, in constant time; an unknown endpoint, a
//!   wrong secret and an empty one all get the same 401; secrets are never in the URL or the logs;
//! - a switched-off endpoint answers 403 only to someone who holds its secret;
//! - body capped at 64 KB (413), no chunked uploads, 30 accepted messages a minute per endpoint.
//!
//! This hand-rolled HTTP/1.1 does one thing (one request per connection, `Connection: close`).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use super::secrets::{Secret, ct_eq};
use super::{Delivery, InboundMessage, InboundSource, Sink, SourceStatus};

pub const MAX_BODY: usize = 64 * 1024;
const MAX_HEAD: usize = 8 * 1024;
const MAX_CONNS: usize = 32;
const PER_MINUTE: usize = 30;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const LOG_LEN: usize = 50;

// ───────────────────────────── the endpoint table ─────────────────────────────

struct Entry {
    name: String,
    enabled: bool,
    secret: Secret,
    hits: VecDeque<Instant>,
    received: i64,
    last_at: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Accepted { name: String },
    /// Unknown endpoint, wrong secret or no secret: indistinguishable on purpose.
    Denied,
    /// Right secret, but the endpoint is switched off.
    Off,
    Limited,
}

/// The live endpoints, shared by the hub (which edits them) and the listener (which reads them
/// on every request, so a revoke or a switch-off takes effect at once).
#[derive(Default)]
pub struct Table {
    entries: Mutex<HashMap<String, Entry>>,
    log: Mutex<VecDeque<Delivery>>,
}

impl Table {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn put(&self, id: &str, name: &str, enabled: bool, secret: Secret) {
        self.entries().insert(id.into(), Entry { name: name.into(), enabled, secret, hits: VecDeque::new(), received: 0, last_at: None });
    }
    pub fn remove(&self, id: &str) {
        self.entries().remove(id);
    }
    pub fn has(&self, id: &str) -> bool {
        self.entries().contains_key(id)
    }
    pub fn set_enabled(&self, id: &str, on: bool) {
        if let Some(e) = self.entries().get_mut(id) {
            e.enabled = on;
        }
    }
    pub fn any_enabled(&self) -> bool {
        self.entries().values().any(|e| e.enabled)
    }
    /// (accepted so far, when last, does it exist)
    pub fn stats(&self, id: &str) -> (i64, Option<i64>, bool) {
        self.entries().get(id).map_or((0, None, false), |e| (e.received, e.last_at, true))
    }

    /// Decide on a request from the endpoint id and the secret it presented. Accepting counts
    /// against the endpoint's per-minute budget.
    pub fn authorize(&self, id: &str, presented: &str, now: Instant) -> Verdict {
        let mut entries = self.entries();
        let entry = entries.get_mut(id);
        // Unknown endpoints are compared with a stand-in, so they cost the same as known ones.
        let stored = entry.as_ref().map_or("", |e| e.secret.expose());
        let usable = !stored.is_empty() && !presented.is_empty();
        let equal = ct_eq(presented.as_bytes(), if stored.is_empty() { b"\0unknown" } else { stored.as_bytes() });
        let Some(e) = entry else { return Verdict::Denied };
        if !equal || !usable {
            return Verdict::Denied;
        }
        if !e.enabled {
            return Verdict::Off;
        }
        while e.hits.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
            e.hits.pop_front();
        }
        if e.hits.len() >= PER_MINUTE {
            return Verdict::Limited;
        }
        e.hits.push_back(now);
        Verdict::Accepted { name: e.name.clone() }
    }

    fn accepted(&self, id: &str) {
        if let Some(e) = self.entries().get_mut(id) {
            e.received += 1;
            e.last_at = Some(crate::model::now_ms());
        }
    }

    pub fn name(&self, id: &str) -> Option<String> {
        self.entries().get(id).map(|e| e.name.clone())
    }

    /// Newest first.
    pub fn deliveries(&self) -> Vec<Delivery> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    fn record(&self, endpoint: &str, outcome: &str, bytes: usize) {
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.push_front(Delivery { at: crate::model::now_ms(), endpoint: endpoint.into(), outcome: outcome.into(), bytes: bytes as i64 });
        log.truncate(LOG_LEN);
    }
}

// ───────────────────────────── the source ─────────────────────────────

struct Running {
    task: tokio::task::JoinHandle<()>,
    addr: SocketAddr,
}

pub struct WebhookSource {
    table: Arc<Table>,
    port: Mutex<u16>,
    running: Mutex<Option<Running>>,
}

impl WebhookSource {
    pub fn new(table: Arc<Table>, port: u16) -> Self {
        WebhookSource { table, port: Mutex::new(port), running: Mutex::new(None) }
    }

    /// The port in use (the bound one while listening, else the configured one).
    pub fn port(&self) -> u16 {
        self.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or_else(|| *self.port.lock().unwrap_or_else(|e| e.into_inner()), |r| r.addr.port())
    }

    /// Returns whether a running listener must restart to use the new port.
    pub fn set_port(&self, port: u16) -> bool {
        let mut cur = self.port.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *cur != port;
        *cur = port;
        changed && self.running.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

impl InboundSource for WebhookSource {
    fn kind(&self) -> &'static str {
        "webhook"
    }

    fn wanted(&self) -> bool {
        self.table.any_enabled()
    }

    fn status(&self) -> SourceStatus {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        SourceStatus { running, detail: None }
    }

    fn start(&self, sink: Sink) -> Result<()> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.is_some() {
            return Ok(());
        }
        let port = *self.port.lock().unwrap_or_else(|e| e.into_inner());
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("could not listen on 127.0.0.1:{port} (is another app using it?)"))?;
        std_listener.set_nonblocking(true)?;
        let addr = std_listener.local_addr()?;
        let listener = TcpListener::from_std(std_listener)?;
        let task = tokio::spawn(serve(listener, self.table.clone(), sink));
        tracing::info!("webhook listener on {addr}");
        *running = Some(Running { task, addr });
        Ok(())
    }

    fn stop(&self) {
        if let Some(r) = self.running.lock().unwrap_or_else(|e| e.into_inner()).take() {
            r.task.abort(); // drops the listener: the port is free again
            tracing::info!("webhook listener stopped");
        }
    }
}

// ───────────────────────────── HTTP ─────────────────────────────

async fn serve(listener: TcpListener, table: Arc<Table>, sink: Sink) {
    let slots = Arc::new(Semaphore::new(MAX_CONNS));
    loop {
        let Ok((stream, peer)) = listener.accept().await else { continue };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
        let (table, sink) = (table.clone(), sink.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(READ_TIMEOUT + Duration::from_secs(5), connection(stream, &table, &sink)).await;
        });
    }
}

struct Head {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

struct Reply(u16, &'static str);

const BAD_REQUEST: Reply = Reply(400, "bad request");
const DENIED: Reply = Reply(401, "unauthorized");

fn reason(code: u16) -> &'static str {
    match code {
        200..=299 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    }
}

async fn respond(stream: &mut TcpStream, code: u16, msg: &str) {
    let ok = code < 300;
    let body = serde_json::json!({ "ok": ok, if ok { "status" } else { "error" }: msg }).to_string();
    let allow = if code == 405 { "Allow: POST\r\n" } else { "" };
    let out = format!("HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{allow}Connection: close\r\n\r\n{body}", reason(code), body.len());
    let _ = stream.write_all(out.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn connection(mut stream: TcpStream, table: &Table, sink: &Sink) {
    let Reply(code, msg) = match handle(&mut stream, table, sink).await {
        Ok(()) => Reply(202, "accepted"),
        Err(r) => r,
    };
    respond(&mut stream, code, msg).await;
}

async fn handle(stream: &mut TcpStream, table: &Table, sink: &Sink) -> Result<(), Reply> {
    let (head, mut body) = tokio::time::timeout(READ_TIMEOUT, read_head(stream)).await.map_err(|_| BAD_REQUEST)??;
    if head.method != "POST" {
        return Err(Reply(405, "use POST"));
    }
    let path = head.path.split(['?', '#']).next().unwrap_or("");
    let id = path.strip_prefix("/hook/").filter(|id| !id.is_empty() && !id.contains('/')).ok_or(Reply(404, "not found"))?;

    // The secret first: nothing below runs, and no body is read, for someone without it.
    let presented = head
        .headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .or_else(|| head.headers.get("x-codly-secret").map(String::as_str))
        .map(str::trim)
        .unwrap_or("");
    let name = match table.authorize(id, presented, Instant::now()) {
        Verdict::Accepted { name } => name,
        Verdict::Denied => {
            if let Some(name) = table.name(id) {
                table.record(&name, "wrong secret", 0);
            }
            return Err(DENIED);
        }
        Verdict::Off => {
            table.record(&table.name(id).unwrap_or_default(), "switched off", 0);
            return Err(Reply(403, "this endpoint is switched off"));
        }
        Verdict::Limited => {
            table.record(&table.name(id).unwrap_or_default(), "too many requests", 0);
            return Err(Reply(429, "too many requests"));
        }
    };

    if head.headers.contains_key("transfer-encoding") {
        return Err(Reply(411, "send a Content-Length, not a chunked body"));
    }
    let len: usize = head.headers.get("content-length").ok_or(Reply(411, "send a Content-Length"))?.parse().map_err(|_| BAD_REQUEST)?;
    if len > MAX_BODY {
        table.record(&name, "too large", len);
        return Err(Reply(413, "body too large (64 KB at most)"));
    }
    if head.headers.get("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await;
    }
    body.truncate(len);
    if body.len() < len {
        let have = body.len();
        body.resize(len, 0);
        tokio::time::timeout(READ_TIMEOUT, stream.read_exact(&mut body[have..])).await.map_err(|_| BAD_REQUEST)?.map_err(|_| BAD_REQUEST)?;
    }

    let content_type = head.headers.get("content-type").map(String::as_str).unwrap_or("");
    let Some(text) = message_text(&body, content_type) else { return Err(Reply(400, "the message is empty")) };
    let msg = InboundMessage { source: format!("webhook “{name}”"), text };
    match tokio::time::timeout(Duration::from_secs(5), sink(msg)).await {
        Ok(Ok(())) => {
            table.accepted(id);
            table.record(&name, "accepted", len);
            Ok(())
        }
        other => {
            // The reason can name paths or state; it goes to the log, not to the caller.
            if let Ok(Err(e)) = other {
                tracing::warn!("webhook “{name}”: not delivered: {e:#}");
            }
            table.record(&name, "not delivered", len);
            Err(Reply(503, "Codly could not take the message right now"))
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> Result<(Head, Vec<u8>), Reply> {
    let mut buf = Vec::with_capacity(1024);
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Err(Reply(431, "headers too large"));
        }
        let mut chunk = [0u8; 2048];
        let n = stream.read(&mut chunk).await.map_err(|_| BAD_REQUEST)?;
        if n == 0 {
            return Err(BAD_REQUEST);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let text = std::str::from_utf8(&buf[..end]).map_err(|_| BAD_REQUEST)?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path, version) = (first.next().unwrap_or(""), first.next().unwrap_or(""), first.next().unwrap_or(""));
    if method.is_empty() || !path.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err(BAD_REQUEST);
    }
    let mut headers = HashMap::new();
    for l in lines {
        let (k, v) = l.split_once(':').ok_or(BAD_REQUEST)?;
        // A repeated header could smuggle a second length or secret past the check.
        if headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string()).is_some() {
            return Err(BAD_REQUEST);
        }
    }
    Ok((Head { method: method.into(), path: path.into(), headers }, buf[end + 4..].to_vec()))
}

/// The message in a body: `{"text": …, "title": …}` JSON, any other JSON as compact text, or
/// plain text. None when there is nothing to say.
pub fn message_text(body: &[u8], content_type: &str) -> Option<String> {
    let raw = String::from_utf8_lossy(body);
    let text = if content_type.to_ascii_lowercase().contains("json") {
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(serde_json::Value::Object(o)) => {
                let field = |k: &str| o.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
                match field("text").or_else(|| field("message")) {
                    Some(t) => field("title").map_or_else(|| t.to_string(), |title| format!("{title}\n\n{t}")),
                    None => serde_json::Value::Object(o).to_string(),
                }
            }
            Ok(v) => v.to_string(),
            Err(_) => raw.trim().to_string(),
        }
    } else {
        raw.trim().to_string()
    };
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        let t = Table::default();
        t.put("wh_a", "alerts", true, Secret::new("s3cret"));
        t.put("wh_off", "quiet", false, Secret::new("other"));
        t
    }

    #[test]
    fn authorize_denies_unknown_wrong_and_empty_alike() {
        let (t, now) = (table(), Instant::now());
        assert_eq!(t.authorize("wh_a", "s3cret", now), Verdict::Accepted { name: "alerts".into() });
        assert_eq!(t.authorize("wh_a", "s3cre", now), Verdict::Denied);
        assert_eq!(t.authorize("wh_a", "", now), Verdict::Denied);
        assert_eq!(t.authorize("wh_nope", "s3cret", now), Verdict::Denied);
        assert_eq!(t.authorize("wh_nope", "", now), Verdict::Denied);
        // another endpoint's secret does not open this one
        assert_eq!(t.authorize("wh_a", "other", now), Verdict::Denied);
    }

    #[test]
    fn a_switched_off_endpoint_says_so_only_to_its_holder() {
        let (t, now) = (table(), Instant::now());
        assert_eq!(t.authorize("wh_off", "other", now), Verdict::Off);
        assert_eq!(t.authorize("wh_off", "wrong", now), Verdict::Denied);
        t.set_enabled("wh_off", true);
        assert!(matches!(t.authorize("wh_off", "other", now), Verdict::Accepted { .. }));
        t.set_enabled("wh_off", false);
        assert!(t.any_enabled(), "wh_a is still on");
        t.set_enabled("wh_a", false);
        assert!(!t.any_enabled());
    }

    #[test]
    fn an_endpoint_without_a_secret_authenticates_nobody() {
        let t = Table::default();
        t.put("wh_x", "broken", true, Secret::new(""));
        assert_eq!(t.authorize("wh_x", "", Instant::now()), Verdict::Denied);
        assert_eq!(t.authorize("wh_x", "\0unknown", Instant::now()), Verdict::Denied);
    }

    #[test]
    fn rate_limit_is_per_endpoint_and_per_minute() {
        let (t, now) = (table(), Instant::now());
        for _ in 0..PER_MINUTE {
            assert!(matches!(t.authorize("wh_a", "s3cret", now), Verdict::Accepted { .. }));
        }
        assert_eq!(t.authorize("wh_a", "s3cret", now), Verdict::Limited);
        assert_eq!(t.authorize("wh_a", "wrong", now), Verdict::Denied, "a flood of bad guesses is not 'limited'");
        assert!(matches!(t.authorize("wh_a", "s3cret", now + Duration::from_secs(61)), Verdict::Accepted { .. }));
    }

    #[test]
    fn message_text_reads_json_and_plain_bodies() {
        let j = "application/json";
        assert_eq!(message_text(br#"{"text":"deploy failed","title":"CI"}"#, j).as_deref(), Some("CI\n\ndeploy failed"));
        assert_eq!(message_text(br#"{"message":"hi"}"#, j).as_deref(), Some("hi"));
        assert_eq!(message_text(br#"{"a":1}"#, j).as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(message_text(b"  plain words \n", "text/plain").as_deref(), Some("plain words"));
        assert_eq!(message_text(b"not json", j).as_deref(), Some("not json"));
        assert_eq!(message_text(b"   ", "text/plain"), None);
        assert_eq!(message_text(br#"{"text":" "}"#, j).as_deref(), Some(r#"{"text":" "}"#));
    }
}
