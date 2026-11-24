//! `codly serve`: the app without its window, the same UI served to a browser. Commands go
//! through Tauri's own IPC dispatch on the (hidden) main webview, so a browser runs exactly
//! the handlers and ACL the window does. The UI bus and terminals, which use Channels in the
//! window, are server-sent events here. Auth: the printed link carries a token, traded once
//! for an HttpOnly cookie. It listens on this Mac only unless `--allow-remote` says otherwise:
//! the traffic is plain HTTP, and a page can open a terminal.

use std::convert::Infallible;
use std::io::{Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use anyhow::Context as _;
use base64::Engine;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};
use hyper::header::HeaderValue;
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::rt::TokioIo;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{mpsc, oneshot};

use crate::commands::AppState;

const USAGE: &str = "usage: codly serve [--port N] [--host ADDR] [--allow-remote]";

pub struct Serve {
    pub host: String,
    pub port: u16,
    /// Other machines can reach `host` (you passed `--allow-remote`).
    pub remote: bool,
}

/// `codly serve [--port N] [--host ADDR] [--allow-remote]` (argv includes the program); None
/// otherwise. A host that other machines can reach is refused without `--allow-remote`.
pub fn from_args(args: &[String]) -> Result<Option<Serve>, String> {
    if args.get(1).map(String::as_str) != Some("serve") {
        return Ok(None);
    }
    let mut s = Serve { host: "127.0.0.1".into(), port: 4280, remote: false };
    let mut allow_remote = false;
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => s.port = it.next().and_then(|p| p.parse().ok()).ok_or(USAGE)?,
            "--host" => s.host = it.next().cloned().ok_or(USAGE)?,
            "--allow-remote" => allow_remote = true,
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    s.remote = !is_loopback(&s.host);
    if s.remote && !allow_remote {
        return Err(format!(
            "codly serve: --host {} would open Codly to your network over plain HTTP.\n{REMOTE_RISK}\n\
             Safer: keep the default host and reach it through a tunnel: ssh -L {port}:127.0.0.1:{port} <this Mac>\n\
             To go on anyway, add --allow-remote.",
            s.host,
            port = s.port
        ));
    }
    Ok(Some(s))
}

const REMOTE_RISK: &str = "Anyone who can reach this Mac and sees the link can open terminals and control your agents, and the traffic is not encrypted.";

/// What `codly serve --host <addr> --allow-remote` says before it goes on.
pub fn remote_warning(s: &Serve) -> String {
    format!(
        "Warning: Codly is open to your network on {}:{port} over plain HTTP.\n{REMOTE_RISK}\n\
         The link works until Codly stops. Safer: an SSH tunnel to the default host: ssh -L {port}:127.0.0.1:{port} <this Mac>\n\
         If the Codly app is already open, nothing is served: quit it, then run this again.",
        s.host,
        port = s.port
    )
}

/// A host that only this Mac can reach.
fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

static SERVING: AtomicBool = AtomicBool::new(false);

/// Serves until the app exits; a second call is a no-op. `open`: also open the browser
/// (when `codly serve` reached an app that was already running, where nobody sees stdout).
pub async fn run(app: AppHandle, s: Serve, open: bool) {
    if SERVING.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(e) = serve(app, s, open).await {
        eprintln!("codly serve: {e:#}");
        tracing::error!("codly serve: {e:#}");
        SERVING.store(false, Ordering::SeqCst);
    }
}

async fn serve(app: AppHandle, s: Serve, open: bool) -> anyhow::Result<()> {
    // A link that crossed the network must not work for ever: only this Mac gets the kept token.
    let token = if s.remote { new_token()? } else { token(&app.path().app_data_dir()?)? };
    let listener = tokio::net::TcpListener::bind((s.host.as_str(), s.port))
        .await
        .with_context(|| format!("could not listen on {}:{} (is another app using it? try --port)", s.host, s.port))?;
    if s.remote {
        tracing::warn!("codly serve is open to the network on {}:{} over plain HTTP (--allow-remote)", s.host, s.port);
    }
    // An IPv6 address goes in brackets in a link.
    let host = if s.host.contains(':') { format!("[{}]", s.host) } else { s.host.clone() };
    let url = format!("http://{host}:{}/?token={token}", s.port);
    println!("Codly is running at {url}\nPress Ctrl+C to stop.");
    if open {
        // The system's own opener: `open` on macOS, `xdg-open` (or the desktop's tool) on Linux.
        if let Err(e) = app.opener().open_url(&url, None::<&str>) {
            tracing::warn!("could not open the browser: {e}");
        }
    }
    loop {
        let (stream, _) = listener.accept().await?;
        let (app, token) = (app.clone(), token.clone());
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req| {
                let (app, token) = (app.clone(), token.clone());
                async move { Ok::<_, Infallible>(handle(app, &token, req).await) }
            });
            // Upgrades: the voice WebSocket (`/api/voice/ws`) is spliced through to the relay.
            let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), svc).with_upgrades().await;
        });
    }
}

