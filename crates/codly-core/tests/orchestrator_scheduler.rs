//! The scheduler as a pure function: readiness, leases, slots, routing and why tasks wait.

use std::collections::HashSet;

use codly_core::model::*;
use codly_core::orchestrator::scheduler::{AccountState, Action, AgentPick, SchedInput, schedule};

fn cli(p: Provider, signed_in: Option<bool>) -> CliInfo {
    CliInfo {
        provider: p,
        path: Some(format!("/bin/{}", p.as_str())),
        version: None,
        logged_in: signed_in,
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "test".into(),
    }
}

fn plan(id: &str, status: PlanStatus, created_at: i64) -> Plan {
    Plan {
        id: id.into(),
        workspace_id: "ws".into(),
        thread: default_lead_thread(),
        request: String::new(),
        summary: id.into(),
        assumptions: vec![],
        open_questions: vec![],
        status,
        base_commit: None,
        integration_branch: None,
        integration_worktree: None,
        merged_sha: None,
        kept: false,
        created_at,
        updated_at: created_at,
    }
}

fn task(id: &str, plan: &str, status: TaskStatus, scope: &[&str], deps: &[&str]) -> Task {
    Task {
        id: id.into(),
        plan_id: Some(plan.into()),
        key: id.into(),
        title: id.into(),
        goal: id.into(),
        acceptance: vec![],
        write_scope: scope.iter().map(|s| s.to_string()).collect(),
        verify: vec![],
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        status,
        assignee: None,
        agent_hint: None,
        account_hint: None,
        branch: None,
        worktree: None,
        attempt: 0,
        status_reason: None,
        summary: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn agent(id: &str, name: &str, p: Provider, role: Role) -> Agent {
    Agent {
        id: id.into(),
        name: name.into(),
        provider: p,
        model: None,
        role,
        status: AgentStatus::Idle,
        activity: None,
        activity_label: None,
        task_id: None,
        run_id: None,
        session_id: None,
        worktree: None,
        billing: Billing::Subscription,
        account: None,
        color: 0,
        look_seed: id.into(),
        thread: None,
        usage: Usage::default(),
        created_at: 1,
        updated_at: 1,
    }
}

struct World {
    tasks: Vec<Task>,
    plans: Vec<Plan>,
    agents: Vec<Agent>,
    clis: Vec<CliInfo>,
    settings: Settings,
    busy: HashSet<Id>,
    accounts: Vec<AccountState>,
}

impl World {
    fn new(tasks: Vec<Task>) -> World {
        World {
            tasks,
            plans: vec![plan("p", PlanStatus::Running, 1)],
            agents: vec![],
            clis: vec![cli(Provider::Claude, Some(true)), cli(Provider::Codex, Some(true))],
            settings: Settings::default(),
            busy: HashSet::new(),
            accounts: vec![],
        }
    }
    fn run(&self) -> Vec<Action> {
        schedule(&SchedInput {
            tasks: &self.tasks,
            plans: &self.plans,
            agents: &self.agents,
            clis: &self.clis,
            settings: &self.settings,
            busy: &self.busy,
            accounts: &self.accounts,
        })
    }
}

/// A Claude sign-in: id "claude:<label>", unknown usage counted as half spent.
fn acct(label: &str, used: f64) -> AccountState {
    AccountState {
        id: format!("claude:{label}"),
        provider: Provider::Claude,
        label: label.into(),
        primary: label == "default",
        enabled: true,
        logged_in: Some(true),
        parked: None,
        used,
    }
}

fn start_accounts(actions: &[Action]) -> Vec<(String, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Start { task_id, account, .. } => Some((task_id.clone(), account.clone())),
            _ => None,
        })
        .collect()
}

