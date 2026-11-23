//! The hub end to end: a real listener on a loopback port, a fake `Deliver`, in-memory secrets.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::secrets::{MemorySecrets, SecretStore};
use super::slack::{APP_KEY, BOT_KEY, SlackConfig};
use super::*;

type Inbox = Arc<Mutex<Vec<(String, InboundMessage)>>>;

fn hub(dir: &std::path::Path, secrets: Arc<MemorySecrets>) -> (Arc<InboundHub>, Inbox) {
    let inbox: Inbox = Arc::default();
    let sink = inbox.clone();
    let deliver: Deliver = Arc::new(move |thread, msg| {
        sink.lock().unwrap().push((thread, msg));
        Box::pin(async { Ok(()) })
    });
    let h = InboundHub::new(dir, secrets, deliver);
    h.set_settings("lead", 0).unwrap(); // any free port
    (h, inbox)
}

/// One raw HTTP request; returns the status code.
async fn post(port: u16, path: &str, headers: &[(&str, &str)], body: &str) -> u16 {
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req += &format!("{k}: {v}\r\n");
    }
    raw(port, &format!("{req}\r\n{body}")).await
}

async fn raw(port: u16, request: &str) -> u16 {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

fn auth(secret: &str) -> [(&'static str, String); 1] {
    [("Authorization", format!("Bearer {secret}"))]
}

async fn post_with(port: u16, id: &str, secret: &str, ctype: &str, body: &str) -> u16 {
    let a = auth(secret);
    post(port, &format!("/hook/{id}"), &[(a[0].0, a[0].1.as_str()), ("Content-Type", ctype)], body).await
}

#[tokio::test]
async fn a_new_endpoint_is_off_and_the_listener_runs_only_while_one_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let (h, inbox) = hub(dir.path(), Arc::default());
    let w = h.create_webhook("  CI alerts ").unwrap();
    assert!(!w.endpoint.enabled, "created switched off");
    assert_eq!(w.endpoint.name, "CI alerts");
    assert!(w.secret.starts_with("cwh_") && w.endpoint.url.starts_with("http://127.0.0.1:") && !w.endpoint.url.contains(&w.secret));
    assert!(!h.state().listening, "nothing on, nothing listening");

    h.set_webhook_enabled(&w.endpoint.id, true).unwrap();
    let st = h.state();
    assert!(st.listening && st.listen_error.is_none());
    let port = st.port;
    assert_ne!(port, 0, "the bound port is reported");

    assert_eq!(post_with(port, &w.endpoint.id, &w.secret, "application/json", r#"{"title":"CI","text":"build 42 failed"}"#).await, 202);
    let got = inbox.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "lead", "the Lead answers by default");
    // The message as it was sent, under its endpoint's name; a new endpoint cannot start work alone.
    assert_eq!(got[0].1, InboundMessage { source: "webhook “CI alerts”".into(), text: "CI\n\nbuild 42 failed".into(), auto_run: false });
    let st = h.state();
    assert_eq!((st.endpoints[0].received, st.deliveries[0].outcome.as_str()), (1, "accepted"));

    // Once you let the endpoint run plans without asking, its next messages say so.
    h.set_webhook_auto_run(&w.endpoint.id, true).unwrap();
    assert_eq!(post_with(port, &w.endpoint.id, &w.secret, "text/plain", "deploy 7 is out").await, 202);
    assert_eq!(inbox.lock().unwrap()[1].1, InboundMessage { source: "webhook “CI alerts”".into(), text: "deploy 7 is out".into(), auto_run: true });

    h.set_webhook_enabled(&w.endpoint.id, false).unwrap();
    assert!(!h.state().listening);
    // The listener's task closes the socket a moment after the switch.
    let closed = async {
        while TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    };
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), closed).await.is_ok(), "the port is closed again");
}

