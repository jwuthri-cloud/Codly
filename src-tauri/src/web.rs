//! `codly serve`: the app without its window, the same UI served to a browser. Commands go
//! through Tauri's own IPC dispatch on the (hidden) main webview, so a browser runs exactly
//! the handlers and ACL the window does. The UI bus and terminals, which use Channels in the
//! window, are server-sent events here. Auth: the printed link carries a token, traded once
//! for an HttpOnly cookie.

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
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::rt::TokioIo;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, oneshot};

use crate::commands::AppState;

const USAGE: &str = "usage: codly serve [--port N] [--host ADDR]";

pub struct Serve {
    pub host: String,
    pub port: u16,
}

/// `codly serve [--port N] [--host ADDR]` (argv includes the program); None otherwise.
pub fn from_args(args: &[String]) -> Result<Option<Serve>, String> {
    if args.get(1).map(String::as_str) != Some("serve") {
        return Ok(None);
    }
    let mut s = Serve { host: "127.0.0.1".into(), port: 4280 };
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => s.port = it.next().and_then(|p| p.parse().ok()).ok_or(USAGE)?,
            "--host" => s.host = it.next().cloned().ok_or(USAGE)?,
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    Ok(Some(s))
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
    let token = token(&app.path().app_data_dir()?)?;
    let listener = tokio::net::TcpListener::bind((s.host.as_str(), s.port))
        .await
        .with_context(|| format!("could not listen on {}:{} (is another app using it? try --port)", s.host, s.port))?;
    let url = format!("http://{}:{}/?token={token}", s.host, s.port);
    println!("Codly is running at {url}\nPress Ctrl+C to stop.");
    if open {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    loop {
        let (stream, _) = listener.accept().await?;
        let (app, token) = (app.clone(), token.clone());
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req| {
                let (app, token) = (app.clone(), token.clone());
                async move { Ok::<_, Infallible>(handle(app, &token, req).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await;
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
    let mut b = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    std::fs::create_dir_all(data_dir)?;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?.write_all(t.as_bytes())?;
    Ok(t)
}

type Res = Response<BoxBody<Bytes, Infallible>>;

async fn handle(app: AppHandle, token: &str, req: Request<Incoming>) -> Res {
    let path = req.uri().path().to_string();
    let cookie = format!("codly_token={token}");
    // The printed link: trade the token for a cookie and drop it from the address bar.
    if req.uri().query().is_some_and(|q| q.split('&').any(|kv| kv == format!("token={token}"))) {
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, path)
            .header(header::SET_COOKIE, format!("{cookie}; Path=/; HttpOnly; SameSite=Strict"))
            .body(full(Bytes::new()))
            .unwrap();
    }
    let authed = req.headers().get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).any(|c| c.trim() == cookie);
    if !authed {
        return text(StatusCode::UNAUTHORIZED, "Open the link `codly serve` printed: it carries the token.");
    }
    if path.starts_with("/api/") && !same_origin(req.headers()) {
        return text(StatusCode::FORBIDDEN, "cross-site request refused");
    }
    match (req.method(), path.as_str()) {
        (&Method::GET, "/api/events") => events(&app).await,
        (&Method::GET, p) if p.starts_with("/api/pty/") => pty(&app, &p["/api/pty/".len()..]),
        (&Method::POST, p) if p.starts_with("/api/cmd/") => {
            let cmd = p["/api/cmd/".len()..].to_string();
            command(&app, cmd, req).await
        }
        (&Method::GET, p) => asset(&app, p),
        _ => text(StatusCode::NOT_FOUND, "not found"),
    }
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

/// Any app command, through the window's own IPC dispatch.
async fn command(app: &AppHandle, cmd: String, req: Request<Incoming>) -> Res {
    // Channels can't cross HTTP (see /api/events and /api/pty); plugins act on the window.
    if cmd.starts_with("plugin:") || matches!(cmd.as_str(), "ui_subscribe" | "pty_attach") {
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
    // ponytail: unbounded, a tab that stops reading grows it; bound it if remote tabs matter.
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = tx.clone();
    state.bus.add_web(Box::new(move |b| sink.send(sse("batch", &serde_json::to_string(b).unwrap_or_default())).is_ok()));
    let _ = match state.core().await {
        Ok(core) => match core.snapshot().await {
            Ok(s) => tx.send(sse("snapshot", &serde_json::to_string(&s).unwrap_or_default())),
            Err(e) => tx.send(sse("failed", &serde_json::to_string(&format!("{e:#}")).unwrap_or_default())),
        },
        Err(e) => tx.send(sse("failed", &serde_json::to_string(&e).unwrap_or_default())),
    };
    stream(rx)
}

/// One terminal's bytes (base64), scrollback first; ends when the terminal closes.
fn pty(app: &AppHandle, id: &str) -> Res {
    let (tx, rx) = mpsc::unbounded_channel();
    let b64 = base64::engine::general_purpose::STANDARD;
    match app.state::<AppState>().pty.attach(id, Box::new(move |bytes| drop(tx.send(sse("data", &b64.encode(bytes)))))) {
        Ok(()) => stream(rx),
        Err(e) => text(StatusCode::NOT_FOUND, &format!("{e:#}")),
    }
}

/// The UI the window loads; index.html tells it to talk to this server.
fn asset(app: &AppHandle, path: &str) -> Res {
    let path = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let Some(a) = app.asset_resolver().get(path.to_string()) else { return text(StatusCode::NOT_FOUND, "not found") };
    if path == "index.html" {
        let html = String::from_utf8_lossy(&a.bytes).replacen("<head>", "<head><script>window.__CODLY_WEB__=true</script>", 1);
        return reply(StatusCode::OK, &a.mime_type, html.into());
    }
    // Tauri's resolver calls what it doesn't know (3D models) text/html.
    let mime = if a.mime_type.starts_with("text/html") && !path.ends_with(".html") { "application/octet-stream" } else { &a.mime_type };
    reply(StatusCode::OK, mime, a.bytes.into())
}

fn sse(event: &str, data: &str) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

struct Sse(mpsc::UnboundedReceiver<Bytes>);

impl Body for Sse {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0.poll_recv(cx).map(|b| b.map(|b| Ok(Frame::data(b))))
    }
}

fn stream(rx: mpsc::UnboundedReceiver<Bytes>) -> Res {
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

    #[test]
    fn serve_args() {
        let a = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(from_args(&a("codly")).unwrap().is_none());
        assert!(from_args(&a("codly mcp")).unwrap().is_none());
        let s = from_args(&a("codly serve")).unwrap().unwrap();
        assert_eq!((s.host.as_str(), s.port), ("127.0.0.1", 4280));
        let s = from_args(&a("codly serve --port 9000 --host 0.0.0.0")).unwrap().unwrap();
        assert_eq!((s.host.as_str(), s.port), ("0.0.0.0", 9000));
        assert!(from_args(&a("codly serve --port x")).is_err());
        assert!(from_args(&a("codly serve --nope")).is_err());
    }
}
