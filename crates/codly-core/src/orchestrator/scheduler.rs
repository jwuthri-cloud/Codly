//! The scheduler (agents-backend §6.5): a pure function of board state → actions, run on
//! every task/run state change and on a slow tick. It decides which ready tasks start, on
//! which provider, account and agent, and why the others wait. The "why" is part of the
//! output, so every choice and every wait becomes a `Decision` the user can inspect.
//!
//! Accounts: one CLI can have several sign-ins (two Claude subscriptions). Worker slots
//! (`max_per_provider`) and parking near a usage limit are per account. A task goes to the
//! account its Lead pinned it to when that one has room, else by `account_strategy`: the
//! account with the most quota left, or the first in your order that has room.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{names, scope};
use crate::model::*;

pub struct SchedInput<'a> {
    /// Every task of the workspace, any status.
    pub tasks: &'a [Task],
    pub plans: &'a [Plan],
    pub agents: &'a [Agent],
    pub clis: &'a [CliInfo],
    pub settings: &'a Settings,
    /// Agents that are not free for a task right now (live 1:1 chats, runs starting).
    pub busy: &'a HashSet<Id>,
    /// Sign-ins with their quota state, in the user's order. An installed CLI with none
    /// listed has one implicit account (its CLI state).
    pub accounts: &'a [AccountState],
}

/// Park an account once this fraction of one of its usage windows is spent (C17).
pub const PARK_AT: f64 = 0.85;

/// What routing needs to know about one sign-in.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountState {
    /// `AccountInfo::id`.
    pub id: String,
    pub provider: Provider,
    pub label: String,
    /// The login shell's sign-in: agents with no recorded account ran on it.
    pub primary: bool,
    /// Turned on in settings.
    pub enabled: bool,
    /// None = unknown (no cheap status check).
    pub logged_in: Option<bool>,
    /// Near its usage limit: why, e.g. "91% of its 5h window used".
    pub parked: Option<String>,
    /// Highest use of its live usage windows, 0..1.
    pub used: f64,
}

impl AccountState {
    /// The single account of a CLI whose sign-ins Codly does not tell apart.
    pub fn implicit(cli: &CliInfo) -> AccountState {
        AccountState {
            id: cli.provider.as_str().into(),
            provider: cli.provider,
            label: "default".into(),
            primary: true,
            enabled: true,
            logged_in: cli.logged_in,
            parked: None,
            used: 0.0,
        }
    }