#[tokio::test]
async fn bad_requests_are_refused_and_nothing_reaches_the_lead() {
    let dir = tempfile::tempdir().unwrap();
    let (h, inbox) = hub(dir.path(), Arc::default());
    let w = h.create_webhook("hooks").unwrap();
    let off = h.create_webhook("quiet").unwrap();
    h.set_webhook_enabled(&w.endpoint.id, true).unwrap();
    let (port, id) = (h.state().port, w.endpoint.id.clone());

    // no secret, wrong secret, secret of another endpoint, unknown endpoint: all the same 401
    assert_eq!(post(port, &format!("/hook/{id}"), &[], "hi").await, 401);
    assert_eq!(post_with(port, &id, "cwh_wrong", "text/plain", "hi").await, 401);
    assert_eq!(post_with(port, &id, &off.secret, "text/plain", "hi").await, 401);
    assert_eq!(post_with(port, "wh_unknown", &w.secret, "text/plain", "hi").await, 401);
    // the secret in the URL or the query is not a secret
    assert_eq!(post(port, &format!("/hook/{id}?secret={}", w.secret), &[], "hi").await, 401);
    // the legacy header works
    assert_eq!(post(port, &format!("/hook/{id}"), &[("X-Codly-Secret", &w.secret)], "via header").await, 202);
    // switched off: only its holder is told
    h.set_webhook_enabled(&off.endpoint.id, false).unwrap();
    assert_eq!(post_with(port, &off.endpoint.id, &off.secret, "text/plain", "hi").await, 403);
    // method, route, size, framing
    assert_eq!(raw(port, &format!("OPTIONS /hook/{id} HTTP/1.1\r\nOrigin: https://evil.example\r\nAccess-Control-Request-Headers: authorization\r\n\r\n")).await, 405);
    assert_eq!(raw(port, &format!("GET /hook/{id} HTTP/1.1\r\n\r\n")).await, 405);
    assert_eq!(post_with(port, "", &w.secret, "text/plain", "hi").await, 404);
    assert_eq!(post(port, "/elsewhere", &[], "hi").await, 404);
    // refused on the declared length, before any of the body is read
    let a = auth(&w.secret);
    assert_eq!(raw(port, &format!("POST /hook/{id} HTTP/1.1\r\n{}: {}\r\nContent-Length: {}\r\n\r\n", a[0].0, a[0].1, webhook::MAX_BODY + 1)).await, 413);
    assert_eq!(post_with(port, &id, &w.secret, "text/plain", "   ").await, 400);
    assert_eq!(raw(port, &format!("POST /hook/{id} HTTP/1.1\r\n{}: {}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", a[0].0, a[0].1)).await, 411);
    assert_eq!(raw(port, &format!("POST /hook/{id} HTTP/1.1\r\n{}: {}\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab", a[0].0, a[0].1)).await, 400, "repeated headers");
    assert_eq!(raw(port, "garbage\r\n\r\n").await, 400);

    let msgs = inbox.lock().unwrap().clone();
    assert_eq!(msgs.len(), 1, "only the one valid request got through: {msgs:?}");
    assert_eq!(msgs[0].1.text, "via header");
    let outcomes: Vec<_> = h.state().deliveries.into_iter().map(|d| d.outcome).collect();
    assert!(outcomes.iter().any(|o| o == "wrong secret") && outcomes.iter().any(|o| o == "too large") && outcomes.iter().any(|o| o == "switched off"));
}

/// What the thread stores for an inbound message names its source and is never the user's
/// words; whether work may follow without the user is read back from what was stored.
#[test]
fn an_inbound_message_is_stored_under_its_source() {
    let m = InboundMessage { source: "webhook “CI alerts”".into(), text: "build 42 failed".into(), auto_run: false };
    assert_eq!(m.shown(), "From webhook “CI alerts” (outside Codly):\nbuild 42 failed");
    let att = m.attachment();
    assert_eq!(att, serde_json::json!({ "inbound": { "source": "webhook “CI alerts”", "auto_run": false } }));
    assert!(is_inbound(Some(&att)) && needs_ok(Some(&att)));
    let trusted = InboundMessage { auto_run: true, ..m }.attachment();
    assert!(is_inbound(Some(&trusted)) && !needs_ok(Some(&trusted)), "its endpoint runs plans without asking");
    // Anything but a plain `true` asks; other attachments (a plan card) and none are not inbound.
    assert!(needs_ok(Some(&serde_json::json!({ "inbound": { "source": "x", "auto_run": "true" } }))));
    assert!(needs_ok(Some(&serde_json::json!({ "inbound": { "source": "x" } }))));
    for other in [Some(serde_json::json!({ "plan_id": "p1" })), Some(serde_json::json!({ "inbound": true })), None] {
        assert!(!is_inbound(other.as_ref()) && !needs_ok(other.as_ref()), "{other:?}");
    }
}

