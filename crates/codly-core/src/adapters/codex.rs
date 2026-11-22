//! Codex adapter: one shared `codex app-server` process per Codex account (JSON-RPC over
//! stdio, without the "jsonrpc" field), one thread per agent (cli-claude-codex §3.3, §5.3.2).
//! `CODEX_HOME` (auth, sessions, threads) is per process, so agents on different sign-ins
//! need different servers: they are keyed by the spawn env's `CODEX_HOME` (unset = default),
//! each with its own thread routing. Thread-less notifications (account rate limits) reach
//! only that account's agents.
//!
//! A server starts lazily with its account's first agent and is restarted by the next spawn
//! if it died (agents on a dead server get `Exited`; respawn them with `SessionMode::Resume`).
//! It exits by itself when its stdin closes, so it never outlives Codly.
//!
//! Verified for free against 0.161.0 with `--strict-config` (CRITIQUE §10): nested
//! `thread/start.config` objects are read as dotted `-c` keys; `mcp_servers.codly.{command,
//! args, env, default_tools_approval_mode}`, `features.memories` and `memories.*` are valid.
//!
//! A prompt sent while a turn runs is steered into it (`turn/steer`), so it does not get a
//! `TurnStarted`/`TurnCompleted` of its own: a `PromptSteered` says it was delivered. Codex
//! may report the turn's end before it answers the steer: the adapter keeps that end back
//! until the answer, so a `PromptSteered` always comes before its turn's `TurnCompleted`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use tokio::process::ChildStdin;
use tokio::sync::{Mutex, mpsc, oneshot};

use super::classify::{self, OUTPUT_MAX};
use super::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec, startup};
use crate::model::{
    ActivityKind, AgentStatus, Approval, ApprovalChoice, ApprovalDecision, ApprovalStatus, ApprovalSubject, Billing, EventKind,
    Id, MessagePhase, PlanStep, Provider, TurnStatus, Usage, new_id, now_ms,
};
use crate::proc::{self, RawLog, Record, StderrTail};

/// Notifications Codly never uses (opted out in `initialize`).
const OPT_OUT: &[&str] = &["remoteControl/status/changed"];

/// The account's usage windows. Codex sends it after every model response, changed or not.
const RATE_LIMITS: &str = "account/rateLimits/updated";
/// An unchanged `RATE_LIMITS` reading is passed on again after this long: a window without a
/// reset time counts only while it was seen recently.
const LIMITS_REFRESH_MS: i64 = 60_000;

/// How long Codex has to open a thread, and to answer `turn/start`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a turn has to end after `turn/interrupt`. Past either limit the thread is let go:
/// the run ends and the next one resumes the thread.
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(15);

/// One app-server per sign-in: its CODEX_HOME (None = unset) holds the account's auth and
/// threads, and its thread-less notifications (rate limits) concern that account only.
/// Each account has a slot with its own lock, held while its server starts (up to ~45 s),
/// so a slow start makes only that account's agents wait.
static SERVERS: Servers = Mutex::const_new(Vec::new());
/// Keyed by (CODEX_HOME, `auth_tag`): a change of BYOK credential gets a server of its own.
type Servers = Mutex<Vec<(ServerKey, Arc<Mutex<Option<Arc<AppServer>>>>)>>;
type ServerKey = (Option<String>, String);

/// Codly's own model provider for BYOK (byok.rs): the OpenAI API with the user's key or token
/// command, instead of the account's ChatGPT sign-in.
const BYOK_PROVIDER: &str = "codly_openai";
/// How often Codex re-runs a token command before the token can expire.
const BYOK_REFRESH_MS: u64 = 5 * 60 * 1000;

/// `-c` overrides that point Codex at `BYOK_PROVIDER`: a token command (`spec.byok_command`,
/// re-run by Codex) or the saved key (OPENAI_API_KEY in the env, present only with BYOK on since
/// `env::child_env` scrubs it otherwise). Empty = the account's own sign-in.
fn byok_overrides(spec: &SpawnSpec) -> Vec<String> {
    let q = |v: &str| serde_json::Value::from(v).to_string(); // a JSON string is a TOML basic string
    let auth: Vec<(String, String)> = if let Some(cmd) = &spec.byok_command {
        vec![
            ("auth.command".into(), q("/bin/sh")),
            ("auth.args".into(), serde_json::json!(["-c", cmd]).to_string()),
            ("auth.timeout_ms".into(), crate::byok::COMMAND_TIMEOUT.as_millis().to_string()),
            ("auth.refresh_interval_ms".into(), BYOK_REFRESH_MS.to_string()),
        ]
    } else if spec.env.contains_key(crate::byok::Vendor::Openai.env_var()) {
        vec![("env_key".into(), q(crate::byok::Vendor::Openai.env_var()))]
    } else {
        return vec![];
    };
    let base = [("name", q("OpenAI (your credentials)")), ("base_url", q("https://api.openai.com/v1")), ("wire_api", q("responses"))];
    let mut a = Vec::new();
    for (k, v) in base.into_iter().map(|(k, v)| (k.to_string(), v)).chain(auth) {
        a.extend(["-c".to_string(), format!("model_providers.{BYOK_PROVIDER}.{k}={v}")]);
    }
    a.extend(["-c".to_string(), format!("model_provider={}", q(BYOK_PROVIDER))]);
    a
}

/// Which credential a server was started with, without the key itself (a hash, in memory only).
fn auth_tag(spec: &SpawnSpec) -> String {
    use std::hash::{Hash, Hasher};
    if let Some(cmd) = &spec.byok_command {
        return format!("cmd:{cmd}");
    }
    match spec.env.get(crate::byok::Vendor::Openai.env_var()) {
        Some(key) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            format!("key:{:x}", h.finish())
        }
        None => String::new(),
    }
}

#[derive(Debug, Clone)]
struct ServerInfo {
    cli_version: Option<String>,
    billing: Billing,
}

enum Waiter {
    Oneshot(oneshot::Sender<Result<Value, Value>>, Arc<RawLog>),
    /// Delivered in stream order on the agent's channel. `register`: the result is a thread
    /// (`thread/start`/`resume`) whose id is routed to this agent before anything else is read.
    Agent { route: Route, register: bool },
}

#[derive(Clone)]
struct Route {
    tx: mpsc::UnboundedSender<ServerMsg>,
    raw: Arc<RawLog>,
}

enum ServerMsg {
    Response { id: i64, result: Result<Value, Value> },
    Notification { method: String, params: Value },
    Request { id: Value, method: String, params: Value },
    Exited { code: Option<i64>, stderr: String },
}

#[derive(Default)]
struct State {
    pending: HashMap<i64, Waiter>,
    /// threadId → the agent that owns it
    routes: HashMap<String, Route>,
    /// The last `RATE_LIMITS` params passed on to the agents, and when.
    last_limits: Option<(Value, i64)>,
}

impl State {
    /// Is this `RATE_LIMITS` reading worth passing on: a new one, or the same one seen long ago?
    fn limits_news(&mut self, params: &Value, now: i64) -> bool {
        if self.last_limits.as_ref().is_some_and(|(last, at)| last == params && now - at < LIMITS_REFRESH_MS) {
            return false;
        }
        self.last_limits = Some((params.clone(), now));
        true
    }
}

struct AppServer {
    stdin: Mutex<Option<ChildStdin>>,
    next_id: AtomicI64,
    state: StdMutex<State>,
    alive: AtomicBool,
    /// Codly gave this server up (`give_up`): its process is being ended, and no new agent
    /// may start on it.
    abandoned: AtomicBool,
    /// When the server last wrote a line (ms): how long it has said nothing to anybody.
    last_inbound_ms: AtomicI64,
    pgid: i32,
    /// With `pgid`, what every run on this server records for crash recovery.
    started_ms: i64,
    stderr: StderrTail,
    info: StdMutex<ServerInfo>,
    /// Raw log of the agent that started the server: lines that belong to nobody else.
    bootstrap_raw: Arc<RawLog>,
}