/// One token per install, kept in the data dir (0600) so the link survives restarts.
fn token(data_dir: &Path) -> anyhow::Result<String> {
    let path = data_dir.join("web-token");
    if let Ok(t) = std::fs::read_to_string(&path)
        && t.trim().len() >= 32
    {
        return Ok(t.trim().to_string());
    }
    let t = new_token()?;
    std::fs::create_dir_all(data_dir)?;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?.write_all(t.as_bytes())?;
    Ok(t)
}

fn new_token() -> anyhow::Result<String> {
    let mut b = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

type Res = Response<BoxBody<Bytes, Infallible>>;

async fn handle(app: AppHandle, token: &str, req: Request<Incoming>) -> Res {
    let path = req.uri().path().to_string();
    let mut res = route(app, token, req).await;
    secure(&mut res, &path);
    res
}

/// Headers every answer carries: no type guessing, no referrer (the first link holds the
/// token), no framing by another page, and nothing from the API or the token trade is stored.
fn secure(res: &mut Res, path: &str) {
    let h = res.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if path.starts_with("/api/") || h.contains_key(header::SET_COOKIE) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

/// The answer to a request that stops at the door: the printed link's token trade, a request
/// without the cookie, another site's call to the API. None: the request may go on.
fn gate(token: &str, uri: &hyper::Uri, headers: &hyper::HeaderMap) -> Option<Res> {
    let path = uri.path();
    let cookie = format!("codly_token={token}");
    // The printed link: trade the token for a cookie and drop it from the address bar.
    if uri.query().is_some_and(|q| q.split('&').any(|kv| kv.strip_prefix("token=").is_some_and(|t| same_token(t, token)))) {
        return Some(
            Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, path)
                .header(header::SET_COOKIE, format!("{cookie}; Path=/; HttpOnly; SameSite=Strict"))
                .body(full(Bytes::new()))
                .unwrap(),
        );
    }
    let authed = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .any(|c| c.trim().strip_prefix("codly_token=").is_some_and(|t| same_token(t, token)));
    if !authed {
        return Some(text(StatusCode::UNAUTHORIZED, "Open the link `codly serve` printed: it carries the token."));
    }
    if path.starts_with("/api/") && !same_origin(headers) {
        return Some(text(StatusCode::FORBIDDEN, "cross-site request refused"));
    }
    None
}

/// Is `given` the page's token? Every byte is compared, wherever the first difference is, so
/// the time of the answer does not depend on how much of the token matched.
fn same_token(given: &str, token: &str) -> bool {
    let differ = given.bytes().zip(token.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b));
    given.len() == token.len() && std::hint::black_box(differ) == 0
}

async fn route(app: AppHandle, token: &str, req: Request<Incoming>) -> Res {
    if let Some(res) = gate(token, req.uri(), req.headers()) {
        return res;
    }
    let path = req.uri().path().to_string();
    match (req.method(), path.as_str()) {
        (&Method::GET, "/api/events") => events(&app).await,
        (&Method::GET, "/api/voice/ws") => voice(req).await,
        (&Method::GET, p) if p.starts_with("/api/pty/") => pty(&app, &p["/api/pty/".len()..]),
        (&Method::POST, p) if p.starts_with("/api/cmd/") => {
            let cmd = p["/api/cmd/".len()..].to_string();
            command(&app, cmd, req).await
        }
        (&Method::GET, WEB_FLAG) => reply(StatusCode::OK, "text/javascript", Bytes::from_static(b"window.__CODLY_WEB__=true")),
        (&Method::GET, p) => asset(&app, p, req.headers().get(header::HOST).and_then(|v| v.to_str().ok())),
        _ => text(StatusCode::NOT_FOUND, "not found"),
    }
}

/// Port of the voice relay (`voice/relay.py`) on this machine.
fn voice_port() -> u16 {
    std::env::var("CODLY_VOICE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8766)
}

/// Where the relay leaves its token at each start (`voice/guard.py`, `token_path`): a file of
/// this machine's user, one per port.
fn voice_token_path(home: &Path, port: u16) -> std::path::PathBuf {
    home.join(".config/codly").join(format!("voice-{port}.token"))
}

/// The relay's token, read for each call so a relay that started again is followed. None when
/// the file is missing or does not hold a token.
fn voice_token(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let token = text.trim();
    (token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit())).then(|| token.to_string())
}

/// The handshake `codly serve` sends to the relay in the browser's place. It carries the relay's
/// token and no `Origin`: the relay refuses a handshake without the first or with the second, so
/// a web page or another program that finds the relay's port cannot open a voice session.
fn relay_handshake(port: u16, key: &str, version: &str, token: &str) -> String {
    format!(
        "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: {version}\r\nX-Codly-Voice-Token: {token}\r\n\r\n"
    )
}

