//! The circuit breaker's pure policy (a port of Munder's `breaker.ts`): a floor-wide guard
//! against runaway agents and spend. No IO, no clock, no locks, so every rule is unit-tested.
//!
//! - `evaluate`  one agent, one beat: signals + its previous level → the next level and what
//!   to do about it. Steer first, then constrain, then (only with `hard_stop`) stop; one level
//!   per beat, never a jump to a kill; one level back down per healthy beat.
//! - `Tracker`   folds the event stream (usage, tool calls, errors, compaction) into those
//!   signals and runs `evaluate` for every agent on each beat.
//!
//! `breaker_run` enforces the verdicts with the existing mechanisms (a prompt that rides the
//! agent's next turn, an interrupt, `stop_agent`) and records each change as a `Decision`.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{EventKind, Id, TurnStatus};

/// How often the breaker looks at the floor.
pub const BEAT_MS: i64 = 30_000;
/// Velocity is measured over this window of usage samples (and needs at least `MIN_SPAN_MS`).
const WINDOW_MS: i64 = 120_000;
const MIN_SPAN_MS: i64 = 10_000;
/// Compaction rewrites the context and bursts tokens: exempt from velocity, for at most this long.
const COMPACT_GRACE_MS: i64 = 300_000;

// ───────────────────────────── config ─────────────────────────────

/// Settings → Autonomy & Budgets. A limit of 0 is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BreakerConfig {
    pub enabled: bool,
    /// Work tokens (input + output + cache writes; cache reads excluded) all agents may use
    /// together since the app started or you pressed Reset.
    pub token_budget: i64,
    /// Output tokens per minute one agent may sustain.
    pub velocity_per_min: i64,
    /// The same tool call with the same input this many times in a row.
    pub repeated_tool_limit: i64,
    /// Errors in a row (no success in between).
    pub error_storm_limit: i64,
    /// Kill the agent as the last step. Off: the breaker never goes beyond constraining.
    pub hard_stop: bool,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        BreakerConfig { enabled: true, token_budget: 0, velocity_per_min: 60_000, repeated_tool_limit: 8, error_storm_limit: 5, hard_stop: false }
    }
}

impl BreakerConfig {
    /// Negative limits are nonsense; a loop limit of 1 would trip on every call.
    pub fn normalized(mut self) -> Self {
        self.token_budget = self.token_budget.max(0);
        self.velocity_per_min = self.velocity_per_min.max(0);
        self.repeated_tool_limit = if self.repeated_tool_limit <= 0 { 0 } else { self.repeated_tool_limit.max(3) };
        self.error_storm_limit = if self.error_storm_limit <= 0 { 0 } else { self.error_storm_limit.max(2) };
        self
    }
}

// ───────────────────────────── the policy ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BreakerLevel {
    Healthy,
    /// A note was injected into its next turn.
    Steering,
    /// Its turn was interrupted and it was told to stop active work.
    Constrained,
    /// Killed (only with `hard_stop`).
    Stopped,
}

impl BreakerLevel {
    fn up(self) -> Self {
        match self {
            BreakerLevel::Healthy => BreakerLevel::Steering,
            BreakerLevel::Steering => BreakerLevel::Constrained,
            _ => BreakerLevel::Stopped,
        }
    }
    fn down(self) -> Self {
        match self {
            BreakerLevel::Stopped => BreakerLevel::Stopped, // sticky: the process is gone
            BreakerLevel::Constrained => BreakerLevel::Steering,
            _ => BreakerLevel::Healthy,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            BreakerLevel::Healthy => "healthy",
            BreakerLevel::Steering => "steering",
            BreakerLevel::Constrained => "constrained",
            BreakerLevel::Stopped => "stopped",
        }
    }
}

/// What the breaker looks at for one agent on one beat.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    /// All agents' work tokens together.
    pub floor_tokens: i64,
    /// Output tokens per minute over the recent window (None: not enough samples, or compacting).
    pub velocity_per_min: Option<i64>,
    pub repeat_run: i64,
    pub error_run: i64,
}