impl AppServer {
    async fn start(spec: &SpawnSpec, raw: Arc<RawLog>) -> anyhow::Result<Arc<AppServer>> {
        let mut env = spec.env.clone();
        env.insert("NO_COLOR".into(), "1".into()); // its tracing logs on stderr are ANSI otherwise
        let cwd = spec.env.get("HOME").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| spec.cwd.clone());
        let mut args = byok_overrides(spec);
        args.push("app-server".into());
        let mut child = proc::command(&spec.bin, &args, &cwd, &env)
            .spawn()
            .with_context(|| format!("failed to start {} app-server", spec.bin.display()))?;
        let server = Arc::new(AppServer {
            stdin: Mutex::new(child.stdin.take()),
            next_id: AtomicI64::new(0),
            state: StdMutex::new(State::default()),
            alive: AtomicBool::new(true),
            abandoned: AtomicBool::new(false),
            last_inbound_ms: AtomicI64::new(now_ms()),
            pgid: child.id().map(|p| p as i32).unwrap_or(0),
            started_ms: now_ms(),
            stderr: StderrTail::spawn(child.stderr.take().context("no stderr")?, 64 << 10),
            info: StdMutex::new(ServerInfo { cli_version: None, billing: Billing::Unknown }),
            bootstrap_raw: raw.clone(),
        });
        let reader = server.clone();
        proc::spawn_reader(child.stdout.take().context("no stdout")?, move |rec| reader.on_record(rec));
        let waiter = server.clone();
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(proc::exit_code);
            waiter.on_exit(code);
        });

        let caps = json!({ "experimentalApi": false, "optOutNotificationMethods": OPT_OUT });
        let client = json!({ "name": "codly", "title": "Codly", "version": crate::version() });
        let init = server.call("initialize", json!({ "clientInfo": client, "capabilities": caps }), &raw, Duration::from_secs(30)).await;
        let init = match init {
            Ok(v) => v,
            Err(e) => {
                let tail = server.stderr.tail(1500);
                server.close().await;
                return Err(if tail.is_empty() { e } else { e.context(tail) });
            }
        };
        server.send(&json!({ "method": "initialized" }), &raw).await?;
        let account = server.call("account/read", json!({ "refreshToken": false }), &raw, Duration::from_secs(15)).await.ok();
        // Detection can be stale (signed out since): say so now, not as a token error mid-turn.
        if !startup::uses_own_key(spec) && account.as_ref().is_some_and(signed_out) {
            server.close().await;
            return Err(anyhow!(startup::not_signed_in(Provider::Codex, startup::account_home(spec), None)));
        }
        *server.info.lock().unwrap() = ServerInfo {
            // "codly/0.161.0 (Mac OS 26.5.2; arm64) dumb (codly; 0.1.0)"
            cli_version: init["userAgent"].as_str().and_then(|ua| ua.split_whitespace().next()).and_then(|t| t.split_once('/')).map(|(_, v)| v.to_string()),
            billing: match account.as_ref().map(|a| a["account"]["type"].as_str().unwrap_or("")) {
                Some("chatgpt") => Billing::Subscription,
                Some("apiKey") => Billing::ApiKey,
                Some("amazonBedrock") => Billing::ThirdParty,
                _ => Billing::Unknown,
            },
        };
        Ok(server)
    }

    fn on_record(self: &Arc<Self>, rec: Record<'_>) {
        match rec {
            Record::Line(bytes) => self.route_line(bytes),
            Record::TooLong(n) => self.bootstrap_raw.note(&format!("dropped a {n}-byte record from codex app-server")),
            Record::Eof => {}
        }
    }

    /// Write each line to the owning agent's raw log, then hand it over in stream order.
    fn route_line(self: &Arc<Self>, bytes: &[u8]) {
        self.last_inbound_ms.store(now_ms(), Ordering::SeqCst);
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            self.bootstrap_raw.inbound(bytes);
            return;
        };
        let mut st = self.state.lock().unwrap();
        let method = v.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = v.get("id").cloned();
        match (method, id) {
            (None, Some(id)) => {
                let id = id.as_i64().unwrap_or(-1);
                let result = match v.get("error") {
                    Some(e) if !e.is_null() => Err(e.clone()),
                    _ => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                };
                match st.pending.remove(&id) {
                    Some(Waiter::Oneshot(tx, raw)) => {
                        raw.inbound(bytes);
                        let _ = tx.send(result);
                    }
                    Some(Waiter::Agent { route, register }) => {
                        route.raw.inbound(bytes);
                        if register
                            && let Ok(r) = &result
                            && let Some(tid) = r["thread"]["id"].as_str()
                        {
                            st.routes.insert(tid.to_string(), route.clone());
                        }
                        let _ = route.tx.send(ServerMsg::Response { id, result });
                    }
                    None => self.bootstrap_raw.inbound(bytes),
                }
            }
            (Some(method), id) => {
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                let tid = params["threadId"].as_str().or(params["thread"]["id"].as_str()).map(str::to_owned);
                if let Some(route) = tid.as_deref().and_then(|t| st.routes.get(t)).cloned() {
                    route.raw.inbound(bytes);
                    let msg = match id {
                        Some(id) => ServerMsg::Request { id, method, params },
                        None => ServerMsg::Notification { method, params },
                    };
                    let _ = route.tx.send(msg);
                } else if let Some(id) = id {
                    // A server request no agent can answer: refuse it, or that thread waits forever.
                    self.bootstrap_raw.inbound(bytes);
                    let me = self.clone();
                    tokio::spawn(async move {
                        let msg = json!({ "id": id, "error": { "code": -32601, "message": format!("Codly has no handler for {method}") } });
                        let raw = me.bootstrap_raw.clone();
                        let _ = me.send(&msg, &raw).await;
                    });
                } else if tid.is_some() {
                    // A thread nobody owns (any more): its `turn/completed` must not end another agent's turn.
                    self.bootstrap_raw.inbound(bytes);
                } else if method == "account/updated" {
                    // Who is signed in: this server's own state, nothing for an agent to report.
                    self.bootstrap_raw.inbound(bytes);
                    if let Some(billing) = params["authMode"].as_str().and_then(auth_mode_billing) {
                        self.info.lock().unwrap().billing = billing;
                    }
                } else if method == RATE_LIMITS && !st.routes.is_empty() && !st.limits_news(&params, now_ms()) {
                    // The reading the agents already have: in the raw log once, reported by nobody.
                    self.bootstrap_raw.inbound(bytes);
                } else {
                    // Thread-less notifications (account rate limits, warnings) concern every agent.
                    let routes: Vec<Route> = st.routes.values().cloned().collect();
                    if routes.is_empty() {
                        self.bootstrap_raw.inbound(bytes);
                    }
                    for r in routes {
                        r.raw.inbound(bytes);
                        let _ = r.tx.send(ServerMsg::Notification { method: method.clone(), params: params.clone() });
                    }
                }
            }
            (None, None) => self.bootstrap_raw.inbound(bytes),
        }
    }

    fn on_exit(&self, code: Option<i64>) {
        self.alive.store(false, Ordering::SeqCst);
        let tail = self.stderr.tail(2000);
        let mut st = self.state.lock().unwrap();
        for (id, w) in st.pending.drain() {
            if let Waiter::Agent { route, .. } = w {
                let _ = route.tx.send(ServerMsg::Response { id, result: Err(json!({ "message": "codex app-server exited" })) });
            } // dropping a Oneshot sender fails its receiver
        }
        for (_, r) in st.routes.drain() {
            let _ = r.tx.send(ServerMsg::Exited { code, stderr: tail.clone() });
        }
    }

    async fn send(&self, msg: &Value, raw: &RawLog) -> std::io::Result<()> {
        self.send_within(msg, raw, proc::WRITE_TIMEOUT).await
    }

    /// Every agent of the account writes through this one lock, so a write may never wait
    /// without end: a server that does not take a line within `limit` has stopped reading. It
    /// is given up, and every later write fails at once instead of waiting behind this one.
    async fn send_within(&self, msg: &Value, raw: &RawLog, limit: Duration) -> std::io::Result<()> {
        let mut stdin = self.stdin.lock().await;
        let Some(w) = stdin.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "codex app-server stdin is closed"));
        };
        let sent = proc::write_json_within(w, raw, msg, limit).await;
        if sent.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::TimedOut) {
            *stdin = None;
            self.give_up("it stopped reading its input");
        }
        sent
    }

    /// Usable for a new agent: its process runs and Codly did not give it up.
    fn usable(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.abandoned.load(Ordering::SeqCst)
    }

    /// How long the server has written nothing at all (ms).
    fn silent_ms(&self) -> i64 {
        now_ms() - self.last_inbound_ms.load(Ordering::SeqCst)
    }

    /// The server is hung, or runs a turn nobody can stop: end its process. Its agents receive
    /// `Exited` when it is gone, and the next agent of the account starts a new server.
    fn give_up(&self, why: &str) {
        if self.abandoned.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::warn!("stopping a codex app-server: {why}");
        self.bootstrap_raw.note(&format!("stopping the codex app-server: {why}"));
        tokio::spawn(proc::kill_group(self.pgid, Duration::from_secs(2)));
    }

    /// `give_up` when no agent is on this server and nothing else waits for an answer from it
    /// (an agent that starts, an account read). Requests of `mine`, the agent that leaves, do
    /// not count: its own unanswered ones are why it leaves.
    fn give_up_if_unused(&self, mine: &Route, why: &str) {
        let unused = {
            let st = self.state.lock().unwrap();
            st.routes.is_empty() && st.pending.values().all(|w| matches!(w, Waiter::Agent { route, .. } if route.tx.same_channel(&mine.tx)))
        };
        if unused {
            self.give_up(why);
        }
    }

    async fn request(&self, method: &str, params: Value, waiter: Waiter, raw: &RawLog) -> std::io::Result<i64> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.state.lock().unwrap().pending.insert(id, waiter);
        if let Err(e) = self.send(&json!({ "method": method, "id": id, "params": params }), raw).await {
            self.state.lock().unwrap().pending.remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    async fn call(&self, method: &str, params: Value, raw: &Arc<RawLog>, timeout: Duration) -> anyhow::Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.request(method, params, Waiter::Oneshot(tx, raw.clone()), raw).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("{method} failed: {}", e["message"].as_str().map(str::to_owned).unwrap_or_else(|| e.to_string()))),
            Ok(Err(_)) => Err(anyhow!("codex app-server exited during {method}")),
            Err(_) => Err(anyhow!("codex app-server did not answer {method} within {} s", timeout.as_secs())),
        }
    }

    fn unroute(&self, thread_id: &str) {
        self.state.lock().unwrap().routes.remove(thread_id);
    }

    /// Close stdin (the server exits on EOF), then SIGTERM → SIGKILL if it lingers. A write
    /// that is still under way holds stdin: after 1 s the close goes straight to the signals.
    async fn close(&self) {
        if let Ok(mut stdin) = tokio::time::timeout(Duration::from_secs(1), self.stdin.lock()).await {
            stdin.take();
            drop(stdin);
            for _ in 0..30 {
                if !self.alive.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        proc::kill_group(self.pgid, Duration::from_secs(3)).await;
    }
}

/// `account/read` when nobody is signed in and the model provider needs an OpenAI sign-in.
fn signed_out(account: &Value) -> bool {
    account["account"].is_null() && account["requiresOpenaiAuth"] == true
}

/// Who pays under an `account/updated.authMode`. None = a mode Codly does not know: the
/// billing `account/read` gave stays.
fn auth_mode_billing(mode: &str) -> Option<Billing> {
    match mode {
        "chatgpt" => Some(Billing::Subscription),
        "apikey" => Some(Billing::ApiKey),
        _ => None,
    }
}

async fn server_for(servers: &Servers, spec: &SpawnSpec, raw: &Arc<RawLog>) -> anyhow::Result<Arc<AppServer>> {
    let key: ServerKey = (spec.env.get("CODEX_HOME").cloned(), auth_tag(spec));
    // The list is locked for the lookup only; the start below holds the account's slot.
    let slot = {
        let mut guard = servers.lock().await;
        match guard.iter().find(|(k, _)| *k == key) {
            Some((_, slot)) => slot.clone(),
            None => {
                let slot = Arc::default();
                guard.push((key, Arc::clone(&slot)));
                slot
            }
        }
    };
    let mut cur = slot.lock().await;
    if let Some(s) = cur.as_ref().filter(|s| s.usable()) {
        return Ok(s.clone());
    }
    let s = AppServer::start(spec, raw.clone()).await?;
    *cur = Some(s.clone()); // in place of the dead or hung one, if any
    Ok(s)
}

/// Stop every app-server (call at app exit). Their agents receive `Exited`.
pub async fn shutdown_server() {
    let mut set = tokio::task::JoinSet::new();
    for (_, slot) in SERVERS.lock().await.drain(..) {
        set.spawn(async move {
            if let Some(s) = slot.lock().await.take() {
                s.close().await
            }
        });
    }
    set.join_all().await;
}

/// `thread/start` (or `thread/resume`) and its params for one agent.
pub fn thread_request(spec: &SpawnSpec, model: Option<&str>, effort: Option<&str>) -> (&'static str, Value) {
    let mut config = json!({
        "model_reasoning_summary": "auto",
        // Codly keeps its own memory; Codly threads must not feed the user's Codex memories
        // (memory-knowledge §5.6).
        "features": { "memories": false },
        "memories": { "generate_memories": false, "use_memories": false },
    });
    if let Some(e) = effort {
        config["model_reasoning_effort"] = json!(e);
    }
    if let Some(m) = &spec.mcp {
        let env: serde_json::Map<String, Value> = m.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        // Codly validates every call server-side, so its tools need no extra approval prompt.
        config["mcp_servers"] = json!({ "codly": { "command": m.command, "args": m.args, "env": env, "default_tools_approval_mode": "approve" } });
        // The user's default servers (Settings → Connections), as enabled when this run started.
        // Read-only ones run without asking; the others use Codex's own approval prompt.
        for x in crate::housekeeping::connections::load_ready(&spec.raw_log) {
            let env: serde_json::Map<String, Value> = x.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
            let mut server = json!({ "command": x.command, "args": x.args, "env": env });
            if x.read_only {
                server["default_tools_approval_mode"] = json!("approve");
            }
            config["mcp_servers"][&x.name] = server;
        }
    }
    let mut p = json!({
        "cwd": spec.cwd,
        "approvalPolicy": "on-request",
        "sandbox": if spec.read_only { "read-only" } else { "workspace-write" },
        "config": config,
    });
    if let Some(m) = model {
        p["model"] = json!(m);
    }
    if !spec.system_prompt.trim().is_empty() {
        p["developerInstructions"] = json!(spec.system_prompt);
    }
    match &spec.session {
        SessionMode::Resume { session_id } => {
            p["threadId"] = json!(session_id);
            p["excludeTurns"] = json!(true);
            ("thread/resume", p)
        }
        // Codex assigns thread ids itself; a pre-assigned id cannot be honoured.
        SessionMode::New { .. } => {
            p["threadSource"] = json!("codly");
            ("thread/start", p)
        }
    }
}

pub async fn spawn(spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> anyhow::Result<AgentHandle> {
    let raw = Arc::new(RawLog::open(&spec.raw_log).with_context(|| format!("cannot open raw log {}", spec.raw_log.display()))?);
    let server = server_for(&SERVERS, &spec, &raw).await.map_err(|e| startup::explain(&spec, None, e))?;
    proc::record_group(&spec.raw_log, server.pgid, server.started_ms);
    let (msg_tx, msgs) = mpsc::unbounded_channel();
    let route = Route { tx: msg_tx, raw: raw.clone() };
    let (model, effort) = classify::split_model_effort(spec.model.as_deref());
    let (method, params) = thread_request(&spec, model.as_deref(), effort.as_deref());
    let start_id = server.request(method, params, Waiter::Agent { route: route.clone(), register: true }, &raw).await?;

    let (tx, cmds) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let resumed = matches!(spec.session, SessionMode::Resume { .. });
    let mut agent = CodexAgent {
        agent_id: spec.agent_id.clone(),
        run_id: spec.run_id.clone(),
        cwd: spec.cwd.to_string_lossy().into_owned(),
        events,
        server: server.clone(),
        route,
        raw: raw.clone(),
        effort,
        thread_id: None,
        started: false,
        norm: CodexNormalizer::new(&spec.agent_id, &spec.run_id, spec.cwd.clone(), resumed),
        pending: HashMap::new(),
        active_turn: None,
        turn_pending: false,
        interrupt_after_start: false,
        steer_queue: Vec::new(),
        queue: VecDeque::new(),
        held: Vec::new(),
        stage: Stage::Running,
        deadline: None,
    };
    agent.pending.insert(start_id, Pending::Thread);
    tokio::spawn(agent.run(cmds, msgs, ready_tx));
    match tokio::time::timeout(START_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(()))) => Ok(AgentHandle { agent_id: spec.agent_id, run_id: spec.run_id, tx }),
        Ok(Ok(Err(e))) => Err(startup::explain(&spec, server.info.lock().unwrap().cli_version.as_deref(), e)),
        Ok(Err(_)) => Err(anyhow!("codex adapter stopped during startup")),
        Err(_) => {
            let _ = tx.send(AgentCmd::Shutdown);
            // A server that said nothing to anybody all that time is hung: the next start gets
            // a new one, instead of waiting 60 s behind this one again.
            if server.silent_ms() >= START_TIMEOUT.as_millis() as i64 {
                server.give_up("it answered nothing while a thread waited to open");
            }
            // The thread may still open, and by then its adapter is gone: take the answer
            // here and unsubscribe, so the thread is neither routed to a dead channel nor
            // left loaded on the shared server. (The server's exit drops the waiter.)
            let (late_tx, late) = oneshot::channel();
            if let Some(w) = server.state.lock().unwrap().pending.get_mut(&start_id) {
                *w = Waiter::Oneshot(late_tx, raw.clone());
            }
            tokio::spawn(async move {
                if let Ok(Ok(r)) = late.await
                    && let Some(tid) = r["thread"]["id"].as_str()
                {
                    let _ = server.call("thread/unsubscribe", json!({ "threadId": tid }), &raw, Duration::from_secs(3)).await;
                }
            });
            Err(anyhow!("codex app-server did not answer {method} within {} s", START_TIMEOUT.as_secs()))
        }
    }
}

enum Pending {
    Thread,
    TurnStart,
    Steer(String),
    Interrupt,
    Unsubscribe,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Running,
    Interrupting,
    Unsubscribing,
    Done,
}

struct CodexAgent {
    agent_id: Id,
    run_id: Id,
    cwd: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
    server: Arc<AppServer>,
    route: Route,
    raw: Arc<RawLog>,
    effort: Option<String>,
    thread_id: Option<String>,
    /// `spawn` got its Ok. If it gave up first (60 s), it failed the run itself and this
    /// adapter must not report on it any more.
    started: bool,
    norm: CodexNormalizer,
    pending: HashMap<i64, Pending>,
    active_turn: Option<String>,
    /// `turn/start` sent, its response (the turn id) not yet back.
    turn_pending: bool,
    interrupt_after_start: bool,
    /// Prompts sent while the turn id is unknown: steered in once it is.
    steer_queue: Vec<String>,
    /// Prompts that could not be steered (the turn ended first): new turns.
    queue: VecDeque<String>,
    /// A `turn/completed` that came while a `turn/steer` was unanswered, and the notifications
    /// after it: handled, in order, once the last steer is answered.
    held: Vec<(String, Value)>,
    stage: Stage,
    /// While running: when `turn/start` must be answered, or an interrupted turn must have
    /// ended. In a shutdown: the stage's limit.
    deadline: Option<tokio::time::Instant>,
}

