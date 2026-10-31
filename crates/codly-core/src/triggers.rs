//! Triggers: everything that can start work without you typing. Pure, unit-tested policy;
//! the loop that acts on it lives in `orchestrator/triggers.rs`.
//!
//! - **Schedules** run a prompt on a repeating clock, aimed at the Lead (`due_schedules`).
//! - **Network mode** says what teammates may do on this machine; `network_gate` is the one
//!   function every future inbound path (webhooks, Slack, peers) asks. No networking here.
//!
//! The whole config is one JSON value in the store's `kv` table (key `triggers`), kept apart
//! from `Settings` so a bad trigger never resets your settings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Id, new_id};

const MIN: i64 = 60_000;

/// Shortest and longest schedule interval, in minutes (5 minutes … 7 days).
pub const MIN_EVERY_MINUTES: i64 = 5;
pub const MAX_EVERY_MINUTES: i64 = 7 * 24 * 60;
/// Most entries kept in the activity log.
const LOG_CAP: usize = 50;

// ───────────── schedules ─────────────

/// A prompt sent to the Lead every `every_minutes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Schedule {
    /// Filled in by Codly when empty.
    pub id: Id,
    pub name: String,
    /// What the Lead is asked, e.g. "Review every agent: who is stuck, who is idle?".
    pub prompt: String,
    pub every_minutes: i64,
    pub enabled: bool,
    pub created_at: i64,
    /// Owned by Codly: a save from the UI never moves it.
    pub last_fired_at: Option<i64>,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule { id: String::new(), name: String::new(), prompt: String::new(), every_minutes: 60, enabled: true, created_at: 0, last_fired_at: None }
    }
}

/// When a schedule next runs (ms): one interval after it last fired, or was created.
pub fn next_run_at(s: &Schedule) -> i64 {
    s.last_fired_at.unwrap_or(s.created_at) + s.every_minutes.max(MIN_EVERY_MINUTES) * MIN
}

/// Ids of the enabled schedules that are due. One that was missed (app closed, no repo open)
/// is due once, not once per missed interval.
pub fn due_schedules(list: &[Schedule], now: i64) -> Vec<Id> {
    list.iter().filter(|s| s.enabled && !s.prompt.trim().is_empty() && next_run_at(s) <= now).map(|s| s.id.clone()).collect()
}

// ───────────── network mode ─────────────

/// What teammates may do on this machine, each mode including the one before.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NetworkMode {
    /// Nobody can reach your agents. The default.
    #[default]
    Off,
    /// Teammates can see what your team is working on. Read-only.
    Listen,
    /// Teammates can also message your Lead and answer its questions. They cannot start work.
    Converse,
    /// Teammates can also hand your agents work.
    Open,
}

/// What a teammate is trying to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NetworkAction {
    /// See status, tasks, activity.
    Read,
    /// Send a message to the Lead.
    Message,
    /// Ask an agent to do work.
    Delegate,
}

