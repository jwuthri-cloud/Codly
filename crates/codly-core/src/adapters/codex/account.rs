//! Questions to a Codex sign-in's app-server that are no part of a run: its models
//! (`model/list`), the account's usage windows (`account/rateLimits/read`), the MCP servers the
//! user's own Codex config names (`config/read`), and a compaction of one thread
//! (`thread/compact/start`).
//!
//! Every request and response shape here is from the protocol types codex-cli 0.166.0
//! generates (`codex app-server generate-ts`): `ModelListParams`, `ModelListResponse`, `Model`,
//! `GetAccountRateLimitsParams`, `GetAccountRateLimitsResponse`, `RateLimitSnapshot`,
//! `RateLimitWindow`, `ConfigReadParams`, `ConfigReadResponse`, `ThreadCompactStartParams`.
//! None of them was recorded from a live server. `ConfigReadResponse.config` carries the keys
//! of `config.toml` (it allows keys the type does not name); `mcp_servers.<name>` with its
//! `enabled` switch is the key docs/research/cli-claude-codex.md §3.4 lists.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{AppServer, SERVERS, ServerKey, ServerMsg, Servers, auth_tag, server_for};
use crate::adapters::{SessionMode, SpawnSpec, startup};
use crate::model::{ModelOption, Provider, Role};
use crate::proc::RawLog;
use crate::quota::Reading;

/// How long the app-server has to answer one of these questions.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// `model/list` is read page by page; a catalog never has this many pages.
const MAX_PAGES: usize = 20;

/// What a question to a sign-in's app-server needs: the binary, env and BYOK command the
/// agents of that sign-in start with, so the question goes to the server they share.
#[derive(Debug, Clone)]
pub struct AccountQuery {
    pub bin: PathBuf,
    /// The child env of an agent on this sign-in (its `CODEX_HOME` names the sign-in).
    pub env: HashMap<String, String>,
    pub byok_command: Option<String>,
    /// Where the questions and their answers are logged (NDJSON, append-only).
    pub raw_log: PathBuf,
}

impl AccountQuery {
    /// The spawn spec of an agent on this sign-in, as far as starting its server reads it.
    fn spec(&self) -> SpawnSpec {
        SpawnSpec {
            agent_id: String::new(),
            run_id: String::new(),
            provider: Provider::Codex,
            role: Role::Solo,
            model: None,
            cwd: PathBuf::from("/"),
            bin: self.bin.clone(),
            env: self.env.clone(),
            session: SessionMode::New { session_id: None },
            session_cost_usd: 0.0,
            system_prompt: String::new(),
            read_only: true,
            mcp: None,
            use_user_setup: true,
            sandbox_network: false,
            sandbox_shell: false,
            local_settings: false,
            raw_log: self.raw_log.clone(),
            byok_command: self.byok_command.clone(),
        }
    }

    fn raw(&self) -> anyhow::Result<Arc<RawLog>> {
        Ok(Arc::new(RawLog::open(&self.raw_log).with_context(|| format!("cannot open the log {}", self.raw_log.display()))?))
    }
}

/// The sign-in's app-server, started when none runs. A server started here stays for the
/// sign-in's next agent.
async fn server(servers: &Servers, q: &AccountQuery, raw: &Arc<RawLog>) -> anyhow::Result<Arc<AppServer>> {
    let spec = q.spec();
    server_for(servers, &spec, raw).await.map_err(|e| startup::explain(&spec, None, e))
}

/// The sign-in's app-server if one runs. A thread lives on the server that opened it, so a
/// question about a thread never starts one.
async fn running_server(servers: &Servers, q: &AccountQuery) -> Option<Arc<AppServer>> {
    let spec = q.spec();
    let key: ServerKey = (spec.env.get("CODEX_HOME").cloned(), auth_tag(&spec));
    let slot = servers.lock().await.iter().find(|(k, _)| *k == key).map(|(_, slot)| slot.clone())?;
    let cur = slot.lock().await;
    cur.as_ref().filter(|s| s.usable()).cloned()
}

// ───────────────────────────── models ─────────────────────────────

