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
//! Questions that are no part of a run (the models, the account's usage, a compaction, the
//! user's own MCP servers) are in `account.rs`.
//!
//! A prompt sent while a turn runs is steered into it (`turn/steer`), so it does not get a
//! `TurnStarted`/`TurnCompleted` of its own: a `PromptSteered` says it was delivered. Codex
//! may report the turn's end before it answers the steer: the adapter keeps that end back
//! until the answer, so a `PromptSteered` always comes before its turn's `TurnCompleted`.
//!
//! A Codex subagent works on a thread of its own (`thread.parentThreadId` names the thread
//! that spawned it). That thread is routed to the agent that owns the parent: the subagent's
//! approvals reach Codly, its tool calls are rows of the agent marked `input.subagent`, its
//! tokens count for the run. Its words, plan and turns stay out: they are not the agent's.

mod account;

pub use account::{AccountQuery, compact_thread, models, rate_limits};

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
use super::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec, attachments, startup};
use crate::model::{
    ActivityKind, AgentStatus, Approval, ApprovalChoice, ApprovalDecision, ApprovalStatus, ApprovalSubject, Billing, EventKind,
    Id, MessagePhase, ModelOption, PlanStep, Provider, TurnStatus, Usage, new_id, now_ms,
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
/// How long Codex has to start the turn of a compaction it was asked for. Prompts wait for
/// that turn's end; past this limit they get their turns as if nothing was asked.
const COMPACT_START_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// A `Oneshot` whose answer holds what the user keeps in their own Codex config (keys of
    /// their MCP servers and model providers): the raw log gets a note in its place.
    Unlogged(oneshot::Sender<Result<Value, Value>>, Arc<RawLog>),
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
    /// Not from the server: Codly asks the thread's agent to have it compacted
    /// (`account::compact_thread`). The agent decides, so a compaction and the agent's own
    /// turns never start into each other.
    Compact(oneshot::Sender<anyhow::Result<()>>),
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

    /// One route per agent: an agent whose subagents have threads is in `routes` once per thread.
    fn agents(&self) -> Vec<Route> {
        let mut agents: Vec<Route> = Vec::new();
        for r in self.routes.values() {
            if !agents.iter().any(|a| a.tx.same_channel(&r.tx)) {
                agents.push(r.clone());
            }
        }
        agents
    }

    /// Give the subagent threads this message names to the agent that owns their parent: a
    /// `thread/started` with a routed `parentThreadId`, the agents a routed thread's `spawnAgent`
    /// call created, the thread of its `subAgentActivity`. A thread that has a route keeps it.
    fn adopt_subagents(&mut self, method: &str, params: &Value) {
        let (parent, children): (Option<&str>, Vec<&str>) = match method {
            "thread/started" => {
                let thread = &params["thread"];
                (thread["parentThreadId"].as_str(), thread["id"].as_str().into_iter().collect())
            }
            "item/started" | "item/completed" => {
                let item = &params["item"];
                let children: Vec<&str> = match item["type"].as_str() {
                    Some("collabAgentToolCall") if item["tool"] == "spawnAgent" => {
                        item["receiverThreadIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect()
                    }
                    Some("subAgentActivity") => item["agentThreadId"].as_str().into_iter().collect(),
                    _ => return,
                };
                (params["threadId"].as_str(), children)
            }
            _ => return,
        };
        let Some(route) = parent.and_then(|p| self.routes.get(p)).cloned() else { return };
        for child in children {
            self.routes.entry(child.to_string()).or_insert_with(|| route.clone());
        }
    }
}

/// The thread a server message is about: its `threadId`, or the thread of a `thread/started`.
fn thread_of(params: &Value) -> Option<&str> {
    params["threadId"].as_str().or(params["thread"]["id"].as_str())
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
    /// The model list this server gave (`account::models`), asked once.
    models: Mutex<Option<Vec<ModelOption>>>,
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
            models: Mutex::new(None),
        });
        let reader = server.clone();
        proc::spawn_reader(child.stdout.take().context("no stdout")?, move |rec| reader.on_record(rec));
        let waiter = server.clone();
        // Also ends what the server left running in its group (the commands of its agents) once it is gone.
        proc::reap(child, move |code| waiter.on_exit(code));

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
                    Some(Waiter::Unlogged(tx, raw)) => {
                        raw.note(&format!("the answer to request {id} is not logged: it holds your own Codex config"));
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
                st.adopt_subagents(&method, &params);
                let tid = thread_of(&params).map(str::to_owned);
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
                    let routes = st.agents();
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
        self.call_as(Waiter::Oneshot, method, params, raw, timeout).await
    }

    /// `call` for an answer that must not reach the raw log (`Waiter::Unlogged`).
    async fn call_unlogged(&self, method: &str, params: Value, raw: &Arc<RawLog>, timeout: Duration) -> anyhow::Result<Value> {
        self.call_as(Waiter::Unlogged, method, params, raw, timeout).await
    }

    async fn call_as(
        &self,
        waiter: fn(oneshot::Sender<Result<Value, Value>>, Arc<RawLog>) -> Waiter,
        method: &str,
        params: Value,
        raw: &Arc<RawLog>,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let (tx, rx) = oneshot::channel();
        let id = self.request(method, params, waiter(tx, raw.clone()), raw).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("{method} failed: {}", e["message"].as_str().map(str::to_owned).unwrap_or_else(|| e.to_string()))),
            Ok(Err(_)) => Err(anyhow!("codex app-server exited during {method}")),
            Err(_) => {
                // Nobody waits any more: a request left here would make the server look in use
                // for good (`give_up_if_unused`).
                self.state.lock().unwrap().pending.remove(&id);
                Err(anyhow!("codex app-server did not answer {method} within {} s", timeout.as_secs()))
            }
        }
    }

    /// Forget an agent's threads: its own and its subagents'.
    fn unroute(&self, agent: &Route) {
        self.state.lock().unwrap().routes.retain(|_, r| !r.tx.same_channel(&agent.tx));
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

/// `thread/start` (or `thread/resume`) and its params for one agent. `own_servers` = the MCP
/// servers of the user's own Codex config that this thread must not start (`use_user_setup`
/// off; `account::own_mcp_servers`).
pub fn thread_request(spec: &SpawnSpec, model: Option<&str>, effort: Option<&str>, own_servers: &[String]) -> (&'static str, Value) {
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
    // A `-c` key is merged into the user's config, so `enabled` alone switches the user's own
    // entry off for this thread. A server Codly passes itself under the same name stays.
    for name in own_servers {
        if config["mcp_servers"].get(name).is_none() {
            config["mcp_servers"][name] = json!({ "enabled": false });
        }
    }
    if spec.sandbox_network && !spec.read_only {
        config["sandbox_workspace_write"] = json!({ "network_access": true });
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
    spawn_on(&SERVERS, spec, events).await
}

/// `spawn` on the app-server that `servers` has for the agent's sign-in.
async fn spawn_on(servers: &Servers, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> anyhow::Result<AgentHandle> {
    let raw = Arc::new(RawLog::open(&spec.raw_log).with_context(|| format!("cannot open raw log {}", spec.raw_log.display()))?);
    let server = server_for(servers, &spec, &raw).await.map_err(|e| startup::explain(&spec, None, e))?;
    proc::record_group(&spec.raw_log, server.pgid, server.started_ms);
    let (msg_tx, msgs) = mpsc::unbounded_channel();
    let route = Route { tx: msg_tx, raw: raw.clone() };
    let (model, effort) = classify::split_model_effort(spec.model.as_deref());
    let own_servers = if spec.use_user_setup {
        Vec::new()
    } else {
        match account::own_mcp_servers(&server, &spec.cwd, &raw).await {
            Ok(names) => names,
            // The thread still starts, with the user's servers, and says so.
            Err(e) => {
                let message = format!("Codly could not read your Codex config ({e:#}), so your own MCP servers may start with this agent.");
                let _ = events.send(AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind: EventKind::Error { message, fatal: false } });
                Vec::new()
            }
        }
    };
    let (method, params) = thread_request(&spec, model.as_deref(), effort.as_deref(), &own_servers);
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
        compaction: Compaction::No,
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
    /// `thread/compact/start`, and who hears whether Codex took it.
    Compact(oneshot::Sender<anyhow::Result<()>>),
}

/// A compaction of the thread that Codly asked Codex for (`CodexAgent::compact`). Codex
/// compacts in a turn of its own. Codly did not start that turn and it answers no prompt, so
/// it is no turn of the agent: its start and end are not reported as one (a worker's turn end
/// runs the gate, a Lead's forwards its reply). A prompt sent from the request to that turn's
/// end waits in `queue`: a `turn/start` sent before the turn has started could be taken for
/// it, and Codex takes no prompt into it (`activeTurnNotSteerable`, turn kind `compact`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Compaction {
    No,
    /// `thread/compact/start` is sent; the turn Codex compacts in has not started.
    Asked,
    /// `active_turn` is that turn. An interrupt or a shutdown stops it like any turn.
    Running,
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
    compaction: Compaction,
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
    /// While running: when `turn/start` must be answered, an interrupted turn must have ended,
    /// or a compaction that was asked for must have started. In a shutdown: the stage's limit.
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
                    Some(ServerMsg::Compact(reply)) => self.compact(reply).await,
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
                self.norm.thread = self.thread_id.clone();
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
            // Codex took the request: the turn it compacts in is still to start (`on_deadline`).
            (Pending::Compact(reply), Ok(_)) => {
                let _ = reply.send(Ok(()));
            }
            (Pending::Compact(reply), Err(e)) => {
                let _ = reply.send(Err(anyhow!(err_text(&e))));
                self.compaction_off().await;
            }
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
        if !self.held.is_empty() || (self.ends_own_turn(method, params) && self.steer_unanswered()) {
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

    /// The end of the agent's own turn. A subagent's turn ends on its own thread, and the
    /// agent's turn goes on.
    fn ends_own_turn(&self, method: &str, params: &Value) -> bool {
        method == "turn/completed" && self.norm.subagent_thread(params).is_none()
    }

    async fn handle_notification(&mut self, method: &str, params: &Value) {
        let own_turn_ended = self.ends_own_turn(method, params);
        let mut events = self.norm.on_notification(method, params);
        // No turn of the agent's own starts while a compaction is asked for (`compact`,
        // `on_cmd`): the turn that starts and ends on the thread now is the one Codex compacts in.
        if method == "turn/started"
            && self.compaction == Compaction::Asked
            && self.stage == Stage::Running
            && let Some(turn) = params["turn"]["id"].as_str()
        {
            self.active_turn = Some(turn.to_owned());
            self.compaction = Compaction::Running;
            self.answered_in_time();
        }
        if self.compaction != Compaction::No {
            events.retain(|e| !matches!(e, EventKind::TurnStarted { .. } | EventKind::TurnCompleted { .. }));
        }
        self.emit_all(events);
        if own_turn_ended {
            self.active_turn = None;
            self.compaction = Compaction::No;
            self.answered_in_time();
            match self.stage {
                Stage::Interrupting => self.unsubscribe().await,
                Stage::Running => self.next_turn().await,
                _ => {}
            }
        }
    }

    /// `account::compact_thread`: ask Codex to compact the thread (`thread/compact/start`),
    /// when no turn runs on it and no prompt waits for one. `reply` hears whether Codex took
    /// the request.
    async fn compact(&mut self, reply: oneshot::Sender<anyhow::Result<()>>) {
        let refusal = if self.stage != Stage::Running {
            Some("the agent is stopping")
        } else if self.compaction != Compaction::No {
            Some("a compaction is already under way")
        } else if self.active_turn.is_some() || self.turn_pending || !self.queue.is_empty() {
            Some("a turn started on it first")
        } else {
            None
        };
        if let Some(why) = refusal {
            let _ = reply.send(Err(anyhow!(why)));
            return;
        }
        let Some(tid) = self.thread_id.clone() else {
            let _ = reply.send(Err(anyhow!("its conversation is not open yet")));
            return;
        };
        let (method, params) = account::compact_request(&tid);
        let waiter = Waiter::Agent { route: self.route.clone(), register: false };
        match self.server.request(method, params, waiter, &self.raw).await {
            Ok(id) => {
                self.pending.insert(id, Pending::Compact(reply));
                self.compaction = Compaction::Asked;
                self.wait_at_most(COMPACT_START_TIMEOUT);
            }
            Err(e) => {
                let _ = reply.send(Err(anyhow!("{method}: {e}")));
            }
        }
    }

    /// The compaction that was asked for starts no turn (Codex refused it, or started none in
    /// time): the prompts that waited for it get their turns.
    async fn compaction_off(&mut self) {
        if self.compaction == Compaction::Asked {
            self.compaction = Compaction::No;
            self.answered_in_time();
            self.next_turn().await;
        }
    }

    async fn on_cmd(&mut self, cmd: AgentCmd) {
        match cmd {
            AgentCmd::Prompt(text) => {
                if self.stage != Stage::Running {
                    return;
                }
                if self.compaction != Compaction::No {
                    // Its turn comes when the compaction's turn has ended (`Compaction`).
                    self.queue.push_back(text);
                } else if self.active_turn.is_some() {
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
            // Claude's control requests: the orchestrator sends them to Claude agents only.
            AgentCmd::Session { reply, .. } => {
                let _ = reply.send(Err("Codex has no such request".into()));
            }
        }
    }

    /// The `input` of a turn: the text, which keeps the paths of its attached files, and the
    /// attached images as items of their own.
    async fn input(&self, text: &str) -> Value {
        let attached = attachments::images(text).await;
        if let Some(note) = attached.note {
            self.emit(EventKind::Error { message: note, fatal: false });
        }
        attachments::codex_input(text, &attached.images)
    }

    async fn start_turn(&mut self, text: String) {
        let Some(tid) = self.thread_id.clone() else { return };
        let mut params = json!({ "threadId": tid, "input": self.input(&text).await });
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
        let params = json!({ "threadId": tid, "input": self.input(&text).await, "expectedTurnId": turn });
        self.request("turn/steer", params, Pending::Steer(text)).await;
    }

    async fn interrupt(&mut self) {
        match (self.thread_id.clone(), self.active_turn.clone()) {
            (Some(tid), Some(turn)) => {
                self.norm.interrupt_requested = true;
                self.request("turn/interrupt", json!({ "threadId": tid, "turnId": turn }), Pending::Interrupt).await;
                self.interrupt_subagents().await;
                self.wait_at_most(INTERRUPT_TIMEOUT);
            }
            // The limit of its `turn/start` still runs.
            _ if self.turn_pending => self.interrupt_after_start = true,
            // No turn of its own, but its subagents may still work.
            _ => self.interrupt_subagents().await,
        }
    }

    /// A subagent's turn runs on its own thread: stopping the agent's turn does not stop it,
    /// and nobody else can.
    async fn interrupt_subagents(&mut self) {
        for (thread, turn) in self.norm.subagent_turns() {
            self.request("turn/interrupt", json!({ "threadId": thread, "turnId": turn }), Pending::Interrupt).await;
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
            self.interrupt_subagents().await;
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
            // Nothing else has a limit while a compaction is asked for: no turn is pending or runs.
            Stage::Running if self.compaction == Compaction::Asked => {
                let message = format!("Codex did not start the compaction within {} s.", COMPACT_START_TIMEOUT.as_secs());
                self.emit(EventKind::Error { message, fatal: false });
                self.compaction_off().await;
            }
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
        self.emit(EventKind::Error { message: format!("{what}, so Codly ended this run. The session is kept: its next run resumes it.{unsent}"), fatal: false });
        if std::mem::take(&mut self.turn_pending) {
            // An answer that comes after all starts nothing here: the turn is over for Codly.
            self.pending.retain(|_, what| !matches!(what, Pending::TurnStart));
            let events = self.norm.turn_failed();
            self.emit_all(events);
            // Nothing from the server for anybody all that time: it is hung for every agent on it.
            if self.server.silent_ms() >= START_TIMEOUT.as_millis() as i64 {
                self.server.give_up("it answered nothing while a turn waited to start");
            }
        } else {
            // The exit that follows ends an interrupt nobody answered: it is no crash.
            self.emit(super::gave_up("Codex", INTERRUPT_TIMEOUT));
        }
        self.unsubscribe().await;
    }

    fn finish(&mut self) {
        if self.stage == Stage::Done {
            return;
        }
        self.server.unroute(&self.route);
        let open = self.active_turn.take().is_some() || std::mem::take(&mut self.turn_pending);
        let foreign = std::mem::replace(&mut self.compaction, Compaction::No) == Compaction::Running;
        if self.started {
            let events = self.norm.cancel_all_approvals();
            self.emit_all(events);
            // Once unrouted, Codex's own `turn/completed` cannot reach us: close the turn here.
            // Interrupted, not Failed: this is a shutdown Codly asked for, and a failed turn
            // would send a stopped worker to the gate.
            if open && !foreign {
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
        if (self.active_turn.is_some() && self.compaction != Compaction::Running) || self.turn_pending {
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
    /// The subagent thread that runs it. None: the agent's own thread.
    thread: Option<String>,
}

/// A thread one of the agent's Codex subagents works on.
#[derive(Default)]
struct Subagent {
    /// Its nickname and role, when Codex gave them.
    name: Option<String>,
    /// What it was asked to do: the `prompt` of the `spawnAgent` call that names its thread.
    prompt: Option<String>,
    /// The key of its row in the agent's list of helpers: the `spawnAgent` call that named its
    /// thread before the row was first told (one row for the call and its subagent), else
    /// its thread.
    row: Option<String>,
    /// Its running turn: stopped when the agent is.
    turn: Option<String>,
    /// Its thread's token totals before this run first heard of them.
    baseline: Option<[i64; 4]>,
    /// What it spent since (input, cached input, cache write, output).
    spent: [i64; 4],
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
    /// The subagent thread that asks. None: the agent's own thread.
    thread: Option<String>,
}

/// Turns an agent's app-server notifications and server requests into `EventKind`s: those
/// of its own thread, and those of its subagents' threads.
pub struct CodexNormalizer {
    agent_id: Id,
    run_id: Id,
    cwd: PathBuf,
    resumed: bool,
    /// The agent's own thread, once it is open. A message about another thread is a subagent's.
    thread: Option<String>,
    /// Its subagents, by thread id.
    subagents: HashMap<String, Subagent>,
    /// The thread's goal as last told: (objective, status).
    goal: Option<(String, String)>,
    status: Option<AgentStatus>,
    /// The thread is flagged waitingOnApproval / waitingOnUserInput.
    waiting: bool,
    items: HashMap<String, ItemInfo>,
    approvals: HashMap<Id, PendingApproval>,
    last_final: Option<String>,
    usage: Usage,
    /// Thread totals before this run (a resumed thread's `total` includes earlier runs).
    baseline: Option<[i64; 4]>,
    /// What the agent's own thread spent in this run (`usage` adds its subagents').
    spent: [i64; 4],
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
            thread: None,
            subagents: HashMap::new(),
            goal: None,
            status: None,
            waiting: false,
            items: HashMap::new(),
            approvals: HashMap::new(),
            last_final: None,
            usage: Usage::default(),
            baseline: None,
            spent: [0; 4],
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

    /// A subagent still runs a tool or waits for an answer. The agent's own turn may be over.
    fn subagents_busy(&self) -> bool {
        self.items.values().any(|i| i.thread.is_some()) || self.approvals.values().any(|a| a.thread.is_some())
    }

    /// The status when the agent's own thread has no turn: free, unless a subagent is busy.
    fn idle_status(&self) -> AgentStatus {
        if self.subagents_busy() { self.busy_status() } else { AgentStatus::Idle }
    }

    /// The status once a tool call or an approval is over.
    fn resting_status(&self) -> AgentStatus {
        if self.in_turn { self.busy_status() } else { self.idle_status() }
    }

    /// The subagent thread a message is about. None: the agent's own thread, or no thread.
    fn subagent_thread<'a>(&self, p: &'a Value) -> Option<&'a str> {
        let own = self.thread.as_deref()?;
        thread_of(p).filter(|t| *t != own)
    }

    /// "Subagent Ada (explorer)", or "A subagent" when Codex gave it no name.
    fn subagent_label(&self, thread: &str) -> String {
        match self.subagents.get(thread).and_then(|s| s.name.as_deref()) {
            Some(name) => format!("Subagent {name}"),
            None => "A subagent".into(),
        }
    }

    /// The subagent's row in the agent's list of helpers. It says who works and on what, as far
    /// as Codex told: "Ada (explorer): <prompt>", the name, the prompt, or "Codex subagent".
    fn subagent_event(&mut self, thread: &str, status: &str) -> EventKind {
        let sub = self.subagents.entry(thread.to_string()).or_default();
        let call_id = sub.row.get_or_insert_with(|| thread.to_string()).clone();
        let description = match (&sub.name, &sub.prompt) {
            (Some(name), Some(prompt)) => format!("{name}: {prompt}"),
            (name, prompt) => name.clone().or_else(|| prompt.clone()).unwrap_or_else(|| "Codex subagent".into()),
        };
        EventKind::SubAgent { call_id, description, status: status.into() }
    }

    /// A `spawnAgent` call names the thread it created (`receiverThreadIds`) and what it asked
    /// of it (`prompt`). A thread whose row was not told yet takes the row of the call: the
    /// list of helpers then has one row for the call and the subagent it started, not two.
    fn spawned(&mut self, call: &str, item: &Value) {
        if item["tool"] != "spawnAgent" {
            return;
        }
        let threads: Vec<&str> = item["receiverThreadIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let prompt = item["prompt"].as_str().map(str::trim).filter(|p| !p.is_empty()).map(|p| classify::truncate_chars(p, NOTICE_MAX));
        for thread in &threads {
            let sub = self.subagents.entry(thread.to_string()).or_default();
            if sub.prompt.is_none() {
                sub.prompt = prompt.clone();
            }
            // One row cannot stand for two subagents: a call that names several keeps its own.
            if threads.len() == 1 && sub.row.is_none() {
                sub.row = Some(call.to_string());
            }
        }
    }

    /// A message of a subagent's thread names the turn it is part of. A thread the agent got
    /// after its `turn/started` has its running turn from here, so a stop reaches it.
    fn subagent_in_turn(&mut self, thread: &str, turn: &Value) {
        if let Some(turn) = turn.as_str() {
            self.subagents.entry(thread.to_string()).or_default().turn.get_or_insert_with(|| turn.to_string());
        }
    }

    /// The status once the agent's own turn is over. Codly takes an agent for free when its
    /// turn ends, so the status a busy subagent keeps it at is told again, changed or not.
    fn rest_after_turn(&mut self, out: &mut Vec<EventKind>) {
        let st = self.idle_status();
        if st != AgentStatus::Idle {
            self.status = None;
        }
        self.set_status(st, out);
    }

    /// The running turns of the agent's subagents: (thread, turn).
    fn subagent_turns(&self) -> Vec<(String, String)> {
        self.subagents.iter().filter_map(|(thread, s)| Some((thread.clone(), s.turn.clone()?))).collect()
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
        self.rest_after_turn(&mut out);
        out
    }

    pub fn on_notification(&mut self, method: &str, p: &Value) -> Vec<EventKind> {
        match self.subagent_thread(p) {
            Some(thread) => self.subagent_notification(thread, method, p),
            None => self.own_notification(method, p),
        }
    }

    /// What a subagent does on its own thread. Its tool calls, approvals, errors and tokens
    /// are told as the agent's, marked as the subagent's. Its words, reasoning, plan and
    /// context are not the agent's own: they stay in the raw log.
    fn subagent_notification(&mut self, thread: &str, method: &str, p: &Value) -> Vec<EventKind> {
        let mut out = Vec::new();
        match method {
            "thread/started" => self.subagents.entry(thread.to_string()).or_default().name = subagent_name(&p["thread"]),
            "turn/started" => {
                self.subagents.entry(thread.to_string()).or_default().turn = p["turn"]["id"].as_str().map(str::to_owned);
                out.push(self.subagent_event(thread, "running"));
            }
            "turn/completed" | "thread/closed" => {
                let running = self.subagents.entry(thread.to_string()).or_default().turn.take().is_some();
                // What it still waited for or ran ends with its turn.
                out.extend(self.cancel_approvals(Some(thread)));
                self.items.retain(|_, i| i.thread.as_deref() != Some(thread));
                if method == "turn/completed" {
                    let turn = &p["turn"];
                    let status = match turn["status"].as_str() {
                        Some("completed") => "completed",
                        Some("interrupted") => "stopped",
                        _ => "failed",
                    };
                    if status == "failed"
                        && let Some(m) = turn["error"]["message"].as_str()
                    {
                        out.push(EventKind::Error { message: format!("{}: {m}", self.subagent_label(thread)), fatal: false });
                    }
                    out.push(self.subagent_event(thread, status));
                } else if running {
                    out.push(self.subagent_event(thread, "stopped"));
                }
                let st = self.resting_status();
                self.set_status(st, &mut out);
            }
            "item/started" | "item/completed" => {
                self.subagent_in_turn(thread, &p["turnId"]);
                let item = &p["item"];
                let (ty, id) = (item["type"].as_str().unwrap_or(""), s(item, "id"));
                if is_tool_item(ty) {
                    if method == "item/started" {
                        self.tool_started(ty, &id, item, Some(thread), &mut out);
                    } else {
                        self.tool_ended(ty, &id, item, Some(thread), &mut out);
                    }
                }
            }
            "thread/tokenUsage/updated" => out.push(self.subagent_usage(thread, &p["tokenUsage"])),
            "error" => {
                let who = self.subagent_label(thread);
                out = self.own_notification(method, p);
                for e in &mut out {
                    if let EventKind::Error { message, .. } | EventKind::Retrying { reason: message, .. } = e {
                        *message = format!("{who}: {message}");
                    }
                }
            }
            // These name their tool call, request or hook: the thread makes no difference.
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" | "serverRequest/resolved" | "hook/started" | "hook/completed" => {
                out = self.own_notification(method, p);
            }
            _ => {}
        }
        out
    }

    fn own_notification(&mut self, method: &str, p: &Value) -> Vec<EventKind> {
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
                // A subagent's turn is its own: what it runs or waits for outlives this one.
                out.extend(self.cancel_approvals(None));
                self.items.retain(|_, i| i.thread.is_some());
                self.waiting = false;
                self.interrupt_requested = false;
                self.turn_announced = false;
                self.in_turn = false;
                out.push(EventKind::TurnCompleted { status, result_text: self.last_final.take(), duration_ms: turn["durationMs"].as_i64() });
                self.rest_after_turn(&mut out);
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
                    let st = self.idle_status();
                    self.set_status(st, &mut out);
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
                    let st = self.resting_status();
                    self.set_status(st, &mut out);
                }
            }
            // A hook that starts is bookkeeping. What it said or stopped is told when it ends.
            "hook/started" => {}
            "hook/completed" => out.extend(hook_row(&p["run"])),
            "thread/goal/updated" => {
                let goal = &p["goal"];
                let told = (s(goal, "objective"), s(goal, "status"));
                // An update of the tokens or the time it used is no news: a row when it changes.
                if self.goal.as_ref() != Some(&told) {
                    out.push(EventKind::Notice { text: goal_text(&told.0, &told.1, goal["tokenBudget"].as_i64()) });
                    self.goal = Some(told);
                }
            }
            "thread/goal/cleared" => {
                self.goal = None;
                out.push(EventKind::Notice { text: "Goal cleared".into() });
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
            ty if is_tool_item(ty) => self.tool_started(ty, &id, item, None, out),
            other => out.push(EventKind::Unknown { raw_type: format!("item/started:{other}") }),
        }
    }

    /// `thread`: the subagent thread that runs the call (None: the agent's own).
    fn tool_started(&mut self, ty: &str, id: &str, item: &Value, thread: Option<&str>, out: &mut Vec<EventKind>) {
        if self.items.contains_key(id) {
            return;
        }
        let (activity, target, confidence) = classify::classify_tool(Provider::Codex, ty, item);
        let changes = item["changes"].as_array().cloned().unwrap_or_default();
        let diff = (!changes.is_empty()).then(|| changes.iter().filter_map(|c| c["diff"].as_str()).collect::<Vec<_>>().join("\n"));
        let mut input = item.clone();
        if let Some(o) = input.as_object_mut() {
            o.remove("aggregatedOutput"); // empty at start; the output streams separately
            if let Some(t) = thread {
                // The mark of a subagent's work: the row is the agent's, the call is not its own.
                o.insert("subagent".into(), json!({ "thread": t, "name": self.subagents.get(t).and_then(|s| s.name.clone()) }));
            }
        }
        if ty == "collabAgentToolCall" {
            self.spawned(id, item);
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
                thread: thread.map(str::to_owned),
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
            ty if is_tool_item(ty) => self.tool_ended(ty, &id, item, None, out),
            other => out.push(EventKind::Unknown { raw_type: format!("item/completed:{other}") }),
        }
    }

    /// A tool item completed. `thread` as in `tool_started`.
    fn tool_ended(&mut self, ty: &str, id: &str, item: &Value, thread: Option<&str>, out: &mut Vec<EventKind>) {
        if !self.items.contains_key(id) {
            self.tool_started(ty, id, item, thread, out); // completed without a start event
        }
        self.tool_completed(ty, id, item, out);
        // Generated images land outside the worktree; announce them so they become artifacts.
        if ty == "imageGeneration"
            && let Some(path) = item["savedPath"].as_str()
        {
            out.push(EventKind::FilesChanged { call_id: Some(id.to_string()), paths: vec![path.to_string()], source: "provider".into() });
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
            self.spawned(id, item);
            // A subagent that took the call's row and still runs keeps it open: its own end closes it.
            if !self.subagents.values().any(|sub| sub.row.as_deref() == Some(id) && sub.turn.is_some()) {
                out.push(EventKind::SubAgent { call_id: id.to_string(), description: s(item, "prompt"), status: status.to_string() });
            }
        }
        let st = self.resting_status();
        self.set_status(st, out);
    }

    fn usage_update(&mut self, tu: &Value) -> EventKind {
        let (total, last) = (token_counts(&tu["total"]), token_counts(&tu["last"]));
        let resumed = self.resumed;
        let base = *self.baseline.get_or_insert_with(|| if resumed { std::array::from_fn(|i| total[i] - last[i]) } else { [0; 4] });
        self.spent = std::array::from_fn(|i| (total[i] - base[i]).max(0));
        self.usage.context_used = tu["last"]["totalTokens"].as_i64().or(self.usage.context_used);
        self.usage.context_window = tu["modelContextWindow"].as_i64().or(self.usage.context_window);
        self.usage_event()
    }

    /// A subagent spends on its own thread: its tokens count for the run, its context is not
    /// the agent's. What its thread had used before the first reading is not this run's.
    fn subagent_usage(&mut self, thread: &str, tu: &Value) -> EventKind {
        let (total, last) = (token_counts(&tu["total"]), token_counts(&tu["last"]));
        let sub = self.subagents.entry(thread.to_string()).or_default();
        let base = *sub.baseline.get_or_insert_with(|| std::array::from_fn(|i| total[i] - last[i]));
        sub.spent = std::array::from_fn(|i| (total[i] - base[i]).max(0));
        self.usage_event()
    }

    /// The run's tokens: the agent's own thread plus its subagents'.
    fn usage_event(&mut self) -> EventKind {
        let mut d = self.spent;
        for sub in self.subagents.values() {
            for (sum, n) in d.iter_mut().zip(sub.spent) {
                *sum += n;
            }
        }
        // Codex counts cached tokens inside inputTokens; Usage keeps them apart (as Claude does).
        self.usage.input_tokens = (d[0] - d[1]).max(0);
        self.usage.cache_read_tokens = d[1];
        self.usage.cache_write_tokens = d[2];
        self.usage.output_tokens = d[3];
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
        // A subagent's request is the agent's to get answered: the card says who asks.
        let thread = self.subagent_thread(p).map(str::to_owned);
        let reason = match &thread {
            Some(t) => {
                self.subagent_in_turn(t, &p["turnId"]);
                let who = self.subagent_label(t);
                Some(reason.map_or_else(|| format!("{who} asks for this"), |r| format!("{who}: {r}")))
            }
            None => reason,
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
        self.approvals.insert(approval.id.clone(), PendingApproval { request_id: id.clone(), shape, thread });
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
        // "cancel" also stops the turn that asked. Only the agent's own turn is then told as
        // stopped by the user: a subagent's cancelled card says nothing about that turn.
        if d.choice == ApprovalChoice::Cancel && p.thread.is_none() {
            self.interrupt_requested = true;
        }
        let mut out = Vec::new();
        self.waiting = false;
        let st = self.resting_status();
        self.set_status(st, &mut out);
        Some((json!({ "id": p.request_id, "result": result }), out))
    }

    /// Close the cards of one thread (None: the agent's own): its turn is over.
    fn cancel_approvals(&mut self, thread: Option<&str>) -> Vec<EventKind> {
        let ids: Vec<Id> = self.approvals.iter().filter(|(_, a)| a.thread.as_deref() == thread).map(|(id, _)| id.clone()).collect();
        ids.into_iter()
            .map(|id| {
                self.approvals.remove(&id);
                EventKind::ApprovalResolved { approval_id: id, status: ApprovalStatus::Cancelled, by: "provider".into() }
            })
            .collect()
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

/// Thread items that are tool calls: a row when they start, a result when they end.
fn is_tool_item(ty: &str) -> bool {
    matches!(
        ty,
        "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" | "webSearch" | "collabAgentToolCall" | "imageView" | "imageGeneration" | "sleep" | "subAgentActivity"
    )
}

/// A `TokenUsageBreakdown` as (input, cached input, cache write, output).
fn token_counts(b: &Value) -> [i64; 4] {
    [
        b["inputTokens"].as_i64().unwrap_or(0),
        b["cachedInputTokens"].as_i64().unwrap_or(0),
        b["cacheWriteInputTokens"].as_i64().unwrap_or(0),
        b["outputTokens"].as_i64().unwrap_or(0),
    ]
}

/// What Codex calls a subagent, from its thread: "Ada (explorer)", "Ada", "explorer" or nothing.
fn subagent_name(thread: &Value) -> Option<String> {
    let part = |key: &str| thread[key].as_str().map(str::trim).filter(|v| !v.is_empty());
    match (part("agentNickname"), part("agentRole")) {
        (Some(nick), Some(role)) => Some(format!("{nick} ({role})")),
        (nick, role) => nick.or(role).map(str::to_owned),
    }
}

/// How much of a hook's, a goal's or a subagent prompt's own text a row shows.
const NOTICE_MAX: usize = 300;

/// The row for a hook run that ended (`HookRunSummary`). A hook that ran and said nothing
/// gets none: only what it stopped or told the agent explains the turn.
fn hook_row(run: &Value) -> Option<EventKind> {
    let entries: Vec<(&str, &str)> = run["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e["kind"].as_str()?, e["text"].as_str()?.trim())))
        .filter(|(_, text)| !text.is_empty())
        .collect();
    let status = run["status"].as_str().unwrap_or("");
    let did = match status {
        "blocked" | "failed" | "stopped" => status,
        _ => match entries.first()?.0 {
            "context" => "added context",
            "feedback" => "gave feedback",
            "warning" => "gave a warning",
            "stop" => "asked to stop",
            "error" => "reported an error",
            _ => return None,
        },
    };
    // `statusMessage` is not what the hook said: it is the progress text of its configuration.
    let said = entries.iter().map(|(_, text)| *text).collect::<Vec<_>>().join(" ");
    // The file the hook comes from. The types do not say if that is its script or the file
    // that declares it, so the row only names it.
    let source = run["sourcePath"].as_str().and_then(|p| std::path::Path::new(p).file_name()).map(|f| format!(" ({})", f.to_string_lossy()));
    let mut text = format!("Codex hook {}{} {did}", run["eventName"].as_str().unwrap_or("?"), source.unwrap_or_default());
    if !said.is_empty() {
        text = format!("{text}: {}", classify::truncate_chars(&said, NOTICE_MAX));
    }
    Some(if status == "failed" { EventKind::Error { message: text, fatal: false } } else { EventKind::Notice { text } })
}

/// The row for a thread's goal (`ThreadGoal`) when its objective or status changes.
fn goal_text(objective: &str, status: &str, token_budget: Option<i64>) -> String {
    let objective = classify::truncate_chars(objective, NOTICE_MAX);
    match status {
        "active" => match token_budget {
            Some(budget) => format!("Goal: {objective} (budget: {budget} tokens)"),
            None => format!("Goal: {objective}"),
        },
        "paused" => format!("Goal paused: {objective}"),
        "blocked" => format!("Goal blocked: {objective}"),
        "usageLimited" => format!("Goal stopped at the usage limit: {objective}"),
        "budgetLimited" => format!("Goal stopped at its token budget: {objective}"),
        "complete" => format!("Goal complete: {objective}"),
        other => format!("Goal ({other}): {objective}"),
    }
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
        let ev = norm.on_notification("thread/archived", &json!({}));
        assert!(matches!(&ev[0], EventKind::Unknown { raw_type } if raw_type == "thread/archived"));
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

    /// A goal and a hook are rows a person can read. No capture has them: the lines are written
    /// from the codex-cli 0.166.0 schema types (ThreadGoalUpdatedNotification, ThreadGoal,
    /// ThreadGoalClearedNotification, HookStartedNotification, HookCompletedNotification,
    /// HookRunSummary, HookOutputEntry).
    #[test]
    fn goals_and_hooks_are_rows_one_can_read() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        let notices = |ev: &[EventKind]| -> Vec<String> { ev.iter().filter_map(|e| if let EventKind::Notice { text } = e { Some(text.clone()) } else { None }).collect() };
        let goal = |status: &str, used: i64| {
            let goal = json!({ "threadId": "t", "objective": "Make the parser pass its tests", "status": status, "tokenBudget": 50000,
                "tokensUsed": used, "timeUsedSeconds": 12, "createdAt": 1, "updatedAt": 2 });
            json!({ "threadId": "t", "turnId": null, "goal": goal })
        };
        assert_eq!(notices(&norm.on_notification("thread/goal/updated", &goal("active", 0))), ["Goal: Make the parser pass its tests (budget: 50000 tokens)"]);
        assert!(norm.on_notification("thread/goal/updated", &goal("active", 900)).is_empty(), "only its tokens moved");
        assert_eq!(notices(&norm.on_notification("thread/goal/updated", &goal("budgetLimited", 50000))), ["Goal stopped at its token budget: Make the parser pass its tests"]);
        assert_eq!(notices(&norm.on_notification("thread/goal/cleared", &json!({ "threadId": "t" }))), ["Goal cleared"]);
        assert_eq!(notices(&norm.on_notification("thread/goal/updated", &goal("budgetLimited", 50000))).len(), 1, "the same goal set again is news");

        // `statusMessage` is the progress text of the hook's configuration (ConfiguredHookHandler,
        // HookMetadata): it is never told as what the hook said.
        let hook = |status: &str, entries: Value| {
            let run = json!({ "id": "h1", "eventName": "preToolUse", "handlerType": "command", "executionMode": "sync", "scope": "turn",
                "sourcePath": "/Users/me/.codex/hooks.json", "source": "user", "displayOrder": 0, "status": status, "statusMessage": "Checking the command",
                "startedAt": 1, "completedAt": 2, "durationMs": 40, "entries": entries });
            json!({ "threadId": "t", "turnId": "u", "run": run })
        };
        assert!(norm.on_notification("hook/started", &hook("running", json!([]))).is_empty());
        assert!(norm.on_notification("hook/completed", &hook("completed", json!([]))).is_empty(), "a hook that ran and said nothing is no row");
        let ev = norm.on_notification("hook/completed", &hook("blocked", json!([{ "kind": "feedback", "text": "rm -rf is not allowed here" }])));
        assert_eq!(notices(&ev), ["Codex hook preToolUse (hooks.json) blocked: rm -rf is not allowed here"]);
        let ev = norm.on_notification("hook/completed", &hook("completed", json!([{ "kind": "context", "text": "Branch: main" }])));
        assert_eq!(notices(&ev), ["Codex hook preToolUse (hooks.json) added context: Branch: main"]);
        let ev = norm.on_notification("hook/completed", &hook("failed", json!([{ "kind": "error", "text": "guard.sh: command not found" }])));
        assert!(
            matches!(&ev[..], [EventKind::Error { message, fatal: false }] if message == "Codex hook preToolUse (hooks.json) failed: guard.sh: command not found"),
            "{ev:?}"
        );
        let ev = norm.on_notification("hook/completed", &hook("failed", json!([])));
        assert!(matches!(&ev[..], [EventKind::Error { message, fatal: false }] if message == "Codex hook preToolUse (hooks.json) failed"), "{ev:?}");
    }

    /// A Codex subagent works on a thread of its own. No capture has one: the lines are
    /// written from the codex-cli 0.166.0 schema types (ThreadStartedNotification, Thread,
    /// TurnStartedNotification, TurnCompletedNotification, Turn, ItemStartedNotification,
    /// ItemCompletedNotification, ThreadItem, CommandExecutionRequestApprovalParams,
    /// AgentMessageDeltaNotification, TurnPlanUpdatedNotification,
    /// ThreadTokenUsageUpdatedNotification, TokenUsageBreakdown).
    #[test]
    fn a_subagents_work_is_told_as_the_agents() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        norm.thread = Some("main".into());
        let mut events = norm.turn_started("go");
        let thread = json!({ "id": "sub", "parentThreadId": "main", "agentNickname": "Ada", "agentRole": "explorer", "preview": "" });
        events.extend(norm.on_notification("thread/started", &json!({ "thread": thread })));
        events.extend(norm.on_notification("turn/started", &json!({ "threadId": "sub", "turn": { "id": "su1", "status": "inProgress", "items": [] } })));
        assert_eq!(events.iter().filter(|e| matches!(e, EventKind::TurnStarted { .. })).count(), 1, "its turn is not the agent's");
        assert!(
            matches!(events.last(), Some(EventKind::SubAgent { call_id, description, status }) if call_id == "sub" && description == "Ada (explorer)" && status == "running"),
            "{events:?}"
        );
        assert_eq!(norm.subagent_turns(), [("sub".to_string(), "su1".to_string())]);

        // its command is a row of the agent, marked as the subagent's
        let cmd = |status: &str, exit: Value| {
            json!({ "type": "commandExecution", "id": "c1", "pluginId": null, "scriptPath": null, "command": "/bin/zsh -lc 'cargo build'", "cwd": "/w",
                "processId": null, "source": "agent", "status": status, "commandActions": [], "aggregatedOutput": null, "exitCode": exit, "durationMs": null })
        };
        let ev = norm.on_notification("item/started", &json!({ "threadId": "sub", "turnId": "su1", "item": cmd("inProgress", Value::Null), "startedAtMs": 1 }));
        assert!(
            matches!(&ev[..], [EventKind::ToolStarted { call_id, input, .. }, EventKind::StatusChanged { status: AgentStatus::Working }]
                if call_id == "c1" && input["subagent"] == json!({ "thread": "sub", "name": "Ada (explorer)" })),
            "{ev:?}"
        );

        // its request is the agent's to get answered, and says who asks
        let ask = json!({ "kind": "command", "threadId": "sub", "turnId": "su1", "itemId": "c1", "startedAtMs": 2, "environmentId": null, "reason": "needs the network" });
        let (ev, reply) = norm.on_request(&json!(7), "item/commandExecution/requestApproval", &ask);
        assert!(reply.is_none());
        let a = approval_of(&ev);
        assert_eq!(a.reason.as_deref(), Some("Subagent Ada (explorer): needs the network"));
        assert!(matches!(&a.subject, ApprovalSubject::Command { command, .. } if command == "cargo build"), "{:?}", a.subject);

        // its words and its plan are not the agent's
        let said = json!({ "type": "agentMessage", "id": "m1", "text": "It builds.", "phase": "final_answer", "memoryCitation": null, "delivery": null, "questions": null });
        assert!(norm.on_notification("item/agentMessage/delta", &json!({ "threadId": "sub", "turnId": "su1", "itemId": "m1", "delta": "It" })).is_empty());
        assert!(norm.on_notification("item/completed", &json!({ "threadId": "sub", "turnId": "su1", "item": said, "completedAtMs": 3 })).is_empty());
        let plan = json!({ "threadId": "sub", "turnId": "su1", "explanation": null, "plan": [{ "step": "build", "status": "inProgress" }] });
        assert!(norm.on_notification("turn/plan/updated", &plan).is_empty());

        // The agent's own turn ends first. Its result is not the subagent's last words and the
        // subagent's card stays open. Codly rests an agent when its turn ends, so the agent
        // says again that it still waits on that card.
        let ev = norm.on_notification("turn/completed", &json!({ "threadId": "main", "turn": { "id": "u1", "status": "completed", "items": [] } }));
        assert!(
            matches!(&ev[..], [EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, .. }, EventKind::StatusChanged { status: AgentStatus::WaitingApproval }]),
            "{ev:?}"
        );

        // answered under the request's id; the subagent's command still runs
        let (resp, ev) = norm.answer(&a.id, &ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None }).unwrap();
        assert_eq!(resp, json!({ "id": 7, "result": { "decision": "accept" } }));
        assert!(matches!(&ev[..], [EventKind::StatusChanged { status: AgentStatus::Working }]), "{ev:?}");
        let ev = norm.on_notification("item/completed", &json!({ "threadId": "sub", "turnId": "su1", "item": cmd("completed", json!(0)), "completedAtMs": 4 }));
        assert!(
            matches!(&ev[..], [EventKind::ToolCompleted { call_id, ok: true, .. }, EventKind::StatusChanged { status: AgentStatus::Idle }] if call_id == "c1"),
            "{ev:?}"
        );

        // its tokens count for the run; the agent's context stays its own
        let tu = |total: i64, last: i64| {
            let counts = |n: i64| json!({ "totalTokens": n, "inputTokens": n, "cachedInputTokens": 0, "cacheWriteInputTokens": 0, "outputTokens": 0, "reasoningOutputTokens": 0 });
            json!({ "total": counts(total), "last": counts(last), "modelContextWindow": 1000 })
        };
        let input = |ev: Vec<EventKind>| match &ev[..] {
            [EventKind::UsageUpdated { usage }] => (usage.input_tokens, usage.context_used),
            other => panic!("{other:?}"),
        };
        assert_eq!(input(norm.on_notification("thread/tokenUsage/updated", &json!({ "threadId": "main", "turnId": "u1", "tokenUsage": tu(100, 100) }))), (100, Some(100)));
        assert_eq!(input(norm.on_notification("thread/tokenUsage/updated", &json!({ "threadId": "sub", "turnId": "su1", "tokenUsage": tu(40, 40) }))), (140, Some(100)));
        assert_eq!(input(norm.on_notification("thread/tokenUsage/updated", &json!({ "threadId": "sub", "turnId": "su1", "tokenUsage": tu(70, 30) }))), (170, Some(100)));

        let ev = norm.on_notification("turn/completed", &json!({ "threadId": "sub", "turn": { "id": "su1", "status": "completed", "items": [] } }));
        assert!(matches!(&ev[..], [EventKind::SubAgent { call_id, status, .. }] if call_id == "sub" && status == "completed"), "{ev:?}");
        assert!(norm.subagent_turns().is_empty());
    }

    /// A request ends with the turn of the thread that asked, not with another thread's.
    /// (Lines written from the 0.166.0 schema types, as above; `TurnError` for the failure.)
    #[test]
    fn a_request_ends_with_the_turn_of_its_own_thread() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        norm.thread = Some("main".into());
        norm.turn_started("go");
        let ask = |thread: &str, item: &str| json!({ "kind": "command", "threadId": thread, "turnId": "x", "itemId": item, "startedAtMs": 1, "environmentId": null, "command": "ls", "cwd": "/w" });
        let closed = |ev: &[EventKind]| -> Vec<Id> {
            ev.iter().filter_map(|e| if let EventKind::ApprovalResolved { approval_id, status: ApprovalStatus::Cancelled, .. } = e { Some(approval_id.clone()) } else { None }).collect()
        };
        let own = approval_of(&norm.on_request(&json!(1), "item/commandExecution/requestApproval", &ask("main", "i1")).0);
        assert_eq!(own.reason, None);
        let subs = approval_of(&norm.on_request(&json!(2), "item/commandExecution/requestApproval", &ask("sub", "i2")).0);
        assert_eq!(subs.reason.as_deref(), Some("A subagent asks for this"), "Codex gave it no name");

        let failed = json!({ "id": "su1", "status": "failed", "items": [], "error": { "message": "the model is overloaded", "codexErrorInfo": null, "additionalDetails": null } });
        let ev = norm.on_notification("turn/completed", &json!({ "threadId": "sub", "turn": failed }));
        assert_eq!(closed(&ev), [subs.id.clone()], "{ev:?}");
        assert!(ev.iter().any(|e| matches!(e, EventKind::Error { message, fatal: false } if message == "A subagent: the model is overloaded")), "{ev:?}");
        assert!(ev.iter().any(|e| matches!(e, EventKind::SubAgent { status, .. } if status == "failed")), "{ev:?}");
        assert!(!ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { .. })), "{ev:?}");

        // A cancelled card stops the turn that asked. The subagent's is not the agent's: when
        // the agent's own turn fails later, it is told as failed, with its error.
        let again = approval_of(&norm.on_request(&json!(3), "item/commandExecution/requestApproval", &ask("sub", "i3")).0);
        // the request names the subagent's turn: a stop reaches it without its turn/started
        assert_eq!(norm.subagent_turns(), [("sub".to_string(), "x".to_string())]);
        let (resp, _) = norm.answer(&again.id, &ApprovalDecision { choice: ApprovalChoice::Cancel, message: None, answers: None }).unwrap();
        assert_eq!(resp, json!({ "id": 3, "result": { "decision": "cancel" } }));
        assert!(!norm.interrupt_requested);

        let own_failed = json!({ "id": "u1", "status": "failed", "items": [], "error": { "message": "stream disconnected", "codexErrorInfo": null, "additionalDetails": null } });
        let ev = norm.on_notification("turn/completed", &json!({ "threadId": "main", "turn": own_failed }));
        assert_eq!(closed(&ev), [own.id.clone()], "{ev:?}");
        assert!(ev.iter().any(|e| matches!(e, EventKind::Error { message, fatal: false } if message == "stream disconnected")), "{ev:?}");
        assert!(ev.iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Failed, .. })), "{ev:?}");
    }

    /// A subagent has one row in the list of helpers, and the row always says something. A
    /// thread the agent got late can still be stopped. (Lines written from the codex-cli
    /// 0.166.0 schema types: the `collabAgentToolCall` and `commandExecution` items of
    /// ThreadItem, ItemStartedNotification, ItemCompletedNotification, ThreadStartedNotification,
    /// Thread, TurnStartedNotification, TurnCompletedNotification.)
    #[test]
    fn a_subagent_has_one_row_that_says_what_it_does() {
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        norm.thread = Some("main".into());
        norm.turn_started("go");
        let rows = |ev: &[EventKind]| -> Vec<(String, String, String)> {
            ev.iter()
                .filter_map(|e| if let EventKind::SubAgent { call_id, description, status } = e { Some((call_id.clone(), description.clone(), status.clone())) } else { None })
                .collect()
        };
        let row = |key: &str, description: &str, status: &str| (key.to_string(), description.to_string(), status.to_string());
        let spawn = |id: &str, status: &str, threads: Value| {
            json!({ "type": "collabAgentToolCall", "id": id, "tool": "spawnAgent", "status": status, "senderThreadId": "main", "receiverThreadIds": threads,
                "prompt": "Find the flaky test", "model": null, "reasoningEffort": null, "agentsStates": {} })
        };
        let begun = |thread: &str, turn: &str, item: Value| json!({ "threadId": thread, "turnId": turn, "item": item, "startedAtMs": 1 });
        let ended = |thread: &str, turn: &str, item: Value| json!({ "threadId": thread, "turnId": turn, "item": item, "completedAtMs": 2 });
        let turn = |thread: &str, id: &str, status: &str| json!({ "threadId": thread, "turn": { "id": id, "status": status, "items": [] } });

        // The call names its thread when it ends: the subagent takes the call's row.
        let ev = norm.on_notification("item/started", &begun("main", "u1", spawn("k1", "inProgress", json!([]))));
        assert_eq!(rows(&ev), [row("k1", "Find the flaky test", "started")]);
        let ev = norm.on_notification("item/completed", &ended("main", "u1", spawn("k1", "completed", json!(["s1"]))));
        assert_eq!(rows(&ev), [row("k1", "Find the flaky test", "completed")]);
        let thread = json!({ "id": "s1", "parentThreadId": "main", "agentNickname": "Ada", "agentRole": "explorer", "preview": "" });
        norm.on_notification("thread/started", &json!({ "thread": thread }));
        let ev = norm.on_notification("turn/started", &turn("s1", "su1", "inProgress"));
        assert_eq!(rows(&ev), [row("k1", "Ada (explorer): Find the flaky test", "running")]);
        let ev = norm.on_notification("turn/completed", &turn("s1", "su1", "completed"));
        assert_eq!(rows(&ev), [row("k1", "Ada (explorer): Find the flaky test", "completed")]);

        // The call names its thread when it starts and ends while the subagent works: the row
        // stays open until the subagent's turn ends.
        let ev = norm.on_notification("item/started", &begun("main", "u1", spawn("k2", "inProgress", json!(["s2"]))));
        assert_eq!(rows(&ev), [row("k2", "Find the flaky test", "started")]);
        let ev = norm.on_notification("turn/started", &turn("s2", "su2", "inProgress"));
        assert_eq!(rows(&ev), [row("k2", "Find the flaky test", "running")], "Codex gave it no name: the row says what it was asked");
        let ev = norm.on_notification("item/completed", &ended("main", "u1", spawn("k2", "completed", json!(["s2"]))));
        assert!(rows(&ev).is_empty(), "{ev:?}");
        let ev = norm.on_notification("turn/completed", &turn("s2", "su2", "completed"));
        assert_eq!(rows(&ev), [row("k2", "Find the flaky test", "completed")]);

        // A thread nothing named yet has a row of its own, and the row is not blank. The call
        // that names it afterwards keeps its own row.
        let ev = norm.on_notification("turn/started", &turn("s3", "su3", "inProgress"));
        assert_eq!(rows(&ev), [row("s3", "Codex subagent", "running")]);
        let ev = norm.on_notification("item/completed", &ended("main", "u1", spawn("k3", "completed", json!(["s3"]))));
        assert_eq!(rows(&ev), [row("k3", "Find the flaky test", "started"), row("k3", "Find the flaky test", "completed")]);
        let ev = norm.on_notification("turn/completed", &turn("s3", "su3", "completed"));
        assert_eq!(rows(&ev), [row("s3", "Find the flaky test", "completed")]);

        // A thread the agent got after its turn/started: its items name its turn, so a stop
        // reaches it. And the agent is not free while that subagent works, whatever its own turn does.
        assert!(norm.subagent_turns().is_empty());
        let cmd = json!({ "type": "commandExecution", "id": "c4", "pluginId": null, "scriptPath": null, "command": "cargo test", "cwd": "/w", "processId": null,
            "source": "agent", "status": "inProgress", "commandActions": [], "aggregatedOutput": null, "exitCode": null, "durationMs": null });
        norm.on_notification("item/started", &begun("s4", "su4", cmd));
        assert_eq!(norm.subagent_turns(), [("s4".to_string(), "su4".to_string())]);
        let ev = norm.turn_failed();
        assert!(matches!(&ev[..], [EventKind::TurnCompleted { status: TurnStatus::Failed, .. }, EventKind::StatusChanged { status: AgentStatus::Working }]), "{ev:?}");
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
            session_cost_usd: 0.0,
            system_prompt: String::new(),
            read_only: false,
            mcp: None,
            use_user_setup: false,
            sandbox_network: false,
            local_settings: false,
            raw_log: PathBuf::from("/tmp/r/raw.ndjson"),
            byok_command: cmd.map(str::to_owned),
            sandbox_shell: false,
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
            session_cost_usd: 0.0,
            system_prompt: "rules".into(),
            read_only: false,
            mcp: Some(McpLaunch { command: "/x/codly".into(), args: vec!["mcp".into()], env: vec![("CODLY_MCP_TOKEN".into(), "tok".into())] }),
            use_user_setup: false,
            sandbox_network: false,
            local_settings: false,
            raw_log: PathBuf::from("/tmp/r/raw.ndjson"),
            byok_command: None,
            sandbox_shell: false,
        };
        let (model, effort) = classify::split_model_effort(spec.model.as_deref());
        let (m, p) = thread_request(&spec, model.as_deref(), effort.as_deref(), &[]);
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
        let (m, p) = thread_request(&ro, None, None, &[]);
        assert_eq!((m, &p["threadId"], &p["sandbox"]), ("thread/resume", &json!("th-1"), &json!("read-only")));
        assert!(p.get("threadSource").is_none() && p.get("model").is_none());
        assert!(p["config"].get("sandbox_workspace_write").is_none(), "the sandbox keeps the network closed unless you open it");

        // "Use my setup" off: each server of the user's own config is switched off for the
        // thread (`mcp_servers.<name>.enabled`); Codly's own server keeps its definition.
        let own = ["linear".to_string(), "codly".to_string()];
        let (_, p) = thread_request(&spec, None, None, &own);
        assert_eq!(p["config"]["mcp_servers"]["linear"], json!({ "enabled": false }));
        assert_eq!(p["config"]["mcp_servers"]["codly"]["command"], "/x/codly");
        assert!(p["config"]["mcp_servers"]["codly"].get("enabled").is_none());
        // ... also for an agent that has no Codly server
        let bare = SpawnSpec { mcp: None, ..spec.clone() };
        assert_eq!(thread_request(&bare, None, None, &own).1["config"]["mcp_servers"], json!({ "codly": { "enabled": false }, "linear": { "enabled": false } }));
        assert!(thread_request(&bare, None, None, &[]).1["config"].get("mcp_servers").is_none());

        // Settings → "Let Codex agents use the network": the workspace sandbox opens the
        // network (`sandbox_workspace_write.network_access`); a read-only agent has no such sandbox.
        let net = SpawnSpec { sandbox_network: true, ..spec.clone() };
        assert_eq!(thread_request(&net, None, None, &[]).1["config"]["sandbox_workspace_write"], json!({ "network_access": true }));
        let net_ro = SpawnSpec { sandbox_network: true, ..ro.clone() };
        assert!(thread_request(&net_ro, None, None, &[]).1["config"].get("sandbox_workspace_write").is_none());
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
            models: Mutex::new(None),
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
            compaction: Compaction::No,
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

    /// The input of a turn names an attached image as an item of its own beside the text; one
    /// over the size limit stays a path, and the user is told.
    #[tokio::test]
    async fn an_attached_image_is_an_input_item_of_its_own() {
        use crate::adapters::attachments::testing::{attach, message, png_of};
        let dir = tempfile::tempdir().unwrap();
        let (agent, mut events) = fake_agent(&fake_server(dir.path()));
        let shot = attach(dir.path(), "shot.png", &png_of(300));
        let big = attach(dir.path(), "big.png", &png_of(4_000_000));
        let text = message("Make it look like this.", &[&shot, &big]);

        // Written from the schema types of codex-cli 0.166.0 (`app-server generate-ts`):
        // `TurnStartParams.input` and `TurnSteerParams.input` are `Array<UserInput>`, with the
        // items `{ "type": "text", text, text_elements }` and `{ "type": "localImage", path }`.
        let expected = json!([{ "type": "text", "text": text, "text_elements": [] }, { "type": "localImage", "path": shot.to_str().unwrap() }]);
        assert_eq!(agent.input(&text).await, expected);
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::Error { message, fatal: false }] if message.starts_with("big.png is over 3.5 MB")), "{kinds:?}");

        assert_eq!(agent.input("no files").await, json!([{ "type": "text", "text": "no files", "text_elements": [] }]));
        assert!(emitted(&mut events).is_empty());
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

    /// Codex runs a subagent on a thread of its own. What that thread sends belongs to the
    /// agent that owns the thread it came from: a request nobody takes is refused, and the
    /// subagent's command fails without a word. (Lines written from the codex-cli 0.166.0
    /// schema types: ThreadStartedNotification, Thread, ItemCompletedNotification, the
    /// `collabAgentToolCall` and `subAgentActivity` items, CommandExecutionRequestApprovalParams.)
    #[tokio::test]
    async fn a_subagents_thread_reaches_the_agent_that_owns_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let mut agents = Vec::new();
        for tid in ["t1", "t2"] {
            let (tx, rx) = mpsc::unbounded_channel();
            server.state.lock().unwrap().routes.insert(tid.into(), Route { tx, raw: server.bootstrap_raw.clone() });
            agents.push(rx);
        }
        // what each agent got since the last look
        let got = |rx: &mut mpsc::UnboundedReceiver<ServerMsg>| -> Vec<String> {
            std::iter::from_fn(|| rx.try_recv().ok())
                .map(|m| match m {
                    ServerMsg::Notification { method, .. } | ServerMsg::Request { method, .. } => method,
                    _ => "not a server message".into(),
                })
                .collect()
        };
        let none: [&str; 0] = [];

        // the thread says which thread spawned it
        server.route_line(br#"{"method":"thread/started","params":{"thread":{"id":"s1","parentThreadId":"t1","agentNickname":"Ada","agentRole":null}}}"#);
        server.route_line(br#"{"method":"item/commandExecution/requestApproval","id":4,"params":{"kind":"command","threadId":"s1","turnId":"u","itemId":"c1","startedAtMs":1}}"#);
        assert_eq!(got(&mut agents[0]), ["thread/started", "item/commandExecution/requestApproval"]);
        assert_eq!(got(&mut agents[1]), none);
        // a subagent of the subagent
        server.route_line(br#"{"method":"thread/started","params":{"thread":{"id":"s3","parentThreadId":"s1"}}}"#);
        assert_eq!(got(&mut agents[0]), ["thread/started"]);

        // the parent's items name the threads of the agents it started
        server.route_line(
            br#"{"method":"item/completed","params":{"threadId":"t2","turnId":"u","completedAtMs":1,"item":{"type":"collabAgentToolCall","id":"k1","tool":"spawnAgent","status":"completed","senderThreadId":"t2","receiverThreadIds":["s2"],"prompt":"look around","agentsStates":{}}}}"#,
        );
        server.route_line(br#"{"method":"turn/started","params":{"threadId":"s2","turn":{"id":"u2"}}}"#);
        server.route_line(
            br#"{"method":"item/started","params":{"threadId":"t2","turnId":"u","startedAtMs":2,"item":{"type":"subAgentActivity","id":"k2","kind":"started","agentThreadId":"s4","agentPath":"/root/helper"}}}"#,
        );
        server.route_line(br#"{"method":"turn/started","params":{"threadId":"s4","turn":{"id":"u4"}}}"#);
        assert_eq!(got(&mut agents[1]), ["item/completed", "turn/started", "item/started", "turn/started"]);
        assert_eq!(got(&mut agents[0]), none);

        // a thread that has an owner keeps it; a subagent of nobody's thread reaches nobody
        server.route_line(br#"{"method":"thread/started","params":{"thread":{"id":"t2","parentThreadId":"t1"}}}"#);
        server.route_line(br#"{"method":"thread/started","params":{"thread":{"id":"s9","parentThreadId":"gone"}}}"#);
        server.route_line(br#"{"method":"turn/started","params":{"threadId":"s9","turn":{"id":"u9"}}}"#);
        assert_eq!(got(&mut agents[1]), ["thread/started"]);
        assert_eq!(got(&mut agents[0]), none);

        // what concerns the account reaches each agent once, however many threads it has
        server.route_line(br#"{"method":"account/rateLimits/updated","params":{"rateLimits":{}}}"#);
        assert_eq!((got(&mut agents[0]).len(), got(&mut agents[1]).len()), (1, 1));

        // an agent that ends leaves with its subagents' threads
        let first = server.state.lock().unwrap().routes["t1"].clone();
        server.unroute(&first);
        let mut left: Vec<String> = server.state.lock().unwrap().routes.keys().cloned().collect();
        left.sort();
        assert_eq!(left, ["s2", "s4", "t2"]);
    }

    /// A subagent's turn ends on its own thread: the agent's turn goes on. Its end is neither
    /// kept back behind a steer nor taken for the end of the agent's turn.
    #[tokio::test]
    async fn a_subagents_turn_end_is_not_the_agents() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let (mut agent, mut events) = fake_agent(&server);
        (agent.thread_id, agent.active_turn) = (Some("t".into()), Some("u".into()));
        agent.norm.thread = agent.thread_id.clone();
        agent.norm.turn_started("go");
        agent.pending.insert(7, Pending::Steer("also run the tests".into()));
        agent.on_notification("turn/started", &json!({ "threadId": "s", "turn": { "id": "su" } })).await;
        agent.on_notification("turn/completed", &json!({ "threadId": "s", "turn": { "id": "su", "status": "completed" } })).await;
        assert!(agent.held.is_empty() && agent.active_turn.is_some());
        let kinds = emitted(&mut events);
        assert!(
            matches!(&kinds[..], [EventKind::SubAgent { status: began, .. }, EventKind::SubAgent { status: ended, .. }] if began == "running" && ended == "completed"),
            "{kinds:?}"
        );
        // the agent's own turn end still waits for the steer's answer
        agent.on_notification("turn/completed", &json!({ "threadId": "t", "turn": { "id": "u", "status": "completed" } })).await;
        assert!(agent.active_turn.is_some() && emitted(&mut events).is_empty());
    }

    /// Stop means stop. A subagent's turn runs on its own thread, so it gets an interrupt of
    /// its own: with the agent's turn, and when the agent has no turn left to stop.
    #[tokio::test]
    async fn an_interrupt_stops_the_subagents_too() {
        let dir = tempfile::tempdir().unwrap();
        let env = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        // an app-server that only keeps what it is sent
        let mut sink = proc::command(Path::new("/bin/sh"), &["-c".into(), "cat > sent".into()], dir.path(), &env).spawn().unwrap();
        let server = fake_server(dir.path());
        *server.stdin.lock().await = sink.stdin.take();
        let (mut agent, _events) = fake_agent(&server);
        (agent.started, agent.thread_id, agent.active_turn) = (true, Some("t".into()), Some("u".into()));
        agent.norm.thread = agent.thread_id.clone();
        agent.on_notification("turn/started", &json!({ "threadId": "s", "turn": { "id": "su" } })).await;

        agent.on_cmd(AgentCmd::Interrupt).await;
        // the agent's turn ends, the subagent's does not: the next stop is for the subagent alone
        agent.on_notification("turn/completed", &json!({ "threadId": "t", "turn": { "id": "u", "status": "interrupted" } })).await;
        agent.on_cmd(AgentCmd::Interrupt).await;
        // and so is a shutdown, before the agent lets its thread go
        agent.on_cmd(AgentCmd::Shutdown).await;

        server.stdin.lock().await.take(); // end of input: `cat` exits
        sink.wait().await.unwrap();
        let sent: Vec<Value> = std::fs::read_to_string(dir.path().join("sent")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let asked: Vec<(&str, &Value)> = sent.iter().map(|m| (m["method"].as_str().unwrap(), &m["params"])).collect();
        let (own, subs) = (json!({ "threadId": "t", "turnId": "u" }), json!({ "threadId": "s", "turnId": "su" }));
        assert_eq!(
            asked,
            [("turn/interrupt", &own), ("turn/interrupt", &subs), ("turn/interrupt", &subs), ("turn/interrupt", &subs), ("thread/unsubscribe", &json!({ "threadId": "t" }))]
        );
    }

    /// `TurnStartedNotification { threadId, turn: Turn }` and the `turn/completed` of the same
    /// turn, written from the codex-cli 0.166.0 protocol types.
    fn turn_started_on_t(id: &str) -> Value {
        json!({ "threadId": "t", "turn": { "id": id, "rootTurnId": null, "items": [], "itemsView": "full", "status": "inProgress", "error": null, "startedAt": 1, "completedAt": null, "durationMs": null } })
    }

    fn turn_completed_on_t(id: &str) -> Value {
        json!({ "threadId": "t", "turn": { "id": id, "status": "completed" } })
    }

    /// The turn of a compaction that was asked for (`thread/compact/start`) is the thread's
    /// running turn: an interrupt or a shutdown stops it. It is no turn of the agent: Codly
    /// hears its status, not a turn's start and end (a worker's turn end runs the gate, a
    /// Lead's forwards its reply).
    #[tokio::test]
    async fn a_compaction_turn_runs_on_the_thread_but_is_no_turn_of_the_agent() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let (mut agent, mut events) = fake_agent(&server);
        agent.pending.clear();
        agent.thread_id = Some("t".into());
        (agent.compaction, agent.deadline) = (Compaction::Asked, Some(tokio::time::Instant::now() + COMPACT_START_TIMEOUT));
        agent.on_notification("turn/started", &turn_started_on_t("u-compact")).await;
        assert_eq!((agent.active_turn.as_deref(), agent.compaction), (Some("u-compact"), Compaction::Running));
        assert!(agent.deadline.is_none(), "it started in time");
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::StatusChanged { status: AgentStatus::Thinking }]), "{kinds:?}");
        agent.on_notification("item/started", &json!({ "threadId": "t", "turnId": "u-compact", "item": { "type": "contextCompaction", "id": "i1" } })).await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::Compacting { .. }]), "{kinds:?}");
        // its end frees the thread for the next turn
        agent.on_notification("turn/completed", &turn_completed_on_t("u-compact")).await;
        assert_eq!((agent.active_turn.as_deref(), agent.compaction), (None, Compaction::No));
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::StatusChanged { status: AgentStatus::Idle }]), "{kinds:?}");

        // a turn Codly started keeps the id the answer to its `turn/start` gives, and is reported
        agent.turn_pending = true;
        agent.on_notification("turn/started", &turn_started_on_t("u-early")).await;
        assert_eq!((agent.active_turn.as_deref(), agent.compaction), (None, Compaction::No));
        assert!(matches!(&emitted(&mut events)[..], [EventKind::TurnStarted { input: None }, EventKind::StatusChanged { status: AgentStatus::Thinking }]));
        (agent.turn_pending, agent.active_turn) = (false, Some("u-mine".into()));
        agent.on_notification("turn/completed", &turn_completed_on_t("u-mine")).await;
        assert!(emitted(&mut events).iter().any(|e| matches!(e, EventKind::TurnCompleted { status: TurnStatus::Completed, .. })));

        // a turn nobody asked for is reported as before
        agent.on_notification("turn/started", &turn_started_on_t("u-unknown")).await;
        assert_eq!((agent.active_turn.as_deref(), agent.compaction), (None, Compaction::No));
        assert!(matches!(&emitted(&mut events)[..], [EventKind::TurnStarted { input: None }, ..]));
        agent.on_notification("turn/completed", &turn_completed_on_t("u-unknown")).await;
        emitted(&mut events);

        // a server that exits during a compaction ends no turn of the agent
        agent.compaction = Compaction::Asked;
        agent.on_notification("turn/started", &turn_started_on_t("u-compact-2")).await;
        assert_eq!(agent.compaction, Compaction::Running);
        emitted(&mut events);
        agent.server_exited(Some(1), "", &mut None);
        let kinds = emitted(&mut events);
        assert!(!kinds.iter().any(|e| matches!(e, EventKind::TurnCompleted { .. })), "{kinds:?}");
        assert!(matches!(kinds.last(), Some(EventKind::Exited { code: Some(1) })), "{kinds:?}");
    }

    /// The fake server takes no write, so a `turn/start` that the agent sends shows as this error.
    fn tried_to_start_a_turn(kind: &EventKind) -> bool {
        matches!(kind, EventKind::Error { message, fatal: false } if message.starts_with("turn/start:"))
    }

    /// A prompt that comes after a compaction was asked for must not start a turn before the
    /// compaction's turn has ended: Codex would refuse it or run it after the compaction, and
    /// either way the compaction's turn would be taken for the prompt's.
    #[tokio::test]
    async fn a_prompt_waits_for_the_end_of_a_compaction_that_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        let (mut agent, mut events) = fake_agent(&server);
        agent.pending.clear();
        (agent.started, agent.thread_id) = (true, Some("t".into()));

        // asked, its turn not started yet
        agent.compaction = Compaction::Asked;
        agent.on_cmd(AgentCmd::Prompt("go on".into())).await;
        assert_eq!(agent.queue, ["go on"]);
        assert!(!agent.turn_pending && emitted(&mut events).is_empty(), "nothing is sent to Codex yet");
        // its turn runs: a prompt is not steered at it either
        agent.on_notification("turn/started", &turn_started_on_t("u-compact")).await;
        agent.on_cmd(AgentCmd::Prompt("and the docs".into())).await;
        assert_eq!(agent.queue, ["go on", "and the docs"]);
        assert!(agent.pending.is_empty(), "no turn/steer");
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::StatusChanged { status: AgentStatus::Thinking }]), "{kinds:?}");
        // its end: the first prompt that waited gets its turn
        agent.on_notification("turn/completed", &turn_completed_on_t("u-compact")).await;
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::StatusChanged { status: AgentStatus::Idle }, start] if tried_to_start_a_turn(start)), "{kinds:?}");
        assert_eq!(agent.queue, ["and the docs"]);
    }

    /// A compaction that starts no turn must not hold the prompts for ever: Codex refuses the
    /// request, or takes it and starts nothing in time (the deadline is fired by hand).
    #[tokio::test]
    async fn a_compaction_that_never_starts_frees_the_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        for refused in [true, false] {
            let (mut agent, mut events) = fake_agent(&server);
            agent.pending.clear();
            (agent.started, agent.thread_id) = (true, Some("t".into()));
            let (reply, mut answer) = oneshot::channel();
            agent.pending.insert(9, Pending::Compact(reply));
            (agent.compaction, agent.deadline) = (Compaction::Asked, Some(tokio::time::Instant::now() + COMPACT_START_TIMEOUT));
            agent.on_cmd(AgentCmd::Prompt("go on".into())).await;
            assert!(emitted(&mut events).is_empty());

            if refused {
                agent.on_response(9, Err(json!({ "code": -32600, "message": "thread not found: t" })), &mut None).await;
                assert_eq!(answer.try_recv().unwrap().unwrap_err().to_string(), "thread not found: t", "Codex's own words reach who asked");
                let kinds = emitted(&mut events);
                assert!(matches!(&kinds[..], [start] if tried_to_start_a_turn(start)), "{kinds:?}");
            } else {
                agent.on_response(9, Ok(json!({})), &mut None).await;
                assert!(answer.try_recv().unwrap().is_ok());
                assert_eq!(agent.compaction, Compaction::Asked, "taken, its turn still to start");
                agent.on_deadline().await;
                let kinds = emitted(&mut events);
                assert!(
                    matches!(&kinds[..], [EventKind::Error { message, fatal: false }, start]
                        if message == "Codex did not start the compaction within 15 s." && tried_to_start_a_turn(start)),
                    "{kinds:?}"
                );
            }
            assert_eq!((agent.compaction, agent.stage, agent.deadline), (Compaction::No, Stage::Running, None), "the run goes on");
            assert!(agent.queue.is_empty());
        }
    }

    /// A compaction is asked for only when nothing runs on the thread and nothing waits to:
    /// the agent decides, at the moment the request reaches it.
    #[tokio::test]
    async fn a_compaction_is_refused_when_a_turn_got_there_first() {
        let dir = tempfile::tempdir().unwrap();
        let server = fake_server(dir.path());
        async fn refusal(agent: &mut CodexAgent) -> String {
            let (reply, mut answer) = oneshot::channel();
            agent.compact(reply).await;
            answer.try_recv().unwrap().unwrap_err().to_string()
        }
        let (mut agent, _events) = fake_agent(&server);
        agent.pending.clear();
        agent.thread_id = Some("t".into());

        agent.turn_pending = true;
        assert_eq!(refusal(&mut agent).await, "a turn started on it first");
        (agent.turn_pending, agent.active_turn) = (false, Some("u".into()));
        assert_eq!(refusal(&mut agent).await, "a turn started on it first");
        agent.active_turn = None;
        agent.queue.push_back("go on".into());
        assert_eq!(refusal(&mut agent).await, "a turn started on it first");
        agent.queue.clear();
        agent.compaction = Compaction::Asked;
        assert_eq!(refusal(&mut agent).await, "a compaction is already under way");
        agent.compaction = Compaction::No;
        agent.stage = Stage::Unsubscribing;
        assert_eq!(refusal(&mut agent).await, "the agent is stopping");
        // free: the request goes to Codex (the fake server takes no write)
        agent.stage = Stage::Running;
        let e = refusal(&mut agent).await;
        assert!(e.starts_with("thread/compact/start:"), "{e}");
        assert_eq!((agent.compaction, agent.deadline), (Compaction::No, None), "a request that was not sent is not waited for");
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

    /// Did the adapter mark the exit as the end of an interrupt that was not answered?
    fn gave_up(kinds: &[EventKind]) -> bool {
        kinds.iter().any(|k| matches!(k, EventKind::Decision { decision_type, .. } if decision_type == crate::adapters::GAVE_UP))
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
            assert!(!gave_up(&kinds), "no interrupt was asked: this exit is handled as a crash: {kinds:?}");
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
            assert!(gave_up(&kinds), "the exit that follows is marked as Codly's doing: {kinds:?}");
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
            session_cost_usd: 0.0,
            system_prompt: String::new(),
            read_only: false,
            mcp: None,
            use_user_setup: false,
            sandbox_network: false,
            local_settings: false,
            raw_log: cwd.join("raw.ndjson"),
            byok_command: None,
            sandbox_shell: false,
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

    /// The start-up of a `/bin/sh` app-server and the opening of thread `th-1`; `more` = the
    /// `case` branches of the test. The thread's `thread/start` line is kept in `thread-start`.
    fn sh_server(more: &str) -> String {
        let script = r#"while read -r line; do
  id=${line#*'"id":'}; id=${id%%,*}
  case "$line" in
    *'"initialize"'*) echo '{"id":0,"result":{"userAgent":"codex/0.166.0 (sh)"}}' ;;
    *'"account/read"'*) echo '{"id":1,"result":{"account":{"type":"chatgpt"}}}' ;;
    *'"thread/start"'*) printf '%s\n' "$line" > thread-start; echo "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"th-1\"}}}" ;;
    *'"thread/unsubscribe"'*) echo "{\"id\":$id,\"result\":{}}" ;;