/// The switch of an endpoint, without a listener (the endpoint stays off): what the hub saves,
/// and what the table the listener reads would give a request.
#[tokio::test]
async fn an_endpoint_runs_plans_without_asking_only_when_you_say_so() {
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = Arc::default();
    let (h, _) = hub(dir.path(), secrets.clone());
    let w = h.create_webhook("deploys").unwrap();
    let accepted = |h: &InboundHub| {
        h.table.set_enabled(&w.endpoint.id, true);
        h.table.authorize(&w.endpoint.id, &w.secret, std::time::Instant::now())
    };
    assert!(!w.endpoint.auto_run && !h.state().endpoints[0].auto_run, "off for a new endpoint");
    assert_eq!(accepted(&h), webhook::Verdict::Accepted { name: "deploys".into(), auto_run: false });
    h.set_webhook_auto_run(&w.endpoint.id, true).unwrap();
    assert!(h.state().endpoints[0].auto_run);
    assert_eq!(accepted(&h), webhook::Verdict::Accepted { name: "deploys".into(), auto_run: true });
    assert!(h.set_webhook_auto_run("wh_nope", true).is_err());

    // The choice is saved with the endpoint, and can be taken back.
    let (h, _) = hub_keeping_settings(dir.path(), secrets.clone());
    assert!(h.state().endpoints[0].auto_run);
    assert_eq!(accepted(&h), webhook::Verdict::Accepted { name: "deploys".into(), auto_run: true });
    h.set_webhook_auto_run(&w.endpoint.id, false).unwrap();
    assert!(!h.state().endpoints[0].auto_run);
    assert_eq!(accepted(&h), webhook::Verdict::Accepted { name: "deploys".into(), auto_run: false });

    // A config written before the switch existed reads as off.
    h.set_webhook_auto_run(&w.endpoint.id, true).unwrap();
    let path = dir.path().join("inbound.json");
    let mut saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["endpoints"][0].as_object_mut().unwrap().remove("auto_run"), Some(serde_json::json!(true)));
    std::fs::write(&path, saved.to_string()).unwrap();
    let (h, _) = hub_keeping_settings(dir.path(), secrets);
    assert!(!h.state().endpoints[0].auto_run);
}

/// A long message is cut before it is handed over, so the thread and the agent get the same text.
#[tokio::test]
async fn a_long_message_is_cut_once_for_everyone() {
    let dir = tempfile::tempdir().unwrap();
    let (h, inbox) = hub(dir.path(), Arc::default());
    let long = InboundMessage { source: "webhook “logs”".into(), text: "é".repeat(MAX_TEXT + 500), auto_run: false };
    (h.sink)(long).await.unwrap();
    let got = inbox.lock().unwrap();
    assert_eq!((got[0].0.as_str(), got[0].1.text.chars().count()), ("lead", MAX_TEXT));
}

#[tokio::test]
async fn revoking_kills_the_url_and_deletes_the_secret() {
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = Arc::default();
    let (h, _) = hub(dir.path(), secrets.clone());
    let (a, b) = (h.create_webhook("a").unwrap(), h.create_webhook("b").unwrap());
    h.set_webhook_enabled(&a.endpoint.id, true).unwrap();
    h.set_webhook_enabled(&b.endpoint.id, true).unwrap();
    let port = h.state().port;
    assert_eq!(post_with(port, &a.endpoint.id, &a.secret, "text/plain", "one").await, 202);

    h.revoke_webhook(&a.endpoint.id).unwrap();
    assert_eq!(post_with(port, &a.endpoint.id, &a.secret, "text/plain", "two").await, 401, "revoked alone");
    assert_eq!(post_with(port, &b.endpoint.id, &b.secret, "text/plain", "three").await, 202, "the other one still works");
    assert!(secrets.get(&format!("webhook:{}", a.endpoint.id)).unwrap().is_none());
    assert!(secrets.get(&format!("webhook:{}", b.endpoint.id)).unwrap().is_some());
    assert!(h.state().listening);
    h.revoke_webhook(&b.endpoint.id).unwrap();
    assert!(!h.state().listening, "no endpoint left on");
    assert!(h.revoke_webhook("wh_nope").is_err());
}

#[tokio::test]
async fn secrets_never_touch_the_config_file_or_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = Arc::default();
    let (h, _) = hub(dir.path(), secrets.clone());
    let w = h.create_webhook("a").unwrap();
    let file = std::fs::read_to_string(dir.path().join("inbound.json")).unwrap();
    let state = serde_json::to_string(&h.state()).unwrap();
    for hay in [&file, &state] {
        assert!(!hay.contains(&w.secret), "{hay}");
    }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(dir.path().join("inbound.json")).unwrap().permissions().mode() & 0o777, 0o600);
}