/// `model/list` for the page after `cursor` (None = the first). Hidden models are not asked
/// for: the list is the one Codex's own picker shows.
pub fn model_list_request(cursor: Option<&str>) -> (&'static str, Value) {
    let mut params = json!({});
    if let Some(c) = cursor {
        params["cursor"] = json!(c);
    }
    ("model/list", params)
}

/// The models of one `ModelListResponse` page, in the server's order, and the cursor of the
/// next page. A hidden model is left out.
pub fn parse_model_page(page: &Value) -> anyhow::Result<(Vec<ModelOption>, Option<String>)> {
    let data = page["data"].as_array().context("Codex answered model/list without a list")?;
    let models = data
        .iter()
        .filter(|m| m["hidden"] != true)
        .filter_map(|m| {
            let id = m["model"].as_str().filter(|s| !s.is_empty())?;
            Some(ModelOption {
                id: id.to_string(),
                name: m["displayName"].as_str().filter(|s| !s.is_empty()).unwrap_or(id).to_string(),
                is_default: m["isDefault"] == true,
                efforts: m["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e["reasoningEffort"].as_str().map(str::to_owned))
                    .collect(),
                default_effort: m["defaultReasoningEffort"].as_str().map(str::to_owned),
            })
        })
        .collect();
    Ok((models, page["nextCursor"].as_str().map(str::to_owned)))
}

/// Every page of the server's model list. Asked once per server: the answer is kept on it.
async fn fetch_models(server: &AppServer, raw: &Arc<RawLog>) -> anyhow::Result<Vec<ModelOption>> {
    let mut kept = server.models.lock().await;
    if let Some(models) = kept.as_ref() {
        return Ok(models.clone());
    }
    let mut models = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let (method, params) = model_list_request(cursor.as_deref());
        let (page, next) = parse_model_page(&server.call(method, params, raw, ANSWER_TIMEOUT).await?)?;
        models.extend(page);
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }
    if cursor.is_some() {
        bail!("Codex's model list did not end after {MAX_PAGES} pages");
    }
    *kept = Some(models.clone());
    Ok(models)
}

/// The models this sign-in can run, as Codex's own picker lists them, with the thinking
/// efforts each one takes.
pub async fn models(q: &AccountQuery) -> anyhow::Result<Vec<ModelOption>> {
    let raw = q.raw()?;
    let server = server(&SERVERS, q, &raw).await?;
    fetch_models(&server, &raw).await
}

// ───────────────────────────── usage windows ─────────────────────────────

/// `account/rateLimits/read`, without the reset-credit details (Codly does not show them).
pub fn rate_limits_request() -> (&'static str, Value) {
    ("account/rateLimits/read", json!({ "excludeResetCreditDetails": true }))
}

/// The usage windows of a `GetAccountRateLimitsResponse`: its `rateLimits` snapshot, with each
/// window named and judged as `CodexNormalizer::rate_limits` names and judges the snapshot of
/// an `account/rateLimits/updated`, so a read and a notification update the same windows.
pub fn parse_rate_limits(response: &Value) -> anyhow::Result<Vec<Reading>> {
    let rl = &response["rateLimits"];
    if !rl.is_object() {
        bail!("Codex answered account/rateLimits/read without the limits");
    }
    let reached = !rl["rateLimitReachedType"].is_null();
    let full = |key: &str| rl[key]["usedPercent"].as_f64().is_some_and(|p| p >= 100.0);
    let any_full = full("primary") || full("secondary");
    let mut out = Vec::new();
    for key in ["primary", "secondary"] {
        let w = &rl[key];
        let Some(used) = w["usedPercent"].as_f64() else { continue };
        let window = match w["windowDurationMins"].as_i64() {
            Some(300) => "five_hour".to_string(),
            Some(10080) => "seven_day".to_string(),
            Some(m) => format!("{m}m"),
            None => key.to_string(),
        };
        out.push(Reading {
            window,
            used_fraction: used / 100.0,
            resets_at: w["resetsAt"].as_i64().map(|s| s * 1000),
            status: Some(if reached && (full(key) || !any_full) { "rejected" } else { "allowed" }.into()),
        });
    }
    Ok(out)
}