impl CodexAgent {
    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(AdapterEvent { agent_id: self.agent_id.clone(), run_id: self.run_id.clone(), kind });
    }

    fn emit_all(&self, kinds: Vec<EventKind>) {
        for k in kinds {
            self.emit(k);
        }
    }

    async fn request(&mut self, method: &str, params: Value, what: Pending) -> bool {
        let waiter = Waiter::Agent { route: self.route.clone(), register: false };
        match self.server.request(method, params, waiter, &self.raw).await {
            Ok(id) => {
                self.pending.insert(id, what);
                true
            }
            Err(e) => {
                self.emit(EventKind::Error { message: format!("{method}: {e}"), fatal: false });
                false
            }
        }
    }

    async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<AgentCmd>, mut msgs: mpsc::UnboundedReceiver<ServerMsg>, ready: oneshot::Sender<anyhow::Result<()>>) {
        let mut ready = Some(ready);
        let mut cmds_open = true;
        while self.stage != Stage::Done {
            let deadline = self.deadline;
            let timer = async move {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                msg = msgs.recv() => match msg {
                    Some(ServerMsg::Response { id, result }) => self.on_response(id, result, &mut ready).await,
                    Some(ServerMsg::Notification { method, params }) => self.on_notification(&method, &params).await,
                    Some(ServerMsg::Request { id, method, params }) => {
                        let (events, reply) = self.norm.on_request(&id, &method, &params);
                        self.emit_all(events);
                        if let Some(reply) = reply {
                            let _ = self.server.send(&reply, &self.raw).await;
                        }
                    }
                    Some(ServerMsg::Exited { code, stderr }) => self.server_exited(code, &stderr, &mut ready),
                    None => self.stage = Stage::Done,
                },
                cmd = cmds.recv(), if cmds_open => match cmd {
                    Some(cmd) => self.on_cmd(cmd).await,
                    None => { cmds_open = false; self.begin_shutdown().await }
                },
                _ = timer => self.on_deadline().await,
            }
        }
    }

    async fn on_response(&mut self, id: i64, result: Result<Value, Value>, ready: &mut Option<oneshot::Sender<anyhow::Result<()>>>) {
        let Some(what) = self.pending.remove(&id) else { return };
        let err_text = |e: &Value| e["message"].as_str().map(str::to_owned).unwrap_or_else(|| e.to_string());
        match (what, result) {
            (Pending::Thread, Ok(r)) => {
                self.thread_id = r["thread"]["id"].as_str().map(str::to_owned);
                // The spawner gave up: no session to announce, only a thread to let go.
                let Some(waiting) = ready.take().filter(|w| !w.is_closed()) else {
                    self.unsubscribe().await;
                    return;
                };
                let info = self.server.info.lock().unwrap().clone();
                self.emit(EventKind::SessionStarted {
                    provider: Provider::Codex,
                    model: r["model"].as_str().map(str::to_owned),
                    cwd: r["cwd"].as_str().map(str::to_owned).unwrap_or_else(|| self.cwd.clone()),
                    cli_version: info.cli_version,
                    session_id: self.thread_id.clone(),
                    billing: info.billing,
                    tools: Vec::new(),
                });
                self.started = waiting.send(Ok(())).is_ok();
            }
            (Pending::Thread, Err(e)) => {
                if let Some(r) = ready.take() {
                    let _ = r.send(Err(anyhow!("codex could not open the thread: {}", err_text(&e))));
                }
                self.stage = Stage::Done;
            }
            (Pending::TurnStart, Ok(r)) => {
                self.turn_pending = false;
                self.active_turn = r["turn"]["id"].as_str().map(str::to_owned);
                self.answered_in_time();
                if self.interrupt_after_start {
                    self.interrupt_after_start = false;
                    self.interrupt().await;
                }
                for text in std::mem::take(&mut self.steer_queue) {
                    self.steer(text).await;
                }
            }
            (Pending::TurnStart, Err(e)) => {
                self.turn_pending = false;
                self.answered_in_time();
                self.emit(EventKind::Error { message: format!("codex refused the turn: {}", err_text(&e)), fatal: false });
                let events = self.norm.turn_failed();
                self.emit_all(events);
                self.queue.extend(std::mem::take(&mut self.steer_queue));
                self.next_turn().await;
            }
            // The turn ended before the steer landed: run it as the next turn instead.
            (Pending::Steer(text), Err(_)) => {
                self.queue.push_back(text);
                self.release_held().await;
                if self.active_turn.is_none() && !self.turn_pending {
                    self.next_turn().await;
                }
            }
            // In the running turn now: it gets no turn events, so say it was delivered.
            (Pending::Steer(text), Ok(_)) => {
                self.emit(EventKind::PromptSteered { input: text });
                self.release_held().await;
            }
            (Pending::Unsubscribe, _) => self.finish(),
            (Pending::Interrupt, _) => {}
        }
    }

    fn steer_unanswered(&self) -> bool {
        self.pending.values().any(|p| matches!(p, Pending::Steer(_)))
    }

    /// Codex answers a steer and ends a turn on two streams, in no fixed order. A turn's end
    /// that comes before the answer waits for it: a `PromptSteered` after its `TurnCompleted`
    /// would tell Codly the agent ended its turn with a prompt still to come, and no later
    /// turn would end to say it is done. What follows the end waits behind it, to stay in order.
    async fn on_notification(&mut self, method: &str, params: &Value) {
        if !self.held.is_empty() || (method == "turn/completed" && self.steer_unanswered()) {
            self.held.push((method.to_owned(), params.clone()));
            return;
        }
        self.handle_notification(method, params).await;
    }

    /// Every steer is answered (Codex answers each request, and a server that exits fails the
    /// open ones): handle what waited for that.
    async fn release_held(&mut self) {
        if self.steer_unanswered() {
            return;
        }
        for (method, params) in std::mem::take(&mut self.held) {
            self.handle_notification(&method, &params).await;
        }
    }

    async fn handle_notification(&mut self, method: &str, params: &Value) {
        let events = self.norm.on_notification(method, params);
        self.emit_all(events);
        if method == "turn/completed" {
            self.active_turn = None;
            self.answered_in_time();
            match self.stage {
                Stage::Interrupting => self.unsubscribe().await,
                Stage::Running => self.next_turn().await,
                _ => {}
            }
        }
    }

    async fn on_cmd(&mut self, cmd: AgentCmd) {
        match cmd {
            AgentCmd::Prompt(text) => {
                if self.stage != Stage::Running {
                    return;
                }
                if self.active_turn.is_some() {
                    self.steer(text).await;
                } else if self.turn_pending {
                    self.steer_queue.push(text);
                } else {
                    self.start_turn(text).await;
                }
            }
            AgentCmd::Interrupt => self.interrupt().await,
            AgentCmd::Approval { approval_id, decision } => match self.norm.answer(&approval_id, &decision) {
                Some((reply, events)) => match self.server.send(&reply, &self.raw).await {
                    Ok(()) => self.emit_all(events),
                    Err(e) => {
                        // The answer never reached the app-server: the card must not say it did.
                        tracing::warn!("codex: approval answer not sent: {e}");
                        self.emit(EventKind::ApprovalResolved { approval_id, status: ApprovalStatus::Expired, by: "provider".into() });
                    }
                },
                None => tracing::warn!("codex: no pending approval {approval_id}"),
            },
            AgentCmd::Shutdown => self.begin_shutdown().await,
        }
    }

    fn input(text: &str) -> Value {
        json!([{ "type": "text", "text": text, "text_elements": [] }])
    }

    async fn start_turn(&mut self, text: String) {
        let Some(tid) = self.thread_id.clone() else { return };
        let mut params = json!({ "threadId": tid, "input": Self::input(&text) });
        if let Some(e) = &self.effort {
            params["effort"] = json!(e);
        }
        if self.request("turn/start", params, Pending::TurnStart).await {
            self.turn_pending = true;
            self.wait_at_most(START_TIMEOUT);
            let events = self.norm.turn_started(&text);
            self.emit_all(events);
        }
    }

    /// While running, wait `limit` for what was just asked (a turn to start, a turn to end);
    /// `on_deadline` lets the thread go when it does not come. A shutdown keeps its own limits.
    fn wait_at_most(&mut self, limit: Duration) {
        if self.stage == Stage::Running {
            self.deadline = Some(tokio::time::Instant::now() + limit);
        }
    }

    /// What `wait_at_most` waited for came.
    fn answered_in_time(&mut self) {
        if self.stage == Stage::Running {
            self.deadline = None;
        }
    }

    async fn next_turn(&mut self) {
        if let Some(text) = self.queue.pop_front() {
            self.start_turn(text).await;
        }
    }

    async fn steer(&mut self, text: String) {
        let (Some(tid), Some(turn)) = (self.thread_id.clone(), self.active_turn.clone()) else {
            self.queue.push_back(text);
            return;
        };
        let params = json!({ "threadId": tid, "input": Self::input(&text), "expectedTurnId": turn });
        self.request("turn/steer", params, Pending::Steer(text)).await;
    }

    async fn interrupt(&mut self) {
        match (self.thread_id.clone(), self.active_turn.clone()) {
            (Some(tid), Some(turn)) => {
                self.norm.interrupt_requested = true;
                self.request("turn/interrupt", json!({ "threadId": tid, "turnId": turn }), Pending::Interrupt).await;
                self.wait_at_most(INTERRUPT_TIMEOUT);
            }
            // The limit of its `turn/start` still runs.
            _ if self.turn_pending => self.interrupt_after_start = true,
            _ => {}
        }
    }

    /// interrupt the turn (≤ 5 s) → thread/unsubscribe (≤ 3 s) → done. The server keeps
    /// running for the other agents.
    async fn begin_shutdown(&mut self) {
        if self.stage != Stage::Running {
            return;
        }
        self.queue.clear();
        self.steer_queue.clear();
        if self.active_turn.is_some() || self.turn_pending {
            self.interrupt().await;
            self.stage = Stage::Interrupting;
            self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
        } else {
            self.unsubscribe().await;
        }
    }

    async fn unsubscribe(&mut self) {
        self.stage = Stage::Unsubscribing;
        self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(3));
        match self.thread_id.clone() {
            Some(tid) => {
                if !self.request("thread/unsubscribe", json!({ "threadId": tid }), Pending::Unsubscribe).await {
                    self.finish();
                }
            }
            None => self.finish(),
        }
    }

    async fn on_deadline(&mut self) {
        self.deadline = None;
        match self.stage {
            Stage::Interrupting => self.unsubscribe().await,
            Stage::Unsubscribing => self.finish(),
            Stage::Running => self.let_go().await,
            Stage::Done => {}
        }
    }

    /// `turn/start` got no answer, or the turn did not end after `turn/interrupt`: the thread
    /// is stuck. Codly says so and ends the run (`Exited`); the next run resumes the thread. A
    /// turn that never started ends as failed, one that would not stop as interrupted.
    async fn let_go(&mut self) {
        let unsent = super::unsent_note(self.queue.len() + self.steer_queue.len());
        self.queue.clear();
        self.steer_queue.clear();
        let what = if self.turn_pending {
            format!("Codex did not start the turn within {} s", START_TIMEOUT.as_secs())
        } else {
            format!("Codex did not stop within {} s", INTERRUPT_TIMEOUT.as_secs())
        };
        self.emit(EventKind::Error { message: format!("{what}, so Codly ended this run. Your next message resumes the session.{unsent}"), fatal: false });
        if std::mem::take(&mut self.turn_pending) {
            // An answer that comes after all starts nothing here: the turn is over for Codly.
            self.pending.retain(|_, what| !matches!(what, Pending::TurnStart));
            let events = self.norm.turn_failed();
            self.emit_all(events);
            // Nothing from the server for anybody all that time: it is hung for every agent on it.
            if self.server.silent_ms() >= START_TIMEOUT.as_millis() as i64 {
                self.server.give_up("it answered nothing while a turn waited to start");
            }
        }
        self.unsubscribe().await;
    }

    fn finish(&mut self) {
        if self.stage == Stage::Done {
            return;
        }
        if let Some(tid) = &self.thread_id {
            self.server.unroute(tid);
        }
        let open = self.active_turn.take().is_some() || std::mem::take(&mut self.turn_pending);
        if self.started {
            let events = self.norm.cancel_all_approvals();
            self.emit_all(events);
            // Once unrouted, Codex's own `turn/completed` cannot reach us: close the turn here.
            // Interrupted, not Failed: this is a shutdown Codly asked for, and a failed turn
            // would send a stopped worker to the gate.
            if open {
                self.emit(EventKind::TurnCompleted { status: TurnStatus::Interrupted, result_text: None, duration_ms: None });
            }
            self.emit(EventKind::StatusChanged { status: AgentStatus::Offline });
            self.emit(EventKind::Exited { code: None });
        }
        // The turn never ended, so it may still run on the server, with nobody to stop it or
        // to read it. A server that no other agent uses is ended with it.
        if open {
            self.server.give_up_if_unused(&self.route, "a turn did not stop and no other agent uses the server");
        }
        self.stage = Stage::Done;
    }

    fn server_exited(&mut self, code: Option<i64>, stderr: &str, ready: &mut Option<oneshot::Sender<anyhow::Result<()>>>) {
        self.stage = Stage::Done;
        if let Some(r) = ready.take() {
            let _ = r.send(Err(anyhow!("codex app-server exited during startup ({}): {stderr}", startup::exit_text(code))));
            return;
        }
        let cancelled = self.norm.cancel_all_approvals();
        self.emit_all(cancelled);
        if self.active_turn.is_some() || self.turn_pending {
            self.emit(EventKind::TurnCompleted { status: TurnStatus::Failed, result_text: Some("codex app-server exited".into()), duration_ms: None });
        }
        let detail = if stderr.is_empty() { String::new() } else { format!(": {stderr}") };
        self.emit(EventKind::Error { message: format!("codex app-server exited (code {code:?}){detail}"), fatal: true });
        self.emit(EventKind::StatusChanged { status: AgentStatus::Offline });
        self.emit(EventKind::Exited { code });
    }
}