/// The voice WebSocket: the handshake is replayed to the relay on 127.0.0.1, its `101` goes back to
/// the browser, then bytes are copied both ways. Only Codly's authenticated, same-origin page gets
/// here (`handle` checks the cookie and the origin first); the relay checks that the call comes
/// from here (`relay_handshake`).
async fn voice(req: Request<Incoming>) -> Res {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let h = req.headers();
    let (Some(key), Some(version)) = (h.get("sec-websocket-key"), h.get("sec-websocket-version")) else {
        return text(StatusCode::BAD_REQUEST, "expected a WebSocket handshake");
    };
    let port = voice_port();
    let mut upstream = match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
        Ok(s) => s,
        Err(_) => return text(StatusCode::BAD_GATEWAY, &format!("the voice relay is not running on port {port} (start it with voice/start.sh)")),
    };
    let token = std::env::var_os("HOME").and_then(|home| voice_token(&voice_token_path(Path::new(&home), port)));
    let Some(token) = token else {
        return text(StatusCode::BAD_GATEWAY, &format!("the voice relay on port {port} left no token for Codly (start it again with voice/start.sh)"));
    };
    let handshake = relay_handshake(port, key.to_str().unwrap_or_default(), version.to_str().unwrap_or_default(), &token);
    if upstream.write_all(handshake.as_bytes()).await.is_err() {
        return text(StatusCode::BAD_GATEWAY, "the voice relay closed the connection");
    }
    // Read the relay's answer up to the end of its headers (bytes after it belong to the stream).
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    let end = loop {
        match upstream.read(&mut buf).await {
            Ok(0) | Err(_) => return text(StatusCode::BAD_GATEWAY, "the voice relay closed the connection"),
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
        if let Some(i) = memchr_crlf2(&head) {
            break i;
        }
        if head.len() > 16 << 10 {
            return text(StatusCode::BAD_GATEWAY, "the voice relay sent an oversized handshake");
        }
    };
    let rest = head.split_off(end + 4);
    let head = String::from_utf8_lossy(&head).to_string();
    let mut lines = head.split("\r\n");
    if !lines.next().is_some_and(|l| l.split(' ').nth(1) == Some("101")) {
        return text(StatusCode::BAD_GATEWAY, "the voice relay refused the WebSocket");
    }
    let mut res = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for line in lines.filter(|l| !l.is_empty()) {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            if matches!(k.as_str(), "upgrade" | "connection" | "sec-websocket-accept") {
                res = res.header(k, v.trim());
            }
        }
    }
    tokio::spawn(async move {
        let Ok(up) = hyper::upgrade::on(req).await else { return };
        let mut client = TokioIo::new(up);
        if !rest.is_empty() && client.write_all(&rest).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    });
    res.body(full(Bytes::new())).unwrap()
}

/// Index of the blank line that ends HTTP headers.
fn memchr_crlf2(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Only Codly's own page may call the API. Cookies ignore the port, so without this any page on
/// another localhost port (an agent's dev server…) could drive the app with your cookie.
fn same_origin(h: &hyper::HeaderMap) -> bool {
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok());
    match get("sec-fetch-site") {
        Some(site) => site == "same-origin" || site == "none",
        // Browsers without Fetch Metadata still send Origin on cross-origin requests.
        None => get("origin").is_none_or(|o| get("host").is_some_and(|host| o.strip_prefix("http://") == Some(host))),
    }
}

/// A command only the window can run: Channels can't cross HTTP (see /api/events and
/// /api/pty), and plugins act on the window.
fn window_only(cmd: &str) -> bool {
    cmd.starts_with("plugin:") || matches!(cmd, "ui_subscribe" | "pty_attach")
}