impl NetworkAction {
    /// The lowest mode that allows this action.
    pub fn needs(self) -> NetworkMode {
        match self {
            NetworkAction::Read => NetworkMode::Listen,
            NetworkAction::Message => NetworkMode::Converse,
            NetworkAction::Delegate => NetworkMode::Open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct NetworkAdvanced {
    /// Hand-offs wait for your approval even in Open mode. On by default.
    pub confirm_handoffs: bool,
    /// Only these teammates (by name or id) get through. Empty = anyone the mode allows.
    pub peers: Vec<String>,
}

impl Default for NetworkAdvanced {
    fn default() -> Self {
        NetworkAdvanced { confirm_handoffs: true, peers: vec![] }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct NetworkSettings {
    pub mode: NetworkMode,
    pub advanced: NetworkAdvanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Gate {
    Allow,
    /// Allowed once you approve it.
    Ask,
    Deny,
}

/// The one question every inbound path asks before acting on a teammate's request.
pub fn network_gate(net: &NetworkSettings, action: NetworkAction, peer: &str) -> Gate {
    if net.mode < action.needs() {
        return Gate::Deny;
    }
    let peers = &net.advanced.peers;
    if !peers.is_empty() && !peers.iter().any(|p| p.trim().eq_ignore_ascii_case(peer.trim())) {
        return Gate::Deny;
    }
    if action == NetworkAction::Delegate && net.advanced.confirm_handoffs { Gate::Ask } else { Gate::Allow }
}

// ───────────── the stored config ─────────────

/// One line of "what did my triggers do".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TriggerLog {
    pub ts: i64,
    /// "schedule" | "compact" | "clear"
    pub kind: String,
    /// The schedule's or agent's name.
    pub label: String,
    pub detail: String,
    pub ok: bool,
}

/// Owned by Codly, never taken from the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct TriggerState {
    /// Newest first.
    pub log: Vec<TriggerLog>,
}

impl TriggerState {
    pub fn push_log(&mut self, entry: TriggerLog) {
        self.log.insert(0, entry);
        self.log.truncate(LOG_CAP);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct TriggersConfig {
    pub schedules: Vec<Schedule>,
    pub network: NetworkSettings,
    pub state: TriggerState,
}

/// A config from the UI, made safe: trimmed names, clamped numbers, ids filled in, and the
/// fields Codly owns (`state`, each schedule's `last_fired_at`) carried over from `old`.
pub fn normalize(mut new: TriggersConfig, old: &TriggersConfig, now: i64) -> TriggersConfig {
    new.state = old.state.clone();
    for s in &mut new.schedules {
        s.name = s.name.split_whitespace().collect::<Vec<_>>().join(" ");
        s.prompt = s.prompt.trim().to_string();
        s.every_minutes = s.every_minutes.clamp(MIN_EVERY_MINUTES, MAX_EVERY_MINUTES);
        if s.id.is_empty() {
            s.id = new_id();
            s.created_at = now;
        }
        match old.schedules.iter().find(|o| o.id == s.id) {
            Some(o) => {
                s.created_at = o.created_at;
                s.last_fired_at = o.last_fired_at;
                // Changing the interval restarts the clock instead of firing at once.
                if o.every_minutes != s.every_minutes && o.next_is_overdue(now) {
                    s.last_fired_at = Some(now);
                }
            }
            None => s.last_fired_at = None,
        }
        if s.name.is_empty() {
            s.name = s.prompt.chars().take(40).collect();
        }
    }
    new.network.advanced.peers = new.network.advanced.peers.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    new
}

impl Schedule {
    fn next_is_overdue(&self, now: i64) -> bool {
        next_run_at(self) <= now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * MIN;

    fn sched(every: i64, created: i64, fired: Option<i64>) -> Schedule {
        Schedule { id: "s1".into(), name: "n".into(), prompt: "review".into(), every_minutes: every, created_at: created, last_fired_at: fired, ..Default::default() }
    }

    #[test]
    fn schedule_next_run_and_due() {
        let s = sched(60, 0, None);
        assert_eq!(next_run_at(&s), 60 * MIN);
        assert!(due_schedules(&[s.clone()], 59 * MIN).is_empty());
        assert_eq!(due_schedules(&[s.clone()], 60 * MIN), vec!["s1".to_string()]);
        // Missed for days: due once. After it fires the clock restarts from the firing.
        let fired = Schedule { last_fired_at: Some(100 * HOUR), ..s.clone() };
        assert!(due_schedules(&[fired], 100 * HOUR + 30 * MIN).is_empty());
        let off = Schedule { enabled: false, ..s.clone() };
        let blank = Schedule { prompt: "  ".into(), ..s };
        assert!(due_schedules(&[off, blank], 1000 * HOUR).is_empty());
    }

    #[test]
    fn network_modes_nest_and_default_off() {
        let mut net = NetworkSettings::default();
        assert_eq!(net.mode, NetworkMode::Off);
        for a in [NetworkAction::Read, NetworkAction::Message, NetworkAction::Delegate] {
            assert_eq!(network_gate(&net, a, "ana"), Gate::Deny);
        }
        net.mode = NetworkMode::Listen;
        assert_eq!(network_gate(&net, NetworkAction::Read, "ana"), Gate::Allow);
        assert_eq!(network_gate(&net, NetworkAction::Message, "ana"), Gate::Deny);
        net.mode = NetworkMode::Converse;
        assert_eq!(network_gate(&net, NetworkAction::Message, "ana"), Gate::Allow);
        assert_eq!(network_gate(&net, NetworkAction::Delegate, "ana"), Gate::Deny);
        net.mode = NetworkMode::Open;
        assert_eq!(network_gate(&net, NetworkAction::Delegate, "ana"), Gate::Ask);
        net.advanced.confirm_handoffs = false;
        assert_eq!(network_gate(&net, NetworkAction::Delegate, "ana"), Gate::Allow);
    }

    #[test]
    fn network_peer_allowlist() {
        let net = NetworkSettings { mode: NetworkMode::Open, advanced: NetworkAdvanced { confirm_handoffs: false, peers: vec!["Ana".into()] } };
        assert_eq!(network_gate(&net, NetworkAction::Message, " ana "), Gate::Allow);
        assert_eq!(network_gate(&net, NetworkAction::Message, "bo"), Gate::Deny);
    }

    #[test]
    fn normalize_keeps_codly_owned_fields_and_clamps() {
        let mut old = TriggersConfig::default();
        old.schedules.push(sched(60, 5, Some(77)));
        old.state.push_log(TriggerLog { ts: 9, kind: "schedule".into(), label: "x".into(), detail: String::new(), ok: true });
        let mut new = TriggersConfig::default();
        new.schedules = vec![
            Schedule { last_fired_at: Some(999), every_minutes: 60, ..sched(60, 0, None) }, // existing: UI cannot move last_fired_at
            Schedule { id: String::new(), name: "  ".into(), prompt: " standup ".into(), every_minutes: 1, ..Default::default() }, // new
        ];
        new.network.advanced.peers = vec![" ".into(), " bo ".into()];
        let out = normalize(new, &old, 1_000_000);
        assert_eq!(out.schedules[0].last_fired_at, Some(77));
        assert_eq!(out.schedules[0].created_at, 5);
        assert!(!out.schedules[1].id.is_empty());
        assert_eq!(out.schedules[1].every_minutes, MIN_EVERY_MINUTES);
        assert_eq!(out.schedules[1].name, "standup");
        assert_eq!(out.schedules[1].created_at, 1_000_000);
        assert_eq!(out.network.advanced.peers, vec!["bo".to_string()]);
        assert_eq!(out.state.log.len(), 1);
    }

    #[test]
    fn changing_an_overdue_interval_restarts_the_clock() {
        let mut old = TriggersConfig::default();
        old.schedules.push(sched(60, 0, Some(0)));
        let mut new = old.clone();
        new.schedules[0].every_minutes = 120;
        let out = normalize(new, &old, 500 * HOUR);
        assert_eq!(out.schedules[0].last_fired_at, Some(500 * HOUR));
        assert!(due_schedules(&out.schedules, 500 * HOUR + HOUR).is_empty());
    }

    #[test]
    fn log_is_newest_first_and_capped() {
        let mut st = TriggerState::default();
        for i in 0..(LOG_CAP as i64 + 5) {
            st.push_log(TriggerLog { ts: i, kind: "schedule".into(), label: "x".into(), detail: String::new(), ok: true });
        }
        assert_eq!(st.log.len(), LOG_CAP);
        assert_eq!(st.log[0].ts, LOG_CAP as i64 + 4);
    }

    #[test]
    fn config_roundtrips_and_tolerates_missing_fields() {
        let cfg: TriggersConfig = serde_json::from_str(r#"{"schedules":[{"prompt":"hi"}],"network":{"mode":"converse"}}"#).unwrap();
        assert_eq!(cfg.schedules[0].every_minutes, 60);
        assert_eq!(cfg.network.mode, NetworkMode::Converse);
        assert!(cfg.network.advanced.confirm_handoffs);
    }
}
