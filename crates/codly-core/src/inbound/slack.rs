//! Slack as an inbound source, over Socket Mode (your own Slack app: a bot token `xoxb-…` and an
//! app-level token `xapp-…`; no public URL, no tunnel).
//!
//! STATUS: the settings, the Keychain-held tokens and the envelope parsing with its gates are
//! done and tested (`parse_envelope`). The WebSocket itself (`apps.connections.open` → wss
//! → ack each envelope) needs an HTTPS and a WebSocket client that the core does not depend on
//! yet, so `start` says so and the source never runs. The UI shows "not connected yet".
//!
//! Gates, because a Slack message is open ingress into an agent team: only `app_mention` events
//! and direct messages, never bots or edited/system subtypes, only from allowed users (the list
//! must not be empty: nobody is allowed by default), optionally only in allowed channels, and
//! each event once.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::secrets::SecretStore;
use super::{InboundMessage, InboundSource, Sink, SourceStatus};

pub const BOT_KEY: &str = "slack-bot-token";
pub const APP_KEY: &str = "slack-app-token";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SlackConfig {
    pub enabled: bool,
    /// Slack user ids (`U0123…`) whose messages are accepted. Empty = nobody.
    #[serde(default)]
    pub user_ids: Vec<String>,
    /// Channel ids (`C0123…`) the bot listens in. Empty = any channel the bot is in.
    #[serde(default)]
    pub channel_ids: Vec<String>,
}

impl SlackConfig {
    /// Trimmed, no empties or repeats.
    pub fn cleaned(&self) -> SlackConfig {
        let tidy = |l: &[String]| {
            let mut out: Vec<String> = vec![];
            for s in l.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            out
        };
        SlackConfig { enabled: self.enabled, user_ids: tidy(&self.user_ids), channel_ids: tidy(&self.channel_ids) }
    }
}

/// Reject a token that cannot be the right kind before it reaches the Keychain.
pub fn check_tokens(bot: Option<&str>, app: Option<&str>) -> Result<()> {
    for (t, prefix, what) in [(bot, "xoxb-", "bot token"), (app, "xapp-", "app-level token")] {
        if let Some(t) = t.filter(|t| !t.is_empty()) {
            if !t.starts_with(prefix) || t.len() > 400 || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
                bail!("a Slack {what} starts with {prefix}");
            }
        }
    }
    Ok(())
}

pub struct SlackSource {
    cfg: Mutex<SlackConfig>,
    bot_set: AtomicBool,
    app_set: AtomicBool,
}

impl SlackSource {
    pub fn new(cfg: SlackConfig, secrets: Arc<dyn SecretStore>) -> Self {
        let has = |k: &str| secrets.get(k).ok().flatten().is_some_and(|t| !t.is_empty());
        SlackSource { cfg: Mutex::new(cfg), bot_set: AtomicBool::new(has(BOT_KEY)), app_set: AtomicBool::new(has(APP_KEY)) }
    }

    /// Refresh what the Keychain holds after a token change.
    pub fn refresh_tokens(&self, secrets: &dyn SecretStore) {
        let has = |k: &str| secrets.get(k).ok().flatten().is_some_and(|t| !t.is_empty());
        self.bot_set.store(has(BOT_KEY), Ordering::Relaxed);
        self.app_set.store(has(APP_KEY), Ordering::Relaxed);
    }

    pub fn tokens_set(&self) -> (bool, bool) {
        (self.bot_set.load(Ordering::Relaxed), self.app_set.load(Ordering::Relaxed))
    }

    pub fn set_config(&self, cfg: SlackConfig) {
        *self.cfg.lock().unwrap_or_else(|e| e.into_inner()) = cfg;
    }
}

impl InboundSource for SlackSource {
    fn kind(&self) -> &'static str {
        "slack"
    }

    fn wanted(&self) -> bool {
        let (bot, app) = self.tokens_set();
        let cfg = self.cfg.lock().unwrap_or_else(|e| e.into_inner());
        cfg.enabled && bot && app && !cfg.user_ids.is_empty()
    }

    fn status(&self) -> SourceStatus {
        SourceStatus { running: false, detail: Some("Not connected yet: your settings and tokens are saved, but this build cannot open the Socket Mode connection.".into()) }
    }

    fn start(&self, _sink: Sink) -> Result<()> {
        bail!("Not connected yet: the Slack Socket Mode connection is not built in this version.")
    }

    fn stop(&self) {}
}

// ───────────────────────────── envelopes ─────────────────────────────

/// Event ids already handled (Slack redelivers when an ack is late). Bounded.
#[derive(Default)]
pub struct Seen(VecDeque<String>);

impl Seen {
    /// True the first time an id is shown.
    fn first(&mut self, id: &str) -> bool {
        if self.0.iter().any(|s| s == id) {
            return false;
        }
        self.0.push_back(id.into());
        if self.0.len() > 256 {
            self.0.pop_front();
        }
        true
    }
}

#[derive(Debug, Default)]
pub struct Parsed {
    /// Send this envelope id back as `{"envelope_id": …}` at once, whatever else happens.
    pub ack: Option<String>,
    pub message: Option<InboundMessage>,
    /// Why the event was dropped, for the log (never the text).
    pub dropped: Option<&'static str>,
}