/// Any app command, through the window's own IPC dispatch.
async fn command(app: &AppHandle, cmd: String, req: Request<Incoming>) -> Res {
    if window_only(&cmd) {
        return text(StatusCode::NOT_FOUND, &format!("{cmd} is not available in the browser"));
    }
    let Some(wv) = app.get_webview_window("main") else { return text(StatusCode::SERVICE_UNAVAILABLE, "Codly is still starting") };
    let url = match wv.url() {
        Ok(u) => u,
        Err(e) => return text(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
    };
    let headers = req.headers().clone();
    let json = headers.get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    let bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return text(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let body = if json {
        match serde_json::from_slice(&bytes) {
            Ok(v) => InvokeBody::Json(v),
            Err(e) => return text(StatusCode::BAD_REQUEST, &e.to_string()),
        }
    } else {
        InvokeBody::Raw(bytes.to_vec())
    };
    let (tx, rx) = oneshot::channel();
    let request = InvokeRequest { cmd, callback: CallbackFn(0), error: CallbackFn(1), url, body, headers, invoke_key: app.invoke_key().to_string() };
    wv.as_ref().clone().on_message(request, Box::new(move |_, _, res, _, _| drop(tx.send(res))));
    match rx.await {
        Ok(InvokeResponse::Ok(InvokeResponseBody::Json(s))) => reply(StatusCode::OK, "application/json", s.into()),
        Ok(InvokeResponse::Ok(InvokeResponseBody::Raw(b))) => reply(StatusCode::OK, "application/octet-stream", b.into()),
        // The command's error value, as `invoke` rejects with it.
        Ok(InvokeResponse::Err(e)) => reply(StatusCode::BAD_REQUEST, "application/json", serde_json::to_vec(&e.0).unwrap_or_default().into()),
        Err(_) => text(StatusCode::INTERNAL_SERVER_ERROR, "the command never answered"),
    }
}

/// The UI bus as server-sent events: deltas start flowing, then the snapshot follows
/// (the `ui_subscribe` contract: the UI drops rows with id <= snapshot.as_of).
async fn events(app: &AppHandle) -> Res {
    let state = app.state::<AppState>();
    let (tab, rx) = TabQueue::new();
    let tab = std::sync::Arc::new(tab);
    let sink = tab.clone();
    // A tab let go is out of the bus: the page takes a new snapshot when its stream ends.
    state.bus.add_web(Box::new(move |b| sink.send(sse("batch", &serde_json::to_string(b).unwrap_or_default()))));
    match state.core().await {
        Ok(core) => match core.snapshot().await {
            Ok(s) => tab.send(sse("snapshot", &serde_json::to_string(&s).unwrap_or_default())),
            Err(e) => tab.send(sse("failed", &serde_json::to_string(&format!("{e:#}")).unwrap_or_default())),
        },
        Err(e) => tab.send(sse("failed", &serde_json::to_string(&e).unwrap_or_default())),
    };
    stream(rx)
}

/// One terminal's bytes (base64), scrollback first; ends when the terminal closes, or when the
/// tab stopped reading them (`TabQueue`): the page then attaches again, and the scrollback
/// brings its screen back.
fn pty(app: &AppHandle, id: &str) -> Res {
    let (tab, rx) = TabQueue::new();
    let b64 = base64::engine::general_purpose::STANDARD;
    let sink = move |bytes: Vec<u8>| {
        tab.send(sse("data", &b64.encode(bytes)));
    };
    match app.state::<AppState>().pty.attach(id, Box::new(sink)) {
        Ok(()) => stream(rx),
        Err(e) => text(StatusCode::NOT_FOUND, &format!("{e:#}")),
    }
}

/// How many messages may wait for one browser tab's stream.
const TAB_QUEUE: usize = 8192;

/// The messages that wait for one tab's stream, at most `TAB_QUEUE`. A tab that is that far
/// behind (it stopped reading: asleep, or on a network that stalls) is let go: its stream ends
/// after what waits, which the page reads as an error, and the page takes a new snapshot (or
/// attaches its terminal again). So such a tab cannot fill the memory of Codly.
struct TabQueue(std::sync::Mutex<Option<mpsc::Sender<Bytes>>>);

impl TabQueue {
    fn new() -> (TabQueue, mpsc::Receiver<Bytes>) {
        let (tx, rx) = mpsc::channel(TAB_QUEUE);
        (TabQueue(std::sync::Mutex::new(Some(tx))), rx)
    }

    /// Queue `message`. False: the tab is gone or was let go, now or before.
    fn send(&self, message: Bytes) -> bool {
        let mut tx = self.0.lock().unwrap();
        let sent = tx.as_ref().is_some_and(|tx| tx.try_send(message).is_ok());
        if !sent {
            *tx = None;
        }
        sent
    }
}

/// The script that tells the UI to talk to this server (`isWebApp` in src/lib/api.ts). A file
/// of its own, not an inline script: the page's content security policy refuses those.
const WEB_FLAG: &str = "/codly-web.js";

/// The UI the window loads; index.html tells it to talk to this server.
fn asset(app: &AppHandle, path: &str, host: Option<&str>) -> Res {
    let path = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let Some(a) = app.asset_resolver().get(path.to_string()) else { return text(StatusCode::NOT_FOUND, "not found") };
    // Tauri's resolver calls what it doesn't know (3D models) text/html.
    let mime = if a.mime_type.starts_with("text/html") && !path.ends_with(".html") { "application/octet-stream" } else { &a.mime_type };
    let body: Bytes = if path == "index.html" { web_index(&String::from_utf8_lossy(&a.bytes)).into() } else { a.bytes.into() };
    let mut res = reply(StatusCode::OK, mime, body);
    // The window gets the page's policy from Tauri (tauri.conf.json); a browser gets it here.
    if let Some(csp) = &a.csp_header {
        match HeaderValue::from_str(&web_csp(csp, host)) {
            Ok(csp) => drop(res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, csp)),
            Err(e) => return text(StatusCode::INTERNAL_SERVER_ERROR, &format!("the page's content security policy is not a valid header: {e}")),
        }
    }
    res
}

fn web_index(html: &str) -> String {
    html.replacen("<head>", &format!("<head><script src=\"{WEB_FLAG}\"></script>"), 1)
}

/// The window's content security policy, for a browser: Tauri's own schemes go (a browser has
/// no `ipc:` or `asset:`), what only the browser UI loads stays allowed, and no other page may
/// put the app in a frame.
fn web_csp(csp: &str, host: Option<&str>) -> String {
    const TAURI_ONLY: [&str; 4] = ["ipc:", "http://ipc.localhost", "asset:", "http://asset.localhost"];
    // Next to the nonces and hashes Tauri lists for the page's scripts: `WEB_FLAG` is a file of
    // the page's own, and the voice worklet is loaded from a blob (src/lib/state/voice.svelte.ts).
    const SCRIPTS: [&str; 2] = ["'self'", "blob:"];
    // Only a plain `host[:port]` may go into the header.
    let host = host.filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)));
    let mut directives: Vec<(&str, Vec<String>)> = Vec::new();
    for directive in csp.split(';') {
        let mut words = directive.split_ascii_whitespace();
        match words.next() {
            None | Some("frame-ancestors") => {}
            Some(name) => directives.push((name, words.filter(|s| !TAURI_ONLY.contains(s)).map(String::from).collect())),
        }
    }
    if !directives.iter().any(|(name, _)| *name == "script-src") {
        directives.push(("script-src", Vec::new()));
    }
    for (name, sources) in &mut directives {
        match (*name, host) {
            ("script-src", _) => {
                for s in SCRIPTS {
                    if !sources.iter().any(|x| x == s) {
                        sources.push(s.into());
                    }
                }
            }
            // The voice WebSocket: older browsers do not count `ws:` on the same host as 'self'.
            ("connect-src", Some(host)) => sources.push(format!("ws://{host}")),
            _ => {}
        }
    }
    directives.push(("frame-ancestors", vec!["'none'".into()]));
    directives.iter().map(|(name, sources)| format!("{name} {}", sources.join(" "))).collect::<Vec<_>>().join("; ")
}

