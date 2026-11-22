//! The circuit breaker's pure policy (a port of Munder's `breaker.ts`): a floor-wide guard
//! against runaway agents and spend. No IO, no clock, no locks, so every rule is unit-tested.
//!
//! - `evaluate`  one agent, one beat: signals + its previous level → the next level and what
//!   to do about it. Steer first, then constrain, then (only with `hard_stop`, and only for a
//!   limit the agent crossed itself) stop; one level per beat, never a jump to a kill; one
//!   level back down per healthy beat. A budget alone never kills.
//! - `Tracker`   folds the event stream (usage, tool calls, errors, compaction) into those
//!   signals and runs `evaluate` for every agent on each beat.
//! - `over_budget`  the budget the whole team is over. The scheduler starts no task past it,
//!   and an agent with no turn running gets no note about it (a note would start a turn).
//! - `Spent`, `Today`  what was used: since the last Reset, on one day, by one plan.
//!
//! `breaker_run` enforces the verdicts with the existing mechanisms (a prompt that rides the
//! agent's next turn, an interrupt, `stop_agent`) and records each change as a `Decision`.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{EventKind, Id, TurnStatus, Usage};

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
    /// together since you last pressed Reset. The count is kept when you quit.
    pub token_budget: i64,
    /// Dollars all agents may use together since you last pressed Reset, as their CLIs report
    /// them (Claude Code: an estimate at list price; Codex and Gemini report none).
    #[serde(default)]
    pub cost_budget_usd: f64,
    /// Output tokens per minute one agent may sustain.
    pub velocity_per_min: i64,
    /// The same tool call with the same input this many times in a row.
    pub repeated_tool_limit: i64,
    /// Errors in a row (no success in between).
    pub error_storm_limit: i64,
    /// Kill the agent as the last step, when it is over a limit of its own (loop, errors,
    /// velocity). Off: the breaker never goes beyond constraining. A budget never kills.
    pub hard_stop: bool,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        BreakerConfig {
            enabled: true,
            token_budget: 0,
            cost_budget_usd: 0.0,
            velocity_per_min: 60_000,
            repeated_tool_limit: 8,
            error_storm_limit: 5,
            hard_stop: false,
        }
    }
}

impl BreakerConfig {
    /// Negative limits are nonsense; a loop limit of 1 would trip on every call.
    pub fn normalized(mut self) -> Self {
        self.token_budget = self.token_budget.max(0);
        self.cost_budget_usd = if self.cost_budget_usd.is_finite() { self.cost_budget_usd.max(0.0) } else { 0.0 };
        self.velocity_per_min = self.velocity_per_min.max(0);
        self.repeated_tool_limit = if self.repeated_tool_limit <= 0 { 0 } else { self.repeated_tool_limit.max(3) };
        self.error_storm_limit = if self.error_storm_limit <= 0 { 0 } else { self.error_storm_limit.max(2) };
        self
    }
}

// ───────────────────────────── what was used ─────────────────────────────

/// Work tokens and the cost the CLIs reported for them, added up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Spent {
    /// Input + output + cache writes; cache reads excluded.
    pub tokens: i64,
    /// What the CLIs reported (Claude Code: an estimate at list price). 0 when none did.
    pub cost_usd: f64,
}

impl Spent {
    /// One run's total. A run's counts start from zero and only grow (every adapter reports
    /// them so), so its last usage report is its total.
    pub fn of(u: &Usage) -> Spent {
        Spent { tokens: u.input_tokens + u.output_tokens + u.cache_write_tokens, cost_usd: u.cost_usd.unwrap_or(0.0).max(0.0) }
    }

    pub fn add(&mut self, more: Spent) {
        self.tokens += more.tokens;
        self.cost_usd += more.cost_usd;
    }
}

impl std::iter::Sum for Spent {
    fn sum<I: Iterator<Item = Spent>>(iter: I) -> Spent {
        iter.fold(Spent::default(), |mut total, s| {
            total.add(s);
            total
        })
    }
}

/// What the team used on one local day ("2026-11-01").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Today {
    day: String,
    spent: Spent,
}

impl Today {
    /// Count what was just used on `day`. A new day starts from zero.
    pub fn add(&mut self, day: &str, more: Spent) {
        if self.day != day {
            *self = Today { day: day.to_string(), spent: Spent::default() };
        }
        self.spent.add(more);
    }