/// Which limit an agent crossed.
#[derive(Debug, Clone, PartialEq)]
pub enum Trip {
    FloorBudget { used: i64, limit: i64 },
    RepeatedTool { count: i64, limit: i64 },
    ErrorStorm { count: i64, limit: i64 },
    Velocity { per_min: i64, limit: i64 },
}

impl Trip {
    pub fn kind(&self) -> &'static str {
        match self {
            Trip::FloorBudget { .. } => "floor_budget",
            Trip::RepeatedTool { .. } => "repeated_tool",
            Trip::ErrorStorm { .. } => "error_storm",
            Trip::Velocity { .. } => "velocity",
        }
    }

    /// One sentence, the same in the note the agent reads and in the timeline.
    pub fn reason(&self) -> String {
        match self {
            Trip::FloorBudget { used, limit } => format!("the team has used {} work tokens, over the budget of {}", tok(*used), tok(*limit)),
            Trip::RepeatedTool { count, limit } => format!("it ran the same tool call {count} times in a row (limit {limit})"),
            Trip::ErrorStorm { count, limit } => format!("it hit {count} errors in a row (limit {limit})"),
            Trip::Velocity { per_min, limit } => format!("it is producing {} output tokens a minute (limit {})", tok(*per_min), tok(*limit)),
        }
    }
}

fn tok(n: i64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        10_000.. => format!("{}k", n / 1000),
        _ => n.to_string(),
    }
}

/// What the caller must do about the new level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Send the steer note (it rides the agent's next turn).
    Steer,
    /// Interrupt the turn and tell it to stop active work.
    Constrain,
    /// Shut it down.
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub level: BreakerLevel,
    pub trip: Option<Trip>,
    pub action: Action,
}

/// The first limit crossed, in order of severity. Limits that are 0 are off.
fn first_trip(cfg: &BreakerConfig, s: &Signals) -> Option<Trip> {
    let on = |limit: i64| limit > 0;
    if on(cfg.token_budget) && s.floor_tokens > cfg.token_budget {
        return Some(Trip::FloorBudget { used: s.floor_tokens, limit: cfg.token_budget });
    }
    if on(cfg.repeated_tool_limit) && s.repeat_run >= cfg.repeated_tool_limit {
        return Some(Trip::RepeatedTool { count: s.repeat_run, limit: cfg.repeated_tool_limit });
    }
    if on(cfg.error_storm_limit) && s.error_run >= cfg.error_storm_limit {
        return Some(Trip::ErrorStorm { count: s.error_run, limit: cfg.error_storm_limit });
    }
    match s.velocity_per_min {
        Some(v) if on(cfg.velocity_per_min) && v > cfg.velocity_per_min => Some(Trip::Velocity { per_min: v, limit: cfg.velocity_per_min }),
        _ => None,
    }
}

/// One agent, one beat. A tripped limit moves it one level up (capped at `Constrained`, or
/// `Stopped` with `hard_stop`); a healthy beat moves it one level down. Only a move up acts.
pub fn evaluate(cfg: &BreakerConfig, s: &Signals, prev: BreakerLevel) -> Verdict {
    if !cfg.enabled {
        return Verdict { level: BreakerLevel::Healthy, trip: None, action: Action::None };
    }
    if prev == BreakerLevel::Stopped {
        return Verdict { level: prev, trip: None, action: Action::None };
    }
    let trip = first_trip(cfg, s);
    let ceiling = if cfg.hard_stop { BreakerLevel::Stopped } else { BreakerLevel::Constrained };
    let level = if trip.is_some() { prev.up().min(ceiling) } else { prev.down() };
    let action = match level {
        l if l <= prev => Action::None,
        BreakerLevel::Steering => Action::Steer,
        BreakerLevel::Constrained => Action::Constrain,
        _ => Action::Stop,
    };
    Verdict { level, trip, action }
}

/// The note an agent reads when it is steered or constrained.
pub fn note(level: BreakerLevel, trip: &Trip) -> String {
    let why = trip.reason();
    match level {
        BreakerLevel::Steering => format!(
            "[Codly circuit breaker] Heads up: {why}. Re-check your approach: stop repeating what is not working, \
             say what is blocking you, and try something different."
        ),
        _ => format!(
            "[Codly circuit breaker] Stop active work now: {why}. Summarize where you are and what is blocking you, \
             then wait for instructions. Do not start new tool calls."
        ),
    }
}