// ───────────────────────────── normalization (pure) ─────────────────────────────

struct ItemInfo {
    activity: ActivityKind,
    started: Instant,
    paths: Vec<String>,
    diff: Option<String>,
    command: Option<String>,
}

enum AnswerShape {
    /// `{decision: <one of these>}` (command and file-change approvals)
    Decisions(Vec<(ApprovalChoice, Value)>),
    /// `{permissions, scope}`
    Permissions(Value),
    /// `{answers}`
    UserInput,
    /// `{action, content}`
    Elicitation,
}

struct PendingApproval {
    request_id: Value,
    shape: AnswerShape,
}

/// Turns one thread's app-server notifications and server requests into `EventKind`s.
pub struct CodexNormalizer {
    agent_id: Id,
    run_id: Id,
    cwd: PathBuf,
    resumed: bool,
    status: Option<AgentStatus>,
    /// The thread is flagged waitingOnApproval / waitingOnUserInput.
    waiting: bool,
    items: HashMap<String, ItemInfo>,
    approvals: HashMap<Id, PendingApproval>,
    last_final: Option<String>,
    usage: Usage,
    /// Thread totals before this run (a resumed thread's `total` includes earlier runs).
    baseline: Option<[i64; 4]>,
    retries: i64,
    turn_announced: bool,
    /// Between a turn's start and its `turn/completed`.
    in_turn: bool,
    pub interrupt_requested: bool,
}

/// Command and file approvals offer these when the server sends no `availableDecisions`.
const DEFAULT_DECISIONS: [&str; 4] = ["accept", "acceptForSession", "decline", "cancel"];

impl CodexNormalizer {
    pub fn new(agent_id: &str, run_id: &str, cwd: PathBuf, resumed: bool) -> Self {
        CodexNormalizer {
            agent_id: agent_id.into(),
            run_id: run_id.into(),
            cwd,
            resumed,
            status: None,
            waiting: false,
            items: HashMap::new(),
            approvals: HashMap::new(),
            last_final: None,
            usage: Usage::default(),
            baseline: None,
            retries: 0,
            turn_announced: false,
            in_turn: false,
            interrupt_requested: false,
        }
    }

    fn set_status(&mut self, s: AgentStatus, out: &mut Vec<EventKind>) {
        if self.status != Some(s) {
            self.status = Some(s);
            out.push(EventKind::StatusChanged { status: s });
        }
    }

    /// What the agent is doing mid-turn, given approvals and running tools.
    fn busy_status(&self) -> AgentStatus {
        if self.waiting || !self.approvals.is_empty() {
            AgentStatus::WaitingApproval
        } else if !self.items.is_empty() {
            AgentStatus::Working
        } else {
            AgentStatus::Thinking
        }
    }

    pub fn turn_started(&mut self, text: &str) -> Vec<EventKind> {
        let mut out = vec![EventKind::TurnStarted { input: Some(text.to_string()) }];
        self.turn_announced = true;
        self.in_turn = true;
        self.interrupt_requested = false;
        self.last_final = None;
        self.set_status(AgentStatus::Thinking, &mut out);
        out
    }

    /// `turn/start` was refused: close the turn we announced.
    pub fn turn_failed(&mut self) -> Vec<EventKind> {
        let mut out = vec![EventKind::TurnCompleted { status: TurnStatus::Failed, result_text: None, duration_ms: None }];
        self.turn_announced = false;
        self.in_turn = false;
        self.set_status(AgentStatus::Idle, &mut out);
        out
    }