/// The usage windows of this sign-in, read now (what Codex's `/status` shows). Codex reads
/// them with its own sign-in; Codly sees no credential.
pub async fn rate_limits(q: &AccountQuery) -> anyhow::Result<Vec<Reading>> {
    let raw = q.raw()?;
    let server = server(&SERVERS, q, &raw).await?;
    let (method, params) = rate_limits_request();
    parse_rate_limits(&server.call(method, params, &raw, ANSWER_TIMEOUT).await?)
}

// ───────────────────────────── the user's own MCP servers ─────────────────────────────

/// `config/read` for the config a thread in `cwd` runs with (the user's and the project's).
pub fn config_read_request(cwd: &Path) -> (&'static str, Value) {
    ("config/read", json!({ "includeLayers": false, "cwd": cwd }))
}

/// The MCP servers the user's own Codex config starts in a thread: the `mcp_servers` tables
/// of a `ConfigReadResponse` that are not switched off, by name.
pub fn parse_own_mcp_servers(response: &Value) -> Vec<String> {
    let Some(servers) = response["config"]["mcp_servers"].as_object() else { return Vec::new() };
    let mut names: Vec<String> = servers.iter().filter(|(_, s)| s.is_object() && s["enabled"] != false).map(|(name, _)| name.clone()).collect();
    names.sort();
    names
}

/// The MCP servers a thread in `cwd` would start from the user's own Codex config. The answer
/// is that whole config, with the keys of its MCP servers and model providers: the raw log
/// gets the server names only, as it gets no key of what Codly sends (`proc::write_json`).
pub(super) async fn own_mcp_servers(server: &AppServer, cwd: &Path, raw: &Arc<RawLog>) -> anyhow::Result<Vec<String>> {
    let (method, params) = config_read_request(cwd);
    let names = parse_own_mcp_servers(&server.call_unlogged(method, params, raw, ANSWER_TIMEOUT).await?);
    raw.note(&format!("config/read: your own Codex config starts the MCP servers {names:?}"));
    Ok(names)
}

// ───────────────────────────── compaction ─────────────────────────────

/// `thread/compact/start`: Codex answers `{}` at once and compacts in a turn of its own,
/// reported on the thread like any other turn.
pub fn compact_request(thread_id: &str) -> (&'static str, Value) {
    ("thread/compact/start", json!({ "threadId": thread_id }))
}

/// Ask the agent that owns a thread on `server` to have it compacted. The agent sends the
/// request itself (`CodexAgent::compact`): only it knows whether a turn of its own runs or is
/// about to, and it holds the prompts that come until the compaction has ended.
pub(super) async fn compact_on(server: &AppServer, thread_id: &str) -> anyhow::Result<()> {
    let route = server.state.lock().unwrap().routes.get(thread_id).cloned();
    let route = route.ok_or_else(|| anyhow!("its conversation is not open on the Codex server"))?;
    let (reply, answer) = oneshot::channel();
    route.tx.send(ServerMsg::Compact(reply)).map_err(|_| anyhow!("the agent stopped"))?;
    match tokio::time::timeout(ANSWER_TIMEOUT, answer).await {
        Ok(Ok(asked)) => asked,
        Ok(Err(_)) => Err(anyhow!("the agent stopped")),
        Err(_) => Err(anyhow!("Codex did not answer within {} s", ANSWER_TIMEOUT.as_secs())),
    }
}

/// Ask Codex to compact a thread's history now. Ok = Codex took the request; the compaction
/// itself is reported by the thread's own events.
pub async fn compact_thread(q: &AccountQuery, thread_id: &str) -> anyhow::Result<()> {
    let server = running_server(&SERVERS, q).await.ok_or_else(|| anyhow!("its Codex server is not running"))?;
    compact_on(&server, thread_id).await
}

#[cfg(test)]
mod tests {
    use super::super::{CodexNormalizer, RATE_LIMITS};
    use super::*;
    use crate::model::EventKind;

    // The JSON in these tests is written from the codex-cli 0.166.0 protocol types
    // (`codex app-server generate-ts`), not recorded from a server.