    pub fn usable(&self) -> bool {
        self.enabled && self.logged_in != Some(false)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentPick {
    Existing(Id),
    /// A consultant hired for this task (`model`: the Lead's pick, None = the CLI default).
    New { name: String, color: i64, model: Option<String> },
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Every dependency is merged: pending → ready.
    MakeReady {
        task_id: Id,
    },
    Start {
        task_id: Id,
        provider: Provider,
        /// `AccountState::id` the run goes on.
        account: String,
        agent: AgentPick,
        summary: String,
        detail: Value,
    },
    Wait {
        task_id: Id,
        reason: String,
        detail: Value,
    },
}

/// Statuses that hold a worker slot and a write-scope lease.
pub fn is_active(s: TaskStatus) -> bool {
    matches!(s, TaskStatus::Running | TaskStatus::Verifying | TaskStatus::Reviewing | TaskStatus::ChangesRequested | TaskStatus::Merging)
}

/// Installed and not known to be signed out (`logged_in: None` = no cheap status check).
pub fn usable(cli: &CliInfo) -> bool {
    cli.path.is_some() && cli.logged_in != Some(false)
}

/// The accounts routing chooses from: the given ones plus an implicit one per installed CLI
/// that has none.
pub struct Accounts {
    list: Vec<AccountState>,
}

impl Accounts {
    pub fn new(given: &[AccountState], clis: &[CliInfo]) -> Accounts {
        let mut list = given.to_vec();
        for c in clis.iter().filter(|c| c.path.is_some()) {
            if !list.iter().any(|a| a.provider == c.provider) {
                list.push(AccountState::implicit(c));
            }
        }
        Accounts { list }
    }

    pub fn of(&self, p: Provider) -> impl Iterator<Item = &AccountState> {
        self.list.iter().filter(move |a| a.provider == p)
    }

    pub fn get(&self, id: &str) -> Option<&AccountState> {
        self.list.iter().find(|a| a.id == id)
    }

    /// The account an agent's session lives on: its recorded one, else its CLI's primary.
    pub fn of_agent(&self, a: &Agent) -> Option<&AccountState> {
        a.account
            .as_deref()
            .and_then(|id| self.get(id))
            .filter(|x| x.provider == a.provider)
            .or_else(|| self.of(a.provider).find(|x| x.primary))
            .or_else(|| self.of(a.provider).next())
    }

    /// Several usable sign-ins of this CLI (names then say which one).
    pub fn multi(&self, p: Provider) -> bool {
        self.of(p).filter(|a| a.usable()).count() > 1
    }

    /// "Claude Code", or "Claude Code · work" when the CLI has several usable accounts.
    pub fn name(&self, a: &AccountState) -> String {
        if self.multi(a.provider) { format!("{} · {}", a.provider.label(), a.label) } else { a.provider.label().to_string() }
    }
}

pub fn schedule(input: &SchedInput) -> Vec<Action> {
    let settings = input.settings;
    let max_parallel = settings.max_parallel.max(1) as usize;
    let cap = settings.max_per_provider.max(1) as usize;
    let accounts = Accounts::new(input.accounts, input.clis);
    let by_id: HashMap<&str, &Task> = input.tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let agent_by_id: HashMap<&str, &Agent> = input.agents.iter().map(|a| (a.id.as_str(), a)).collect();
    let running_plans: HashSet<&str> = input.plans.iter().filter(|p| p.status == PlanStatus::Running).map(|p| p.id.as_str()).collect();
    let plan_created: HashMap<&str, i64> = input.plans.iter().map(|p| (p.id.as_str(), p.created_at)).collect();

    let mut active = 0usize;
    // Active tasks per account.
    let mut per: HashMap<String, usize> = HashMap::new();
    let mut leases: Vec<(String, Vec<String>)> = vec![];
    let mut busy: HashSet<Id> = input.busy.clone();
    // Agent → the active task it works on (why an assigned task waits).
    let mut doing: HashMap<Id, String> = HashMap::new();
    for t in input.tasks.iter().filter(|t| is_active(t.status)) {
        active += 1;
        leases.push((t.key.clone(), t.write_scope.clone()));
        if let Some(a) = t.assignee.as_deref().and_then(|id| agent_by_id.get(id)) {
            if let Some(acc) = accounts.of_agent(a) {
                *per.entry(acc.id.clone()).or_default() += 1;
            }
            busy.insert(a.id.clone());
            doing.insert(a.id.clone(), t.key.clone());
        }
    }
    for a in input.agents {
        if a.task_id.as_deref().and_then(|id| by_id.get(id)).is_some_and(|t| is_active(t.status)) {
            busy.insert(a.id.clone());
        }
    }

    let deps_done = |t: &Task| t.depends_on.iter().all(|d| by_id.get(d.as_str()).is_some_and(|d| d.status == TaskStatus::Done));
    let mut actions = vec![];
    let mut ready: Vec<&Task> = vec![];
    for t in input.tasks {
        let in_running_plan = t.plan_id.as_deref().is_some_and(|p| running_plans.contains(p));
        match t.status {
            TaskStatus::Pending if in_running_plan && deps_done(t) => {
                actions.push(Action::MakeReady { task_id: t.id.clone() });
                ready.push(t);
            }
            TaskStatus::Ready if (in_running_plan || t.plan_id.is_none()) && deps_done(t) => ready.push(t),
            TaskStatus::Pending if in_running_plan => {
                // Say what it waits for; a dependency that cannot finish blocks it.
                let undone: Vec<&Task> =
                    t.depends_on.iter().filter_map(|d| by_id.get(d.as_str()).copied()).filter(|d| d.status != TaskStatus::Done).collect();
                let stuck: Vec<String> = undone
                    .iter()
                    .filter(|d| matches!(d.status, TaskStatus::Failed | TaskStatus::Blocked | TaskStatus::Cancelled))
                    .map(|d| format!("{} ({})", d.key, crate::store::enum_str(&d.status)))
                    .collect();
                let reason = if !stuck.is_empty() {
                    format!("blocked: depends on {}", stuck.join(", "))
                } else if undone.is_empty() {
                    "waiting: depends on a task that no longer exists".to_string()
                } else {
                    format!("waiting: depends on {}", undone.iter().map(|d| d.key.as_str()).collect::<Vec<_>>().join(", "))
                };
                actions.push(Action::Wait {
                    task_id: t.id.clone(),
                    detail: json!({ "task": t.key, "plan_id": t.plan_id, "reason": reason }),
                    reason,
                });
            }
            _ => {}
        }
    }

    // Longest chain of dependents first (critical path), older plans first.
    let cp = critical_paths(input.tasks);
    ready.sort_by_key(|t| {
        let plan_at = t.plan_id.as_deref().and_then(|p| plan_created.get(p)).copied().unwrap_or(t.created_at);
        (plan_at, Reverse(cp.get(t.id.as_str()).copied().unwrap_or(1)), t.created_at, t.key.clone())
    });

    let mut free = max_parallel.saturating_sub(active);
    let mut taken_names: Vec<String> = input.agents.iter().map(|a| a.name.clone()).collect();
    let mut used_colors: Vec<i64> = input.agents.iter().map(|a| a.color).collect();
    for t in ready {
        let wait = |reason: String| Action::Wait {
            task_id: t.id.clone(),
            detail: json!({ "task": t.key, "plan_id": t.plan_id, "reason": reason }),
            reason,
        };
        if settings.paused {
            actions.push(wait("paused: no new tasks start until you resume".into()));
            continue;
        }
        if free == 0 {
            actions.push(wait(format!("waiting: all {max_parallel} worker slots are busy")));
            continue;
        }
        if let Some((key, (a, b))) = leases.iter().find_map(|(k, s)| scope::overlap(&t.write_scope, s).map(|o| (k, o))) {
            actions.push(wait(format!("waiting: write scope '{a}' overlaps running task {key} ('{b}')")));
            continue;
        }
        let ctx = RouteCtx {
            clis: input.clis,
            settings,
            per: &per,
            cap,
            busy: &busy,
            agents: input.agents,
            agent_by_id: &agent_by_id,
            accounts: &accounts,
            doing: &doing,
        };
        match route(t, &ctx, &taken_names, &used_colors) {
            Err(reason) => actions.push(wait(reason)),
            Ok(r) => {
                free -= 1;
                *per.entry(r.account.clone()).or_default() += 1;
                leases.push((t.key.clone(), t.write_scope.clone()));
                match &r.pick {
                    AgentPick::Existing(id) => {
                        busy.insert(id.clone());
                    }
                    AgentPick::New { name, color, .. } => {
                        taken_names.push(name.clone());
                        used_colors.push(*color);
                    }
                }
                actions.push(Action::Start {
                    task_id: t.id.clone(),
                    provider: r.provider,
                    account: r.account,
                    agent: r.pick,
                    summary: r.summary,
                    detail: r.detail,
                });
            }
        }
    }
    actions
}

struct RouteCtx<'a> {
    clis: &'a [CliInfo],
    settings: &'a Settings,
    per: &'a HashMap<String, usize>,
    cap: usize,
    busy: &'a HashSet<Id>,
    agents: &'a [Agent],
    agent_by_id: &'a HashMap<&'a str, &'a Agent>,
    accounts: &'a Accounts,
    doing: &'a HashMap<Id, String>,
}

/// The account a run goes on, with its running count, (with several accounts) why, and the
/// accounts of that provider it passed over (parked or full).
struct AccountPick<'a> {
    account: &'a AccountState,
    n: usize,
    why: Option<String>,
    skipped: Vec<String>,
}