    /// What was used on `day`: nothing when the count is another day's.
    pub fn on(&self, day: &str) -> Spent {
        if self.day == day { self.spent } else { Spent::default() }
    }
}

/// A budget the whole team is over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Budget {
    Tokens { used: i64, limit: i64 },
    Cost { used: f64, limit: f64 },
}

impl Budget {
    pub fn kind(&self) -> &'static str {
        match self {
            Budget::Tokens { .. } => "floor_budget",
            Budget::Cost { .. } => "cost_budget",
        }
    }

    /// One sentence, with the count.
    pub fn reason(&self) -> String {
        match self {
            Budget::Tokens { used, limit } => format!("the team has used {} work tokens, over the budget of {}", tok(*used), tok(*limit)),
            Budget::Cost { used, limit } => format!("the team has used about {}, over the cost budget of {}", usd(*used), usd(*limit)),
        }
    }

    /// Why a task waits, and the way out. Without the count: the count moves while agents
    /// finish their turns, and a waiting task's reason must not change with it.
    pub fn hold(&self) -> String {
        let which = match self {
            Budget::Tokens { limit, .. } => format!("the token budget of {} work tokens", tok(*limit)),
            Budget::Cost { limit, .. } => format!("the cost budget of {}", usd(*limit)),
        };
        format!("paused: {which} is used up; raise it or press Reset counters in Settings → Autonomy & Budgets")
    }
}

/// The budget the team is over, tokens first. A limit of 0 is off, and so is every limit of a
/// breaker that is off.
pub fn over_budget(cfg: &BreakerConfig, used: Spent) -> Option<Budget> {
    if !cfg.enabled {
        return None;
    }
    if cfg.token_budget > 0 && used.tokens > cfg.token_budget {
        return Some(Budget::Tokens { used: used.tokens, limit: cfg.token_budget });
    }
    if cfg.cost_budget_usd > 0.0 && used.cost_usd > cfg.cost_budget_usd {
        return Some(Budget::Cost { used: used.cost_usd, limit: cfg.cost_budget_usd });
    }
    None
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
    /// Killed (only with `hard_stop`, for a limit of its own).
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
    /// What all agents used together since the last Reset.
    pub floor: Spent,
    /// Output tokens per minute over the recent window (None: not enough samples, or compacting).
    pub velocity_per_min: Option<i64>,
    pub repeat_run: i64,
    pub error_run: i64,
}

/// Which limit an agent crossed.
#[derive(Debug, Clone, PartialEq)]
pub enum Trip {
    Budget(Budget),
    RepeatedTool { count: i64, limit: i64 },
    ErrorStorm { count: i64, limit: i64 },
    Velocity { per_min: i64, limit: i64 },
}

