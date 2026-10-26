//! Plan validation: rejections the Lead can act on, and the mechanical repairs Codly makes.

use codly_core::orchestrator::planner::{PlanInput, TaskInput, find_cycle, validate};

fn task(id: &str, scope: &[&str], deps: &[&str]) -> TaskInput {
    TaskInput {
        id: id.into(),
        title: format!("task {id}"),
        goal: format!("do {id}"),
        acceptance_criteria: vec![format!("{id} works")],
        write_scope: scope.iter().map(|s| s.to_string()).collect(),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        verify: vec!["true".into()],
        size: Some("M".into()),
        ..Default::default()
    }
}

fn plan(tasks: Vec<TaskInput>) -> PlanInput {
    PlanInput { summary: "Do things".into(), tasks, ..Default::default() }
}

fn deps_of<'a>(p: &'a PlanInput, id: &str) -> &'a [String] {
    &p.tasks.iter().find(|t| t.id == id).unwrap().depends_on
}

#[test]
fn rejects_cycles_with_the_path() {
    let err = validate(plan(vec![task("t1", &["a/**"], &["t2"]), task("t2", &["b/**"], &["t1"])])).unwrap_err();
    assert!(err.contains("dependency cycle"), "{err}");
    assert!(err.contains("t1 → t2 → t1") || err.contains("t2 → t1 → t2"), "{err}");
    let three = vec![task("a", &[], &["c"]), task("b", &[], &["a"]), task("c", &[], &["b"])];
    assert_eq!(find_cycle(&three).unwrap().len(), 4);
}

#[test]
fn reports_every_problem_at_once() {
    let mut bad = task("t 1", &["/etc/**"], &["nope"]);
    bad.verify.clear();
    bad.acceptance_criteria.clear();
    let dup = task("T2", &["x/**"], &[]);
    let dup2 = task("t2", &["y/**"], &[]);
    let err = validate(plan(vec![bad, dup, dup2])).unwrap_err();
    assert!(err.starts_with("Plan rejected:"), "{err}");
    for needle in ["task id 't 1'", "acceptance_criteria", "verify command", "repo-relative", "duplicate task id 't2'"] {
        assert!(err.contains(needle), "missing {needle:?} in {err}");
    }
    assert!(err.ends_with("call codly_submit_plan again."));

    let err = validate(plan(vec![task("t1", &["a/**"], &["t9"])])).unwrap_err();
    assert!(err.contains("unknown task 't9'") && err.contains("known: t1"), "{err}");
    assert!(validate(plan(vec![])).unwrap_err().contains("at least one task"));
    let nine: Vec<TaskInput> = (1..=9).map(|i| task(&format!("t{i}"), &[&format!("d{i}/**")], &[])).collect();
    assert!(validate(plan(nine)).unwrap_err().contains("max 8"));
}

#[test]
fn overlapping_scopes_of_unordered_tasks_get_an_edge() {
    let v = validate(plan(vec![
        task("t1", &["src/ui/**"], &[]),
        task("t2", &["src/server/**"], &[]),
        task("t3", &["src/**"], &[]), // overlaps both
    ]))
    .unwrap();
    assert!(deps_of(&v.plan, "t1").is_empty());
    assert!(deps_of(&v.plan, "t2").is_empty(), "disjoint scopes stay parallel");
    let t3 = deps_of(&v.plan, "t3");
    assert!(t3.contains(&"t1".to_string()) && t3.contains(&"t2".to_string()), "{t3:?}");
    assert_eq!(v.fixes.len(), 2);
    assert!(v.fixes[0].summary.contains("t3 now waits for t1"), "{}", v.fixes[0].summary);
    assert_eq!(v.fixes[0].detail["rule"], "scope_overlap");
}

#[test]
fn already_ordered_overlaps_need_no_edge() {
    let v = validate(plan(vec![task("t1", &["src/**"], &[]), task("t2", &["src/**"], &["t1"])])).unwrap();
    assert!(v.fixes.is_empty(), "{:?}", v.fixes.iter().map(|f| &f.summary).collect::<Vec<_>>());
    // An unrestricted (empty) scope overlaps everything.
    let v = validate(plan(vec![task("t1", &[], &[]), task("t2", &["docs/**"], &[])])).unwrap();
    assert_eq!(deps_of(&v.plan, "t2"), ["t1"]);
}

#[test]
fn hot_file_owners_go_first() {
    let v = validate(plan(vec![task("t1", &["src/app/**"], &[]), task("t2", &["package.json", "scripts/**"], &[])])).unwrap();
    assert_eq!(deps_of(&v.plan, "t1"), ["t2"], "the manifest owner runs first");
    assert_eq!(v.fixes[0].detail["rule"], "hot_file");
    // A package-local manifest only orders tasks inside that package.
    let v = validate(plan(vec![task("t1", &["packages/web/package.json"], &[]), task("t2", &["packages/api/src/**"], &[])])).unwrap();
    assert!(v.fixes.is_empty());
}

#[test]
fn trivially_small_plans_collapse_to_one_task() {
    let mut a = task("t1", &["README.md"], &[]);
    let mut b = task("t2", &["docs/intro.md"], &[]);
    a.size = Some("S".into());
    b.size = Some("s".into()); // normalized
    b.verify = vec!["true".into(), "test -f docs/intro.md".into()];
    b.agent_hint = Some("codex".into());
    a.agent_hint = Some("codex".into());
    let v = validate(plan(vec![a, b])).unwrap();
    assert_eq!(v.plan.tasks.len(), 1);
    let t = &v.plan.tasks[0];
    assert_eq!(t.id, "t1");
    assert!(t.goal.contains("1. task t1") && t.goal.contains("2. task t2"), "{}", t.goal);
    assert_eq!(t.verify, vec!["true", "test -f docs/intro.md"], "deduplicated, in order");
    assert_eq!(t.write_scope, vec!["README.md", "docs/intro.md"]);
    assert_eq!(t.agent_hint.as_deref(), Some("codex"));
    assert!(v.fixes.last().unwrap().summary.starts_with("collapsed 2 small tasks"));

    // Not collapsed: a medium task, or small tasks in parallel over wildcard scopes.
    let mut c = task("t1", &["a/**"], &[]);
    let mut d = task("t2", &["b/**"], &[]);
    c.size = Some("S".into());
    d.size = Some("S".into());
    assert_eq!(validate(plan(vec![c, d])).unwrap().plan.tasks.len(), 2);
}

#[test]
fn normalizes_fields() {
    let mut t = task(" t1 ", &["./src/**", "src/**"], &[]);
    t.agent_hint = Some("any".into());
    t.risk = Some("HIGH".into());
    let mut u = task("t2", &["lib/**"], &["t1", "t1"]);
    u.agent_hint = Some("chatgpt".into());
    let v = validate(plan(vec![t, u])).unwrap();
    let t1 = &v.plan.tasks[0];
    assert_eq!(t1.id, "t1");
    assert_eq!(t1.write_scope, vec!["src/**"]);
    assert_eq!(t1.agent_hint, None);
    assert_eq!(t1.risk.as_deref(), Some("high"));
    assert_eq!(deps_of(&v.plan, "t2"), ["t1"]);
    assert!(v.fixes.iter().any(|f| f.summary.contains("unknown agent_hint 'chatgpt'")));
    assert_eq!(v.plan.tasks[1].hint(), None);
}
