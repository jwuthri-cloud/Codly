//! Inbound work sources: ways for something outside Codly to hand the Lead a message.
//!
//! - `webhook`  a loopback HTTP listener; each endpoint has its own secret, is created switched
//!              off, and can be revoked alone. Anyone holding the URL and its secret can post.
//! - `slack`    Slack over Socket Mode (message parsing and gating are here; the socket is not
//!              connected yet, and the UI says so).
//! - `secrets`  the Keychain-backed store for endpoint secrets and Slack tokens.
//!
//! A source implements `InboundSource` and pushes `InboundMessage`s into the `Sink` the hub
//! gives it. The hub hands every message, with the name of its source, to whoever answers
//! inbound messages (the Lead unless you picked another thread). A source only runs while
//! something it serves is switched on.
//!
//! An inbound message is never the user's: it is stored and shown under its source's name
//! (`InboundMessage::shown`, `attachment`), the agent reads it as quoted data, and a plan made
//! after it waits for the user's Run plan (`needs_ok`) unless its endpoint is one the user
//! lets run plans without asking (`auto_run`, off for every new endpoint).
//!
//! The hub is independent of `Core`: the app gives it a `Deliver` closure
//! (`Core::send_inbound`), so it is testable with a fake. Config lives in
//! `<data_dir>/inbound.json` (owner-only; no secrets in it).

pub mod secrets;
pub mod slack;
pub mod webhook;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::model::now_ms;
use secrets::{Secret, SecretStore, random_token};
use slack::{SlackConfig, SlackSource};
use webhook::{Table, WebhookSource};

pub type BoxFut = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
/// Where a source puts what it received. The future resolves once the message is handed over.
pub type Sink = Arc<dyn Fn(InboundMessage) -> BoxFut + Send + Sync>;
/// The app's hand-over: (chat thread, message). In the app this is `Core::send_inbound`.
pub type Deliver = Arc<dyn Fn(String, InboundMessage) -> BoxFut + Send + Sync>;

/// Default port of the webhook listener (loopback only).
pub const DEFAULT_PORT: u16 = 47821;
/// Longest message text handed to the Lead, in characters.
pub const MAX_TEXT: usize = 8_000;

#[derive(Debug, Clone, PartialEq)]
pub struct InboundMessage {
    /// Where it came from, for the thread and the Lead: `webhook “ci-alerts”`, `Slack #ops`.
    pub source: String,
    pub text: String,
    /// The user lets this source start work alone: a plan made from its messages may run
    /// without the Run plan click, in a request that runs plans at once.
    pub auto_run: bool,
}

impl InboundMessage {
    /// What the thread stores and shows: the text under the name of its source, so it never
    /// reads as something the user wrote.
    pub fn shown(&self) -> String {
        format!("From {} (outside Codly):\n{}", self.source, self.text)
    }

    /// What is kept with the chat message (`ChatMessage::attachment`): where it came from, and
    /// whether that source may start work alone.
    pub fn attachment(&self) -> Value {
        json!({ "inbound": { "source": self.source, "auto_run": self.auto_run } })
    }
}

/// Whether a chat message's attachment says the message came from outside Codly.
pub fn is_inbound(attachment: Option<&Value>) -> bool {
    attachment.is_some_and(|a| a["inbound"].is_object())
}

/// Whether work that follows a chat message with this attachment waits for the user's OK: the
/// message came from outside, and not from a source the user lets start work alone.
pub fn needs_ok(attachment: Option<&Value>) -> bool {
    is_inbound(attachment) && attachment.is_some_and(|a| a["inbound"]["auto_run"] != true)
}

#[derive(Debug, Clone, Default)]
pub struct SourceStatus {
    pub running: bool,
    /// Why it is not running (or a note), shown in settings.
    pub detail: Option<String>,
}

/// A source of inbound messages. `start` is called when `wanted` turns true and `stop` when it
/// turns false, so a source holds no socket while nothing is switched on.
pub trait InboundSource: Send + Sync {
    fn kind(&self) -> &'static str;
    fn wanted(&self) -> bool;
    fn status(&self) -> SourceStatus;
    fn start(&self, sink: Sink) -> Result<()>;
    fn stop(&self);
}