/// One Socket Mode frame in, the message to hand over (if it passes every gate) out.
pub fn parse_envelope(raw: &str, cfg: &SlackConfig, seen: &mut Seen) -> Parsed {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else { return Parsed { dropped: Some("not json"), ..Default::default() } };
    let s = |p: &str| v.pointer(p).and_then(|x| x.as_str());
    let ack = s("/envelope_id").map(str::to_string);
    let drop = |why| Parsed { ack: ack.clone(), message: None, dropped: Some(why) };
    if s("/type") != Some("events_api") {
        return drop("not an event"); // hello, disconnect, slash commands, interactivity
    }
    let ev = |k: &str| s(&format!("/payload/event/{k}"));
    let Some(kind) = ev("type") else { return drop("no event") };
    let direct = kind == "message" && ev("channel_type") == Some("im");
    if kind != "app_mention" && !direct {
        return drop("not a mention or a direct message");
    }
    if ev("bot_id").is_some() || ev("subtype").is_some() {
        return drop("from a bot, or an edit or system message");
    }
    let (Some(user), Some(channel), Some(text)) = (ev("user"), ev("channel"), ev("text")) else { return drop("incomplete") };
    if !cfg.user_ids.iter().any(|u| u == user) {
        return drop("sender not allowed");
    }
    if !cfg.channel_ids.is_empty() && !direct && !cfg.channel_ids.iter().any(|c| c == channel) {
        return drop("channel not allowed");
    }
    if !s("/payload/event_id").is_none_or(|id| seen.first(id)) {
        return drop("duplicate");
    }
    let text = strip_mentions(text);
    if text.is_empty() {
        return drop("empty");
    }
    Parsed { ack, message: Some(InboundMessage { source: format!("Slack user {user} in {channel}"), text }), dropped: None }
}

/// `<@U123> do the thing` → `do the thing` (the bot's own mention is not part of the request).
fn strip_mentions(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("<@") {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SlackConfig {
        SlackConfig { enabled: true, user_ids: vec!["U1".into()], channel_ids: vec!["C1".into()] }
    }

    fn frame(event: serde_json::Value) -> String {
        serde_json::json!({ "type": "events_api", "envelope_id": "env-1", "payload": { "event_id": "Ev1", "event": event } }).to_string()
    }

    fn mention(over: serde_json::Value) -> String {
        let mut e = serde_json::json!({ "type": "app_mention", "user": "U1", "channel": "C1", "text": "<@UBOT> fix the flaky test", "ts": "1.2" });
        e.as_object_mut().unwrap().extend(over.as_object().unwrap().clone());
        frame(e)
    }

    #[test]
    fn an_allowed_mention_becomes_a_message_without_the_mention() {
        let p = parse_envelope(&mention(serde_json::json!({})), &cfg(), &mut Seen::default());
        assert_eq!(p.ack.as_deref(), Some("env-1"));
        assert_eq!(p.message.unwrap().text, "fix the flaky test");
    }

    #[test]
    fn every_gate_drops_but_still_acks() {
        let c = cfg();
        for (over, why) in [
            (serde_json::json!({ "user": "U9" }), "sender not allowed"),
            (serde_json::json!({ "channel": "C9" }), "channel not allowed"),
            (serde_json::json!({ "bot_id": "B1" }), "from a bot, or an edit or system message"),
            (serde_json::json!({ "subtype": "message_changed" }), "from a bot, or an edit or system message"),
            (serde_json::json!({ "type": "reaction_added" }), "not a mention or a direct message"),
            (serde_json::json!({ "text": "<@UBOT>" }), "empty"),
        ] {
            let p = parse_envelope(&mention(over), &c, &mut Seen::default());
            assert!(p.message.is_none() && p.ack.is_some(), "{why}");
            assert_eq!(p.dropped, Some(why));
        }
        let none = SlackConfig { user_ids: vec![], ..cfg() };
        assert!(parse_envelope(&mention(serde_json::json!({})), &none, &mut Seen::default()).message.is_none(), "nobody is allowed by default");
        assert_eq!(parse_envelope("{", &c, &mut Seen::default()).dropped, Some("not json"));
        assert_eq!(parse_envelope(r#"{"type":"hello"}"#, &c, &mut Seen::default()).dropped, Some("not an event"));
    }

    #[test]
    fn a_redelivered_event_is_handled_once() {
        let (c, mut seen) = (cfg(), Seen::default());
        assert!(parse_envelope(&mention(serde_json::json!({})), &c, &mut seen).message.is_some());
        assert_eq!(parse_envelope(&mention(serde_json::json!({})), &c, &mut seen).dropped, Some("duplicate"));
    }

    #[test]
    fn direct_messages_skip_the_channel_list() {
        let dm = frame(serde_json::json!({ "type": "message", "channel_type": "im", "user": "U1", "channel": "D9", "text": "hello" }));
        assert!(parse_envelope(&dm, &cfg(), &mut Seen::default()).message.is_some());
    }

    #[test]
    fn tokens_are_checked_by_kind() {
        assert!(check_tokens(Some("xoxb-1-2-abc"), Some("xapp-1-A-2-abc")).is_ok());
        assert!(check_tokens(Some("xapp-1"), None).is_err());
        assert!(check_tokens(None, Some("xoxb-1")).is_err());
        assert!(check_tokens(Some("xoxb-a b"), None).is_err());
        assert!(check_tokens(Some(""), Some("")).is_ok(), "empty means remove");
    }

    #[test]
    fn slack_is_honest_about_not_connecting() {
        let s = SlackSource::new(cfg(), Arc::new(super::super::secrets::MemorySecrets::default()));
        assert!(!s.wanted(), "no tokens");
        assert!(!s.status().running);
        assert!(s.start(Arc::new(|_| Box::pin(async { Ok(()) }))).unwrap_err().to_string().contains("Not connected yet"));
    }
}