    #[test]
    fn model_list_asks_for_the_picker_models_page_by_page() {
        // ModelListParams: { cursor?, limit?, includeHidden? }
        assert_eq!(model_list_request(None), ("model/list", json!({})));
        assert_eq!(model_list_request(Some("c2")), ("model/list", json!({ "cursor": "c2" })));
    }

    /// One `Model` as the type lists its fields.
    fn model(slug: &str, name: &str, hidden: bool, default: bool, efforts: &[&str], default_effort: &str) -> Value {
        json!({
            "id": slug, "model": slug, "upgrade": null, "upgradeInfo": null, "availabilityNux": null,
            "displayName": name, "description": "", "modelSpecialty": null, "hidden": hidden,
            "supportedReasoningEfforts": efforts.iter().map(|e| json!({ "reasoningEffort": e, "description": "" })).collect::<Vec<_>>(),
            "defaultReasoningEffort": default_effort, "inputModalities": ["text", "image"], "supportsPersonality": false,
            "multiAgentVersion": null, "additionalSpeedTiers": [], "serviceTiers": [], "defaultServiceTier": null,
            "availableAccessPrograms": null, "isDefault": default,
        })
    }

    #[test]
    fn a_model_page_gives_each_model_its_own_efforts() {
        // ModelListResponse: { data: Model[], nextCursor: string | null }
        let page = json!({
            "data": [
                model("gpt-6.1-sol", "GPT-6.1 Sol", false, true, &["low", "medium", "high", "xhigh"], "medium"),
                model("gpt-6-luna", "", false, false, &["minimal", "low"], "low"),
                model("gpt-reserve", "Reserve", true, false, &["low"], "low"),
            ],
            "nextCursor": "page-2",
        });
        let (models, next) = parse_model_page(&page).unwrap();
        assert_eq!(next.as_deref(), Some("page-2"));
        assert_eq!(
            models,
            [
                ModelOption {
                    id: "gpt-6.1-sol".into(),
                    name: "GPT-6.1 Sol".into(),
                    is_default: true,
                    efforts: ["low", "medium", "high", "xhigh"].map(String::from).to_vec(),
                    default_effort: Some("medium".into()),
                },
                // no display name: its id; and the hidden model is not offered
                ModelOption {
                    id: "gpt-6-luna".into(),
                    name: "gpt-6-luna".into(),
                    is_default: false,
                    efforts: ["minimal", "low"].map(String::from).to_vec(),
                    default_effort: Some("low".into()),
                },
            ]
        );
        let (last, next) = parse_model_page(&json!({ "data": [], "nextCursor": null })).unwrap();
        assert!(last.is_empty() && next.is_none());
        assert!(parse_model_page(&json!({ "models": [] })).is_err(), "an answer without `data` is not an empty catalog");
    }

    #[test]
    fn rate_limit_read_request_skips_the_credit_details() {
        // GetAccountRateLimitsParams: { supportsLunaReserve?, excludeResetCreditDetails? }
        assert_eq!(rate_limits_request(), ("account/rateLimits/read", json!({ "excludeResetCreditDetails": true })));
    }

    /// One `RateLimitSnapshot` as the type lists its fields; windows are `RateLimitWindow`s.
    fn snapshot(primary: Value, secondary: Value, reached: Value) -> Value {
        json!({
            "limitId": "codex", "limitName": null, "normalModelSlug": null, "primary": primary, "secondary": secondary,
            "credits": null, "individualLimit": null, "spendControlReached": null, "planType": "plus", "rateLimitReachedType": reached,
        })
    }

