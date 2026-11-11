//! The scheduler as a pure function: readiness, leases, slots, routing and why tasks wait.

use std::collections::HashSet;

use codly_core::model::*;
use codly_core::orchestrator::scheduler::{AccountState, Action, AgentPick, CALLED_IN, SchedInput, schedule};

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
        install: codly_core::env::install_hint(p),
        default_model: None,
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
        consultant: None,
        fresh_session: false,
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
        member: None,
        title: None,
        consultant: false,
        kept: false,
        archived: false,
        usage: Usage::default(),
        career: Default::default(),
        away_until: None,
        away_since: None,
        tired_since: None,
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
    now: i64,
    elsewhere: Vec<Agent>,
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
            now: 1_000_000,
            elsewhere: vec![],
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
            now: self.now,
            elsewhere: &self.elsewhere,
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

/// A worker that asked you something waits for the answer with its process running: the
/// answer puts its task straight back to Running, so the task keeps its files and its slot.
#[test]
fn a_blocked_task_with_a_live_worker_keeps_its_slot() {
    let mut asked = task("t1", "p", TaskStatus::Blocked, &["a/**"], &[]);
    asked.assignee = Some("w1".into());
    let mut w = World::new(vec![asked, task("t2", "p", TaskStatus::Ready, &["a/ui/**"], &[]), task("t3", "p", TaskStatus::Ready, &["b/**"], &[])]);
    let mut w1 = agent("w1", "Ada", Provider::Claude, Role::Worker);
    w1.task_id = Some("t1".into());
    w.agents = vec![w1];
    w.busy.insert("w1".into());
    w.settings.max_parallel = 2;
    let a = w.run();
    let wt = waits(&a);
    assert_eq!(wt.len(), 1, "{wt:?}");
    assert!(wt[0].0 == "t2" && wt[0].1.contains("overlaps running task t1"), "{wt:?}");
    assert_eq!(starts(&a).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["t3"]);
    w.settings.max_parallel = 1;
    assert_eq!(waits(&w.run())[1], ("t3".to_string(), "waiting: all 1 worker slots are busy".to_string()));
    // Stopped, or its worker gone: a blocked task holds nothing.
    w.busy.clear();
    assert_eq!(starts(&w.run()).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["t2"]);
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
fn other_offices_hold_slots_too() {
    let mut w = World::new(ready(3));
    w.settings.max_parallel = 2;
    w.settings.max_per_provider = 1;
    // Busy in another repo, on Claude: one slot left, and it can't be Claude's.
    w.elsewhere = vec![agent("x", "Xi", Provider::Claude, Role::Worker)];
    let s = starts(&w.run());
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].1, Provider::Codex);
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
    assert!(summary_of(&a, "t1").starts_with("Claude Code · default · new consultant"), "{}", summary_of(&a, "t1"));

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
    assert!(s.starts_with("Claude Code · ok · new consultant") && s.contains("most quota left"), "{s}");
    // The parked account it passed over is named, with why.
    assert!(s.ends_with("; Claude Code · hot parked (90% of 5h used, resets in 1h)"), "{s}");

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

#[test]
fn members_queue_their_tasks_and_say_what_they_are_busy_with() {
    let mut running = task("t1", "p", TaskStatus::Running, &["a/**"], &[]);
    running.assignee = Some("m1".into());
    let mut queued = task("t2", "p", TaskStatus::Ready, &["b/**"], &[]);
    queued.assignee = Some("m1".into());
    let mut w = World::new(vec![running, queued]);
    let mut ana = agent("m1", "Ana", Provider::Claude, Role::Worker);
    ana.member = Some("ana-1".into());
    ana.task_id = Some("t1".into());
    w.agents = vec![ana];
    assert_eq!(waits(&w.run()), [("t2".into(), "waiting: Ana (assigned) is busy with t1".into())]);
}

#[test]
fn unassigned_tasks_hire_consultants_never_idle_members() {
    let mut w = World::new(vec![task("t1", "p", TaskStatus::Ready, &["a/**"], &[])]);
    let mut ana = agent("m1", "Ana", Provider::Claude, Role::Worker);
    ana.member = Some("ana-1".into());
    let mut old = agent("c1", "Bo", Provider::Claude, Role::Worker);
    old.consultant = true;
    w.agents = vec![ana, old];
    let s = starts(&w.run());
    assert!(matches!(&s[0].2, AgentPick::New { name, model: None, .. } if name != "Ana" && name != "Bo"), "{:?}", s[0].2);
}

#[test]
fn the_leads_consultant_spec_picks_harness_and_model_or_waits() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.consultant = Some(ConsultantSpec { provider: Some(Provider::Codex), model: Some("gpt-5-codex:high".into()), title: Some("Backend".into()) });
    let mut w = World::new(vec![t]);
    let a = w.run();
    let s = starts(&a);
    assert_eq!(s[0].1, Provider::Codex);
    assert!(matches!(&s[0].2, AgentPick::New { model: Some(m), .. } if m == "gpt-5-codex:high"), "{:?}", s[0].2);
    assert!(summary_of(&a, "t1").contains("new consultant") && summary_of(&a, "t1").contains("the Lead's pick"), "{}", summary_of(&a, "t1"));

    // The Lead chose Codex: no fallback to another harness.
    w.clis = vec![cli(Provider::Claude, Some(true)), cli(Provider::Codex, Some(false))];
    assert_eq!(waits(&w.run())[0].1, "waiting: consultant on Codex: Codex not signed in");
}

