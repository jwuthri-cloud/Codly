//! The scheduler as a pure function: readiness, leases, slots, routing and why tasks wait.

use std::collections::{HashMap, HashSet};

use codly_core::model::*;
use codly_core::orchestrator::scheduler::{Action, AgentPick, SchedInput, schedule};

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
    parked: HashMap<Provider, String>,
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
            parked: HashMap::new(),
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
            parked: &self.parked,
        })
    }
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
    w.parked.insert(Provider::Claude, "91% of its 5h window used".into());
    let a = w.run();
    assert_eq!(starts(&a)[0].1, Provider::Codex, "the next provider in the worker order takes it");
    let summary = a.iter().find_map(|x| match x {
        Action::Start { summary, .. } => Some(summary.clone()),
        _ => None,
    });
    assert!(summary.unwrap().contains("Claude Code parked (91% of its 5h window used)"));
    w.parked.insert(Provider::Codex, "100% of its weekly window used".into());
    assert!(waits(&w.run())[0].1.contains("Codex parked"));
}