MORE
  esac
done
"#;
        script.replace("MORE", more)
    }

    /// "Use my own CLI setup" off: the thread starts with each MCP server of the user's Codex
    /// config switched off. The answer to `config/read` is the user's whole config, with the
    /// keys of those servers: none of it reaches the run's raw log. A config that cannot be
    /// read does not stop the agent: the thread starts, and the user is told what may differ.
    #[tokio::test]
    async fn the_users_own_servers_are_switched_off_and_their_keys_stay_out_of_the_log() {
        const ENV_KEY: &str = "sk-docs-secret-0123";
        const HEADER_KEY: &str = "lin-secret-4567";
        let dir = tempfile::tempdir().unwrap();
        let servers = Servers::default();
        let thread_config = |spec: &SpawnSpec| -> Value {
            let line = std::fs::read_to_string(spec.cwd.join("thread-start")).unwrap();
            serde_json::from_str::<Value>(&line).unwrap()["params"]["config"].clone()
        };

        // ConfigReadResponse { config, origins, layers }, written from the codex-cli 0.166.0
        // protocol types; `config` has the config.toml keys.
        let answered = sh_server(
            r#"    *'"config/read"'*) echo "{\"id\":$id,\"result\":{\"config\":{\"model\":\"gpt-6.1-sol\",\"mcp_servers\":{\"docs\":{\"command\":\"docs-mcp\",\"env\":{\"DOCS_TOKEN\":\"sk-docs-secret-0123\"}},\"linear\":{\"url\":\"https://mcp.linear.app/mcp\",\"http_headers\":{\"Authorization\":\"Bearer lin-secret-4567\"}}}},\"origins\":{},\"layers\":null}}" ;;"#,
        );
        let spec = sh_account(dir.path(), "answered", &answered);
        let (tx, mut events) = mpsc::unbounded_channel();
        let handle = spawn_on(&servers, spec.clone(), tx).await.unwrap();
        assert_eq!(thread_config(&spec)["mcp_servers"], json!({ "docs": { "enabled": false }, "linear": { "enabled": false } }));
        let kinds = emitted(&mut events);
        assert!(matches!(&kinds[..], [EventKind::SessionStarted { .. }]), "{kinds:?}");
        let log = std::fs::read_to_string(&spec.raw_log).unwrap();
        assert!(log.contains("config/read"), "the question is logged: {log}");
        assert!(!log.contains(ENV_KEY) && !log.contains(HEADER_KEY) && !log.contains("docs-mcp"), "{log}");
        assert!(log.contains("is not logged: it holds your own Codex config"), "{log}");
        assert!(log.contains(r#"your own Codex config starts the MCP servers [\"docs\", \"linear\"]"#), "the names are: {log}");
        handle.send(AgentCmd::Shutdown);

        let refused = sh_server(r#"    *'"config/read"'*) echo "{\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"config.toml is not valid\"}}" ;;"#);
        let spec = sh_account(dir.path(), "refused", &refused);
        let (tx, mut events) = mpsc::unbounded_channel();
        let handle = spawn_on(&servers, spec.clone(), tx).await.unwrap();
        assert!(thread_config(&spec).get("mcp_servers").is_none(), "nothing to switch off is known");
        let kinds = emitted(&mut events);
        assert!(
            matches!(&kinds[..], [EventKind::Error { message, fatal: false }, EventKind::SessionStarted { .. }]
                if message == "Codly could not read your Codex config (config/read failed: config.toml is not valid), so your own MCP servers may start with this agent."),
            "{kinds:?}"
        );
        handle.send(AgentCmd::Shutdown);

        // "Use my own CLI setup" on: the config is not asked for
        let spec = SpawnSpec { use_user_setup: true, ..sh_account(dir.path(), "own-setup", &sh_server("")) };
        let (tx, _events) = mpsc::unbounded_channel();
        let handle = spawn_on(&servers, spec.clone(), tx).await.unwrap();
        assert!(thread_config(&spec).get("mcp_servers").is_none());
        assert!(!std::fs::read_to_string(&spec.raw_log).unwrap().contains("config/read"));
        handle.send(AgentCmd::Shutdown);
        for (_, slot) in servers.lock().await.iter() {
            if let Some(server) = slot.lock().await.as_ref() {
                server.close().await;
            }
        }
    }

    /// "Compact now" and a prompt at the same moment, on a `/bin/sh` app-server that takes the
    /// compaction and starts its turn only when told to (`codly/go`). The prompt's `turn/start`
    /// must reach Codex after that turn's end, and the agent has one turn: the prompt's.
    #[tokio::test]
    async fn a_prompt_sent_while_a_compaction_starts_gets_its_turn_after_it() {
        // The notifications are TurnStartedNotification / TurnCompletedNotification, written
        // from the codex-cli 0.166.0 protocol types.
        let script = sh_server(
            r#"    *'"thread/compact/start"'*) echo compact >> asked; echo "{\"id\":$id,\"result\":{}}" ;;
    *'"codly/go"'*) echo go >> asked; echo "{\"id\":$id,\"result\":{}}"
      echo '{"method":"turn/started","params":{"threadId":"th-1","turn":{"id":"u-compact","items":[],"status":"inProgress","error":null}}}'
      echo '{"method":"turn/completed","params":{"threadId":"th-1","turn":{"id":"u-compact","items":[],"status":"completed","error":null}}}' ;;
    *'"turn/start"'*) echo turn >> asked; echo "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"u-1\"}}}"
      echo '{"method":"turn/started","params":{"threadId":"th-1","turn":{"id":"u-1","items":[],"status":"inProgress","error":null}}}'
      echo '{"method":"turn/completed","params":{"threadId":"th-1","turn":{"id":"u-1","items":[],"status":"completed","error":null}}}' ;;"#,
        );
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec { use_user_setup: true, ..sh_account(dir.path(), "work", &script) };
        let servers = Servers::default();
        let (tx, mut events) = mpsc::unbounded_channel();
        let handle = spawn_on(&servers, spec.clone(), tx).await.unwrap();
        let raw = Arc::new(RawLog::open(&spec.raw_log).unwrap());
        let server = server_for(&servers, &spec, &raw).await.unwrap();

        account::compact_on(&server, "th-1").await.unwrap();
        let e = account::compact_on(&server, "th-1").await.unwrap_err();
        assert_eq!(e.to_string(), "a compaction is already under way");
        let e = account::compact_on(&server, "th-9").await.unwrap_err();
        assert_eq!(e.to_string(), "its conversation is not open on the Codex server");
        assert!(handle.send(AgentCmd::Prompt("go on".into())));
        server.call("codly/go", json!({}), &raw, Duration::from_secs(5)).await.unwrap();

        let mut kinds: Vec<EventKind> = Vec::new();
        while turn_ends(&kinds).is_empty() {
            let event = tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("the prompt's turn did not end");
            kinds.push(event.expect("the adapter stopped").kind);
        }
        let asked = std::fs::read_to_string(spec.cwd.join("asked")).unwrap();
        assert_eq!(asked.lines().collect::<Vec<_>>(), ["compact", "go", "turn"], "the prompt waited for the compaction's end");
        let starts: Vec<&Option<String>> = kinds.iter().filter_map(|k| if let EventKind::TurnStarted { input } = k { Some(input) } else { None }).collect();
        assert_eq!(starts, [&Some("go on".to_string())], "{kinds:?}");
        assert_eq!(turn_ends(&kinds), [TurnStatus::Completed], "{kinds:?}");
        handle.send(AgentCmd::Shutdown);
        server.close().await;
    }
}