// ───────────────────────────── what the UI sees ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebhookEndpoint {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: i64,
    /// `http://127.0.0.1:<port>/hook/<id>`; the secret is not part of it.
    pub url: String,
    /// Messages accepted since Codly started.
    pub received: i64,
    pub last_received_at: Option<i64>,
    /// Its secret is gone from the Keychain: it cannot authenticate anyone. Revoke it.
    pub broken: bool,
    /// You let this endpoint run plans without asking: in a request that runs plans at once, a
    /// plan made from its messages starts without your Run plan click. Off unless you set it.
    pub auto_run: bool,
}

/// One inbound attempt, newest first. Never holds the body or the secret.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Delivery {
    pub at: i64,
    pub endpoint: String,
    /// "accepted" | "wrong secret" | "switched off" | "too many requests" | "too large" | "not delivered".
    pub outcome: String,
    pub bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SlackView {
    pub config: SlackConfig,
    pub bot_token_set: bool,
    pub app_token_set: bool,
    pub connected: bool,
    /// One sentence on where it stands ("Not available yet: …").
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InboundState {
    /// Chat thread that answers inbound messages: "lead" or "agent:<id>".
    pub answerer: String,
    /// Configured port of the webhook listener.
    pub port: u16,
    pub listening: bool,
    pub listen_error: Option<String>,
    pub endpoints: Vec<WebhookEndpoint>,
    pub slack: SlackView,
    pub deliveries: Vec<Delivery>,
}

/// A new endpoint, with its secret. This is the only time the secret leaves the core.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewWebhook {
    pub endpoint: WebhookEndpoint,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SlackUpdate {
    pub config: SlackConfig,
    /// None keeps the stored token, "" removes it. Anything else is refused while Slack is not
    /// available (`InboundHub::set_slack`).
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
}

// ───────────────────────────── the hub ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointRec {
    id: String,
    name: String,
    enabled: bool,
    created_at: i64,
    #[serde(default)]
    auto_run: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    answerer: Option<String>,
    port: Option<u16>,
    #[serde(default)]
    endpoints: Vec<EndpointRec>,
    #[serde(default)]
    slack: SlackConfig,
}

pub struct InboundHub {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    persisted: Mutex<Persisted>,
    answerer: Arc<RwLock<String>>,
    table: Arc<Table>,
    webhook: Arc<WebhookSource>,
    slack: Arc<SlackSource>,
    sink: Sink,
    errors: Mutex<HashMap<&'static str, String>>,
}

fn secret_key(id: &str) -> String {
    format!("webhook:{id}")
}

impl InboundHub {
    /// Load the saved config and its secrets and start whatever is switched on. The methods
    /// below block briefly on the Keychain: call them from a blocking thread in async code.
    /// Needs a tokio runtime.
    pub fn new(data_dir: &std::path::Path, secrets: Arc<dyn SecretStore>, deliver: Deliver) -> Arc<InboundHub> {
        let path = data_dir.join("inbound.json");
        let persisted: Persisted = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let answerer = Arc::new(RwLock::new(persisted.answerer.clone().filter(|a| valid_answerer(a)).unwrap_or_else(|| "lead".into())));
        let table = Arc::new(Table::default());
        for e in &persisted.endpoints {
            // A secret the Keychain no longer has leaves the endpoint out of the table: it
            // cannot authenticate anyone (`state` shows it as broken).
            if let Ok(Some(s)) = secrets.get(&secret_key(&e.id)).map(|s| s.filter(|s| !s.is_empty())) {
                table.put(&e.id, &e.name, e.enabled, e.auto_run, Secret::new(s));
            }
        }
        let webhook = Arc::new(WebhookSource::new(table.clone(), persisted.port.unwrap_or(DEFAULT_PORT)));
        let slack = Arc::new(SlackSource::new(persisted.slack.clone(), secrets.clone()));
        // Every source's messages: cut to size, then to the thread that answers.
        let sink: Sink = {
            let answerer = answerer.clone();
            Arc::new(move |mut m: InboundMessage| {
                let thread = answerer.read().unwrap_or_else(|e| e.into_inner()).clone();
                if m.text.chars().count() > MAX_TEXT {
                    m.text = m.text.chars().take(MAX_TEXT).collect();
                }
                let deliver = deliver.clone();
                Box::pin(async move { deliver(thread, m).await })
            })
        };
        let hub = Arc::new(InboundHub { path, secrets, persisted: Mutex::new(persisted), answerer, table, webhook, slack, sink, errors: Mutex::new(HashMap::new()) });
        hub.reconcile();
        hub
    }