    pub fn on_notification(&mut self, method: &str, p: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        match method {
            "thread/started" | "thread/closed" | "turn/diff/updated" | "item/reasoning/summaryPartAdded" | "item/plan/delta"
            | "item/fileChange/patchUpdated" | "item/mcpToolCall/progress" | "item/commandExecution/terminalInteraction" => {}
            "turn/started" => {
                if !std::mem::take(&mut self.turn_announced) {
                    out.push(EventKind::TurnStarted { input: None });
                }
                self.in_turn = true;
                self.set_status(AgentStatus::Thinking, &mut out);
            }
            "turn/completed" => {
                let turn = &p["turn"];
                let status = match turn["status"].as_str() {
                    Some("completed") => TurnStatus::Completed,
                    Some("interrupted") => TurnStatus::Interrupted,
                    _ if self.interrupt_requested => TurnStatus::Interrupted,
                    _ => TurnStatus::Failed,
                };
                if status == TurnStatus::Failed
                    && let Some(m) = turn["error"]["message"].as_str()
                {
                    out.push(EventKind::Error { message: m.to_string(), fatal: false });
                }
                out.extend(self.cancel_all_approvals());
                self.items.clear();
                self.waiting = false;
                self.interrupt_requested = false;
                self.turn_announced = false;
                self.in_turn = false;
                out.push(EventKind::TurnCompleted { status, result_text: self.last_final.take(), duration_ms: turn["durationMs"].as_i64() });
                self.set_status(AgentStatus::Idle, &mut out);
            }
            "thread/status/changed" => match p["status"]["type"].as_str() {
                Some("active") => {
                    let flags = p["status"]["activeFlags"].as_array().cloned().unwrap_or_default();
                    self.waiting = flags.iter().any(|f| f == "waitingOnApproval" || f == "waitingOnUserInput");
                    let s = self.busy_status();
                    self.set_status(s, &mut out);
                }
                Some("idle") => {
                    self.waiting = false;
                    self.set_status(AgentStatus::Idle, &mut out);
                }
                Some("systemError") => {
                    out.push(EventKind::Error { message: "Codex reported a system error on this thread".into(), fatal: false });
                    self.set_status(AgentStatus::Blocked, &mut out);
                }
                _ => {}
            },
            "item/started" => self.item_started(&p["item"], &mut out),
            "item/completed" => self.item_completed(&p["item"], &mut out),
            "item/agentMessage/delta" => out.push(EventKind::TextDelta { item_id: s(p, "itemId"), text: s(p, "delta") }),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                out.push(EventKind::ReasoningDelta { item_id: s(p, "itemId"), text: s(p, "delta") })
            }
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                out.push(EventKind::ToolOutput { call_id: s(p, "itemId"), chunk: s(p, "delta") })
            }
            "turn/plan/updated" => {
                let steps = p["plan"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|st| PlanStep {
                        text: s(st, "step"),
                        status: match st["status"].as_str() {
                            Some("inProgress") => "in_progress".into(),
                            Some(other) => other.into(),
                            None => "pending".into(),
                        },
                    })
                    .collect();
                out.push(EventKind::PlanUpdated { steps });
            }
            "thread/tokenUsage/updated" => out.push(self.usage_update(&p["tokenUsage"])),
            RATE_LIMITS => self.rate_limits(&p["rateLimits"], &mut out),
            "error" => {
                let err = &p["error"];
                let message = err["message"].as_str().unwrap_or("Codex error").to_string();
                let info = &err["codexErrorInfo"];
                if p["willRetry"] == true {
                    // Codex tries again by itself: a limit it retries is not a limit reached.
                    self.retries += 1;
                    out.push(EventKind::Retrying { attempt: self.retries, reason: message });
                } else {
                    if info == "usageLimitExceeded" || info == "rateLimitExceeded" {
                        self.set_status(AgentStatus::RateLimited, &mut out);
                    }
                    if info == "usageLimitExceeded" {
                        // The account's quota is spent (no window in this error).
                        out.push(EventKind::RateLimit { window: "usage".into(), used_fraction: None, resets_at: None, status: Some("rejected".into()) });
                    }
                    out.push(EventKind::Error { message, fatal: false });
                }
            }
            "serverRequest/resolved" => {
                // resolved without our answer (turn ended, auto-review…): close the card
                let rid = &p["requestId"];
                if let Some(id) = self.approvals.iter().find(|(_, a)| &a.request_id == rid).map(|(id, _)| id.clone()) {
                    self.approvals.remove(&id);
                    out.push(EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() });
                    let st = self.busy_status();
                    self.set_status(st, &mut out);
                }
            }
            "thread/compacted" => out.push(EventKind::Compacting { pre_tokens: None, post_tokens: None }),
            "mcpServer/startupStatus/updated" => {
                if p["name"] == "codly" && p["status"] == "failed" {
                    out.push(EventKind::Error {
                        message: format!("codly MCP server failed to start: {}", p["error"].as_str().unwrap_or("unknown error")),
                        fatal: false,
                    });
                }
            }
            "warning" | "configWarning" | "guardianWarning" | "deprecationNotice" => {
                let message = p["message"].as_str().or(p["summary"].as_str()).map(str::to_owned).unwrap_or_else(|| p.to_string());
                out.push(EventKind::Error { message: format!("Codex {method}: {message}"), fatal: false });
            }
            other => out.push(EventKind::Unknown { raw_type: other.to_string() }),
        }
        out
    }

    fn item_started(&mut self, item: &Value, out: &mut Vec<EventKind>) {
        let ty = item["type"].as_str().unwrap_or("");
        let id = s(item, "id");
        match ty {
            "userMessage" | "hookPrompt" | "enteredReviewMode" | "exitedReviewMode" | "functionCallOutput" => {}
            "agentMessage" | "reasoning" | "plan" => {
                let st = self.busy_status();
                self.set_status(st, out);
            }
            "contextCompaction" => out.push(EventKind::Compacting { pre_tokens: None, post_tokens: None }),
            "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" | "webSearch" | "collabAgentToolCall" | "imageView"
            | "imageGeneration" | "sleep" | "subAgentActivity" => self.tool_started(ty, &id, item, out),
            other => out.push(EventKind::Unknown { raw_type: format!("item/started:{other}") }),
        }
    }

    fn tool_started(&mut self, ty: &str, id: &str, item: &Value, out: &mut Vec<EventKind>) {
        if self.items.contains_key(id) {
            return;
        }
        let (activity, target, confidence) = classify::classify_tool(Provider::Codex, ty, item);
        let changes = item["changes"].as_array().cloned().unwrap_or_default();
        let diff = (!changes.is_empty()).then(|| changes.iter().filter_map(|c| c["diff"].as_str()).collect::<Vec<_>>().join("\n"));
        let mut input = item.clone();
        if let Some(o) = input.as_object_mut() {
            o.remove("aggregatedOutput"); // empty at start; the output streams separately
        }
        if ty == "collabAgentToolCall" {
            out.push(EventKind::SubAgent { call_id: id.to_string(), description: s(item, "prompt"), status: "started".into() });
        }
        out.push(EventKind::ToolStarted { call_id: id.to_string(), name: ty.to_string(), activity, target: target.clone(), input, confidence: confidence.into() });
        self.items.insert(
            id.to_string(),
            ItemInfo {
                activity,
                started: Instant::now(),
                paths: if target.paths.is_empty() { target.path.clone().into_iter().collect() } else { target.paths.clone() },
                diff: diff.map(|d| classify::truncate_output(&d, OUTPUT_MAX)),
                command: item["command"].as_str().map(str::to_owned),
            },
        );
        let st = self.busy_status();
        self.set_status(st, out);
    }

    fn item_completed(&mut self, item: &Value, out: &mut Vec<EventKind>) {
        let ty = item["type"].as_str().unwrap_or("");
        let id = s(item, "id");
        match ty {
            "agentMessage" => {
                let text = s(item, "text");
                let phase = match item["phase"].as_str() {
                    Some("commentary") => MessagePhase::Commentary,
                    Some("final_answer") => MessagePhase::Final,
                    _ => MessagePhase::Unknown,
                };
                if phase != MessagePhase::Commentary {
                    self.last_final = Some(text.clone());
                }
                out.push(EventKind::Message { item_id: id, text, phase });
            }
            "reasoning" => {
                let join = |k: &str| item[k].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n\n")).unwrap_or_default();
                let text = Some(join("summary")).filter(|t| !t.is_empty()).unwrap_or_else(|| join("content"));
                if !text.is_empty() {
                    out.push(EventKind::Reasoning { item_id: id, text });
                }
            }
            "plan" => out.push(EventKind::Message { item_id: id, text: s(item, "text"), phase: MessagePhase::Commentary }),
            "userMessage" | "contextCompaction" | "hookPrompt" | "enteredReviewMode" | "exitedReviewMode" | "functionCallOutput" => {}
            "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" | "webSearch" | "collabAgentToolCall" | "imageView"
            | "imageGeneration" | "sleep" | "subAgentActivity" => {
                if !self.items.contains_key(&id) {
                    self.tool_started(ty, &id, item, out); // completed without a start event
                }
                self.tool_completed(ty, &id, item, out);
                // Generated images land outside the worktree; announce them so they become artifacts.
                if ty == "imageGeneration"
                    && let Some(path) = item["savedPath"].as_str()
                {
                    out.push(EventKind::FilesChanged { call_id: Some(id.clone()), paths: vec![path.to_string()], source: "provider".into() });
                }
            }
            other => out.push(EventKind::Unknown { raw_type: format!("item/completed:{other}") }),
        }
    }

    fn tool_completed(&mut self, ty: &str, id: &str, item: &Value, out: &mut Vec<EventKind>) {
        let info = self.items.remove(id);
        let status = item["status"].as_str().unwrap_or("completed");
        let elapsed = info.as_ref().map(|i| i.started.elapsed().as_millis() as i64);
        let duration_ms = item["durationMs"].as_i64().or(elapsed);
        let (ok, output, exit_code) = match ty {
            "commandExecution" => {
                let exit = item["exitCode"].as_i64();
                let out_text = item["aggregatedOutput"].as_str().map(str::to_owned).or_else(|| (status == "declined").then(|| "declined".to_string()));
                (status == "completed" && exit.unwrap_or(0) == 0, out_text, exit)
            }
            "mcpToolCall" => {
                let err = item["error"]["message"].as_str().map(str::to_owned);
                let text = item["result"]["content"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n"))
                    .filter(|t| !t.is_empty());
                (status == "completed" && err.is_none(), err.or(text), None)
            }
            "dynamicToolCall" => (item["success"] != false && status != "failed", None, None),
            _ => (!matches!(status, "failed" | "declined" | "interrupted"), None, None),
        };
        let tests = match (&info, &output) {
            (Some(i), Some(o)) if i.activity == ActivityKind::Test => classify::parse_test_counts(o),
            _ => None,
        };
        out.push(EventKind::ToolCompleted {
            call_id: id.to_string(),
            ok,
            output: output.map(|o| classify::truncate_output(&o, OUTPUT_MAX)),
            exit_code,
            duration_ms,
            tests_passed: tests.map(|t| t.0),
            tests_failed: tests.map(|t| t.1),
        });
        if ty == "fileChange" && ok {
            let paths: Vec<String> = item["changes"].as_array().into_iter().flatten().filter_map(|c| c["path"].as_str().map(str::to_owned)).collect();
            out.push(EventKind::FilesChanged { call_id: Some(id.to_string()), paths, source: "provider".into() });
        }
        if ty == "collabAgentToolCall" {
            out.push(EventKind::SubAgent { call_id: id.to_string(), description: s(item, "prompt"), status: status.to_string() });
        }
        let st = self.busy_status();
        self.set_status(st, out);
    }

    fn usage_update(&mut self, tu: &Value) -> EventKind {
        let get = |b: &Value| {
            [
                b["inputTokens"].as_i64().unwrap_or(0),
                b["cachedInputTokens"].as_i64().unwrap_or(0),
                b["cacheWriteInputTokens"].as_i64().unwrap_or(0),
                b["outputTokens"].as_i64().unwrap_or(0),
            ]
        };
        let (total, last) = (get(&tu["total"]), get(&tu["last"]));
        let resumed = self.resumed;
        let base = *self.baseline.get_or_insert_with(|| if resumed { std::array::from_fn(|i| total[i] - last[i]) } else { [0; 4] });
        let d: [i64; 4] = std::array::from_fn(|i| (total[i] - base[i]).max(0));
        // Codex counts cached tokens inside inputTokens; Usage keeps them apart (as Claude does).
        self.usage.input_tokens = (d[0] - d[1]).max(0);
        self.usage.cache_read_tokens = d[1];
        self.usage.cache_write_tokens = d[2];
        self.usage.output_tokens = d[3];
        self.usage.context_used = tu["last"]["totalTokens"].as_i64().or(self.usage.context_used);
        self.usage.context_window = tu["modelContextWindow"].as_i64().or(self.usage.context_window);
        EventKind::UsageUpdated { usage: self.usage.clone() }
    }

    fn rate_limits(&mut self, rl: &Value, out: &mut Vec<EventKind>) {
        let reached = !rl["rateLimitReachedType"].is_null();
        // The status before the windows: they are the account's real reading and have the last
        // word on it (an allowed window lifts the park a bare RateLimited status sets).
        if reached {
            self.set_status(AgentStatus::RateLimited, out);
        } else if self.status == Some(AgentStatus::RateLimited) {
            // The limit lifted: back to work mid-turn, free otherwise.
            let s = if self.in_turn { self.busy_status() } else { AgentStatus::Idle };
            self.set_status(s, out);
        }
        let full = |key: &str| rl[key]["usedPercent"].as_f64().is_some_and(|p| p >= 100.0);
        // The window that is full is the one that was reached (both, if neither says so).
        let any_full = full("primary") || full("secondary");
        for key in ["primary", "secondary"] {
            let w = &rl[key];
            if !w.is_object() {
                continue;
            }
            let window = match w["windowDurationMins"].as_i64() {
                Some(300) => "five_hour".to_string(),
                Some(10080) => "seven_day".to_string(),
                Some(m) => format!("{m}m"),
                None => key.to_string(),
            };
            out.push(EventKind::RateLimit {
                window,
                used_fraction: w["usedPercent"].as_f64().map(|p| p / 100.0),
                resets_at: w["resetsAt"].as_i64().map(|s| s * 1000),
                status: Some(if reached && (full(key) || !any_full) { "rejected" } else { "allowed" }.into()),
            });
        }
    }

    /// A server→client request: an approval card, or an immediate refusal for methods Codly
    /// does not implement (returned as the full response message).
    pub fn on_request(&mut self, id: &Value, method: &str, p: &Value) -> (Vec<EventKind>, Option<Value>) {
        let mut out = Vec::new();
        let item_id = p["itemId"].as_str().map(str::to_owned);
        let item = item_id.as_deref().and_then(|i| self.items.get(i));
        let mut risks: Vec<String> = Vec::new();
        let (subject, shape, reason) = match method {
            "item/commandExecution/requestApproval" => {
                let command = p["command"].as_str().map(str::to_owned).or_else(|| item.and_then(|i| i.command.clone())).unwrap_or_default();
                let cwd = p["cwd"].as_str().map(PathBuf::from).unwrap_or_else(|| self.cwd.clone());
                risks = classify::risk_hints(&command, Some(&cwd), &self.cwd);
                if !p["networkApprovalContext"].is_null() && !risks.iter().any(|r| r == "network") {
                    risks.push("network".into());
                }
                let mut defaults: Vec<Value> = vec![json!("accept"), json!("acceptForSession")];
                if let Some(a) = p["proposedExecpolicyAmendment"].as_array() {
                    defaults.push(json!({ "acceptWithExecpolicyAmendment": { "execpolicy_amendment": a } }));
                }
                defaults.extend([json!("decline"), json!("cancel")]);
                let subject = ApprovalSubject::Command { command: classify::unwrap_shell(&command), cwd: Some(cwd.to_string_lossy().into_owned()) };
                (subject, AnswerShape::Decisions(decision_choices(p, defaults)), p["reason"].as_str().map(str::to_owned))
            }
            "item/fileChange/requestApproval" => {
                let paths = item.map(|i| i.paths.clone()).unwrap_or_default();
                let diff = item.and_then(|i| i.diff.clone());
                if !p["grantRoot"].is_null() || paths.iter().any(|x| classify::outside(x, &self.cwd, &self.cwd)) {
                    risks.push("outside worktree".into());
                }
                let defaults = DEFAULT_DECISIONS.iter().map(|d| json!(d)).collect();
                (ApprovalSubject::FileChange { paths, diff }, AnswerShape::Decisions(decision_choices(p, defaults)), p["reason"].as_str().map(str::to_owned))
            }
            "item/permissions/requestApproval" => {
                (ApprovalSubject::Permissions { detail: p.clone() }, AnswerShape::Permissions(p["permissions"].clone()), p["reason"].as_str().map(str::to_owned))
            }
            "item/tool/requestUserInput" => (ApprovalSubject::Question { questions: p["questions"].clone() }, AnswerShape::UserInput, None),
            "mcpServer/elicitation/request" => {
                let reason = p["message"].as_str().map(str::to_owned);
                (ApprovalSubject::Question { questions: p.clone() }, AnswerShape::Elicitation, reason)
            }
            other => {
                out.push(EventKind::Unknown { raw_type: other.to_string() });
                let reply = json!({ "id": id, "error": { "code": -32601, "message": format!("Codly does not handle {other}") } });
                return (out, Some(reply));
            }
        };
        let choices = match &shape {
            AnswerShape::Decisions(list) => list.iter().map(|(c, _)| *c).collect(),
            AnswerShape::Permissions(_) => vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::Deny],
            AnswerShape::UserInput => vec![ApprovalChoice::AllowOnce, ApprovalChoice::Cancel],
            AnswerShape::Elicitation => vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny, ApprovalChoice::Cancel],
        };
        let approval = Approval {
            id: new_id(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            task_id: None,
            call_id: item_id,
            subject,
            reason,
            choices,
            risks,
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        self.approvals.insert(approval.id.clone(), PendingApproval { request_id: id.clone(), shape });
        out.push(EventKind::ApprovalRequested { approval });
        self.set_status(AgentStatus::WaitingApproval, &mut out);
        (out, None)
    }

    /// The JSON-RPC response for a decision, plus status events.
    pub fn answer(&mut self, approval_id: &str, d: &ApprovalDecision) -> Option<(Value, Vec<EventKind>)> {
        let p = self.approvals.remove(approval_id)?;
        let allow = matches!(d.choice, ApprovalChoice::AllowOnce | ApprovalChoice::AllowSession | ApprovalChoice::AllowAlways);
        let result = match p.shape {
            AnswerShape::Decisions(list) => json!({ "decision": pick_decision(&list, d.choice) }),
            AnswerShape::Permissions(perms) => match d.choice {
                ApprovalChoice::AllowOnce => json!({ "permissions": perms, "scope": "turn" }),
                ApprovalChoice::AllowSession | ApprovalChoice::AllowAlways => json!({ "permissions": perms, "scope": "session" }),
                _ => json!({ "permissions": {}, "scope": "turn" }),
            },
            AnswerShape::UserInput => json!({ "answers": if allow { d.answers.clone().unwrap_or_else(|| json!({})) } else { json!({}) } }),
            AnswerShape::Elicitation => match d.choice {
                _ if allow => json!({ "action": "accept", "content": d.answers.clone().unwrap_or(Value::Null) }),
                ApprovalChoice::Deny => json!({ "action": "decline" }),
                _ => json!({ "action": "cancel" }),
            },
        };
        if d.choice == ApprovalChoice::Cancel {
            self.interrupt_requested = true;
        }
        let mut out = Vec::new();
        self.waiting = false;
        let st = self.busy_status();
        self.set_status(st, &mut out);
        Some((json!({ "id": p.request_id, "result": result }), out))
    }

    pub fn cancel_all_approvals(&mut self) -> Vec<EventKind> {
        self.approvals
            .drain()
            .map(|(id, _)| EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() })
            .collect()
    }
}

fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}

/// Choices are exactly what Codex offered (`availableDecisions`, CRITIQUE W11), each mapped
/// to the decision value to send back.
fn decision_choices(p: &Value, defaults: Vec<Value>) -> Vec<(ApprovalChoice, Value)> {
    let list = p["availableDecisions"].as_array().filter(|a| !a.is_empty()).cloned().unwrap_or(defaults);
    let mut out: Vec<(ApprovalChoice, Value)> = Vec::new();
    for d in list {
        let choice = match &d {
            Value::String(x) => match x.as_str() {
                "accept" => ApprovalChoice::AllowOnce,
                "acceptForSession" => ApprovalChoice::AllowSession,
                "decline" => ApprovalChoice::Deny,
                "cancel" => ApprovalChoice::Cancel,
                _ => continue,
            },
            Value::Object(o) if o.contains_key("acceptWithExecpolicyAmendment") => ApprovalChoice::AllowAlways,
            Value::Object(o) if o.contains_key("applyNetworkPolicyAmendment") => {
                if o["applyNetworkPolicyAmendment"]["network_policy_amendment"]["action"] == "deny" {
                    ApprovalChoice::Deny
                } else {
                    ApprovalChoice::AllowAlways
                }
            }
            _ => continue,
        };
        if !out.iter().any(|(c, _)| *c == choice) {
            out.push((choice, d));
        }
    }
    out
}