struct Route {
    provider: Provider,
    account: String,
    pick: AgentPick,
    summary: String,
    detail: Value,
}

impl RouteCtx<'_> {
    /// An account of `p` with a free slot that is not parked: the first of `prefer` that
    /// qualifies, else by the strategy. Err = why none does, one reason per account.
    fn account_for(&self, p: Provider, prefer: &[&str]) -> Result<AccountPick<'_>, Vec<String>> {
        if !self.clis.iter().any(|c| c.provider == p && c.path.is_some()) {
            return Err(vec![format!("{} not installed", p.label())]);
        }
        let all: Vec<&AccountState> = self.accounts.of(p).collect();
        if !all.iter().any(|a| a.usable()) {
            return Err(vec![if all.iter().any(|a| a.enabled) {
                format!("{} not signed in", p.label())
            } else {
                format!("{}: every account is turned off", p.label())
            }]);
        }
        let cap = self.cap;
        let mut skipped = vec![];
        let mut open: Vec<(&AccountState, usize)> = vec![];
        for a in all.into_iter().filter(|a| a.usable()) {
            let n = self.per.get(&a.id).copied().unwrap_or(0);
            match &a.parked {
                Some(why) => skipped.push(format!("{} parked ({why})", self.accounts.name(a))),
                None if n >= cap => skipped.push(format!("{} {n}/{cap} busy", self.accounts.name(a))),
                None => open.push((a, n)),
            }
        }
        if open.is_empty() {
            return Err(skipped);
        }
        let multi = self.accounts.multi(p);
        if let Some(&(a, n)) = prefer.iter().find_map(|id| open.iter().find(|(a, _)| a.id == *id)) {
            return Ok(AccountPick { account: a, n, why: None, skipped });
        }
        let (a, n, why) = match self.settings.account_strategy {
            AccountStrategy::InOrder => (open[0].0, open[0].1, "first account in your order with room".to_string()),
            AccountStrategy::Spread => {
                let &(a, n) = open
                    .iter()
                    .enumerate()
                    .min_by(|(i, (a, n)), (j, (b, m))| a.used.total_cmp(&b.used).then(n.cmp(m)).then(i.cmp(j)))
                    .map(|(_, x)| x)
                    .expect("open is not empty");
                let why = if open.iter().any(|(a, _)| a.used > 0.0) {
                    let uses: Vec<String> = open.iter().map(|(a, _)| format!("{} {:.0}%", a.label, a.used * 100.0)).collect();
                    format!("most quota left ({} used)", uses.join(", "))
                } else {
                    "least busy account".to_string()
                };
                (a, n, why)
            }
        };
        Ok(AccountPick { account: a, n, why: multi.then_some(why), skipped })
    }
}