impl Trip {
    pub fn kind(&self) -> &'static str {
        match self {
            Trip::Budget(b) => b.kind(),
            Trip::RepeatedTool { .. } => "repeated_tool",
            Trip::ErrorStorm { .. } => "error_storm",
            Trip::Velocity { .. } => "velocity",
        }
    }

    /// One sentence, the same in the note the agent reads and in the timeline.
    pub fn reason(&self) -> String {
        match self {
            Trip::Budget(b) => b.reason(),
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

fn usd(n: f64) -> String {
    format!("${n:.2}")
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

/// The first limit of its own the agent crossed, in order of severity. Limits that are 0 are off.
fn own_trip(cfg: &BreakerConfig, s: &Signals) -> Option<Trip> {
    let on = |limit: i64| limit > 0;
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
///
/// A budget alone never goes past `Constrained`, also with `hard_stop`: the budget is the
/// team's, so the agent cannot get under it, and held it spends nothing. A kill would only
/// lose its work. So whether its turn runs when the beat comes does not change how it ends.
/// A stop is for a limit the agent crossed itself, and names that limit.
pub fn evaluate(cfg: &BreakerConfig, s: &Signals, prev: BreakerLevel) -> Verdict {
    if !cfg.enabled {
        return Verdict { level: BreakerLevel::Healthy, trip: None, action: Action::None };
    }
    if prev == BreakerLevel::Stopped {
        return Verdict { level: prev, trip: None, action: Action::None };
    }
    let budget = over_budget(cfg, s.floor).map(Trip::Budget);
    let own = own_trip(cfg, s);
    let ceiling = if cfg.hard_stop && own.is_some() { BreakerLevel::Stopped } else { BreakerLevel::Constrained };
    let level = if budget.is_some() || own.is_some() { prev.up().min(ceiling) } else { prev.down() };
    // The budget first (the note then says to stop, not to try something else), but not on a stop.
    let trip = if level == BreakerLevel::Stopped { own } else { budget.or(own) };
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
    match (level, trip) {
        // Nothing is wrong with its approach: the budget is the team's.
        (BreakerLevel::Steering, Trip::Budget(_)) => format!(
            "[Codly circuit breaker] Heads up: {why}. Bring the step you are on to a safe stop and say where you are. \
             Do not start new work."
        ),
        (BreakerLevel::Steering, _) => format!(
            "[Codly circuit breaker] Heads up: {why}. Re-check your approach: stop repeating what is not working, \
             say what is blocking you, and try something different."
        ),
        _ => format!(
            "[Codly circuit breaker] Stop active work now: {why}. Summarize where you are and what is blocking you, \
             then wait for instructions. Do not start new tool calls."
        ),
    }
}

/// What a task's worker reads when you resume it after the breaker constrained it. `unread`:
/// the trip of a worker you held already. It got no constrain note, so it reads why here.
pub fn resume_note(unread: Option<&Trip>) -> String {
    let why = unread.map(|trip| format!(" The breaker stopped your work: {}.", trip.reason())).unwrap_or_default();
    format!(
        "[Codly circuit breaker] The user resumed you.{why} Continue your task from where you stopped, without \
         repeating what was not working. Call codly_submit_report when the task is done."
    )
}

// ───────────────────────────── the tracker ─────────────────────────────

#[derive(Debug, Default)]
struct AgentTrack {
    level: Option<BreakerLevel>,
    /// Work tokens since the last Reset, over all its runs.
    work: i64,
    /// The current run's own totals, as its CLI last reported them.
    run: Spent,
    /// Between a turn's start (or its first tool call) and its end.
    in_turn: bool,
    /// (time, cumulative output tokens) of the current run, newest last.
    samples: VecDeque<(i64, i64)>,
    last_tool: Option<u64>,
    repeat_run: i64,
    error_run: i64,
    compacting_since: Option<i64>,
}

impl AgentTrack {
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
    /// What everyone used since the last Reset. Agents that exited still count.
    floor: Spent,
}

impl Tracker {
    /// A tracker that goes on from a saved count (the app was restarted).
    pub fn resumed(floor: Spent) -> Tracker {
        Tracker { agents: BTreeMap::new(), floor }
    }

    /// Fold one event in. Returns what it added to the count: a usage report's new tokens and
    /// cost, nothing for any other event.
    pub fn observe(&mut self, agent: &str, now: i64, kind: &EventKind) -> Spent {
        if let EventKind::Exited { .. } = kind {
            self.agents.remove(agent);
            return Spent::default();
        }
        let t = self.agents.entry(agent.to_string()).or_default();
        let mut added = Spent::default();
        match kind {
            EventKind::UsageUpdated { usage } => {
                let run = Spent::of(usage);
                if run.tokens < t.run.tokens {
                    t.run = Spent::default(); // a new run: its counters started over
                    t.samples.clear();
                }
                added = Spent { tokens: run.tokens - t.run.tokens, cost_usd: (run.cost_usd - t.run.cost_usd).max(0.0) };
                t.run = run;
                t.work += added.tokens;
                self.floor.add(added);
                t.samples.push_back((now, usage.output_tokens));
                while t.samples.front().is_some_and(|(ts, _)| now - ts > WINDOW_MS * 2) {
                    t.samples.pop_front();
                }
            }
            EventKind::TurnStarted { .. } => t.in_turn = true,
            EventKind::ToolStarted { name, input, .. } => {
                t.in_turn = true;
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
                t.in_turn = false;
                t.compacting_since = None;
                t.last_tool = None;
                t.repeat_run = 0;
                if *status == TurnStatus::Completed {
                    t.error_run = 0;
                }
            }
            _ => {}
        }
        added
    }

    /// What everyone used since the last Reset, including agents that exited.
    pub fn floor(&self) -> Spent {
        self.floor
    }

    pub fn level(&self, agent: &str) -> BreakerLevel {
        self.agents.get(agent).and_then(|t| t.level).unwrap_or(BreakerLevel::Healthy)
    }

    /// (agent, level, work tokens) for every agent seen.
    pub fn snapshot(&self) -> Vec<(Id, BreakerLevel, i64)> {
        self.agents.iter().map(|(id, t)| (id.clone(), t.level.unwrap_or(BreakerLevel::Healthy), t.work)).collect()
    }

    /// Forget all counters and levels (the user pressed Reset). A run that goes on keeps what
    /// its CLI reported so far, so only what it uses from now on is counted.
    pub fn reset(&mut self) {
        self.floor = Spent::default();
        for t in self.agents.values_mut() {
            *t = AgentTrack { level: Some(BreakerLevel::Healthy), run: t.run, in_turn: t.in_turn, ..Default::default() };
        }
    }

    /// One beat: evaluate every agent that is not `exempt` (the Lead: it only talks to you).
    /// Returns the agents whose level changed.
    ///
    /// Over a budget, an agent with no turn running is left as it is: a note would start a
    /// turn, and that turn spends more. Its level waits with it, so a worker you resume is not
    /// held a second time for the same budget.
    pub fn beat(&mut self, cfg: &BreakerConfig, now: i64, exempt: &dyn Fn(&str) -> bool) -> Vec<Transition> {
        let floor = self.floor;
        let over = over_budget(cfg, floor).is_some();
        let mut out = vec![];
        for (id, t) in self.agents.iter_mut() {
            if exempt(id) || (over && !t.in_turn) {
                continue;
            }
            let prev = t.level.unwrap_or(BreakerLevel::Healthy);
            let signals = Signals { floor, velocity_per_min: t.velocity(now), repeat_run: t.repeat_run, error_run: t.error_run };
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
        BreakerConfig { enabled: true, token_budget: 0, cost_budget_usd: 0.0, velocity_per_min: 0, repeated_tool_limit: 4, error_storm_limit: 3, hard_stop: false }
    }
    fn used(tokens: i64, cost_usd: f64) -> Spent {
        Spent { tokens, cost_usd }
    }
    fn turn_start() -> EventKind {
        EventKind::TurnStarted { input: None }
    }
    fn cost(input: i64, cost_usd: f64) -> EventKind {
        EventKind::UsageUpdated { usage: Usage { input_tokens: input, cost_usd: Some(cost_usd), ..Default::default() } }
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
    fn a_budget_alone_never_kills_also_with_hard_stop() {
        let c = BreakerConfig { hard_stop: true, token_budget: 1000, ..cfg() };
        let over = Signals { floor: used(5000, 0.0), ..Default::default() };
        let v = evaluate(&c, &over, BreakerLevel::Steering);
        assert_eq!((v.level, v.action), (BreakerLevel::Constrained, Action::Constrain));
        let v = evaluate(&c, &over, BreakerLevel::Constrained);
        assert_eq!((v.level, v.action), (BreakerLevel::Constrained, Action::None), "the agent cannot get the team under the budget");
        assert_eq!(v.trip.as_ref().map(Trip::kind), Some("floor_budget"));
        // Over the budget and in a loop of its own: the stop is for the loop, and says so.
        let v = evaluate(&c, &Signals { repeat_run: 4, ..over.clone() }, BreakerLevel::Constrained);
        assert_eq!((v.level, v.action), (BreakerLevel::Stopped, Action::Stop));
        assert_eq!(v.trip, Some(Trip::RepeatedTool { count: 4, limit: 4 }));
        // Below the stop, the budget is named first.
        let v = evaluate(&c, &Signals { repeat_run: 4, ..over }, BreakerLevel::Healthy);
        assert_eq!((v.level, v.trip.as_ref().map(Trip::kind)), (BreakerLevel::Steering, Some("floor_budget")));
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
        let c = BreakerConfig { token_budget: 1000, cost_budget_usd: 2.0, velocity_per_min: 500, ..cfg() };
        let trip = |s: Signals| evaluate(&c, &s, BreakerLevel::Healthy).trip.map(|t| t.kind());
        assert_eq!(trip(Signals { floor: used(1001, 0.0), ..Default::default() }), Some("floor_budget"));
        assert_eq!(trip(Signals { floor: used(1000, 2.0), ..Default::default() }), None, "at the budget is fine");
        assert_eq!(trip(Signals { floor: used(10, 2.01), ..Default::default() }), Some("cost_budget"));
        assert_eq!(trip(Signals { repeat_run: 4, ..Default::default() }), Some("repeated_tool"));
        assert_eq!(trip(Signals { error_run: 3, ..Default::default() }), Some("error_storm"));
        assert_eq!(trip(Signals { velocity_per_min: Some(501), ..Default::default() }), Some("velocity"));
        assert_eq!(trip(Signals { velocity_per_min: None, ..Default::default() }), None);
        let off = BreakerConfig { token_budget: 0, cost_budget_usd: 0.0, velocity_per_min: 0, repeated_tool_limit: 0, error_storm_limit: 0, ..c };
        let all = Signals { floor: used(i64::MAX, f64::MAX), velocity_per_min: Some(i64::MAX), repeat_run: 99, error_run: 99 };
        assert_eq!(evaluate(&off, &all, BreakerLevel::Healthy).trip, None);
    }

    #[test]
    fn notes_name_the_reason_and_differ_by_level() {
        let t = Trip::RepeatedTool { count: 8, limit: 8 };
        let steer = note(BreakerLevel::Steering, &t);
        let constrain = note(BreakerLevel::Constrained, &t);
        assert!(steer.contains("8 times in a row") && steer.contains("try something different"));
        assert!(constrain.contains("Stop active work now"));
        assert_ne!(steer, constrain);
        // Over a budget nothing is wrong with the agent's approach: the note says to stop, not to try something else.
        let budget = note(BreakerLevel::Steering, &Trip::Budget(Budget::Tokens { used: 1_200_000, limit: 1_000_000 }));
        assert!(budget.contains("1.2M work tokens, over the budget of 1.0M") && budget.contains("Do not start new work"), "{budget}");
        assert!(!budget.contains("try something different"));
    }

    #[test]
    fn config_is_clamped() {
        let c = BreakerConfig { token_budget: -5, cost_budget_usd: -1.5, velocity_per_min: -1, repeated_tool_limit: 1, error_storm_limit: -2, ..cfg() }.normalized();
        assert_eq!((c.token_budget, c.velocity_per_min, c.repeated_tool_limit, c.error_storm_limit), (0, 0, 3, 0));
        assert_eq!(c.cost_budget_usd, 0.0);
        assert_eq!(BreakerConfig { cost_budget_usd: f64::NAN, ..cfg() }.normalized().cost_budget_usd, 0.0);
    }

    #[test]
    fn settings_saved_before_the_cost_budget_still_load() {
        let old = r#"{"enabled":true,"token_budget":5000,"velocity_per_min":60000,"repeated_tool_limit":8,"error_storm_limit":5,"hard_stop":true}"#;
        let c: BreakerConfig = serde_json::from_str(old).unwrap();
        assert_eq!((c.token_budget, c.cost_budget_usd, c.hard_stop), (5000, 0.0, true));
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
        assert_eq!(t.floor().tokens, 9000);
        assert!(t.beat(&c, 10, &|_| false).is_empty());
        t.observe("a", 2, &EventKind::Exited { code: Some(0) });
        t.observe("a", 3, &turn_start());
        t.observe("b", 3, &turn_start());
        t.observe("a", 3, &usage(500, 100)); // a started over: counters restart
        t.observe("b", 3, &usage(500, 100)); // b's run restarted too: its 4000 stay counted
        // a's first run 5000 and new run 600, b's first run 4000 and new run 600
        assert_eq!(t.floor().tokens, 5000 + 600 + 4000 + 600);
        assert_eq!(t.beat(&c, 20, &|_| false).len(), 2, "over the budget: everyone at work is steered");
    }

    #[test]
    fn a_usage_report_adds_only_what_is_new() {
        let mut t = Tracker::default();
        assert_eq!(t.observe("a", 1, &cost(1000, 0.5)), used(1000, 0.5));
        assert_eq!(t.observe("a", 2, &cost(1500, 0.75)), used(500, 0.25), "a run's counts are totals: the difference is new");
        assert_eq!(t.observe("a", 3, &turn_start()), Spent::default(), "only usage reports count");
        assert_eq!(t.observe("a", 4, &cost(200, 0.125)), used(200, 0.125), "a new run starts from zero");
        assert_eq!(t.observe("b", 5, &usage(300, 0)), used(300, 0.0), "no cost reported is no cost");
        assert_eq!(t.floor(), used(2000, 0.875));
    }

    #[test]
    fn the_cost_budget_trips_like_the_token_budget() {
        let c = BreakerConfig { cost_budget_usd: 1.0, ..cfg() };
        let mut t = Tracker::default();
        t.observe("a", 1, &turn_start());
        t.observe("a", 2, &cost(100, 1.0));
        assert!(t.beat(&c, 10, &|_| false).is_empty(), "at the budget is fine");
        t.observe("a", 3, &cost(200, 1.5));
        let tr = t.beat(&c, 20, &|_| false);
        assert_eq!(tr[0].trip, Some(Trip::Budget(Budget::Cost { used: 1.5, limit: 1.0 })));
        assert_eq!(tr[0].trip.as_ref().unwrap().reason(), "the team has used about $1.50, over the cost budget of $1.00");
    }

    #[test]
    fn over_budget_names_the_budget_and_the_way_out() {
        let c = BreakerConfig { token_budget: 1_000_000, cost_budget_usd: 5.0, ..cfg() };
        assert_eq!(over_budget(&c, used(1_000_000, 5.0)), None);
        let tokens = over_budget(&c, used(1_200_000, 9.0)).unwrap();
        assert_eq!(tokens, Budget::Tokens { used: 1_200_000, limit: 1_000_000 }, "tokens first");
        assert_eq!(tokens.hold(), "paused: the token budget of 1.0M work tokens is used up; raise it or press Reset counters in Settings → Autonomy & Budgets");
        let cost = over_budget(&c, used(10, 5.01)).unwrap();
        assert_eq!(cost.hold(), "paused: the cost budget of $5.00 is used up; raise it or press Reset counters in Settings → Autonomy & Budgets");
        // The reason a waiting task shows does not move with the count.
        assert_eq!(over_budget(&c, used(1_900_000, 0.0)).unwrap().hold(), tokens.hold());
        assert_eq!(over_budget(&BreakerConfig { enabled: false, ..c }, used(i64::MAX, f64::MAX)), None, "a breaker that is off holds nothing");
    }

    #[test]
    fn over_a_budget_an_agent_with_no_turn_running_is_not_woken() {
        let c = BreakerConfig { token_budget: 1000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("busy", 1, &turn_start());
        t.observe("busy", 2, &usage(5000, 0));
        t.observe("idle", 1, &turn_start());
        t.observe("idle", 2, &usage(10, 0));
        t.observe("idle", 3, &turn_done(TurnStatus::Completed));
        let tr = t.beat(&c, 10, &|_| false);
        assert_eq!(tr.iter().map(|x| x.agent.as_str()).collect::<Vec<_>>(), ["busy"], "a note to the idle one would start a turn");
        assert_eq!(t.beat(&c, 20, &|_| false)[0].to, BreakerLevel::Constrained);
        // Constrained, its turn ends. Its level waits while it is idle: when you resume it, it is
        // not held a second time for the same budget.
        t.observe("busy", 21, &turn_done(TurnStatus::Interrupted));
        assert!(t.beat(&c, 30, &|_| false).is_empty());
        assert_eq!(t.level("busy"), BreakerLevel::Constrained);
        t.observe("busy", 31, &turn_start());
        assert!(t.beat(&c, 40, &|_| false).is_empty());
        // The idle one starts a turn while the team is over: now it is steered.
        t.observe("idle", 41, &turn_start());
        let tr = t.beat(&c, 50, &|_| false);
        assert_eq!((tr[0].agent.as_str(), tr[0].action), ("idle", Action::Steer));
    }

    #[test]
    fn with_hard_stop_a_budget_holds_every_worker_the_same_way() {
        let c = BreakerConfig { hard_stop: true, token_budget: 1000, ..cfg() };
        let mut t = Tracker::default();
        for id in ["quick", "slow"] {
            t.observe(id, 1, &turn_start());
            t.observe(id, 2, &usage(5000, 0));
        }
        assert_eq!(t.beat(&c, 10, &|_| false).len(), 2);
        assert_eq!(t.beat(&c, 20, &|_| false).iter().filter(|x| x.action == Action::Constrain).count(), 2);
        // Both answer the note in a new turn. One is done before the next beat, one is not.
        for id in ["quick", "slow"] {
            t.observe(id, 21, &turn_done(TurnStatus::Interrupted));
            t.observe(id, 22, &turn_start());
        }
        t.observe("quick", 23, &turn_done(TurnStatus::Completed));
        assert!(t.beat(&c, 30, &|_| false).is_empty(), "no kill: the one still in its turn ends like the other");
        t.observe("slow", 31, &turn_done(TurnStatus::Completed));
        assert!(t.beat(&c, 40, &|_| false).is_empty());
        assert_eq!((t.level("quick"), t.level("slow")), (BreakerLevel::Constrained, BreakerLevel::Constrained));
        // A worker that goes on in a loop of its own while the team is over is stopped for the loop.
        t.observe("slow", 41, &turn_start());
        for i in 0..4 {
            t.observe("slow", 42 + i, &tool("cargo test"));
        }
        let tr = t.beat(&c, 50, &|_| false);
        assert_eq!(tr.len(), 1);
        assert_eq!((tr[0].agent.as_str(), tr[0].action, tr[0].trip.as_ref().map(Trip::kind)), ("slow", Action::Stop, Some("repeated_tool")));
    }

    #[test]
    fn the_lead_is_exempt_but_its_tokens_count() {
        let c = BreakerConfig { token_budget: 1000, ..cfg() };
        let mut t = Tracker::default();
        t.observe("lead", 1, &turn_start());
        t.observe("lead", 1, &usage(5000, 0));
        t.observe("w", 1, &turn_start());
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
        t.observe("a", 1, &turn_start());
        t.observe("a", 1, &usage(500, 0));
        t.beat(&c, 10, &|_| false);
        assert_eq!(t.level("a"), BreakerLevel::Steering);
        t.reset();
        assert_eq!((t.floor().tokens, t.level("a")), (0, BreakerLevel::Healthy));
        assert!(t.beat(&c, 20, &|_| false).is_empty());
        // Its run goes on: only what it uses after the reset counts, not the run's total again.
        t.observe("a", 30, &usage(560, 0));
        assert_eq!(t.floor().tokens, 60);
        assert!(t.beat(&c, 40, &|_| false).is_empty());
    }

    #[test]
    fn a_restart_goes_on_from_the_saved_count() {
        let c = BreakerConfig { token_budget: 1000, ..cfg() };
        let mut t = Tracker::resumed(used(900, 0.25));
        assert_eq!(over_budget(&c, t.floor()), None);
        t.observe("a", 1, &cost(200, 0.25)); // a new run after the restart
        assert_eq!(t.floor(), used(1100, 0.5));
        assert!(over_budget(&c, t.floor()).is_some(), "what was used before the restart still counts");
    }

    #[test]
    fn runs_add_up_and_a_day_starts_from_zero() {
        let report = |input, output, cache_write, cost_usd| Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 9_999,
            cache_write_tokens: cache_write,
            cost_usd,
            ..Default::default()
        };
        let runs = [report(100, 20, 5, Some(0.5)), report(1000, 200, 0, None), report(1, 1, 1, Some(0.25))];
        let total: Spent = runs.iter().map(Spent::of).sum();
        assert_eq!(total, used(125 + 1200 + 3, 0.75), "cache reads are not work, and a run with no cost adds none");
        assert_eq!(std::iter::empty::<Spent>().sum::<Spent>(), Spent::default());

        let mut today = Today::default();
        today.add("2026-11-01", used(100, 0.5));
        today.add("2026-11-01", used(50, 0.25));
        assert_eq!(today.on("2026-11-01"), used(150, 0.75));
        assert_eq!(today.on("2026-11-02"), Spent::default(), "yesterday's count is not today's");
        today.add("2026-11-02", used(7, 0.0));
        assert_eq!(today.on("2026-11-02"), used(7, 0.0));
    }
}