    #[test]
    fn a_rate_limit_read_gives_the_account_windows() {
        // GetAccountRateLimitsResponse: { ordinaryUsageAllowed, rateLimits, rateLimitsByLimitId,
        // rateLimitResetCredits, accountId, rateLimitUpsell }
        let response = |rl: Value| json!({ "ordinaryUsageAllowed": true, "rateLimits": rl, "rateLimitsByLimitId": null, "rateLimitResetCredits": null, "accountId": null, "rateLimitUpsell": null });
        let five_hour = json!({ "usedPercent": 42.0, "windowDurationMins": 300, "resetsAt": 1_800_000_000 });
        let week = json!({ "usedPercent": 100.0, "windowDurationMins": 10080, "resetsAt": null });
        let open = parse_rate_limits(&response(snapshot(five_hour.clone(), Value::Null, Value::Null))).unwrap();
        assert_eq!(
            open,
            [Reading { window: "five_hour".into(), used_fraction: 0.42, resets_at: Some(1_800_000_000_000), status: Some("allowed".into()) }],
            "seconds become ms, a percentage a fraction, and a window that is null is no window"
        );
        // reached: the full window is the one that refuses work
        let reached = parse_rate_limits(&response(snapshot(five_hour, week, json!("rate_limit_reached")))).unwrap();
        let seen: Vec<(&str, Option<&str>)> = reached.iter().map(|r| (r.window.as_str(), r.status.as_deref())).collect();
        assert_eq!(seen, [("five_hour", Some("allowed")), ("seven_day", Some("rejected"))]);
        assert_eq!(reached[1].resets_at, None);
        assert!(parse_rate_limits(&json!({ "rateLimits": null })).is_err(), "no snapshot is an error, not an account without limits");
    }