/// Route one ready task: its assigned agent (a team member the Lead picked, or the user's
/// drag), else the consultant the Lead specified, else the provider of the account its Lead
/// pinned it to, its provider hint, then the user's worker order — the first signed-in provider
/// with an account that has a free slot. Without an assignee a consultant is hired (legacy
/// generic workers of that provider are reused first).
fn route(t: &Task, ctx: &RouteCtx, taken: &[String], colors: &[i64]) -> Result<Route, String> {
    let cap = ctx.cap;
    let pin = t.account_hint.as_deref().and_then(|id| ctx.accounts.get(id));
    if let Some(a) = t.assignee.as_deref().and_then(|id| ctx.agent_by_id.get(id)) {
        if ctx.busy.contains(&a.id) {
            return Err(match ctx.doing.get(&a.id) {
                Some(k) => format!("waiting: {} (assigned) is busy with {k}", a.name),
                None => format!("waiting: {} (assigned) is busy", a.name),
            });
        }
        // Its own account first: a retry can resume the task's session there.
        let mut prefer: Vec<&str> = ctx.accounts.of_agent(a).map(|x| x.id.as_str()).into_iter().collect();
        if let Some(pa) = pin.filter(|pa| pa.provider == a.provider) {
            prefer.insert(0, pa.id.as_str());
        }
        let acc = ctx.account_for(a.provider, &prefer).map_err(|e| format!("waiting: {} (assigned): {}", a.name, e.join(", ")))?;
        let by = if a.member.is_some() { "team member" } else { "assigned by you" };
        let mut summary = format!("{} · {}: {by}", ctx.accounts.name(acc.account), a.name);
        if let Some(why) = &acc.why {
            summary.push_str(&format!(", {why}"));
        }
        if !acc.skipped.is_empty() {
            summary.push_str(&format!("; {}", acc.skipped.join(", ")));
        }
        let detail = json!({
            "task": t.key, "plan_id": t.plan_id, "provider": a.provider, "agent": a.name, "rule": "assignee",
            "account": acc.account.id, "account_label": acc.account.label, "pinned_account": t.account_hint,
            "skipped": acc.skipped,
        });
        return Ok(Route {
            provider: a.provider,
            account: acc.account.id.clone(),
            pick: AgentPick::Existing(a.id.clone()),
            summary,
            detail,
        });
    }

    if let Some(spec) = &t.consultant
        && let Some(provider) = spec.provider
    {
        // The Lead chose the harness: wait for it rather than hire another (it can reassign).
        let prefer: Vec<&str> = pin.filter(|a| a.provider == provider).map(|a| a.id.as_str()).into_iter().collect();
        let acc = ctx.account_for(provider, &prefer).map_err(|e| format!("waiting: consultant on {}: {}", provider.label(), e.join(", ")))?;
        let name = names::next_name(taken);
        let model = spec.model.as_deref().map(|m| format!(" {m}")).unwrap_or_default();
        let mut summary = format!("{}{model} · new consultant {name}: the Lead's pick", ctx.accounts.name(acc.account));
        if let Some(why) = &acc.why {
            summary.push_str(&format!(", {why}"));
        }
        let detail = json!({
            "task": t.key, "plan_id": t.plan_id, "provider": provider, "model": spec.model, "title": spec.title, "agent": name, "new_agent": true,
            "rule": "consultant", "account": acc.account.id, "account_label": acc.account.label, "skipped": acc.skipped,
        });
        return Ok(Route {
            provider,
            account: acc.account.id.clone(),
            pick: AgentPick::New { name, color: names::next_color(colors), model: spec.model.clone() },
            summary,
            detail,
        });
    }

    let mut order: Vec<Provider> = pin.map(|a| a.provider).into_iter().chain(t.agent_hint).collect();
    for p in &ctx.settings.worker_providers {
        if !order.contains(p) {
            order.push(*p);
        }
    }
    order.dedup();
    let mut skipped: Vec<String> = vec![];
    let mut chosen = None;
    for p in order {
        let prefer: Vec<&str> = pin.filter(|a| a.provider == p).map(|a| a.id.as_str()).into_iter().collect();
        match ctx.account_for(p, &prefer) {
            Err(e) => skipped.extend(e),
            Ok(acc) => {
                skipped.extend(acc.skipped.iter().cloned());
                chosen = Some((p, acc));
                break;
            }
        }
    }
    let Some((p, acc)) = chosen else {
        let why = if skipped.is_empty() { "no worker providers configured".to_string() } else { skipped.join(", ") };
        return Err(format!("waiting: no free worker ({why})"));
    };
    let n = acc.n;
    let free = cap - n;
    let slots = format!("{free} slot{} free", if free == 1 { "" } else { "s" });
    let mut reason = match (pin, t.agent_hint) {
        (Some(pa), _) if pa.id == acc.account.id => format!("pinned to account '{}', {slots}", pa.label),
        (Some(pa), _) if pa.provider == p => format!("pinned account '{}' unavailable, next account", pa.label),
        (Some(pa), _) => format!("pinned account '{}' unavailable, next in your worker order", pa.label),
        (None, Some(h)) if h == p => format!("hint '{}', {slots}", p.as_str()),
        (None, Some(h)) => format!("hint '{}' unavailable, next in your worker order", h.as_str()),
        (None, None) => format!("first in your worker order with a free slot ({free} free)"),
    };
    if let Some(why) = &acc.why {
        reason.push_str(&format!(", {why}"));
    }

    // An idle generic worker of that provider (from before consultants), one whose session is
    // on that account first. Team members only take tasks assigned to them.
    let idle = ctx
        .agents
        .iter()
        .filter(|a| {
            a.role == Role::Worker
                && a.member.is_none()
                && !a.consultant
                && a.provider == p
                && a.status != AgentStatus::Offline
                && !ctx.busy.contains(&a.id)
        })
        .min_by_key(|a| (ctx.accounts.of_agent(a).is_none_or(|x| x.id != acc.account.id), a.created_at, a.id.clone()));
    let (pick, name, new) = match idle {
        Some(a) => (AgentPick::Existing(a.id.clone()), a.name.clone(), false),
        None => {
            let name = names::next_name(taken);
            (AgentPick::New { name: name.clone(), color: names::next_color(colors), model: None }, name, true)
        }
    };
    let mut summary = format!("{} · {}{}: {reason}", ctx.accounts.name(acc.account), if new { "new consultant " } else { "" }, name);
    if !skipped.is_empty() {
        summary.push_str(&format!("; {}", skipped.join(", ")));
    }
    let detail = json!({
        "task": t.key, "plan_id": t.plan_id, "provider": p, "agent": name, "new_agent": new,
        "hint": t.agent_hint, "slots": format!("{}/{cap}", n + 1), "skipped": skipped, "rule": "worker_order",
        "account": acc.account.id, "account_label": acc.account.label, "pinned_account": t.account_hint,
        "strategy": ctx.settings.account_strategy,
    });
    Ok(Route { provider: p, account: acc.account.id.clone(), pick, summary, detail })
}

/// Longest dependent chain (in tasks) starting at each task.
fn critical_paths(tasks: &[Task]) -> HashMap<&str, usize> {
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for t in tasks {
        for d in &t.depends_on {
            dependents.entry(d.as_str()).or_default().push(t.id.as_str());
        }
    }
    fn walk<'a>(id: &'a str, dep: &HashMap<&'a str, Vec<&'a str>>, memo: &mut HashMap<&'a str, usize>, depth: usize) -> usize {
        if let Some(&v) = memo.get(id) {
            return v;
        }
        if depth > 64 {
            return 1; // defensive: plans are validated acyclic
        }
        let v = 1 + dep.get(id).map_or(0, |ds| ds.iter().map(|d| walk(d, dep, memo, depth + 1)).max().unwrap_or(0));
        memo.insert(id, v);
        v
    }
    let mut memo = HashMap::new();
    for t in tasks {
        walk(t.id.as_str(), &dependents, &mut memo, 0);
    }
    memo
}