#[test]
fn idle_members_hold_no_slot() {
    let mut w = World::new(vec![task("t1", "p", TaskStatus::Ready, &["a/**"], &[]), task("t2", "p", TaskStatus::Ready, &["b/**"], &[])]);
    w.settings.max_parallel = 2;
    w.agents = (0..3)
        .map(|i| {
            let mut m = agent(&format!("m{i}"), &format!("M{i}"), Provider::Claude, Role::Worker);
            m.member = Some(format!("m-{i}"));
            m
        })
        .collect();
    assert_eq!(starts(&w.run()).len(), 2);
}

#[test]
fn a_consultant_without_a_harness_follows_the_worker_order() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.consultant = Some(ConsultantSpec { provider: None, model: None, title: Some("QA".into()) });
    let mut w = World::new(vec![t]);
    w.settings.worker_providers = vec![Provider::Codex, Provider::Claude];
    let s = starts(&w.run());
    assert!(s[0].1 == Provider::Codex && matches!(&s[0].2, AgentPick::New { model: None, .. }), "{s:?}");
}

#[test]
fn paused_starts_nothing_and_says_why() {
    let mut w = World::new(ready(2));
    w.settings.paused = true;
    let a = w.run();
    assert!(starts(&a).is_empty(), "{a:?}");
    assert!(waits(&a).iter().all(|(_, why)| why.starts_with("paused")), "{:?}", waits(&a));
    w.settings.paused = false;
    assert_eq!(starts(&w.run()).len(), 2);
}

fn member(id: &str, name: &str) -> Agent {
    let mut m = agent(id, name, Provider::Claude, Role::Worker);
    m.member = Some(format!("{id}-member"));
    m
}

/// A member at home (the office's energy): its queued task waits until it is back, minutes
/// later, while someone else is at work; once back it starts.
#[test]
fn a_queued_task_waits_for_its_member_at_home_while_others_work() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.assignee = Some("m1".into());
    let mut w = World::new(vec![t]);
    let mut ana = member("m1", "Ana");
    ana.away_until = Some(w.now + 10 * 60_000);
    w.agents = vec![ana, member("m2", "Bo")];
    let wt = waits(&w.run());
    assert_eq!(wt.len(), 1);
    assert!(wt[0].1.starts_with("waiting: Ana (assigned) is at home until ") && wt[0].1.ends_with("; it starts when it is back, or call it in"), "{}", wt[0].1);
    assert!(starts(&w.run()).is_empty());

    // Back (away_until passed): nothing stalls, it starts as usual, with no call-in.
    w.now += 10 * 60_000;
    let a = w.run();
    assert_eq!(starts(&a)[0].2, AgentPick::Existing("m1".into()));
    assert!(!summary_of(&a, "t1").contains("called in"), "{}", summary_of(&a, "t1"));
}

/// Nobody else at work (alone on the team, or the others at home too): the game never stops the
/// crew, so the member is called in and the decision says why.
#[test]
fn a_member_at_home_is_called_in_when_nobody_else_is_free() {
    let mut t = task("t1", "p", TaskStatus::Ready, &["a/**"], &[]);
    t.assignee = Some("m1".into());
    let mut w = World::new(vec![t]);
    let mut ana = member("m1", "Ana");
    ana.away_until = Some(w.now + 10 * 60_000);
    let mut bo = member("m2", "Bo");
    bo.away_until = Some(w.now + 5 * 60_000);
    // The Lead and a reviewer do not take workers' tasks; an archived member is gone.
    let mut gone = member("m3", "Cy");
    gone.archived = true;
    let mut rex = member("r1", "Rex");
    rex.role = Role::Reviewer;
    w.agents = vec![ana.clone(), bo, gone, rex, agent("lead", "Lead", Provider::Claude, Role::Lead)];
    let a = w.run();
    assert_eq!(starts(&a)[0].2, AgentPick::Existing("m1".into()));
    assert!(summary_of(&a, "t1").ends_with("; called in from home: nobody else was free"), "{}", summary_of(&a, "t1"));
    assert!(a.iter().any(|x| matches!(x, Action::Start { detail, .. } if detail["called_in"] == true)));

    w.agents = vec![ana];
    assert!(summary_of(&w.run(), "t1").contains(CALLED_IN), "alone on the team");
}

/// Tiredness is only a tiebreak: an idle generic worker at work and rested goes first, but a
/// tired or home one still gets the task when it is the only one.
#[test]
fn the_scheduler_prefers_rested_workers_but_never_refuses_a_tired_one() {
    let mut w = World::new(vec![task("t1", "p", TaskStatus::Ready, &["a/**"], &[])]);
    let mut old = agent("c1", "Bo", Provider::Claude, Role::Worker);
    old.tired_since = Some(1);
    let mut home = agent("c2", "Cy", Provider::Claude, Role::Worker);
    home.created_at = 0;
    home.away_until = Some(w.now + 60_000);
    let mut fresh = agent("c3", "Di", Provider::Claude, Role::Worker);
    fresh.created_at = 2;
    w.agents = vec![old.clone(), home.clone(), fresh];
    assert_eq!(starts(&w.run())[0].2, AgentPick::Existing("c3".into()), "rested beats older");
    w.agents = vec![old.clone(), home];
    assert_eq!(starts(&w.run())[0].2, AgentPick::Existing("c1".into()), "tired at work beats at home");
    w.agents = vec![old];
    assert_eq!(starts(&w.run())[0].2, AgentPick::Existing("c1".into()), "tired is never refused");
}
