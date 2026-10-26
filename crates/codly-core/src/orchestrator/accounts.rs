//! The account registry: every sign-in of every installed CLI (`env::detect_accounts`), the
//! usage windows its agents last reported (`RateLimit` events, persisted in `kv`), and the
//! choice of account for runs the scheduler does not route (Leads, reviewers, 1:1 chats).
//!
//! Pure, unit-tested policy: `record_window` (one reading into an account's windows) and
//! `load` (how spent an account is, and whether it is parked).

use std::collections::HashMap;
use std::time::Duration;

use super::scheduler::{AccountState, PARK_AT};
use super::{Inner, lock, read, write};
use crate::model::*;

/// `kv` key of the latest usage windows per account id.
const LIMITS_KEY: &str = "account_limits";

/// Used fraction assumed for an account no agent has reported usage for yet.
pub const UNKNOWN_USED: f64 = 0.5;

/// A window without a reset time counts for this long after it was seen.
const STALE_MS: i64 = 30 * 60_000;

/// "rejected" / "limited" / "exceeded": the provider refuses work until the window resets.
pub fn rejected(status: Option<&str>) -> bool {
    matches!(status, Some("rejected" | "limited" | "exceeded"))
}

/// A window that still applies: before its reset, or (no reset time) seen recently.
pub fn live(w: &UsageWindow, now: i64) -> bool {
    w.resets_at.map_or(now - w.seen_at < STALE_MS, |r| r > now)
}

/// Record one `RateLimit` reading in an account's windows (one entry per window name, the
/// latest wins, sorted by name). A reading that says the limit is reached counts as 100%; one
/// without a fraction keeps the window's last live fraction, or is not recorded at all
/// (unknown stays unknown). Returns the window's used fraction, if known.
pub fn record_window(
    windows: &mut Vec<UsageWindow>,
    window: &str,
    used_fraction: Option<f64>,
    resets_at: Option<i64>,
    status: Option<&str>,
    now: i64,
) -> Option<f64> {
    let prev = windows.iter().position(|w| w.window == window);
    let used = if rejected(status) {
        1.0
    } else {
        used_fraction.or_else(|| prev.map(|i| &windows[i]).filter(|w| live(w, now)).map(|w| w.used_fraction))?
    }
    .clamp(0.0, 1.0);
    let w = UsageWindow {
        window: window.to_string(),
        used_fraction: used,
        resets_at: resets_at.or_else(|| prev.and_then(|i| windows[i].resets_at).filter(|r| *r > now)),
        status: status.map(str::to_owned),
        seen_at: now,
    };
    match prev {
        Some(i) => windows[i] = w,
        None => {
            windows.push(w);
            windows.sort_by(|a, b| a.window.cmp(&b.window));
        }
    }
    Some(used)
}

/// How spent an account is, from the windows that still apply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Load {
    /// Highest used fraction of its live windows (None = no agent reported any).
    pub used: Option<f64>,
    /// Why new work stays off it: "91% of 5h used, resets in 2h 10m".
    pub parked: Option<String>,
}

pub fn load(limits: &[UsageWindow], now: i64) -> Load {
    let mut used: Option<f64> = None;
    let mut worst: Option<(f64, &UsageWindow)> = None;
    for w in limits.iter().filter(|w| live(w, now)) {
        let f = if rejected(w.status.as_deref()) { 1.0 } else { w.used_fraction };
        used = Some(used.map_or(f, |u: f64| u.max(f)));
        if f >= PARK_AT && worst.is_none_or(|(g, _)| f > g) {
            worst = Some((f, w));
        }
    }
    let parked = worst.map(|(f, w)| {
        let what = if rejected(w.status.as_deref()) {
            format!("{} limit reached", window_name(&w.window))
        } else {
            format!("{:.0}% of {} used", f * 100.0, window_name(&w.window))
        };
        match w.resets_at {
            Some(r) => format!("{what}, resets in {}", in_time(r - now)),
            None => what,
        }
    });
    Load { used, parked }
}