fn summary_of(actions: &[Action], task: &str) -> String {
    actions
        .iter()
        .find_map(|a| match a {
            Action::Start { task_id, summary, .. } if task_id == task => Some(summary.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn ready(n: usize) -> Vec<Task> {
    (1..=n).map(|i| task(&format!("t{i}"), "p", TaskStatus::Ready, &[&format!("d{i}/**")], &[])).collect()
}

fn starts(actions: &[Action]) -> Vec<(String, Provider, AgentPick)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Start { task_id, provider, agent, .. } => Some((task_id.clone(), *provider, agent.clone())),
            _ => None,
        })
        .collect()
}

fn waits(actions: &[Action]) -> Vec<(String, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Wait { task_id, reason, .. } => Some((task_id.clone(), reason.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn dependencies_gate_readiness() {
    let mut w = World::new(vec![
        task("t1", "p", TaskStatus::Done, &["a/**"], &[]),
        task("t2", "p", TaskStatus::Pending, &["b/**"], &["t1"]),
        task("t3", "p", TaskStatus::Pending, &["c/**"], &["t2"]),
    ]);
    let a = w.run();
    assert!(matches!(&a[0], Action::MakeReady { task_id } if task_id == "t2"));
    assert_eq!(starts(&a).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["t2"], "t3 still waits for t2");
    // Plans that are not running are not scheduled at all.
    w.plans[0].status = PlanStatus::AwaitingApproval;
    assert!(w.run().is_empty());
}

#[test]
fn leases_block_overlapping_scopes() {
    let mut running = task("t1", "p", TaskStatus::Running, &["src/**"], &[]);
    running.assignee = Some("a1".into());
    let mut w = World::new(vec![
        running,
        task("t2", "p", TaskStatus::Ready, &["src/ui/**"], &[]),
        task("t3", "p", TaskStatus::Ready, &["docs/**"], &[]),
    ]);
    w.agents = vec![agent("a1", "Ada", Provider::Claude, Role::Worker)];
    let a = w.run();
    let wt = waits(&a);
    assert_eq!(wt.len(), 1);
    assert_eq!(wt[0].0, "t2");
    assert!(wt[0].1.contains("overlaps running task t1"), "{}", wt[0].1);
    assert_eq!(starts(&a)[0].0, "t3");
}

#[test]
fn slots_and_provider_caps() {
    let mut w = World::new((1..=4).map(|i| task(&format!("t{i}"), "p", TaskStatus::Ready, &[&format!("d{i}/**")], &[])).collect());
    w.settings.max_parallel = 3;
    w.settings.max_per_provider = 2;
    let a = w.run();
    let s = starts(&a);
    assert_eq!(s.len(), 3, "3 slots in total");
    assert_eq!(s.iter().filter(|x| x.1 == Provider::Claude).count(), 2, "2 per provider, first in worker order");
    assert_eq!(s.iter().filter(|x| x.1 == Provider::Codex).count(), 1);
    let wt = waits(&a);
    assert_eq!(wt, vec![("t4".to_string(), "waiting: all 3 worker slots are busy".to_string())]);
    // New agents get distinct friendly names.
    let names: HashSet<String> = s
        .iter()
        .filter_map(|x| match &x.2 {
            AgentPick::New { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 3);
}

#[test]
fn routing_prefers_hint_then_worker_order_and_reuses_idle_agents() {
    let mut hinted = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    hinted.agent_hint = Some(Provider::Codex);
    let mut w = World::new(vec![hinted, task("t2", "p", TaskStatus::Ready, &["b/**"], &[])]);
    w.agents = vec![agent("c1", "Bo", Provider::Codex, Role::Worker), agent("lead", "Lead", Provider::Claude, Role::Lead)];
    let a = w.run();
    let s = starts(&a);
    assert_eq!(s[0], ("t1".into(), Provider::Codex, AgentPick::Existing("c1".into())), "idle Codex worker reused");
    assert_eq!(s[1].1, Provider::Claude);
    assert!(matches!(&s[1].2, AgentPick::New { name, .. } if name != "Lead" && name != "Bo"), "{:?}", s[1].2);
    let summary = a.iter().find_map(|x| match x {
        Action::Start { task_id, summary, .. } if task_id == "t1" => Some(summary.clone()),
        _ => None,
    });
    assert_eq!(summary.as_deref(), Some("Codex · Bo: hint 'codex', 2 slots free"));

    // A hinted provider that is not signed in falls back to the worker order, and says so.
    w.clis = vec![cli(Provider::Claude, Some(true)), cli(Provider::Codex, Some(false))];
    let a = w.run();
    let s = starts(&a);
    assert!(s.iter().all(|x| x.1 == Provider::Claude));
    let summary = a.iter().find_map(|x| match x {
        Action::Start { task_id, summary, .. } if task_id == "t1" => Some(summary.clone()),
        _ => None,
    });
    assert!(summary.unwrap().contains("hint 'codex' unavailable"));
}

#[test]
fn nothing_signed_in_explains_the_wait() {
    let mut w = World::new(vec![task("t1", "p", TaskStatus::Ready, &["a/**"], &[])]);
    w.clis = vec![cli(Provider::Claude, Some(false))];
    w.settings.worker_providers = vec![Provider::Claude, Provider::Gemini];
    let wt = waits(&w.run());
    assert_eq!(wt[0].1, "waiting: no free worker (Claude Code not signed in, Gemini CLI not installed)");
}

#[test]
fn assignee_is_respected_and_waits_when_busy() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.assignee = Some("a1".into());
    let mut w = World::new(vec![t]);
    w.agents = vec![agent("a1", "Ada", Provider::Codex, Role::Worker), agent("a2", "Bo", Provider::Claude, Role::Worker)];
    assert_eq!(starts(&w.run())[0], ("t1".into(), Provider::Codex, AgentPick::Existing("a1".into())));
    w.busy.insert("a1".into());
    assert_eq!(waits(&w.run())[0].1, "waiting: Ada (assigned) is busy");
}

#[test]
fn critical_path_goes_first() {
    // t1 unlocks two more tasks; t9 unlocks nothing. With one slot, t1 starts.
    let mut w = World::new(vec![
        task("t9", "p", TaskStatus::Ready, &["z/**"], &[]),
        task("t1", "p", TaskStatus::Ready, &["a/**"], &[]),
        task("t2", "p", TaskStatus::Pending, &["b/**"], &["t1"]),
        task("t3", "p", TaskStatus::Pending, &["c/**"], &["t2"]),
    ]);
    w.settings.max_parallel = 1;
    let s = starts(&w.run());
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0, "t1");
}

#[test]
fn ad_hoc_tasks_run_only_when_ready() {
    let mut t = task("x1", "p", TaskStatus::Pending, &[], &[]);
    t.plan_id = None;
    let mut w = World::new(vec![t]);
    w.plans.clear();
    assert!(w.run().is_empty(), "backlog");
    w.tasks[0].status = TaskStatus::Ready;
    assert_eq!(starts(&w.run()).len(), 1);
}

#[test]
fn parked_providers_are_skipped_with_the_reason() {
    let mut w = World::new(vec![task("t1", "p", TaskStatus::Ready, &["a/**"], &[])]);
    let mut claude = acct("default", 0.91);
    claude.parked = Some("91% of 5h used, resets in 2h".into());
    w.accounts = vec![claude];
    let a = w.run();
    assert_eq!(starts(&a)[0].1, Provider::Codex, "the next provider in the worker order takes it");
    assert_eq!(start_accounts(&a)[0].1, "codex", "Codex has no listed account: its implicit one");
    assert!(summary_of(&a, "t1").contains("Claude Code parked (91% of 5h used, resets in 2h)"), "{}", summary_of(&a, "t1"));
    w.accounts.push(AccountState { parked: Some("7d limit reached".into()), ..AccountState::implicit(&cli(Provider::Codex, Some(true))) });
    assert!(waits(&w.run())[0].1.contains("Codex parked (7d limit reached)"));
}

// ───────────── accounts ─────────────

#[test]
fn spread_sends_parallel_tasks_to_the_account_with_the_most_quota_left() {
    let mut w = World::new(ready(3));
    w.settings.worker_providers = vec![Provider::Claude];
    w.settings.max_parallel = 4;
    w.accounts = vec![acct("default", 0.5), acct("work", 0.5)];
    let a = w.run();
    let on: Vec<String> = start_accounts(&a).into_iter().map(|s| s.1).collect();
    assert_eq!(on, ["claude:default", "claude:work", "claude:default"], "equal headroom: the less busy account, then your order");
    assert!(summary_of(&a, "t1").starts_with("Claude Code · default · new agent"), "{}", summary_of(&a, "t1"));

    // Known usage wins over unknown (0.5): both slots of the emptier account first.
    w.accounts = vec![acct("default", 0.5), acct("work", 0.31)];
    let on: Vec<String> = start_accounts(&w.run()).into_iter().map(|s| s.1).collect();
    assert_eq!(on, ["claude:work", "claude:work", "claude:default"], "max_per_provider (2) is per account");
    assert!(summary_of(&w.run(), "t1").contains("most quota left"), "{}", summary_of(&w.run(), "t1"));
}

#[test]
fn in_order_fills_your_first_account_before_the_next() {
    let mut w = World::new(ready(3));
    w.settings.worker_providers = vec![Provider::Claude];
    w.settings.account_strategy = AccountStrategy::InOrder;
    w.settings.max_parallel = 4;
    w.accounts = vec![acct("perso", 0.8), acct("work", 0.1)];
    let on: Vec<String> = start_accounts(&w.run()).into_iter().map(|s| s.1).collect();
    assert_eq!(on, ["claude:perso", "claude:perso", "claude:work"], "the next account only takes over when the first is full");
    // …or parked.
    w.accounts[0].parked = Some("88% of 5h used".into());
    let on: Vec<String> = start_accounts(&w.run()).into_iter().map(|s| s.1).collect();
    assert_eq!(on, ["claude:work", "claude:work"]);
}

#[test]
fn disabled_signed_out_and_parked_accounts_are_skipped() {
    let mut w = World::new(ready(2));
    w.settings.worker_providers = vec![Provider::Claude];
    let mut off = acct("off", 0.0);
    off.enabled = false;
    let mut out = acct("out", 0.0);
    out.logged_in = Some(false);
    let mut hot = acct("hot", 0.9);
    hot.parked = Some("90% of 5h used, resets in 1h".into());
    w.accounts = vec![off, out, hot, acct("ok", 0.7)];
    let a = w.run();
    assert_eq!(start_accounts(&a), [("t1".into(), "claude:ok".into()), ("t2".into(), "claude:ok".into())]);
    let s = summary_of(&a, "t1");
    assert!(s.starts_with("Claude Code · ok · new agent") && s.contains("most quota left"), "{s}");

    // Nothing usable left: the task waits and says why, account by account.
    w.accounts[3].parked = Some("86% of 7d used".into());
    let wt = waits(&w.run());
    assert!(wt[0].1.starts_with("waiting: no free worker (Claude Code · hot parked"), "{}", wt[0].1);
    assert!(wt[0].1.contains("ok parked (86% of 7d used)"), "{}", wt[0].1);
    w.accounts.retain(|a| a.label != "hot" && a.label != "ok");
    assert!(waits(&w.run())[0].1.contains("Claude Code not signed in"), "{:?}", waits(&w.run()));
    w.accounts.retain(|a| a.label == "off");
    assert!(waits(&w.run())[0].1.contains("Claude Code: every account is turned off"), "{:?}", waits(&w.run()));
}

#[test]
fn a_pinned_account_is_used_when_it_has_room() {
    let mut tasks = ready(2);
    tasks[0].account_hint = Some("claude:work".into());
    tasks[1].account_hint = Some("claude:work".into());
    let mut w = World::new(tasks);
    w.settings.worker_providers = vec![Provider::Codex, Provider::Claude];
    w.settings.max_per_provider = 1;
    w.accounts = vec![acct("default", 0.1), acct("work", 0.6)];
    let a = w.run();
    assert_eq!(starts(&a)[0].1, Provider::Claude, "the pin's provider comes before your worker order");
    assert_eq!(start_accounts(&a)[0].1, "claude:work");
    assert!(summary_of(&a, "t1").contains("pinned to account 'work'"), "{}", summary_of(&a, "t1"));
    // The pinned account is full now: the second task takes the next account of that CLI.
    assert_eq!(start_accounts(&a)[1].1, "claude:default");
    assert!(summary_of(&a, "t2").contains("pinned account 'work' unavailable"), "{}", summary_of(&a, "t2"));
}

#[test]
fn idle_agents_on_the_chosen_account_are_reused_first() {
    let mut w = World::new(ready(1));
    w.settings.worker_providers = vec![Provider::Claude];
    w.accounts = vec![acct("default", 0.9), acct("work", 0.2)];
    let mut ada = agent("a1", "Ada", Provider::Claude, Role::Worker);
    ada.account = Some("claude:default".into());
    let mut bo = agent("a2", "Bo", Provider::Claude, Role::Worker);
    bo.account = Some("claude:work".into());
    bo.created_at = 5; // younger than Ada, still preferred: same account
    w.agents = vec![ada, bo];
    let a = w.run();
    assert_eq!(starts(&a)[0].2, AgentPick::Existing("a2".into()));
    assert_eq!(start_accounts(&a)[0].1, "claude:work");
}

#[test]
fn slots_are_counted_per_account_from_running_tasks() {
    let mut running = task("r1", "p", TaskStatus::Running, &["r/**"], &[]);
    running.assignee = Some("a1".into());
    let mut w = World::new(vec![running, task("t1", "p", TaskStatus::Ready, &["a/**"], &[])]);
    w.settings.worker_providers = vec![Provider::Claude];
    w.settings.max_per_provider = 1;
    let mut ada = agent("a1", "Ada", Provider::Claude, Role::Worker);
    ada.account = Some("claude:work".into());
    w.agents = vec![ada];
    w.accounts = vec![acct("work", 0.1), acct("default", 0.5)];
    let a = w.run();
    assert_eq!(start_accounts(&a), [("t1".into(), "claude:default".into())], "work's one slot is taken by Ada");
    // An agent without a recorded account ran on the primary sign-in.
    w.agents[0].account = None;
    assert_eq!(start_accounts(&w.run())[0].1, "claude:work");
}

#[test]
fn an_assigned_task_resumes_on_its_agents_account() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.assignee = Some("a1".into());
    let mut w = World::new(vec![t]);
    let mut ada = agent("a1", "Ada", Provider::Claude, Role::Worker);
    ada.account = Some("claude:work".into());
    w.agents = vec![ada];
    w.accounts = vec![acct("default", 0.1), acct("work", 0.7)];
    let a = w.run();
    assert_eq!(start_accounts(&a)[0].1, "claude:work", "its session lives there, even with less headroom");
    assert_eq!(starts(&a)[0].2, AgentPick::Existing("a1".into()));
    // Its account turned off: it continues on another one.
    w.accounts[1].enabled = false;
    assert_eq!(start_accounts(&w.run())[0].1, "claude:default");
}