    fn sources(&self) -> [Arc<dyn InboundSource>; 2] {
        [self.webhook.clone(), self.slack.clone()]
    }

    /// Start the sources that are wanted and stop the ones that are not.
    fn reconcile(&self) {
        for s in self.sources() {
            let running = s.status().running;
            let mut errors = self.errors.lock().unwrap_or_else(|e| e.into_inner());
            if s.wanted() && !running {
                match s.start(self.sink.clone()) {
                    Ok(()) => drop(errors.remove(s.kind())),
                    Err(e) => {
                        tracing::warn!("inbound {} did not start: {e:#}", s.kind());
                        errors.insert(s.kind(), format!("{e:#}"));
                    }
                }
            } else if !s.wanted() {
                if running {
                    s.stop();
                }
                errors.remove(s.kind());
            }
        }
    }

    fn save(&self, p: &Persisted) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(p)?).context("write inbound.json")?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, &self.path).context("save inbound.json")?;
        Ok(())
    }

    fn url(&self, id: &str) -> String {
        format!("http://127.0.0.1:{}/hook/{id}", self.webhook.port())
    }

    fn view(&self, r: &EndpointRec) -> WebhookEndpoint {
        let (received, last_received_at, known) = self.table.stats(&r.id);
        WebhookEndpoint {
            id: r.id.clone(),
            name: r.name.clone(),
            enabled: r.enabled,
            created_at: r.created_at,
            url: self.url(&r.id),
            received,
            last_received_at,
            broken: !known,
            auto_run: r.auto_run,
        }
    }

    pub fn state(&self) -> InboundState {
        let p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
        let errors = self.errors.lock().unwrap_or_else(|e| e.into_inner());
        let wh = self.webhook.status();
        let (bot_set, app_set) = self.slack.tokens_set();
        let sl = self.slack.status();
        InboundState {
            answerer: self.answerer.read().unwrap_or_else(|e| e.into_inner()).clone(),
            port: self.webhook.port(),
            listening: wh.running,
            listen_error: errors.get("webhook").cloned(),
            endpoints: p.endpoints.iter().map(|r| self.view(r)).collect(),
            slack: SlackView {
                config: p.slack.clone(),
                bot_token_set: bot_set,
                app_token_set: app_set,
                connected: sl.running,
                status: errors.get("slack").cloned().or(sl.detail).unwrap_or_default(),
            },
            deliveries: self.table.deliveries(),
        }
    }

    /// A new endpoint, switched off. Its secret is returned here once and kept in the Keychain.
    pub fn create_webhook(&self, name: &str) -> Result<NewWebhook> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            bail!("give the endpoint a short name (1 to 60 characters)");
        }
        let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
        if p.endpoints.len() >= 20 {
            bail!("20 endpoints is the limit; revoke one you no longer use");
        }
        let id = random_token("wh")[..15].to_string();
        let secret = random_token("cwh");
        self.secrets.set(&secret_key(&id), &secret)?;
        let rec = EndpointRec { id: id.clone(), name: name.into(), enabled: false, created_at: now_ms(), auto_run: false };
        p.endpoints.push(rec.clone());
        if let Err(e) = self.save(&p) {
            p.endpoints.pop();
            let _ = self.secrets.delete(&secret_key(&id));
            return Err(e);
        }
        self.table.put(&id, name, false, false, Secret::new(secret.clone()));
        Ok(NewWebhook { endpoint: self.view(&rec), secret })
    }

    /// Switch an endpoint on or off. The listener starts with the first one on and stops with the last.
    pub fn set_webhook_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        {
            let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
            let Some(i) = p.endpoints.iter().position(|e| e.id == id) else { bail!("unknown endpoint") };
            if enabled && !self.table.has(id) {
                bail!("this endpoint's secret is missing from the Keychain; revoke it and make a new one");
            }
            let was = p.endpoints[i].enabled;
            p.endpoints[i].enabled = enabled;
            if let Err(e) = self.save(&p) {
                p.endpoints[i].enabled = was;
                return Err(e);
            }
            self.table.set_enabled(id, enabled);
        }
        self.reconcile();
        Ok(())
    }

    /// Let an endpoint run plans without asking, or take that back (the default). It applies to
    /// the messages that arrive from now on.
    pub fn set_webhook_auto_run(&self, id: &str, auto_run: bool) -> Result<()> {
        let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
        let Some(i) = p.endpoints.iter().position(|e| e.id == id) else { bail!("unknown endpoint") };
        let was = p.endpoints[i].auto_run;
        p.endpoints[i].auto_run = auto_run;
        if let Err(e) = self.save(&p) {
            p.endpoints[i].auto_run = was;
            return Err(e);
        }
        self.table.set_auto_run(id, auto_run);
        Ok(())
    }

    /// Delete an endpoint and its secret. Its URL stops working at once.
    pub fn revoke_webhook(&self, id: &str) -> Result<()> {
        {
            let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
            let Some(i) = p.endpoints.iter().position(|e| e.id == id) else { bail!("unknown endpoint") };
            self.table.remove(id);
            let rec = p.endpoints.remove(i);
            if let Err(e) = self.save(&p) {
                p.endpoints.insert(i, rec);
                return Err(e);
            }
            self.secrets.delete(&secret_key(id))?;
        }
        self.reconcile();
        Ok(())
    }

    /// Who answers inbound messages, and the listener's port (0 = any free port).
    pub fn set_settings(&self, answerer: &str, port: u16) -> Result<()> {
        if !valid_answerer(answerer) {
            bail!("inbound messages go to \"lead\" or to an agent (\"agent:<id>\")");
        }
        if port != 0 && port < 1024 {
            bail!("pick a port from 1024 up");
        }
        let restart = {
            let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
            p.answerer = Some(answerer.into());
            p.port = Some(port);
            self.save(&p)?;
            *self.answerer.write().unwrap_or_else(|e| e.into_inner()) = answerer.into();
            self.webhook.set_port(port)
        };
        if restart {
            self.webhook.stop();
        }
        self.reconcile();
        Ok(())
    }

    /// Save the Slack config, and remove a token an earlier version saved (`Some("")`). A new
    /// token is refused before anything changes: the connection is not built, so nothing would
    /// read it.
    pub fn set_slack(&self, u: SlackUpdate) -> Result<()> {
        if [&u.bot_token, &u.app_token].into_iter().any(|t| t.as_deref().is_some_and(|t| !t.is_empty())) {
            bail!("Slack is not available yet, so Codly does not take Slack tokens");
        }
        let cfg = u.config.cleaned();
        for (key, v) in [(slack::BOT_KEY, &u.bot_token), (slack::APP_KEY, &u.app_token)] {
            if v.is_some() {
                self.secrets.delete(key)?;
            }
        }
        {
            let mut p = self.persisted.lock().unwrap_or_else(|e| e.into_inner());
            p.slack = cfg.clone();
            self.save(&p)?;
        }
        self.slack.refresh_tokens(&*self.secrets);
        self.slack.set_config(cfg);
        self.slack.stop();
        self.reconcile();
        Ok(())
    }

    /// Stop every source.
    pub fn shutdown(&self) {
        for s in self.sources() {
            s.stop();
        }
    }
}

fn valid_answerer(a: &str) -> bool {
    a == "lead" || a.strip_prefix("agent:").is_some_and(|id| !id.trim().is_empty())
}

#[cfg(test)]
mod tests;