/// What the scheduler needs to know about one sign-in (unknown usage counts as half used).
pub fn state_of(a: &AccountInfo, now: i64) -> AccountState {
    let l = load(&a.limits, now);
    AccountState {
        id: a.id.clone(),
        provider: a.provider,
        label: a.label.clone(),
        primary: a.primary,
        enabled: a.enabled,
        logged_in: a.logged_in,
        parked: l.parked,
        used: l.used.unwrap_or(UNKNOWN_USED),
    }
}

/// "five_hour" → "5h", "seven_day" → "7d", "seven_day_opus" → "7d opus".
pub fn window_name(w: &str) -> String {
    w.replace("five_hour", "5h").replace("seven_day", "7d").replace('_', " ")
}

/// "45m", "2h", "2h 10m", "3d 4h".
fn in_time(ms: i64) -> String {
    let mins = (ms.max(0) + 59_999) / 60_000;
    match mins {
        m if m < 60 => format!("{m}m"),
        m if m < 48 * 60 && m % 60 == 0 => format!("{}h", m / 60),
        m if m < 48 * 60 => format!("{}h {}m", m / 60, m % 60),
        m => format!("{}d {}h", m / (24 * 60), m / 60 % 24),
    }
}

/// The usage windows persisted at the last run (empty if none or unreadable).
pub(super) fn stored_limits(store: &crate::store::Store) -> HashMap<String, Vec<UsageWindow>> {
    store.kv_get(LIMITS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Detect every account (≤ 30 s). `known` = the previous registry: only sign-ins it lacks are
/// probed (a settings change); None = probe them all.
pub(super) async fn detect(
    base: &HashMap<String, String>,
    clis: &[CliInfo],
    configs: &[AccountConfig],
    probe: bool,
    known: Option<Vec<AccountInfo>>,
) -> Option<Vec<AccountInfo>> {
    let (base, clis, configs) = (base.clone(), clis.to_vec(), configs.to_vec());
    let job = tokio::spawn(async move {
        match known {
            Some(k) => crate::env::reconcile_accounts(&base, &clis, &configs, probe, &k).await,
            None => crate::env::detect_accounts(&base, &clis, &configs, probe).await,
        }
    });
    match tokio::time::timeout(Duration::from_secs(30), job).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            tracing::warn!("account detection unavailable: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("account detection timed out");
            None
        }
    }
}

impl Inner {
    /// The registry, each account with its last-reported usage windows.
    pub(super) fn accounts(&self) -> Vec<AccountInfo> {
        let quota = lock(&self.quota);
        read(&self.accounts)
            .iter()
            .map(|a| AccountInfo { limits: quota.get(&a.id).cloned().unwrap_or_default(), ..a.clone() })
            .collect()
    }

    pub(super) fn account(&self, id: &str) -> Option<AccountInfo> {
        self.accounts().into_iter().find(|a| a.id == id)
    }

    /// Routing state of every account, in your order.
    pub(super) fn account_states(&self) -> Vec<AccountState> {
        let now = now_ms();
        self.accounts().iter().map(|a| state_of(a, now)).collect()
    }

    /// Detect accounts again and replace the registry. `reuse` = keep the sign-in state
    /// already probed (a settings change), else probe every account.
    pub(super) async fn redetect_accounts(&self, reuse: bool) {
        let known = reuse.then(|| read(&self.accounts).clone());
        let configs = self.settings().accounts;
        if let Some(found) = detect(&self.base_env, &self.clis(), &configs, !self.fixed_clis, known).await {
            *write(&self.accounts) = found;
        }
        self.kick();
    }

    /// The login shell's sign-in of a provider (None = no account detected for it).
    pub(super) fn primary_account(&self, p: Provider) -> Option<String> {
        read(&self.accounts).iter().find(|a| a.provider == p && a.primary).map(|a| a.id.clone())
    }

    /// The account an agent's session lives on: its recorded one, else its CLI's primary.
    pub(super) fn agent_account(&self, a: &Agent) -> Option<String> {
        a.account.clone().or_else(|| self.primary_account(a.provider))
    }

    /// Installed with a sign-in Codly may use (enabled, not known to be signed out).
    pub(super) fn provider_usable(&self, p: Provider, known_good: bool) -> bool {
        let ok = |l: Option<bool>| if known_good { l == Some(true) } else { l != Some(false) };
        let Some(cli) = self.clis().into_iter().find(|c| c.provider == p && c.path.is_some()) else { return false };
        let accounts = read(&self.accounts);
        let mine: Vec<&AccountInfo> = accounts.iter().filter(|a| a.provider == p).collect();
        if mine.is_empty() { ok(cli.logged_in) } else { mine.iter().any(|a| a.enabled && ok(a.logged_in)) }
    }

    /// The account a new session of `p` runs on when the scheduler does not route it (Leads,
    /// reviewers, 1:1 chats): enabled, signed in and not parked, by `strategy` (Spread also
    /// prefers the account with fewer live runs); a parked one if nothing else is left.
    /// None = no account detected for `p` (its login shell's sign-in).
    pub(super) fn choose_account(&self, p: Provider, strategy: AccountStrategy) -> Option<String> {
        let states: Vec<AccountState> = self.account_states().into_iter().filter(|a| a.provider == p).collect();
        let live: HashMap<String, usize> = lock(&self.live).values().filter_map(|l| l.account.clone()).fold(HashMap::new(), |mut m, a| {
            *m.entry(a).or_default() += 1;
            m
        });
        let n = |a: &AccountState| live.get(&a.id).copied().unwrap_or(0);
        let pick = |cands: Vec<&AccountState>| match strategy {
            AccountStrategy::InOrder => cands.first().map(|a| a.id.clone()),
            AccountStrategy::Spread => cands.into_iter().min_by(|a, b| a.used.total_cmp(&b.used).then(n(a).cmp(&n(b)))).map(|a| a.id.clone()),
        };
        pick(states.iter().filter(|a| a.usable() && a.parked.is_none()).collect())
            .or_else(|| pick(states.iter().filter(|a| a.usable()).collect()))
            .or_else(|| self.primary_account(p))
    }

    /// The account an agent's events count against: its live run's, else that run's
    /// recorded one, else the agent's own.
    pub(super) fn event_account(&self, agent_id: &str, run_id: &str) -> Option<String> {
        if let Some(a) = lock(&self.live).get(agent_id).filter(|l| l.run_id == run_id).and_then(|l| l.account.clone()) {
            return Some(a);
        }
        if let Some(a) = self.store.run(run_id).ok().flatten().and_then(|r| r.account) {
            return Some(a);
        }
        self.store.agent(agent_id).ok().flatten().and_then(|a| self.agent_account(&a))
    }

    /// A `RateLimit` reading for one account: update and persist its windows. Returns the
    /// window's used fraction, if known.
    pub(super) fn record_usage(
        &self,
        account: &str,
        window: &str,
        used_fraction: Option<f64>,
        resets_at: Option<i64>,
        status: Option<&str>,
    ) -> Option<f64> {
        let (used, json) = {
            let mut quota = lock(&self.quota);
            let used = record_window(quota.entry(account.to_string()).or_default(), window, used_fraction, resets_at, status, now_ms());
            (used, serde_json::to_string(&*quota))
        };
        if let Ok(json) = json
            && let Err(e) = self.store.kv_set(LIMITS_KEY, &json)
        {
            tracing::debug!("usage windows not persisted: {e}");
        }
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    fn w(window: &str, used: f64, resets_at: Option<i64>, status: Option<&str>, seen_at: i64) -> UsageWindow {
        UsageWindow { window: window.into(), used_fraction: used, resets_at, status: status.map(String::from), seen_at }
    }

    #[test]
    fn rate_limit_readings_update_one_window_each() {
        let now = 1_000 * H;
        let mut ws = vec![];
        assert_eq!(record_window(&mut ws, "seven_day", Some(0.2), Some(now + 48 * H), Some("allowed"), now), Some(0.2));
        assert_eq!(record_window(&mut ws, "five_hour", Some(0.31), Some(now + 2 * H), Some("allowed"), now), Some(0.31));
        assert_eq!(ws.iter().map(|w| w.window.as_str()).collect::<Vec<_>>(), ["five_hour", "seven_day"], "one per window, sorted");
        // A newer reading of the same window replaces it.
        record_window(&mut ws, "five_hour", Some(0.4), Some(now + 2 * H), Some("allowed_warning"), now + 1);
        assert_eq!(ws.len(), 2);
        assert_eq!((ws[0].used_fraction, ws[0].status.as_deref(), ws[0].seen_at), (0.4, Some("allowed_warning"), now + 1));
        // No fraction: keep the known one (and its reset); rejected counts as 100%.
        assert_eq!(record_window(&mut ws, "five_hour", None, None, Some("allowed"), now + 2), Some(0.4));
        assert_eq!(ws[0].resets_at, Some(now + 2 * H));
        assert_eq!(record_window(&mut ws, "five_hour", None, Some(now + H), Some("rejected"), now + 3), Some(1.0));
        assert_eq!(ws[0].used_fraction, 1.0);
        // Unknown stays unknown: a fraction-less reading of a new window is not recorded.
        assert_eq!(record_window(&mut ws, "seven_day_opus", None, None, Some("allowed"), now), None);
        assert_eq!(ws.len(), 2);
        // Out-of-range values are clamped.
        assert_eq!(record_window(&mut ws, "overage", Some(1.7), None, None, now), Some(1.0));
    }

    #[test]
    fn load_parks_near_a_limit_until_the_window_resets() {
        let now = 1_000 * H;
        assert_eq!(load(&[], now), Load::default(), "unknown");
        let ok = [w("five_hour", 0.31, Some(now + 2 * H), None, now), w("seven_day", 0.6, Some(now + 72 * H), None, now)];
        assert_eq!(load(&ok, now), Load { used: Some(0.6), parked: None }, "the most spent window counts");

        let hot = [w("five_hour", 0.91, Some(now + 2 * H + 10 * 60_000), None, now), w("seven_day", 0.6, None, None, now)];
        assert_eq!(load(&hot, now).parked.as_deref(), Some("91% of 5h used, resets in 2h 10m"));
        let refused = [w("seven_day", 0.5, Some(now + 50 * H), Some("rejected"), now)];
        assert_eq!(load(&refused, now), Load { used: Some(1.0), parked: Some("7d limit reached, resets in 2d 2h".into()) });

        // After the reset (or 30 min without a reset time) a reading no longer counts.
        assert_eq!(load(&hot, now + 3 * H).used, None);
        let undated = [w("five_hour", 0.95, None, None, now)];
        assert!(load(&undated, now + 10 * 60_000).parked.is_some());
        assert_eq!(load(&undated, now + 31 * 60_000), Load::default());
    }

    #[test]
    fn routing_state_treats_unknown_usage_as_half_spent() {
        let now = 1_000 * H;
        let mut a = AccountInfo {
            id: "claude:/h/.claude-work".into(),
            provider: Provider::Claude,
            home: Some("/h/.claude-work".into()),
            label: "work".into(),
            enabled: true,
            primary: false,
            configured: true,
            logged_in: Some(true),
            billing: Billing::Subscription,
            plan: None,
            identity: None,
            problem: None,
            login_command: None,
            limits: vec![],
        };
        let s = state_of(&a, now);
        assert_eq!((s.used, s.parked.clone(), s.usable()), (UNKNOWN_USED, None, true));
        a.limits = vec![w("five_hour", 0.9, Some(now + H), None, now)];
        let s = state_of(&a, now);
        assert_eq!((s.used, s.parked.as_deref()), (0.9, Some("90% of 5h used, resets in 1h")));
        assert_eq!(window_name("seven_day_opus"), "7d opus");
    }
}