    /// A read and a notification carry the same snapshot type: both must name and judge its
    /// windows the same way, or one account would get two sets of windows.
    #[test]
    fn a_read_and_a_notification_name_the_same_windows() {
        let rl = snapshot(
            json!({ "usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1_800_000_000 }),
            json!({ "usedPercent": 12.5, "windowDurationMins": 43200, "resetsAt": 1_800_500_000 }),
            json!("rate_limit_reached"),
        );
        let read = parse_rate_limits(&json!({ "rateLimits": rl })).unwrap();
        let mut norm = CodexNormalizer::new("a", "r", PathBuf::from("/w"), false);
        let heard: Vec<Reading> = norm
            .on_notification(RATE_LIMITS, &json!({ "rateLimits": rl }))
            .into_iter()
            .filter_map(|e| match e {
                EventKind::RateLimit { window, used_fraction, resets_at, status } => Some(Reading { window, used_fraction: used_fraction?, resets_at, status }),
                _ => None,
            })
            .collect();
        assert_eq!(read, heard);
        assert_eq!(read.len(), 2);
    }

    #[test]
    fn config_read_names_the_users_own_mcp_servers() {
        // ConfigReadParams: { includeLayers?, cwd? }
        assert_eq!(config_read_request(Path::new("/w/t1")), ("config/read", json!({ "includeLayers": false, "cwd": "/w/t1" })));
        // ConfigReadResponse: { config, origins, layers }; `config` has the config.toml keys
        let response = json!({
            "config": {
                "model": "gpt-6.1-sol",
                "mcp_servers": {
                    "linear": { "url": "https://mcp.linear.app/mcp" },
                    "docs": { "command": "docs-mcp", "args": [] },
                    "old": { "command": "old-mcp", "enabled": false },
                },
            },
            "origins": {},
            "layers": null,
        });
        assert_eq!(parse_own_mcp_servers(&response), ["docs", "linear"], "a server the user switched off needs no override");
        assert!(parse_own_mcp_servers(&json!({ "config": { "model": null }, "origins": {}, "layers": null })).is_empty());
    }

    #[test]
    fn compaction_is_asked_for_one_thread() {
        // ThreadCompactStartParams: { threadId }
        assert_eq!(compact_request("th-1"), ("thread/compact/start", json!({ "threadId": "th-1" })));
    }

    /// A sign-in whose app-server is a `/bin/sh` that runs `script` (the `app-server` file of
    /// the env's HOME).
    fn sh_query(dir: &Path, script: &str) -> AccountQuery {
        std::fs::write(dir.join("app-server"), script).unwrap();
        AccountQuery {
            bin: PathBuf::from("/bin/sh"),
            env: HashMap::from([("HOME".to_string(), dir.to_string_lossy().into_owned()), ("CODEX_HOME".to_string(), "work".to_string())]),
            byok_command: None,
            raw_log: dir.join("raw.ndjson"),
        }
    }

    /// The questions go to the sign-in's own server, which starts for the first one and stays;
    /// the model list is asked once per server.
    #[tokio::test]
    async fn questions_reach_the_sign_ins_server_and_models_are_asked_once() {
        let dir = tempfile::tempdir().unwrap();
        // Requests 0 and 1 are the start-up (`initialize`, `account/read`); each later request
        // is answered by what its method is, and counted in a file named after it.
        let q = sh_query(
            dir.path(),
            r#"while read -r line; do
  id=${line#*'"id":'}; id=${id%%,*}
  case "$line" in
    *'"initialize"'*) echo '{"id":0,"result":{"userAgent":"codex/0.166.0 (sh)"}}' ;;
    *'"account/read"'*) echo '{"id":1,"result":{"account":{"type":"chatgpt"}}}' ;;
    *'"model/list"'*'"cursor":"p2"'*) echo x >> asked-models; echo "{\"id\":$id,\"result\":{\"data\":[{\"model\":\"gpt-6-luna\",\"displayName\":\"Luna\",\"hidden\":false,\"isDefault\":false,\"supportedReasoningEfforts\":[{\"reasoningEffort\":\"low\",\"description\":\"\"}],\"defaultReasoningEffort\":\"low\"}],\"nextCursor\":null}}" ;;
    *'"model/list"'*) echo x >> asked-models; echo "{\"id\":$id,\"result\":{\"data\":[{\"model\":\"gpt-6.1-sol\",\"displayName\":\"Sol\",\"hidden\":false,\"isDefault\":true,\"supportedReasoningEfforts\":[],\"defaultReasoningEffort\":\"medium\"}],\"nextCursor\":\"p2\"}}" ;;
    *'"account/rateLimits/read"'*) echo "{\"id\":$id,\"result\":{\"rateLimits\":{\"primary\":{\"usedPercent\":7,\"windowDurationMins\":300,\"resetsAt\":null},\"secondary\":null,\"rateLimitReachedType\":null}}}" ;;
  esac
done
"#,
        );
        let servers = Servers::default();
        let raw = q.raw().unwrap();
        assert!(running_server(&servers, &q).await.is_none(), "no server before the first question");

        let server = super::server(&servers, &q, &raw).await.unwrap();
        let models = fetch_models(&server, &raw).await.unwrap();
        let ids: Vec<(&str, bool)> = models.iter().map(|m| (m.id.as_str(), m.is_default)).collect();
        assert_eq!(ids, [("gpt-6.1-sol", true), ("gpt-6-luna", false)], "both pages, in the server's order");
        assert_eq!(fetch_models(&server, &raw).await.unwrap(), models);
        let asked = std::fs::read_to_string(dir.path().join("asked-models")).unwrap();
        assert_eq!(asked.lines().count(), 2, "one request per page, and none for the second look");

        // the next question finds the same server
        let again = super::server(&servers, &q, &raw).await.unwrap();
        assert!(Arc::ptr_eq(&server, &again));
        assert!(running_server(&servers, &q).await.is_some_and(|s| Arc::ptr_eq(&s, &server)));
        let (method, params) = rate_limits_request();
        let limits = parse_rate_limits(&server.call(method, params, &raw, ANSWER_TIMEOUT).await.unwrap()).unwrap();
        assert_eq!(limits, [Reading { window: "five_hour".into(), used_fraction: 0.07, resets_at: None, status: Some("allowed".into()) }]);

        // a compaction goes through the agent that owns the thread: none owns one here
        let e = compact_on(&server, "th-1").await.unwrap_err();
        assert_eq!(e.to_string(), "its conversation is not open on the Codex server");

        // a question the server never answers is forgotten when its wait ends, so the server
        // does not look in use for good
        let e = server.call("codly/unanswered", json!({}), &raw, Duration::from_millis(200)).await.unwrap_err();
        assert!(e.to_string().contains("did not answer codly/unanswered"), "{e}");
        assert!(server.state.lock().unwrap().pending.is_empty());

        // another sign-in has another server, and none runs for it
        let mut other = q.clone();
        other.env.insert("CODEX_HOME".into(), "personal".into());
        assert!(running_server(&servers, &other).await.is_none());
        server.close().await;
        assert!(running_server(&servers, &q).await.is_none(), "a server that ended is not one to ask");
    }
}