// ───────────────────────────── the tracker ─────────────────────────────

#[derive(Debug, Default)]
struct AgentTrack {
    level: Option<BreakerLevel>,
    /// Work tokens of runs that restarted its counters, plus the current run's.
    banked: i64,
    current: i64,
    /// (time, cumulative output tokens) of the current run, newest last.
    samples: VecDeque<(i64, i64)>,
    last_tool: Option<u64>,
    repeat_run: i64,
    error_run: i64,
    compacting_since: Option<i64>,
}

impl AgentTrack {
    fn work(&self) -> i64 {
        self.banked + self.current
    }

    /// Output tokens per minute over the window ending now.
    fn velocity(&self, now: i64) -> Option<i64> {
        if self.compacting_since.is_some_and(|t| now - t < COMPACT_GRACE_MS) {
            return None;
        }
        let (t0, o0) = *self.samples.iter().find(|(t, _)| now - t <= WINDOW_MS)?;
        let (_, o1) = *self.samples.back()?;
        let span = now - t0;
        (span >= MIN_SPAN_MS).then(|| (o1 - o0).max(0) * 60_000 / span)
    }
}

/// One level change (or a steer that did nothing new) for the caller to enforce and record.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub agent: Id,
    pub from: BreakerLevel,
    pub to: BreakerLevel,
    pub action: Action,
    pub trip: Option<Trip>,
}

/// Per-agent signals, folded from the event stream.
#[derive(Debug, Default)]
pub struct Tracker {
    agents: BTreeMap<Id, AgentTrack>,
    /// Work tokens of agents that exited: they still count against the floor budget.
    retired: i64,
}

impl Tracker {
    pub fn observe(&mut self, agent: &str, now: i64, kind: &EventKind) {
        if let EventKind::Exited { .. } = kind {
            if let Some(t) = self.agents.remove(agent) {
                self.retired += t.work();
            }
            return;
        }
        let t = self.agents.entry(agent.to_string()).or_default();
        match kind {
            EventKind::UsageUpdated { usage } => {
                let work = usage.input_tokens + usage.output_tokens + usage.cache_write_tokens;
                if work < t.current {
                    t.banked += t.current; // a new run: its counters started over
                    t.samples.clear();
                }
                t.current = work;
                t.samples.push_back((now, usage.output_tokens));
                while t.samples.front().is_some_and(|(ts, _)| now - ts > WINDOW_MS * 2) {
                    t.samples.pop_front();
                }
            }
            EventKind::ToolStarted { name, input, .. } => {
                t.compacting_since = None;
                let mut h = DefaultHasher::new();
                name.hash(&mut h);
                // Long strings are cut so distinct big edits do not look alike, but not so short
                // that distinct commands do (Munder's 200-char prefix did exactly that).
                input.to_string().chars().take(4096).for_each(|c| c.hash(&mut h));
                let key = h.finish();
                t.repeat_run = if t.last_tool == Some(key) { t.repeat_run + 1 } else { 1 };
                t.last_tool = Some(key);
            }
            EventKind::ToolCompleted { ok: true, .. } | EventKind::Message { .. } => t.error_run = 0,
            EventKind::Error { .. } | EventKind::Retrying { .. } => t.error_run += 1,
            EventKind::Compacting { .. } => t.compacting_since = Some(now),
            // A turn that ended is a loop that ended (also after our own interrupt).
            EventKind::TurnCompleted { status, .. } => {
                t.compacting_since = None;
                t.last_tool = None;
                t.repeat_run = 0;
                if *status == TurnStatus::Completed {
                    t.error_run = 0;
                }
            }
            _ => {}
        }
    }

    /// Work tokens of everyone, including agents that exited.
    pub fn floor_tokens(&self) -> i64 {
        self.retired + self.agents.values().map(AgentTrack::work).sum::<i64>()
    }

    pub fn level(&self, agent: &str) -> BreakerLevel {
        self.agents.get(agent).and_then(|t| t.level).unwrap_or(BreakerLevel::Healthy)
    }