/// Slack cannot connect in this version, so a token would sit in the Keychain with nothing to
/// read it. None is taken; the tokens an earlier version saved are shown as saved and can go.
#[tokio::test]
async fn slack_takes_no_token_and_lets_a_saved_one_go() {
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = Arc::default();
    secrets.set(BOT_KEY, "xoxb-1-secret").unwrap();
    secrets.set(APP_KEY, "xapp-1-secret").unwrap();
    let (h, _) = hub(dir.path(), secrets.clone());
    let state = serde_json::to_string(&h.state()).unwrap();
    assert!(state.contains("\"bot_token_set\":true") && state.contains("\"app_token_set\":true"), "{state}");
    assert!(!state.contains("xoxb-1-secret") && !state.contains("xapp-1-secret"), "{state}");

    for update in [
        SlackUpdate { config: SlackConfig::default(), bot_token: Some("xoxb-2-new".into()), app_token: None },
        SlackUpdate { config: SlackConfig::default(), bot_token: Some(String::new()), app_token: Some("xapp-2-new".into()) },
    ] {
        let e = h.set_slack(update).unwrap_err().to_string();
        assert!(e.contains("not available yet"), "{e}");
    }
    assert_eq!(secrets.get(BOT_KEY).unwrap().as_deref(), Some("xoxb-1-secret"), "a refused update changes nothing");
    assert_eq!(secrets.get(APP_KEY).unwrap().as_deref(), Some("xapp-1-secret"));

    h.set_slack(SlackUpdate { config: SlackConfig::default(), bot_token: Some(String::new()), app_token: None }).unwrap();
    assert!(!h.state().slack.bot_token_set && h.state().slack.app_token_set, "one token removed, the other kept");
    h.set_slack(SlackUpdate { config: SlackConfig::default(), bot_token: None, app_token: Some(String::new()) }).unwrap();
    assert!(secrets.get(BOT_KEY).unwrap().is_none() && secrets.get(APP_KEY).unwrap().is_none());
    assert!(!h.state().slack.connected);
}

#[tokio::test]
async fn config_survives_a_restart_and_a_lost_secret_cannot_be_switched_on() {
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = Arc::default();
    let (id, secret) = {
        let (h, _) = hub(dir.path(), secrets.clone());
        let w = h.create_webhook("durable").unwrap();
        h.set_webhook_enabled(&w.endpoint.id, true).unwrap();
        h.set_settings("agent:ag-1", 0).unwrap();
        let lost = h.create_webhook("lost").unwrap();
        secrets.delete(&format!("webhook:{}", lost.endpoint.id)).unwrap();
        h.shutdown();
        (w.endpoint.id, w.secret)
    };
    let (h, inbox) = hub_keeping_settings(dir.path(), secrets);
    let st = h.state();
    assert!(st.listening, "an endpoint left on starts with the app");
    assert_eq!(st.answerer, "agent:ag-1");
    assert_eq!(st.endpoints.len(), 2);
    assert_eq!(st.endpoints.iter().filter(|e| e.broken).count(), 1);
    let lost = st.endpoints.iter().find(|e| e.broken).unwrap().id.clone();
    assert!(h.set_webhook_enabled(&lost, true).is_err());
    assert_eq!(post_with(st.port, &id, &secret, "text/plain", "after restart").await, 202);
    assert_eq!(inbox.lock().unwrap()[0].0, "agent:ag-1", "the chosen thread answers");
    assert!(h.set_settings("somewhere", 0).is_err() && h.set_settings("lead", 80).is_err());
}

/// Like `hub` but without resetting the saved settings.
fn hub_keeping_settings(dir: &std::path::Path, secrets: Arc<MemorySecrets>) -> (Arc<InboundHub>, Inbox) {
    let inbox: Inbox = Arc::default();
    let sink = inbox.clone();
    let deliver: Deliver = Arc::new(move |thread, msg| {
        sink.lock().unwrap().push((thread, msg));
        Box::pin(async { Ok(()) })
    });
    (InboundHub::new(dir, secrets, deliver), inbox)
}

#[tokio::test]
async fn when_the_lead_cannot_take_it_the_caller_gets_503_without_details() {
    let dir = tempfile::tempdir().unwrap();
    let deliver: Deliver = Arc::new(|_, _| Box::pin(async { anyhow::bail!("no repository is open at /Users/me/secret-project") }));
    let h = InboundHub::new(dir.path(), Arc::new(MemorySecrets::default()), deliver);
    h.set_settings("lead", 0).unwrap();
    let w = h.create_webhook("a").unwrap();
    h.set_webhook_enabled(&w.endpoint.id, true).unwrap();
    let port = h.state().port;
    let a = auth(&w.secret);
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("POST /hook/{} HTTP/1.1\r\n{}: {}\r\nContent-Length: 2\r\n\r\nhi", w.endpoint.id, a[0].0, a[0].1).as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 503") && !out.contains("secret-project"), "{out}");
    assert_eq!(h.state().deliveries[0].outcome, "not delivered");
}

#[tokio::test]
async fn a_port_in_use_is_reported_not_fatal() {
    let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let taken = blocker.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let (h, _) = hub(dir.path(), Arc::default());
    h.set_settings("lead", taken.max(1024)).unwrap_or_else(|e| panic!("{e}"));
    let w = h.create_webhook("a").unwrap();
    h.set_webhook_enabled(&w.endpoint.id, true).unwrap();
    let st = h.state();
    assert!(!st.listening && st.listen_error.unwrap().contains("could not listen"));
}