fn sse(event: &str, data: &str) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

struct Sse(mpsc::Receiver<Bytes>);

impl Body for Sse {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0.poll_recv(cx).map(|b| b.map(|b| Ok(Frame::data(b))))
    }
}

fn stream(rx: mpsc::Receiver<Bytes>) -> Res {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Sse(rx).boxed())
        .unwrap()
}

fn full(b: Bytes) -> BoxBody<Bytes, Infallible> {
    Full::new(b).boxed()
}

fn reply(status: StatusCode, mime: &str, body: Bytes) -> Res {
    Response::builder().status(status).header(header::CONTENT_TYPE, mime).body(full(body)).unwrap()
}

fn text(status: StatusCode, msg: &str) -> Res {
    reply(status, "text/plain; charset=utf-8", Bytes::from(msg.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_codly_own_page_calls_the_api() {
        let h = |kv: &[(&'static str, &str)]| {
            let mut m = hyper::HeaderMap::new();
            for (k, v) in kv {
                m.insert(*k, v.parse().unwrap());
            }
            same_origin(&m)
        };
        assert!(h(&[("sec-fetch-site", "same-origin")]));
        assert!(!h(&[("sec-fetch-site", "same-site")]), "another localhost port");
        assert!(!h(&[("sec-fetch-site", "cross-site")]));
        assert!(h(&[("host", "127.0.0.1:4280"), ("origin", "http://127.0.0.1:4280")]));
        assert!(!h(&[("host", "127.0.0.1:4280"), ("origin", "http://127.0.0.1:3000")]));
        assert!(h(&[("host", "127.0.0.1:4280")]), "no Origin: a same-origin GET");
    }

    /// The relay lets in only a handshake with its token and without an Origin: this is the one
    /// `codly serve` sends, whatever the browser sent.
    #[test]
    fn the_relay_handshake_carries_the_token_and_no_origin() {
        let h = relay_handshake(8766, "nonce", "13", &"ab".repeat(32));
        assert!(h.starts_with("GET /ws HTTP/1.1\r\nHost: 127.0.0.1:8766\r\n") && h.ends_with("\r\n\r\n"), "{h}");
        assert!(h.contains(&format!("\r\nX-Codly-Voice-Token: {}\r\n", "ab".repeat(32))), "{h}");
        assert!(h.contains("\r\nSec-WebSocket-Key: nonce\r\n") && h.contains("\r\nSec-WebSocket-Version: 13\r\n"), "{h}");
        assert!(!h.to_ascii_lowercase().contains("origin") && !h.to_ascii_lowercase().contains("cookie"), "{h}");
    }

    #[test]
    fn the_relay_token_is_read_from_the_users_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = voice_token_path(dir.path(), 9000);
        assert_eq!(path, dir.path().join(".config/codly/voice-9000.token"), "the name voice/guard.py writes");
        assert_eq!(voice_token(&path), None, "no relay has started");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let token = "0f".repeat(32);
        std::fs::write(&path, format!("{token}\n")).unwrap();
        assert_eq!(voice_token(&path), Some(token.clone()));
        // Not a token: too short, or text that would add a header line of its own.
        std::fs::write(&path, "abc123").unwrap();
        assert_eq!(voice_token(&path), None);
        std::fs::write(&path, format!("{token}\r\nOrigin: x")).unwrap();
        assert_eq!(voice_token(&path), None);
    }

    #[test]
    fn serve_args() {
        let a = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(from_args(&a("codly")).unwrap().is_none());
        assert!(from_args(&a("codly mcp")).unwrap().is_none());
        let s = from_args(&a("codly serve")).unwrap().unwrap();
        assert_eq!((s.host.as_str(), s.port), ("127.0.0.1", 4280));
        assert!(!s.remote);
        let s = from_args(&a("codly serve --port 9000 --host localhost")).unwrap().unwrap();
        assert_eq!((s.host.as_str(), s.port), ("localhost", 9000));
        assert!(from_args(&a("codly serve --port x")).is_err());
        assert!(from_args(&a("codly serve --nope")).is_err());
    }

    #[test]
    fn only_this_mac_unless_you_allow_the_network() {
        let a = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        // Hosts only this Mac can reach need no flag, and keep the token of the install.
        for host in ["127.0.0.1", "127.0.0.2", "localhost", "LOCALHOST", "::1"] {
            let s = from_args(&a(&format!("codly serve --host {host}"))).unwrap().unwrap();
            assert!(!s.remote, "{host}");
        }
        assert!(!from_args(&a("codly serve --allow-remote")).unwrap().unwrap().remote, "the flag alone opens nothing");
        // Any other host is refused, with the reason and the way on.
        for host in ["0.0.0.0", "192.168.1.20", "::", "my-mac.local"] {
            let Err(e) = from_args(&a(&format!("codly serve --port 9000 --host {host}"))) else { panic!("{host} was accepted") };
            assert!(e.contains(&format!("--host {host}")) && e.contains("plain HTTP") && e.contains("--allow-remote"), "{e}");
            assert!(e.contains("ssh -L 9000:127.0.0.1:9000"), "{e}");
        }
        let s = from_args(&a("codly serve --port 9000 --host 0.0.0.0 --allow-remote")).unwrap().unwrap();
        assert_eq!((s.host.as_str(), s.port, s.remote), ("0.0.0.0", 9000, true));
        assert!(from_args(&a("codly serve --allow-remote --host 192.168.1.20")).unwrap().unwrap().remote, "in any order");
        let w = remote_warning(&s);
        assert!(w.starts_with("Warning: Codly is open to your network on 0.0.0.0:9000 over plain HTTP.") && w.contains("open terminals"), "{w}");
        assert!(w.ends_with("If the Codly app is already open, nothing is served: quit it, then run this again."), "{w}");
    }

    #[test]
    fn every_answer_carries_the_security_headers() {
        let headers = |mut res: Res, path: &str| {
            secure(&mut res, path);
            res.headers().clone()
        };
        let get = |h: &hyper::HeaderMap, k: header::HeaderName| h.get(k).map(|v| v.to_str().unwrap().to_string());
        let page = headers(text(StatusCode::OK, "hello"), "/");
        assert_eq!(get(&page, header::X_CONTENT_TYPE_OPTIONS).as_deref(), Some("nosniff"));
        assert_eq!(get(&page, header::REFERRER_POLICY).as_deref(), Some("no-referrer"));
        assert_eq!(get(&page, header::X_FRAME_OPTIONS).as_deref(), Some("DENY"));
        assert_eq!(get(&page, header::CACHE_CONTROL), None, "the UI's files may be cached");
        // The API, its event streams and the token trade are never stored.
        assert_eq!(get(&headers(text(StatusCode::OK, "{}"), "/api/cmd/list_repos"), header::CACHE_CONTROL).as_deref(), Some("no-store"));
        let (_tab, rx) = TabQueue::new();
        assert_eq!(get(&headers(stream(rx), "/api/events"), header::CACHE_CONTROL).as_deref(), Some("no-store"));
        let trade = Response::builder().status(StatusCode::SEE_OTHER).header(header::SET_COOKIE, "codly_token=t").body(full(Bytes::new())).unwrap();
        assert_eq!(get(&headers(trade, "/"), header::CACHE_CONTROL).as_deref(), Some("no-store"));
    }

    #[test]
    fn the_browser_gets_the_window_policy() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let window = conf["app"]["security"]["csp"].as_str().expect("the app has a content security policy");
        assert_eq!(
            web_csp(window, Some("127.0.0.1:4280")),
            "default-src 'self'; connect-src 'self' ws://127.0.0.1:4280; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; \
             font-src 'self' data:; worker-src 'self' blob:; frame-src blob:; object-src blob:; script-src 'self' blob:; frame-ancestors 'none'"
        );
        // What Tauri adds for the page's own scripts stays; the flag script and the voice worklet are allowed.
        let csp = web_csp("default-src 'self'; script-src 'self' 'nonce-123' 'sha256-abc='; frame-ancestors *", None);
        assert_eq!(csp, "default-src 'self'; script-src 'self' 'nonce-123' 'sha256-abc=' blob:; frame-ancestors 'none'");
        assert_eq!(web_csp("script-src 'nonce-123'", None), "script-src 'nonce-123' 'self' blob:; frame-ancestors 'none'");
        // A Host header is the client's text: only a plain host goes into the policy.
        assert_eq!(web_csp("connect-src 'self'", Some("evil.test; script-src *")), "connect-src 'self'; script-src 'self' blob:; frame-ancestors 'none'");
        assert_eq!(web_csp("connect-src 'self'", Some("[::1]:4280")), "connect-src 'self' ws://[::1]:4280; script-src 'self' blob:; frame-ancestors 'none'");
    }

    #[test]
    fn the_page_learns_it_is_served_without_an_inline_script() {
        let html = web_index("<!doctype html><html><head><title>Codly</title></head><body></body></html>");
        assert_eq!(html, "<!doctype html><html><head><script src=\"/codly-web.js\"></script><title>Codly</title></head><body></body></html>");
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    /// What the door answers to a request for `target` with these headers (None: it may go on).
    fn knock(target: &str, kv: &[(&'static str, &str)]) -> Option<Res> {
        let mut h = hyper::HeaderMap::new();
        for (k, v) in kv {
            h.append(*k, v.parse().unwrap());
        }
        gate(TOKEN, &target.parse().unwrap(), &h)
    }

    fn status(target: &str, kv: &[(&'static str, &str)]) -> Option<StatusCode> {
        knock(target, kv).map(|res| res.status())
    }

    fn value(res: &Res, k: header::HeaderName) -> Option<&str> {
        res.headers().get(k).map(|v| v.to_str().unwrap())
    }

    #[test]
    fn the_printed_link_trades_its_token_for_a_cookie() {
        let cookie = format!("codly_token={TOKEN}; Path=/; HttpOnly; SameSite=Strict");
        let mut res = knock(&format!("/?token={TOKEN}"), &[]).expect("the link is answered at the door");
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(value(&res, header::LOCATION), Some("/"), "the token leaves the address bar");
        assert_eq!(value(&res, header::SET_COOKIE), Some(cookie.as_str()), "a page's script cannot read it, another site cannot send it");
        secure(&mut res, "/");
        assert_eq!(value(&res, header::CACHE_CONTROL), Some("no-store"));
        // On any page, next to other parameters.
        let res = knock(&format!("/board?tab=plan&token={TOKEN}"), &[]).unwrap();
        assert_eq!((res.status(), value(&res, header::LOCATION), value(&res, header::SET_COOKIE)), (StatusCode::SEE_OTHER, Some("/board"), Some(cookie.as_str())));

        // Anything but the whole token trades nothing: it is a request without the cookie.
        let wrong = ["token=".to_string(), "token=wrong".to_string(), format!("token={}", &TOKEN[..47]), format!("token={TOKEN}0"), format!("xtoken={TOKEN}"), TOKEN.to_string()];
        for query in &wrong {
            let res = knock(&format!("/?{query}"), &[]).expect("refused");
            assert_eq!((res.status(), value(&res, header::SET_COOKIE)), (StatusCode::UNAUTHORIZED, None), "{query}");
        }
    }

    #[test]
    fn nothing_is_served_without_the_cookie() {
        let cookie = format!("codly_token={TOKEN}");
        let own_page = ("sec-fetch-site", "same-origin");
        for path in ["/", "/assets/index.js", "/codly-web.js", "/api/events", "/api/pty/1", "/api/voice/ws", "/api/cmd/list_repos"] {
            assert_eq!(status(path, &[own_page]), Some(StatusCode::UNAUTHORIZED), "{path}");
            assert_eq!(status(path, &[own_page, ("cookie", "codly_token=the-token-of-another-install")]), Some(StatusCode::UNAUTHORIZED), "{path}");
            assert_eq!(status(path, &[own_page, ("cookie", cookie.as_str())]), None, "{path}");
        }
        // The cookie is compared whole: no shorter, longer or renamed one passes.
        let wrong = [format!("{cookie}0"), cookie[..cookie.len() - 1].to_string(), format!("x{cookie}"), format!("token={TOKEN}"), "codly_token=".to_string(), TOKEN.to_string()];
        for c in &wrong {
            assert_eq!(status("/api/cmd/list_repos", &[own_page, ("cookie", c.as_str())]), Some(StatusCode::UNAUTHORIZED), "{c}");
        }
        // A browser sends it among the other cookies of this host, in one header or in several.
        let among = format!("theme=dark; {cookie}; lang=en");
        assert_eq!(status("/", &[("cookie", among.as_str())]), None);
        assert_eq!(status("/", &[("cookie", "theme=dark"), ("cookie", cookie.as_str())]), None);
    }

    #[test]
    fn another_site_cannot_call_the_api_with_your_cookie() {
        let cookie = format!("codly_token={TOKEN}");
        for path in ["/api/cmd/stop_agent", "/api/events", "/api/pty/1", "/api/voice/ws"] {
            for site in ["cross-site", "same-site"] {
                assert_eq!(status(path, &[("cookie", cookie.as_str()), ("sec-fetch-site", site)]), Some(StatusCode::FORBIDDEN), "{path} from {site}");
            }
            assert_eq!(status(path, &[("cookie", cookie.as_str()), ("host", "127.0.0.1:4280"), ("origin", "http://127.0.0.1:3000")]), Some(StatusCode::FORBIDDEN), "{path}");
            // Without the cookie the answer does not say more than "no".
            assert_eq!(status(path, &[("sec-fetch-site", "cross-site")]), Some(StatusCode::UNAUTHORIZED), "{path}");
        }
        // A link to the page from another site still opens it: only the API is closed.
        assert_eq!(status("/", &[("cookie", cookie.as_str()), ("sec-fetch-site", "cross-site")]), None);
    }

    #[test]
    fn commands_that_need_the_window_are_refused() {
        for cmd in ["ui_subscribe", "pty_attach", "plugin:dialog|open", "plugin:opener|open_url", "plugin:notification|notify"] {
            assert!(window_only(cmd), "{cmd}");
        }
        for cmd in ["list_repos", "send_chat", "ui_set_visible", "pty_open", "pty_write"] {
            assert!(!window_only(cmd), "{cmd}");
        }
    }

    #[test]
    fn the_install_keeps_one_token_only_its_user_can_read() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data"); // a first run: the folder is not there yet
        let first = token(&data).unwrap();
        assert!(first.len() == 48 && first.bytes().all(|b| b.is_ascii_hexdigit()), "{first}");
        assert_eq!(std::fs::metadata(data.join("web-token")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(token(&data).unwrap(), first, "the link survives a restart");
        assert_ne!(new_token().unwrap(), first, "a link for the network gets a token of its own");
        // A file that holds no token (cut short, emptied) is not served as the secret.
        std::fs::write(data.join("web-token"), "short\n").unwrap();
        let next = token(&data).unwrap();
        assert!(next.len() == 48 && next != first, "{next}");
        assert_eq!(token(&data).unwrap(), next);
    }

    #[test]
    fn only_the_whole_token_opens_the_door() {
        assert!(same_token(TOKEN, TOKEN));
        assert!(!same_token(&TOKEN[..47], TOKEN), "one character short");
        assert!(!same_token(&format!("{TOKEN}0"), TOKEN), "one character more");
        assert!(!same_token(&format!("1{}", &TOKEN[1..]), TOKEN), "the first character differs");
        assert!(!same_token(&format!("{}0", &TOKEN[..47]), TOKEN), "the last character differs");
        assert!(!same_token("", TOKEN));
        // The cookie is read the same way as the link (`the_printed_link_trades_its_token_for_a_cookie`).
        let cookie = |c: &str| status("/api/cmd/list_repos", &[("cookie", c), ("sec-fetch-site", "same-origin")]);
        assert_eq!(cookie(&format!("a=b; codly_token={TOKEN}")), None);
        assert_eq!(cookie(&format!("codly_token={}", &TOKEN[..47])), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(cookie(&format!("codly_token={TOKEN}0")), Some(StatusCode::UNAUTHORIZED));
    }

    /// A tab that stops reading is let go when `TAB_QUEUE` messages wait for it: its stream
    /// gives what waits, then ends (the page then takes a new snapshot), and nothing more is
    /// queued for it.
    #[test]
    fn a_tab_that_stops_reading_is_let_go() {
        let (tab, mut rx) = TabQueue::new();
        for i in 0..TAB_QUEUE {
            assert!(tab.send(Bytes::from(i.to_string())), "message {i} waits for the tab");
        }
        assert!(!tab.send(Bytes::from_static(b"one too many")), "the tab is let go");
        assert!(!tab.send(Bytes::from_static(b"later")), "and stays let go");
        for i in 0..TAB_QUEUE {
            assert_eq!(rx.try_recv().unwrap(), Bytes::from(i.to_string()));
        }
        assert_eq!(rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected), "the stream ends");

        // A tab that reads keeps its stream.
        let (tab, mut rx) = TabQueue::new();
        for i in 0..TAB_QUEUE * 2 {
            assert!(tab.send(Bytes::from(i.to_string())));
            assert_eq!(rx.try_recv().unwrap(), Bytes::from(i.to_string()));
        }
        // A tab that closed is out of the bus at the next message.
        drop(rx);
        assert!(!tab.send(Bytes::from_static(b"gone")));
    }
}