    /// (agent, level, work tokens) for every agent seen.
    pub fn snapshot(&self) -> Vec<(Id, BreakerLevel, i64)> {
        self.agents.iter().map(|(id, t)| (id.clone(), t.level.unwrap_or(BreakerLevel::Healthy), t.work())).collect()
    }

    /// Forget all counters and levels (the user pressed Reset).
    pub fn reset(&mut self) {
        self.retired = 0;
        for t in self.agents.values_mut() {
            *t = AgentTrack { level: Some(BreakerLevel::Healthy), ..Default::default() };
        }
    }

    /// One beat: evaluate every agent that is not `exempt` (the Lead: it only talks to you).
    /// Returns the agents whose level changed.
    pub fn beat(&mut self, cfg: &BreakerConfig, now: i64, exempt: &dyn Fn(&str) -> bool) -> Vec<Transition> {
        let floor_tokens = self.floor_tokens();
        let mut out = vec![];
        for (id, t) in self.agents.iter_mut() {
            if exempt(id) {
                continue;
            }
            let prev = t.level.unwrap_or(BreakerLevel::Healthy);
            let signals = Signals { floor_tokens, velocity_per_min: t.velocity(now), repeat_run: t.repeat_run, error_run: t.error_run };
            let v = evaluate(cfg, &signals, prev);
            t.level = Some(v.level);
            if v.level != prev {
                out.push(Transition { agent: id.clone(), from: prev, to: v.level, action: v.action, trip: v.trip });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActivityKind, Target, Usage};
    use serde_json::json;

    fn cfg() -> BreakerConfig {
        BreakerConfig { enabled: true, token_budget: 0, velocity_per_min: 0, repeated_tool_limit: 4, error_storm_limit: 3, hard_stop: false }
    }
    fn looping() -> Signals {
        Signals { repeat_run: 4, ..Default::default() }
    }
    fn tool(cmd: &str) -> EventKind {
        EventKind::ToolStarted {
            call_id: "c".into(),
            name: "Bash".into(),
            activity: ActivityKind::Thinking,
            target: Target::default(),
            input: json!({ "command": cmd }),
            confidence: "observed".into(),
        }
    }
    fn usage(input: i64, output: i64) -> EventKind {
        EventKind::UsageUpdated { usage: Usage { input_tokens: input, output_tokens: output, ..Default::default() } }
    }
    fn err() -> EventKind {
        EventKind::Error { message: "529 overloaded".into(), fatal: false }
    }
    fn turn_done(status: TurnStatus) -> EventKind {
        EventKind::TurnCompleted { status, result_text: None, duration_ms: None }
    }

    #[test]
    fn healthy_signals_do_nothing() {
        let v = evaluate(&cfg(), &Signals::default(), BreakerLevel::Healthy);
        assert_eq!(v, Verdict { level: BreakerLevel::Healthy, trip: None, action: Action::None });
    }

    #[test]
    fn climbs_one_level_per_beat_and_stops_at_constrained_without_hard_stop() {
        let c = cfg();
        let v1 = evaluate(&c, &looping(), BreakerLevel::Healthy);
        assert_eq!((v1.level, v1.action), (BreakerLevel::Steering, Action::Steer));
        let v2 = evaluate(&c, &looping(), v1.level);
        assert_eq!((v2.level, v2.action), (BreakerLevel::Constrained, Action::Constrain));
        let v3 = evaluate(&c, &looping(), v2.level);
        assert_eq!((v3.level, v3.action), (BreakerLevel::Constrained, Action::None), "no kill, and no repeated nagging");
    }

    #[test]
    fn hard_stop_adds_the_last_rung_and_it_is_sticky() {
        let c = BreakerConfig { hard_stop: true, ..cfg() };
        let v = evaluate(&c, &looping(), BreakerLevel::Constrained);
        assert_eq!((v.level, v.action), (BreakerLevel::Stopped, Action::Stop));
        let after = evaluate(&c, &Signals::default(), BreakerLevel::Stopped);
        assert_eq!((after.level, after.action), (BreakerLevel::Stopped, Action::None));
    }

    #[test]
    fn recovers_one_level_per_healthy_beat() {
        let c = cfg();
        let v = evaluate(&c, &Signals::default(), BreakerLevel::Constrained);
        assert_eq!((v.level, v.action), (BreakerLevel::Steering, Action::None));
        assert_eq!(evaluate(&c, &Signals::default(), v.level).level, BreakerLevel::Healthy);
    }

    #[test]
    fn disabled_breaker_clears_everything() {
        let c = BreakerConfig { enabled: false, ..cfg() };
        let v = evaluate(&c, &looping(), BreakerLevel::Constrained);
        assert_eq!((v.level, v.action), (BreakerLevel::Healthy, Action::None));
    }

    #[test]
    fn each_limit_trips_and_zero_means_off() {
        let c = BreakerConfig { token_budget: 1000, velocity_per_min: 500, ..cfg() };
        let trip = |s: Signals| first_trip(&c, &s).map(|t| t.kind());
        assert_eq!(trip(Signals { floor_tokens: 1001, ..Default::default() }), Some("floor_budget"));
        assert_eq!(trip(Signals { floor_tokens: 1000, ..Default::default() }), None, "at the budget is fine");
        assert_eq!(trip(Signals { repeat_run: 4, ..Default::default() }), Some("repeated_tool"));
        assert_eq!(trip(Signals { error_run: 3, ..Default::default() }), Some("error_storm"));
        assert_eq!(trip(Signals { velocity_per_min: Some(501), ..Default::default() }), Some("velocity"));
        assert_eq!(trip(Signals { velocity_per_min: None, ..Default::default() }), None);
        let off = BreakerConfig { token_budget: 0, velocity_per_min: 0, repeated_tool_limit: 0, error_storm_limit: 0, ..c };
        let all = Signals { floor_tokens: i64::MAX, velocity_per_min: Some(i64::MAX), repeat_run: 99, error_run: 99 };
        assert_eq!(first_trip(&off, &all), None);
    }

    #[test]
    fn notes_name_the_reason_and_differ_by_level() {
        let t = Trip::RepeatedTool { count: 8, limit: 8 };
        let steer = note(BreakerLevel::Steering, &t);
        let constrain = note(BreakerLevel::Constrained, &t);
        assert!(steer.contains("8 times in a row") && steer.contains("try something different"));
        assert!(constrain.contains("Stop active work now"));
        assert_ne!(steer, constrain);
    }

    #[test]
    fn config_is_clamped() {
        let c = BreakerConfig { token_budget: -5, velocity_per_min: -1, repeated_tool_limit: 1, error_storm_limit: -2, ..cfg() }.normalized();
        assert_eq!((c.token_budget, c.velocity_per_min, c.repeated_tool_limit, c.error_storm_limit), (0, 0, 3, 0));
    }

    #[test]
    fn identical_tool_calls_in_a_row_trip_and_a_different_one_resets() {
        let mut t = Tracker::default();
        for i in 0..4 {
            t.observe("a", i, &tool("cargo test"));
        }
        let tr = t.beat(&cfg(), 100, &|_| false);
        assert_eq!(tr.len(), 1);
        assert_eq!((tr[0].to, tr[0].action), (BreakerLevel::Steering, Action::Steer));
        assert_eq!(tr[0].trip.as_ref().map(Trip::kind), Some("repeated_tool"));

        t.observe("a", 101, &tool("cargo build")); // changed approach
        let tr = t.beat(&cfg(), 200, &|_| false);
        assert_eq!((tr[0].from, tr[0].to), (BreakerLevel::Steering, BreakerLevel::Healthy));
    }

    #[test]
    fn long_inputs_that_differ_late_are_not_repeats() {
        let mut t = Tracker::default();
        let long = "x".repeat(300);
        for i in 0..6 {
            t.observe("a", i, &tool(&format!("{long}{i}")));
        }
        assert!(t.beat(&cfg(), 100, &|_| false).is_empty());
    }

    #[test]
    fn a_finished_turn_ends_the_loop() {
        let mut t = Tracker::default();
        for i in 0..4 {
            t.observe("a", i, &tool("ls"));
        }
        t.observe("a", 5, &turn_done(TurnStatus::Interrupted));
        assert!(t.beat(&cfg(), 100, &|_| false).is_empty());
    }

    #[test]
    fn error_storm_needs_consecutive_errors() {
        let mut t = Tracker::default();
        t.observe("a", 1, &err());
        t.observe("a", 2, &err());
        t.observe("a", 3, &EventKind::ToolCompleted { call_id: "c".into(), ok: true, output: None, exit_code: None, duration_ms: None, tests_passed: None, tests_failed: None });
        t.observe("a", 4, &err());
        assert!(t.beat(&cfg(), 10, &|_| false).is_empty(), "a success in between breaks the streak");
        t.observe("a", 5, &err());
        t.observe("a", 6, &EventKind::Retrying { attempt: 3, reason: "overloaded".into() });
        let tr = t.beat(&cfg(), 20, &|_| false);
        assert_eq!(tr[0].trip.as_ref().map(Trip::kind), Some("error_storm"));
    }

    #[test]
    fn velocity_is_output_tokens_over_the_window_and_compaction_is_exempt() {
        let c = BreakerConfig { velocity_per_min: 10_000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("a", 0, &usage(1000, 0));
        t.observe("a", 30_000, &usage(2000, 20_000)); // 20k output in 30 s = 40k/min
        let tr = t.beat(&c, 30_000, &|_| false);
        assert_eq!(tr[0].trip.as_ref().map(Trip::kind), Some("velocity"));

        let mut t = Tracker::default();
        t.observe("a", 0, &usage(1000, 0));
        t.observe("a", 1_000, &EventKind::Compacting { pre_tokens: None, post_tokens: None });
        t.observe("a", 30_000, &usage(2000, 20_000));
        assert!(t.beat(&c, 30_000, &|_| false).is_empty(), "compaction bursts are not a runaway");
    }

    #[test]
    fn velocity_decays_when_the_agent_goes_quiet() {
        let c = BreakerConfig { velocity_per_min: 15_000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("a", 0, &usage(0, 0));
        t.observe("a", 20_000, &usage(0, 20_000)); // 60k/min
        assert_eq!(t.beat(&c, 20_000, &|_| false)[0].to, BreakerLevel::Steering);
        // 100 s later with no new usage the same 20k tokens are 12k/min: back under the limit.
        assert_eq!(t.beat(&c, 100_000, &|_| false)[0].to, BreakerLevel::Healthy);
    }

    #[test]
    fn floor_budget_counts_everyone_including_exited_runs_and_new_runs() {
        let c = BreakerConfig { token_budget: 10_000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("a", 1, &usage(4000, 1000));
        t.observe("b", 1, &usage(3000, 1000));
        assert_eq!(t.floor_tokens(), 9000);
        assert!(t.beat(&c, 10, &|_| false).is_empty());
        t.observe("a", 2, &EventKind::Exited { code: Some(0) });
        t.observe("a", 3, &usage(500, 100)); // a started over: counters restart
        t.observe("b", 3, &usage(500, 100)); // b's run restarted too: banks 4000
        // 5000 retired + a's new run 600 + b's banked 4000 and new run 600
        assert_eq!(t.floor_tokens(), 5000 + 600 + 4000 + 600);
        assert_eq!(t.beat(&c, 20, &|_| false).len(), 2, "over the budget: everyone is steered");
    }

    #[test]
    fn the_lead_is_exempt_but_its_tokens_count() {
        let c = BreakerConfig { token_budget: 1000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("lead", 1, &usage(5000, 0));
        t.observe("w", 1, &usage(10, 0));
        let tr = t.beat(&c, 10, &|id| id == "lead");
        assert_eq!(tr.len(), 1);
        assert_eq!(tr[0].agent, "w", "the worker is over a floor the Lead blew");
        assert_eq!(t.level("lead"), BreakerLevel::Healthy);
    }

    #[test]
    fn reset_clears_levels_and_counters() {
        let c = BreakerConfig { token_budget: 100, ..cfg() };
        let mut t = Tracker::default();
        t.observe("a", 1, &usage(500, 0));
        t.beat(&c, 10, &|_| false);
        assert_eq!(t.level("a"), BreakerLevel::Steering);
        t.reset();
        assert_eq!((t.floor_tokens(), t.level("a")), (0, BreakerLevel::Healthy));
        assert!(t.beat(&c, 20, &|_| false).is_empty());
    }
}