/// The offered decision for `choice`, or the closest offered one (no `decline` → `cancel`).
fn pick_decision(list: &[(ApprovalChoice, Value)], choice: ApprovalChoice) -> Value {
    use ApprovalChoice::*;
    let order: &[ApprovalChoice] = match choice {
        AllowOnce => &[AllowOnce],
        AllowSession => &[AllowSession, AllowOnce],
        AllowAlways => &[AllowAlways, AllowSession, AllowOnce],
        Deny => &[Deny, Cancel],
        Cancel => &[Cancel, Deny],
    };
    order
        .iter()
        .find_map(|c| list.iter().find(|(x, _)| x == c).map(|(_, v)| v.clone()))
        .unwrap_or_else(|| json!(if matches!(choice, AllowOnce | AllowSession | AllowAlways) { "accept" } else { "decline" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::McpLaunch;
    use crate::model::Role;
    use std::path::Path;

    fn fixture() -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures/codex-appserver.jsonl");
        std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn approval_of(events: &[EventKind]) -> Approval {
        events.iter().find_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).expect("approval")
    }

    /// Replay the captured app-server session (2 turns: ls + apply_patch with approvals, then
    /// an interrupt) through the normalizer, answering approvals like the probe did.
    #[test]
    fn replays_the_app_server_fixture() {
        let mut norm = CodexNormalizer::new("a1", "r1", PathBuf::from("/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/fixture-repo"), false);
        let mut events = Vec::new();
        let mut replies = Vec::new();
        for line in fixture() {
            if line["dir"] != "<-" {
                continue;
            }
            let msg = &line["msg"];
            let Some(method) = msg["method"].as_str() else { continue }; // responses go to the actor
            if msg.get("id").is_some() {
                let (ev, reply) = norm.on_request(&msg["id"], method, &msg["params"]);
                assert!(reply.is_none());
                let a = approval_of(&ev);
                events.extend(ev);
                let (resp, ev) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
                replies.push(resp);
                events.extend(ev);
            } else {
                events.extend(norm.on_notification(method, &msg["params"]));
            }
        }
        // the answers match what the probe sent
        assert_eq!(replies, vec![json!({ "id": 0, "result": { "decision": "accept" } }), json!({ "id": 1, "result": { "decision": "accept" } })]);

        let approvals: Vec<Approval> = events.iter().filter_map(|e| if let EventKind::ApprovalRequested { approval } = e { Some(approval.clone()) } else { None }).collect();
        assert_eq!(approvals.len(), 2);
        // command approval: choices exactly from availableDecisions (no decline offered)
        assert_eq!(approvals[0].choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowAlways, ApprovalChoice::Cancel]);
        assert!(matches!(&approvals[0].subject, ApprovalSubject::Command { command, .. } if command == "ls"));
        assert_eq!(approvals[0].call_id.as_deref(), Some("exec-cafc67de-17bc-48a1-bc7f-c5e47026b7b8"));
        // file approval: no availableDecisions → the schema's four decisions; paths from the item
        assert_eq!(approvals[1].choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::Deny, ApprovalChoice::Cancel]);
        match &approvals[1].subject {
            ApprovalSubject::FileChange { paths, diff } => {
                assert!(paths[0].ends_with("fixture-repo/notes.txt"));
                assert_eq!(diff.as_deref(), Some("hello\n"));
            }
            other => panic!("{other:?}"),
        }

        let started: Vec<(String, ActivityKind)> =
            events.iter().filter_map(|e| if let EventKind::ToolStarted { name, activity, .. } = e { Some((name.clone(), *activity)) } else { None }).collect();
        assert_eq!(started, vec![("commandExecution".into(), ActivityKind::Search), ("fileChange".into(), ActivityKind::Create)]);
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: true, exit_code: Some(0), output: Some(o), .. } if o.contains("math_utils.py"))));
        assert!(events.iter().any(|e| matches!(e, EventKind::FilesChanged { paths, source, .. } if paths[0].ends_with("notes.txt") && source == "provider")));
        assert!(events.iter().any(|e| matches!(e, EventKind::Message { phase: MessagePhase::Commentary, text, .. } if text.starts_with("I’ll run"))));
        assert!(events.iter().any(|e| matches!(e, EventKind::TextDelta { text, .. } if text == "DONE")));

        let turns: Vec<(TurnStatus, Option<String>)> =
            events.iter().filter_map(|e| if let EventKind::TurnCompleted { status, result_text, .. } = e { Some((*status, result_text.clone())) } else { None }).collect();
        assert_eq!(turns, vec![(TurnStatus::Completed, Some("DONE".into())), (TurnStatus::Interrupted, None)]);
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::TurnStarted { .. })).count(), 2);

        let usage = events.iter().rev().find_map(|e| if let EventKind::UsageUpdated { usage } = e { Some(usage.clone()) } else { None }).unwrap();
        assert_eq!((usage.input_tokens, usage.cache_read_tokens, usage.output_tokens), (54294 - 48128, 48128, 200));
        assert_eq!((usage.context_used, usage.context_window), (Some(18224), Some(258400)));
        assert!(events.iter().any(|e| matches!(e, EventKind::RateLimit { window, used_fraction: Some(f), .. } if window == "seven_day" && *f == 0.0)));

        // waiting → working → idle, without flicker back to working while waiting
        let statuses: Vec<AgentStatus> = events.iter().filter_map(|e| if let EventKind::StatusChanged { status } = e { Some(*status) } else { None }).collect();
        let first_wait = statuses.iter().position(|s| *s == AgentStatus::WaitingApproval).unwrap();
        assert_ne!(statuses[first_wait + 1], AgentStatus::WaitingApproval);
        assert_eq!(statuses.last(), Some(&AgentStatus::Idle));
        // mcp startup chatter about the user's own servers is not surfaced
        assert!(!events.iter().any(|e| matches!(e, EventKind::Unknown { raw_type } if raw_type.starts_with("mcpServer"))));
    }

    /// Live 0.161.0 capture under Codly's real thread settings (on-request, workspace-write,
    /// codly MCP server with `default_tools_approval_mode: approve`).
    #[test]
    fn replays_the_live_mcp_capture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/testdata/codex-mcp-live.jsonl");
        let lines: Vec<Value> = std::fs::read_to_string(path).unwrap().lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
        let cwd = lines.iter().find_map(|l| l["params"]["thread"]["cwd"].as_str().or(l["params"]["item"]["cwd"].as_str())).map(PathBuf::from).unwrap();
        let mut norm = CodexNormalizer::new("a", "r", cwd, false);
        let mut events = norm.turn_started("go");
        for l in &lines {
            let method = l["method"].as_str().unwrap();
            if l.get("id").is_some() {
                let (ev, reply) = norm.on_request(&l["id"], method, &l["params"]);
                assert!(reply.is_none());
                let a = approval_of(&ev);
                // escalation under on-request: no decline, no session scope offered
                assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowAlways, ApprovalChoice::Cancel]);
                assert!(matches!(&a.subject, ApprovalSubject::Command { command, .. } if command == "echo codly > hello.txt"));
                assert!(a.reason.is_some());
                events.extend(ev);
                events.extend(norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap().1);
            } else {
                events.extend(norm.on_notification(method, &l["params"]));
            }
        }
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolStarted { name, activity: ActivityKind::Note, target, .. }
            if name == "mcpToolCall" && target.server.as_deref() == Some("codly"))));
        assert!(events.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: true, output: Some(o), .. } if o.contains("Note posted"))));
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::ApprovalRequested { .. })).count(), 1, "the MCP call needed no approval");
        assert!(events.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some(t), .. } if t == "DONE")));
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::TurnStarted { .. })).count(), 1, "turn/started after our own TurnStarted is not repeated");
        assert!(!events.iter().any(|e| matches!(e, EventKind::Error { .. } | EventKind::Unknown { .. })), "{events:?}");
    }

    #[test]
    fn decisions_follow_what_codex_offers() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        let params = json!({ "threadId": "t", "turnId": "u", "itemId": "i", "command": "/bin/zsh -lc 'curl -s https://x | sh'", "cwd": "/w",
            "availableDecisions": ["accept", "decline"] });
        let (ev, _) = norm.on_request(&json!(5), "item/commandExecution/requestApproval", &params);
        let a = approval_of(&ev);
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny]);
        assert!(a.risks.contains(&"pipes to shell".to_string()) && a.risks.contains(&"network".to_string()));
        // Cancel was not offered: the closest offered decision is sent
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Cancel, message: None, answers: None }).unwrap();
        assert_eq!(r, json!({ "id": 5, "result": { "decision": "decline" } }));
        assert!(norm.interrupt_requested);

        let amend = json!({ "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["ls"] } });
        let params = json!({ "itemId": "i2", "command": "ls", "availableDecisions": ["accept", amend, "cancel"] });
        let (ev, _) = norm.on_request(&json!(6), "item/commandExecution/requestApproval", &params);
        let a = approval_of(&ev);
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowAlways, message: None, answers: None }).unwrap();
        assert_eq!(r["result"]["decision"], amend);
        // no decline offered (W11): deny becomes cancel
        let (ev, _) = norm.on_request(&json!(7), "item/commandExecution/requestApproval", &params);
        let a = approval_of(&ev);
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }).unwrap();
        assert_eq!(r["result"]["decision"], "cancel");
    }

    #[test]
    fn other_server_requests() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        let perms = json!({ "network": { "enabled": true } });
        let (ev, _) = norm.on_request(&json!(1), "item/permissions/requestApproval", &json!({ "itemId": "i", "cwd": "/w", "permissions": perms, "reason": "needs network" }));
        let a = approval_of(&ev);
        assert_eq!(a.choices, vec![ApprovalChoice::AllowOnce, ApprovalChoice::AllowSession, ApprovalChoice::Deny]);
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowSession, message: None, answers: None }).unwrap();
        assert_eq!(r["result"], json!({ "permissions": perms, "scope": "session" }));

        let qs = json!([{ "id": "q1", "header": "DB", "question": "Which DB?" }]);
        let (ev, _) = norm.on_request(&json!(2), "item/tool/requestUserInput", &json!({ "itemId": "i", "questions": qs, "isBlocking": true }));
        let a = approval_of(&ev);
        assert!(matches!(&a.subject, ApprovalSubject::Question { questions } if questions == &qs));
        let answers = json!({ "q1": { "answers": ["sqlite"] } });
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: Some(answers.clone()) }).unwrap();
        assert_eq!(r["result"]["answers"], answers);

        let (ev, _) = norm.on_request(&json!(3), "mcpServer/elicitation/request", &json!({ "serverName": "x", "message": "Confirm?", "mode": "form", "requestedSchema": {} }));
        let a = approval_of(&ev);
        let (r, _) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }).unwrap();
        assert_eq!(r["result"], json!({ "action": "decline" }));

        let (ev, reply) = norm.on_request(&json!(4), "item/tool/call", &json!({}));
        assert!(matches!(&ev[0], EventKind::Unknown { .. }));
        assert_eq!(reply.unwrap()["error"]["code"], -32601);

        // a request the server resolves itself closes the card
        let (ev, _) = norm.on_request(&json!(9), "item/fileChange/requestApproval", &json!({ "itemId": "f" }));
        let a = approval_of(&ev);
        let ev = norm.on_notification("serverRequest/resolved", &json!({ "threadId": "t", "requestId": 9 }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ApprovalResolved { approval_id, status: ApprovalStatus::Cancelled, .. } if *approval_id == a.id)));
    }

    #[test]
    fn notifications_map_to_events() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), true);
        let ev = norm.on_notification("turn/plan/updated", &json!({ "plan": [{ "step": "read", "status": "completed" }, { "step": "edit", "status": "inProgress" }] }));
        assert!(matches!(&ev[0], EventKind::PlanUpdated { steps } if steps[1].status == "in_progress"));
        let ev = norm.on_notification("item/commandExecution/outputDelta", &json!({ "itemId": "c1", "delta": "line\n" }));
        assert!(matches!(&ev[0], EventKind::ToolOutput { call_id, chunk } if call_id == "c1" && chunk == "line\n"));
        let ev = norm.on_notification("error", &json!({ "error": { "message": "overloaded" }, "willRetry": true }));
        assert!(matches!(&ev[0], EventKind::Retrying { attempt: 1, .. }));
        // A rate limit Codex retries by itself is not a limit reached: nothing parks the account.
        for info in ["rateLimitExceeded", "usageLimitExceeded"] {
            let ev = norm.on_notification("error", &json!({ "error": { "message": "429", "codexErrorInfo": info }, "willRetry": true }));
            assert!(matches!(&ev[..], [EventKind::Retrying { reason, .. }] if reason == "429"), "{ev:?}");
        }
        let ev = norm.on_notification("error", &json!({ "error": { "message": "limit", "codexErrorInfo": "usageLimitExceeded" }, "willRetry": false }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::StatusChanged { status: AgentStatus::RateLimited })));
        assert!(ev.iter().any(|e| matches!(e, EventKind::RateLimit { window, status: Some(s), .. } if window == "usage" && s == "rejected")));
        // Only the full window counts as reached: a spent 5h window must not park the account for its 7d one.
        let windows = |ev: Vec<EventKind>| -> Vec<(String, Option<String>)> {
            ev.into_iter().filter_map(|e| match e { EventKind::RateLimit { window, status, .. } => Some((window, status)), _ => None }).collect()
        };
        let rl = |five: f64, seven: f64| json!({ "rateLimits": { "limitId": "codex", "rateLimitReachedType": "rate_limit_reached",
            "primary": { "usedPercent": five, "windowDurationMins": 300, "resetsAt": 1 },
            "secondary": { "usedPercent": seven, "windowDurationMins": 10080, "resetsAt": 2 } } });
        let (rejected, allowed) = (Some("rejected".to_string()), Some("allowed".to_string()));
        assert_eq!(windows(norm.on_notification("account/rateLimits/updated", &rl(100.0, 40.0))),
            [("five_hour".into(), rejected.clone()), ("seven_day".into(), allowed)]);
        // Reached with neither full (credits depleted): both windows.
        assert_eq!(windows(norm.on_notification("account/rateLimits/updated", &rl(60.0, 40.0))),
            [("five_hour".into(), rejected.clone()), ("seven_day".into(), rejected)]);
        // resumed thread: totals before this run are a baseline
        let tu = json!({ "total": { "inputTokens": 1000, "cachedInputTokens": 400, "outputTokens": 50, "totalTokens": 1050 },
                         "last": { "inputTokens": 100, "cachedInputTokens": 40, "outputTokens": 5, "totalTokens": 105 }, "modelContextWindow": 1000 });
        let EventKind::UsageUpdated { usage } = norm.on_notification("thread/tokenUsage/updated", &json!({ "tokenUsage": tu })).remove(0) else { panic!() };
        assert_eq!((usage.input_tokens, usage.cache_read_tokens, usage.output_tokens, usage.context_used), (60, 40, 5, Some(105)));
        let ev = norm.on_notification("mcpServer/startupStatus/updated", &json!({ "name": "codly", "status": "failed", "error": "boom" }));
        assert!(matches!(&ev[0], EventKind::Error { message, .. } if message.contains("boom")));
        let ev = norm.on_notification("thread/goal/updated", &json!({}));
        assert!(matches!(&ev[0], EventKind::Unknown { raw_type } if raw_type == "thread/goal/updated"));
        let mcp_item = json!({ "type": "mcpToolCall", "id": "m1", "server": "codly", "tool": "codly_post_note", "arguments": { "text": "hi" }, "status": "inProgress" });
        let ev = norm.on_notification("item/started", &json!({ "item": mcp_item }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ToolStarted { activity: ActivityKind::Note, .. })));
        let done = json!({ "type": "mcpToolCall", "id": "m1", "server": "codly", "tool": "codly_post_note", "status": "completed",
                           "result": { "content": [{ "type": "text", "text": "posted" }] }, "error": null, "durationMs": 12 });
        let ev = norm.on_notification("item/completed", &json!({ "item": done }));
        assert!(ev.iter().any(|e| matches!(e, EventKind::ToolCompleted { ok: true, output: Some(o), duration_ms: Some(12), .. } if o == "posted")));
    }

    /// The account's limit reaches every agent on it, the idle ones too: they are free again
    /// when a reading says the limit lifted, not only at their next turn.
    #[test]
    fn a_lifted_limit_frees_the_agent() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        let rl = |reached: bool| {
            let (kind, used) = if reached { (json!("rate_limit_reached"), 100.0) } else { (Value::Null, 3.0) };
            json!({ "rateLimits": { "rateLimitReachedType": kind, "primary": { "usedPercent": used, "windowDurationMins": 300, "resetsAt": 1 } } })
        };
        let statuses = |ev: &[EventKind]| -> Vec<AgentStatus> {
            ev.iter().filter_map(|e| if let EventKind::StatusChanged { status } = e { Some(*status) } else { None }).collect()
        };
        let ev = norm.on_notification(RATE_LIMITS, &rl(true));
        // the status first: the window after it is the real reading, with the last word on the account
        assert!(matches!(&ev[..], [EventKind::StatusChanged { status: AgentStatus::RateLimited }, EventKind::RateLimit { status: Some(s), .. }] if s == "rejected"), "{ev:?}");
        assert_eq!(statuses(&norm.on_notification(RATE_LIMITS, &rl(false))), [AgentStatus::Idle]);
        assert!(statuses(&norm.on_notification(RATE_LIMITS, &rl(false))).is_empty(), "an agent that was not limited keeps its status");
        // mid-turn it goes back to work, not to idle
        norm.turn_started("go");
        norm.on_notification(RATE_LIMITS, &rl(true));
        assert_eq!(statuses(&norm.on_notification(RATE_LIMITS, &rl(false))), [AgentStatus::Thinking]);
        norm.on_notification("turn/completed", &json!({ "turn": { "status": "completed" } }));
        norm.on_notification(RATE_LIMITS, &rl(true));
        assert_eq!(statuses(&norm.on_notification(RATE_LIMITS, &rl(false))), [AgentStatus::Idle], "the turn is over");
    }

    #[test]
    fn byok_points_codex_at_its_own_provider() {
        let spec = |env: &[(&str, &str)], cmd: Option<&str>| SpawnSpec {
            agent_id: "a".into(),
            run_id: "r".into(),
            provider: Provider::Codex,
            role: Role::Worker,
            model: None,
            cwd: PathBuf::from("/w"),
            bin: PathBuf::from("codex"),
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            session: SessionMode::New { session_id: None },
            system_prompt: String::new(),
            read_only: false,
            mcp: None,
            use_user_setup: false,
            raw_log: PathBuf::from("/tmp/r/raw.ndjson"),
            byok_command: cmd.map(str::to_owned),
        };
        let off = spec(&[("CODEX_HOME", "/h")], None);
        assert!(byok_overrides(&off).is_empty());
        assert_eq!(auth_tag(&off), "");

        let key = spec(&[("OPENAI_API_KEY", "sk-proj-x")], None);
        let a = byok_overrides(&key).join(" ");
        assert!(a.contains(r#"-c model_providers.codly_openai.env_key="OPENAI_API_KEY""#), "{a}");
        assert!(a.contains(r#"-c model_providers.codly_openai.base_url="https://api.openai.com/v1""#), "{a}");
        assert!(a.ends_with(r#"-c model_provider="codly_openai""#), "{a}");
        assert!(!a.contains("sk-proj-x"), "the key stays in the env, never in argv");
        assert!(auth_tag(&key).starts_with("key:") && !auth_tag(&key).contains("sk-proj"));
        assert_ne!(auth_tag(&key), auth_tag(&spec(&[("OPENAI_API_KEY", "sk-proj-y")], None)), "a new key gets a new server");

        let cmd = spec(&[], Some(r#"wif "openai""#));
        let a = byok_overrides(&cmd);
        assert!(a.contains(&r#"model_providers.codly_openai.auth.command="/bin/sh""#.to_string()), "{a:?}");
        assert!(a.contains(&r#"model_providers.codly_openai.auth.args=["-c","wif \"openai\""]"#.to_string()), "{a:?}");
        assert!(a.contains(&"model_providers.codly_openai.auth.refresh_interval_ms=300000".to_string()), "{a:?}");
        assert!(!a.iter().any(|x| x.contains("env_key")), "auth and env_key never together");
        assert_eq!(auth_tag(&cmd), r#"cmd:wif "openai""#);
    }

    #[test]
    fn thread_params() {
        let spec = SpawnSpec {
            agent_id: "a".into(),
            run_id: "r".into(),
            provider: Provider::Codex,
            role: Role::Worker,
            model: Some("gpt-6-luna:low".into()),
            cwd: PathBuf::from("/w/t1"),
            bin: PathBuf::from("codex"),
            env: HashMap::new(),
            session: SessionMode::New { session_id: None },
            system_prompt: "rules".into(),
            read_only: false,
            mcp: Some(McpLaunch { command: "/x/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), "tok".into())] }),
            use_user_setup: false,
            raw_log: PathBuf::from("/tmp/r/raw.ndjson"),
            byok_command: None,
        };
        let (model, effort) = classify::split_model_effort(spec.model.as_deref());
        let (m, p) = thread_request(&spec, model.as_deref(), effort.as_deref());
        assert_eq!(m, "thread/start");
        assert_eq!(p["model"], "gpt-6-luna");
        assert_eq!(p["approvalPolicy"], "on-request");
        assert_eq!(p["sandbox"], "workspace-write");
        assert_eq!(p["developerInstructions"], "rules");
        assert_eq!(p["threadSource"], "codly");
        assert_eq!(p["config"]["model_reasoning_effort"], "low");
        assert_eq!(p["config"]["features"]["memories"], false);
        assert_eq!(p["config"]["mcp_servers"]["codly"]["env"]["CODLY_MCP_TOKEN"], "tok");
        assert_eq!(p["config"]["mcp_servers"]["codly"]["default_tools_approval_mode"], "approve");
        let mut ro = spec.clone();
        ro.read_only = true;
        ro.session = SessionMode::Resume { session_id: "th-1".into() };
        let (m, p) = thread_request(&ro, None, None);
        assert_eq!((m, &p["threadId"], &p["sandbox"]), ("thread/resume", &json!("th-1"), &json!("read-only")));
        assert!(p.get("threadSource").is_none() && p.get("model").is_none());
    }

    /// An app-server with no process behind it: lines are fed by hand, every write fails.
    fn fake_server(dir: &Path) -> Arc<AppServer> {
        Arc::new(AppServer {
            stdin: Mutex::new(None),
            next_id: AtomicI64::new(0),
            state: StdMutex::new(State::default()),
            alive: AtomicBool::new(true),
            abandoned: AtomicBool::new(false),
            last_inbound_ms: AtomicI64::new(0),
            pgid: 0,
            started_ms: 0,
            stderr: StderrTail::default(),
            info: StdMutex::new(ServerInfo { cli_version: None, billing: Billing::Unknown }),
            bootstrap_raw: Arc::new(RawLog::open(&dir.join("raw.ndjson")).unwrap()),
        })
    }

    /// An agent on `fake_server` as `spawn` leaves it: waiting for its thread (request 0).
    fn fake_agent(server: &Arc<AppServer>) -> (CodexAgent, mpsc::UnboundedReceiver<AdapterEvent>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let raw = server.bootstrap_raw.clone();
        let agent = CodexAgent {
            agent_id: "a".into(),
            run_id: "r".into(),
            cwd: "/w".into(),
            events,
            server: server.clone(),
            route: Route { tx: mpsc::unbounded_channel().0, raw: raw.clone() },
            raw,
            effort: None,
            thread_id: None,
            started: false,
            norm: CodexNormalizer::new("a", "r", PathBuf::from("/w"), false),
            pending: HashMap::from([(0, Pending::Thread)]),
            active_turn: None,
            turn_pending: false,
            interrupt_after_start: false,
            steer_queue: Vec::new(),
            queue: VecDeque::new(),
            held: Vec::new(),
            stage: Stage::Running,
            deadline: None,
        };
        (agent, events_rx)
    }

    fn emitted(events: &mut mpsc::UnboundedReceiver<AdapterEvent>) -> Vec<EventKind> {
        std::iter::from_fn(|| events.try_recv().ok()).map(|e| e.kind).collect()
    }

    #[tokio::test]
    async fn thread_notifications_without_a_route_reach_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let mut agents = Vec::new();
        for tid in ["t1", "t2"] {
            let (tx, rx) = mpsc::unbounded_channel();
            server.state.lock().unwrap().routes.insert(tid.into(), Route { tx, raw: server.bootstrap_raw.clone() });
            agents.push(rx);
        }
        let got = |rx: &mut mpsc::UnboundedReceiver<ServerMsg>| match rx.try_recv() {
            Ok(ServerMsg::Notification { method, .. }) => Some(method),
            _ => None,
        };
        // a thread unrouted on a deadline, or one Codly never opened
        server.route_line(br#"{"method":"turn/completed","params":{"threadId":"gone","turn":{"status":"failed"}}}"#);
        assert_eq!((got(&mut agents[0]), got(&mut agents[1])), (None, None), "another thread's turn/completed reached an agent");
        // a routed thread still gets its own, and only its own
        server.route_line(br#"{"method":"turn/completed","params":{"threadId":"t1","turn":{"status":"completed"}}}"#);
        assert_eq!((got(&mut agents[0]), got(&mut agents[1])), (Some("turn/completed".into()), None));
        // what concerns the whole account still reaches everyone
        server.route_line(br#"{"method":"account/rateLimits/updated","params":{"rateLimits":{}}}"#);
        let limits = Some("account/rateLimits/updated".to_string());
        assert_eq!((got(&mut agents[0]), got(&mut agents[1])), (limits.clone(), limits));
    }

    /// Codex repeats the account's usage after every model response. Each agent would record
    /// every copy: only a reading that says something new is passed on.
    #[tokio::test]
    async fn an_unchanged_rate_limit_reading_is_passed_on_once() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let mut agents = Vec::new();
        for tid in ["t1", "t2"] {
            let (tx, rx) = mpsc::unbounded_channel();
            server.state.lock().unwrap().routes.insert(tid.into(), Route { tx, raw: server.bootstrap_raw.clone() });
            agents.push(rx);
        }
        // how many notifications each agent got since the last look
        let mut got = || -> Vec<usize> { agents.iter_mut().map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).count()).collect() };
        // `emittedAtMs` differs on every line: only the reading itself (`params`) is compared
        let reading = |used: u32, reached: &str, at: u32| {
            format!(r#"{{"method":"account/rateLimits/updated","params":{{"rateLimits":{{"primary":{{"usedPercent":{used}}},"rateLimitReachedType":{reached}}}}},"emittedAtMs":{at}}}"#)
        };
        server.route_line(reading(4, "null", 1).as_bytes());
        assert_eq!(got(), [1, 1], "the first reading reaches every agent");
        server.route_line(reading(4, "null", 2).as_bytes());
        server.route_line(reading(4, "null", 3).as_bytes());
        assert_eq!(got(), [0, 0], "the same reading again reaches nobody");
        server.route_line(reading(5, "null", 4).as_bytes());
        assert_eq!(got(), [1, 1], "more usage is news");
        server.route_line(reading(5, r#""rate_limit_reached""#, 5).as_bytes());
        assert_eq!(got(), [1, 1], "so is a limit reached at the same usage");
        // a minute on, the same reading counts again (a window without a reset time goes stale)
        server.state.lock().unwrap().last_limits.as_mut().unwrap().1 -= LIMITS_REFRESH_MS;
        server.route_line(reading(5, r#""rate_limit_reached""#, 6).as_bytes());
        assert_eq!(got(), [1, 1]);

        // who is signed in is the server's to know: no agent reports it
        server.route_line(br#"{"method":"account/updated","params":{"authMode":"chatgpt","planType":"plus"}}"#);
        assert_eq!(got(), [0, 0]);
        assert_eq!(server.info.lock().unwrap().billing, Billing::Subscription);
        server.route_line(br#"{"method":"account/updated","params":{"authMode":"a mode from the future"}}"#);
        assert_eq!(server.info.lock().unwrap().billing, Billing::Subscription, "an unknown mode changes nothing");
    }

    /// A steered prompt gets no turn of its own. The adapter says it was delivered, or Codly
    /// would wait for that turn and never see the agent as done.
    #[tokio::test]
    async fn a_steered_prompt_is_reported_as_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let (mut agent, mut events) = fake_agent(&server);
        (agent.thread_id, agent.active_turn) = (Some("t".into()), Some("u".into()));
        agent.pending.insert(7, Pending::Steer("also run the tests".into()));
        agent.on_response(7, Ok(json!({ "turnId": "u" })), &mut None).await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::PromptSteered { input }] if input == "also run the tests"), "{kinds:?}");

        // Codex refused it (the turn was ending): not delivered yet, it waits for a turn of its own
        agent.pending.insert(8, Pending::Steer("and the docs".into()));
        agent.on_response(8, Err(json!({ "message": "no active turn to steer" })), &mut None).await;
        assert!(emitted(&mut events).is_empty());
        assert_eq!(agent.queue, ["and the docs"]);
    }

    /// Codex may report a turn's end before it answers a steer sent into that turn. Codly
    /// must hear "delivered" first: after the turn's end it would wait for a turn that never
    /// comes, and the task would never reach its gate.
    #[tokio::test]
    async fn a_turn_end_waits_for_the_steer_answer() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let done = json!({ "threadId": "t", "turn": { "status": "completed" } });
        let ended = |k: &[EventKind]| {
            matches!(k, [EventKind::TurnCompleted { status: TurnStatus::Completed, .. }, EventKind::StatusChanged { status: AgentStatus::Idle }])
        };

        // accepted: delivered, then the turn ends
        let (mut agent, mut events) = fake_agent(&server);
        (agent.thread_id, agent.active_turn) = (Some("t".into()), Some("u".into()));
        agent.pending.insert(7, Pending::Steer("also run the tests".into()));
        agent.on_notification("turn/completed", &done).await;
        // what Codex sends after the end stays behind it
        agent.on_notification("thread/status/changed", &json!({ "threadId": "t", "status": { "type": "idle" } })).await;
        assert!(emitted(&mut events).is_empty());
        assert!(agent.active_turn.is_some());
        agent.on_response(7, Ok(json!({ "turnId": "u" })), &mut None).await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::PromptSteered { input }, end @ ..] if input == "also run the tests" && ended(end)), "{kinds:?}");
        assert!(agent.active_turn.is_none() && agent.held.is_empty());

        // two steers: the end waits for the last answer. That one is refused (the turn was
        // over), so its prompt starts the next turn, after the end.
        let (mut agent, mut events) = fake_agent(&server);
        (agent.thread_id, agent.active_turn) = (Some("t".into()), Some("u".into()));
        agent.pending.insert(7, Pending::Steer("also run the tests".into()));
        agent.pending.insert(8, Pending::Steer("and the docs".into()));
        agent.on_notification("turn/completed", &done).await;
        agent.on_response(7, Ok(json!({ "turnId": "u" })), &mut None).await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::PromptSteered { .. }]), "{kinds:?}");
        agent.on_response(8, Err(json!({ "message": "no active turn to steer" })), &mut None).await;
        let kinds = emitted(&mut events);
        // the fake server takes no write: the next turn's `turn/start` shows as this error
        assert!(
            matches!(&kinds[..], [end @ .., EventKind::Error { message, fatal: false }] if ended(end) && message.starts_with("turn/start:")),
            "{kinds:?}"
        );
        assert!(agent.queue.is_empty());
    }

    /// The spawner gave up after 60 s and failed the run itself: the adapter stays silent.
    #[tokio::test]
    async fn a_start_nobody_waits_for_emits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let announces = |k: &EventKind| matches!(k, EventKind::SessionStarted { .. } | EventKind::StatusChanged { .. } | EventKind::Exited { .. });
        let gave_up = || Some(oneshot::channel::<anyhow::Result<()>>().0); // its receiver is dropped

        // the thread never opens: the spawner's Shutdown ends the adapter
        let (mut agent, mut events) = fake_agent(&server);
        agent.on_cmd(AgentCmd::Shutdown).await;
        assert_eq!(agent.stage, Stage::Done);
        let kinds = emitted(&mut events);
        assert!(!kinds.iter().any(announces), "{kinds:?}");

        // the thread opens after all: it is let go, not announced
        let (mut agent, mut events) = fake_agent(&server);
        agent.on_response(0, Ok(json!({ "thread": { "id": "t" } })), &mut gave_up()).await;
        agent.on_cmd(AgentCmd::Shutdown).await;
        assert_eq!(agent.stage, Stage::Done);
        let kinds = emitted(&mut events);
        assert!(!kinds.iter().any(announces), "{kinds:?}");
    }

    /// Codex never confirmed the interrupt, so its `turn/completed` will not come: the adapter
    /// closes the turn itself. (The 5 s deadline is fired by hand: tokio's paused clock is not
    /// among the workspace's features.)
    #[tokio::test]
    async fn shutdown_closes_the_open_turn_as_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        // the turn runs (false), or its `turn/start` is not answered yet (true)
        for pending in [false, true] {
            let (mut agent, mut events) = fake_agent(&server);
            (agent.started, agent.thread_id) = (true, Some("t".into()));
            agent.active_turn = (!pending).then(|| "u".to_string());
            agent.turn_pending = pending;
            agent.on_cmd(AgentCmd::Shutdown).await;
            assert_eq!(agent.stage, Stage::Interrupting);
            agent.on_deadline().await;
            assert_eq!(agent.stage, Stage::Done);
            let kinds = emitted(&mut events);
            // Interrupted, not Failed: a failed turn sends a stopped worker to the gate
            let closed = kinds.iter().position(|k| matches!(k, EventKind::TurnCompleted { status: TurnStatus::Interrupted, .. }));
            let exited = kinds.iter().position(|k| matches!(k, EventKind::Exited { .. }));
            assert!(closed.is_some() && closed < exited, "{kinds:?}");
            assert_eq!(kinds.iter().filter(|k| matches!(k, EventKind::TurnCompleted { .. })).count(), 1, "{kinds:?}");
        }
    }

    fn turn_ends(kinds: &[EventKind]) -> Vec<TurnStatus> {
        kinds.iter().filter_map(|k| if let EventKind::TurnCompleted { status, .. } = k { Some(*status) } else { None }).collect()
    }

    /// `turn/start` is never answered: the thread is let go and the turn ends as failed. A
    /// server that said nothing to anybody meanwhile is hung and is given up; one that still
    /// talks is kept for its other agents. (Deadlines are fired by hand, as above.)
    #[tokio::test]
    async fn a_turn_that_never_starts_ends_the_run() {
        let dir = tempfile::tempdir().unwrap();
        for talks in [false, true] {
            let server = fake_server(dir.path());
            if talks {
                server.last_inbound_ms.store(now_ms(), Ordering::SeqCst);
            }
            let (mut agent, mut events) = fake_agent(&server);
            (agent.started, agent.thread_id, agent.turn_pending) = (true, Some("t".into()), true);
            agent.pending.insert(3, Pending::TurnStart);
            agent.steer_queue.push("then test it".into());
            agent.on_deadline().await;
            assert_eq!(agent.stage, Stage::Done);
            let kinds = emitted(&mut events);
            assert!(
                matches!(&kinds[0], EventKind::Error { message, fatal: false }
                    if message.starts_with("Codex did not start the turn within 60 s") && message.contains("1 message that waited for this turn was not sent")),
                "{kinds:?}"
            );
            assert_eq!(turn_ends(&kinds), [TurnStatus::Failed], "{kinds:?}");
            assert!(matches!(kinds.last(), Some(EventKind::Exited { .. })), "{kinds:?}");
            assert_eq!(server.usable(), talks);
            // An answer that comes after all starts nothing here.
            agent.on_response(3, Ok(json!({ "turn": { "id": "u" } })), &mut None).await;
            assert!(agent.active_turn.is_none() && emitted(&mut events).is_empty());
        }
    }

    /// The turn does not end after `turn/interrupt`: the thread is let go and the turn ends as
    /// interrupted. It may still run on the server, so a server no other agent uses is ended too.
    #[tokio::test]
    async fn an_interrupt_that_does_not_end_the_turn_ends_the_run() {
        let dir = tempfile::tempdir().unwrap();
        // Who else is on the server: nobody, an agent with a thread, an agent that opens one.
        for other in ["nobody", "a thread", "a start"] {
            let shared = other != "nobody";
            let server = fake_server(dir.path());
            server.last_inbound_ms.store(now_ms(), Ordering::SeqCst);
            let (mut agent, mut events) = fake_agent(&server);
            let route = || Route { tx: mpsc::unbounded_channel().0, raw: server.bootstrap_raw.clone() };
            {
                let mut st = server.state.lock().unwrap();
                // The agent's own `turn/interrupt`, never answered, keeps nobody's server alive.
                st.pending.insert(7, Waiter::Agent { route: agent.route.clone(), register: false });
                if other == "a thread" {
                    st.routes.insert("other".into(), route());
                }
                if other == "a start" {
                    st.pending.insert(8, Waiter::Agent { route: route(), register: true });
                }
            }
            (agent.started, agent.thread_id, agent.active_turn) = (true, Some("t".into()), Some("u".into()));
            agent.on_cmd(AgentCmd::Interrupt).await;
            assert!(agent.deadline.is_some() && agent.stage == Stage::Running);
            emitted(&mut events);

            agent.on_deadline().await;
            assert_eq!(agent.stage, Stage::Done);
            let kinds = emitted(&mut events);
            assert!(matches!(&kinds[0], EventKind::Error { message, fatal: false } if message.starts_with("Codex did not stop within 15 s")), "{kinds:?}");
            assert_eq!(turn_ends(&kinds), [TurnStatus::Interrupted], "{kinds:?}");
            assert!(matches!(kinds.last(), Some(EventKind::Exited { .. })), "{kinds:?}");
            assert_eq!(server.usable(), shared);
        }
    }

    #[tokio::test]
    async fn an_interrupted_turn_that_ends_in_time_keeps_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let (mut agent, _events) = fake_agent(&server);
        (agent.started, agent.thread_id, agent.active_turn) = (true, Some("t".into()), Some("u".into()));
        agent.on_cmd(AgentCmd::Interrupt).await;
        assert!(agent.deadline.is_some());
        agent.on_notification("turn/completed", &json!({ "threadId": "t", "turn": { "status": "interrupted" } })).await;
        assert!(agent.deadline.is_none() && agent.active_turn.is_none());
        assert_eq!(agent.stage, Stage::Running);
        assert!(server.usable());
    }

    /// Every agent of an account writes to one app-server. One that stops reading must not
    /// hold them all behind a write: the write gives up, and so does every later one, at once.
    #[tokio::test]
    async fn a_server_that_stops_reading_is_given_up() {
        let dir = tempfile::tempdir().unwrap();
        let env = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        let mut child = proc::command(Path::new("/bin/sh"), &["-c".into(), "exec sleep 30".into()], dir.path(), &env).spawn().unwrap();
        let server = fake_server(dir.path());
        *server.stdin.lock().await = child.stdin.take();
        let raw = server.bootstrap_raw.clone();
        let big = json!({ "method": "turn/start", "params": { "text": "x".repeat(1 << 20) } });
        let e = server.send_within(&big, &raw, Duration::from_millis(200)).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut, "{e}");
        assert!(!server.usable(), "no new agent starts on a server that stopped reading");
        let e = server.send(&json!({ "method": "initialized" }), &raw).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
    }

    #[test]
    fn account_read_tells_a_signed_out_codex() {
        assert!(signed_out(&json!({ "account": null, "requiresOpenaiAuth": true })));
        assert!(!signed_out(&json!({ "account": { "type": "chatgpt" }, "requiresOpenaiAuth": true })));
        assert!(!signed_out(&json!({ "account": null, "requiresOpenaiAuth": false })), "another model provider needs no OpenAI sign-in");
        assert!(!signed_out(&json!({})), "an answer that does not say is not a sign-out");
    }

    /// An account whose app-server is a `/bin/sh` that runs `script` (the `app-server` file of
    /// its cwd, `<dir>/<home>`).
    fn sh_account(dir: &Path, home: &str, script: &str) -> SpawnSpec {
        let cwd = dir.join(home);
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("app-server"), script).unwrap();
        SpawnSpec {
            agent_id: home.into(),
            run_id: home.into(),
            provider: Provider::Codex,
            role: Role::Worker,
            model: None,
            bin: PathBuf::from("/bin/sh"),
            env: HashMap::from([("CODEX_HOME".to_string(), home.to_string())]),
            session: SessionMode::New { session_id: None },
            system_prompt: String::new(),
            read_only: false,
            mcp: None,
            use_user_setup: false,
            raw_log: cwd.join("raw.ndjson"),
            byok_command: None,
            cwd,
        }
    }

    /// Detection said signed in, but nobody is any more: the start fails with the command that
    /// signs this account in, not a token error at the first turn.
    #[tokio::test]
    async fn a_signed_out_account_fails_at_start_with_the_sign_in_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = sh_account(
            dir.path(),
            "out",
            r#"while read -r line; do
  case "$line" in
    *'"id":0'*) echo '{"id":0,"result":{"userAgent":"codex/0.161.0 (sh)"}}' ;;
    *'"id":1'*) echo '{"id":1,"result":{"account":null,"requiresOpenaiAuth":true}}' ;;
  esac
done
"#,
        );
        let raw = Arc::new(RawLog::open(&dir.path().join("raw.ndjson")).unwrap());
        let e = server_for(&Servers::default(), &out, &raw).await.map(|_| ()).unwrap_err();
        assert_eq!(e.to_string(), "Codex is not signed in: run `CODEX_HOME=out codex login` in a terminal");
    }

    /// Two accounts, each with a `/bin/sh` for app-server (it runs the `app-server` script of
    /// its cwd). One never answers `initialize`: the other's agents must not wait behind it.
    /// The list is the test's own: `SERVERS` belongs to the whole process, and a test that
    /// shuts a Core down closes every server in it.
    #[tokio::test]
    async fn a_slow_account_does_not_block_another() {
        let dir = tempfile::tempdir().unwrap();
        let account = |home: &str, script: &str| sh_account(dir.path(), home, script);
        let slow = account("slow", ": > up\nwhile read -r line; do :; done\n");
        let fast = account(
            "fast",
            r#"while read -r line; do
  case "$line" in
    *'"id":0'*) echo '{"id":0,"result":{"userAgent":"codex/0.0.0 (sh)"}}' ;;
    *'"id":1'*) echo '{"id":1,"result":{"account":{"type":"apiKey"}}}' ;;
  esac
done
"#,
        );
        let servers = Servers::default();
        let raw = Arc::new(RawLog::open(&dir.path().join("raw.ndjson")).unwrap());

        let stuck = server_for(&servers, &slow, &raw);
        tokio::pin!(stuck);
        let others = async {
            while !slow.cwd.join("up").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await; // until the slow start is under way
            }
            tokio::join!(server_for(&servers, &fast, &raw), server_for(&servers, &fast, &raw))
        };
        let (a, b) = tokio::select! {
            r = &mut stuck => panic!("the start that never gets an answer ended: {:?}", r.map(|_| ())),
            r = tokio::time::timeout(Duration::from_secs(10), others) => r.expect("waited behind the other account's start"),
        };
        // still one server per account, however many agents ask at once
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.info.lock().unwrap().billing, Billing::ApiKey);
        a.close().await;
    }
}
