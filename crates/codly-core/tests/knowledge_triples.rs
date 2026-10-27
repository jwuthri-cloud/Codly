//! Knowledge graph: temporal triples (dedupe, single-valued predicates, validity intervals),
//! entity queries, and facts derived from real events.

use codly_core::memory::{self, derive_triples, derive_triples_with};
use codly_core::model::*;
use rusqlite::Connection;

const SCHEMA: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/store/schema.sql"));

fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(SCHEMA).unwrap();
    c
}

fn t(ws: Option<&str>, s: &str, p: &str, o: &str, from: i64) -> Triple {
    Triple {
        id: String::new(),
        workspace_id: ws.map(String::from),
        subject: s.into(),
        predicate: p.into(),
        object: o.into(),
        source: "agent".into(),
        source_event: None,
        confidence: 1.0,
        valid_from: from,
        valid_to: None,
    }
}

fn live(db: &Connection, ws: Option<&str>, entity: Option<&str>, pred: Option<&str>) -> Vec<(String, String, String)> {
    let mut v: Vec<_> =
        memory::query_triples(db, ws, entity, pred, 100).unwrap().into_iter().map(|t| (t.subject, t.predicate, t.object)).collect();
    v.sort();
    v
}

#[test]
fn identical_live_triples_are_deduplicated_and_ids_are_filled_in() {
    let db = db();
    let a = memory::add_triple(&db, &t(Some("ws"), "agent:Ada", "edited", "file:a.rs", 100)).unwrap().expect("first insert");
    assert!(!a.id.is_empty());
    assert!(memory::add_triple(&db, &t(Some("ws"), "agent:Ada", "edited", "file:a.rs", 200)).unwrap().is_none());
    // the same fact in another workspace, or without one, is another fact
    assert!(memory::add_triple(&db, &t(Some("other"), "agent:Ada", "edited", "file:a.rs", 100)).unwrap().is_some());
    assert!(memory::add_triple(&db, &t(None, "agent:Ada", "edited", "file:a.rs", 100)).unwrap().is_some());
    assert!(memory::add_triple(&db, &t(None, "agent:Ada", "edited", "file:a.rs", 100)).unwrap().is_none(), "NULL workspaces dedupe too");
    // zero timestamps become "now"
    let z = memory::add_triple(&db, &t(Some("ws"), "agent:Ada", "edited", "file:b.rs", 0)).unwrap().unwrap();
    assert!(z.valid_from > 0);
}

#[test]
fn many_valued_predicates_keep_every_object() {
    let db = db();
    for f in ["a.rs", "b.rs", "c.rs"] {
        memory::add_triple(&db, &t(Some("ws"), "agent:Ada", "edited", &format!("file:{f}"), 10)).unwrap().unwrap();
    }
    assert_eq!(live(&db, Some("ws"), Some("agent:Ada"), Some("edited")).len(), 3);
}

#[test]
fn single_valued_predicates_close_the_previous_value_at_the_same_instant() {
    let db = db();
    let ws = Some("ws");
    memory::add_triple(&db, &t(ws, "task:t1", "status", "ready", 100)).unwrap().unwrap();
    memory::add_triple(&db, &t(ws, "task:t1", "status", "running", 200)).unwrap().unwrap();
    memory::add_triple(&db, &t(ws, "task:t1", "status", "done", 300)).unwrap().unwrap();
    // live: only the last
    assert_eq!(live(&db, ws, Some("task:t1"), Some("status")), [("task:t1".into(), "status".into(), "done".into())]);
    // history: contiguous half-open intervals
    let h = memory::triple_history(&db, ws, "task:t1", "status", 10).unwrap();
    let spans: Vec<_> = h.iter().map(|x| (x.object.as_str(), x.valid_from, x.valid_to)).collect();
    assert_eq!(spans, [("ready", 100, Some(200)), ("running", 200, Some(300)), ("done", 300, None)]);
    // as-of queries: the boundary instant belongs to the successor
    let at = |ms| {
        memory::triples_as_of(&db, ws, Some("task:t1"), Some("status"), ms, 10).unwrap().into_iter().map(|x| x.object).collect::<Vec<_>>()
    };
    assert_eq!(at(99), Vec::<String>::new());
    assert_eq!(at(100), ["ready"]);
    assert_eq!(at(199), ["ready"]);
    assert_eq!(at(200), ["running"]);
    assert_eq!(at(10_000), ["done"]);
    // other subjects and workspaces are untouched
    memory::add_triple(&db, &t(ws, "task:t2", "status", "running", 250)).unwrap().unwrap();
    memory::add_triple(&db, &t(Some("other"), "task:t1", "status", "failed", 250)).unwrap().unwrap();
    assert_eq!(live(&db, ws, Some("task:t1"), Some("status")).len(), 1);
    assert_eq!(live(&db, ws, None, Some("status")).len(), 2);
    // asserting the current value again changes nothing; re-asserting an old one reopens it
    assert!(memory::add_triple(&db, &t(ws, "task:t1", "status", "done", 400)).unwrap().is_none());
    memory::add_triple(&db, &t(ws, "task:t1", "status", "running", 500)).unwrap().unwrap();
    let h = memory::triple_history(&db, ws, "task:t1", "status", 10).unwrap();
    assert_eq!(h.len(), 4);
    assert_eq!(h[2].valid_to, Some(500));
}

#[test]
fn reassignment_is_a_state_change_not_a_second_owner() {
    let db = db();
    memory::add_triple(&db, &t(Some("ws"), "task:t1", "assigned_to", "agent:Ada", 1)).unwrap().unwrap();
    memory::add_triple(&db, &t(Some("ws"), "task:t1", "Assigned To", "agent:Bo", 2)).unwrap().unwrap();
    assert_eq!(live(&db, Some("ws"), Some("task:t1"), Some("assigned_to")), [("task:t1".into(), "assigned_to".into(), "agent:Bo".into())]);
    assert!(memory::is_single_valued("owned_by") && memory::is_single_valued("status") && !memory::is_single_valued("depends_on"));
}

#[test]
fn entity_queries_match_subject_object_and_untyped_names() {
    let db = db();
    let ws = Some("ws");
    memory::add_triple(&db, &t(ws, "agent:Ada", "edited", "file:src/a.rs", 10)).unwrap();
    memory::add_triple(&db, &t(ws, "task:t1", "touches", "file:src/a.rs", 20)).unwrap();
    memory::add_triple(&db, &t(ws, "task:t2", "depends_on", "task:t1", 30)).unwrap();
    memory::add_triple(&db, &t(ws, "agent:Bo", "edited", "file:src/b.rs", 40)).unwrap();
    memory::add_triple(&db, &t(None, "concept:retry", "related_to", "file:src/a.rs", 50)).unwrap();

    assert_eq!(live(&db, ws, Some("file:src/a.rs"), None).len(), 3, "as object, and a workspace-less fact");
    assert_eq!(live(&db, ws, Some("src/a.rs"), None).len(), 3, "an untyped name finds the typed entity");
    assert_eq!(live(&db, ws, Some("task:t1"), None).len(), 2, "as subject and as object");
    assert_eq!(live(&db, ws, Some("Ada"), Some("edited")).len(), 1);
    assert_eq!(live(&db, ws, Some("task:t1"), Some("depends on")).len(), 1, "predicate filters are normalized");
    assert!(live(&db, Some("ws2"), Some("task:t1"), None).is_empty());
    assert_eq!(live(&db, None, None, None).len(), 1, "no workspace: only workspace-less facts");

    let newest_first = memory::query_triples(&db, ws, None, None, 100).unwrap();
    assert!(newest_first.windows(2).all(|w| w[0].valid_from >= w[1].valid_from));
    assert_eq!(memory::query_triples(&db, ws, None, None, 2).unwrap().len(), 2);
}

#[test]
fn invalid_triples_are_rejected_and_confidence_is_clamped() {
    let db = db();
    assert!(memory::add_triple(&db, &t(None, "", "p", "o", 1)).is_err());
    assert!(memory::add_triple(&db, &t(None, "s", "  ", "o", 1)).is_err());
    assert!(memory::add_triple(&db, &t(None, "s", "p", "", 1)).is_err());
    assert!(memory::add_triple(&db, &t(None, "s", "p", &"x".repeat(1000), 1)).is_err());
    let mut hi = t(None, "s", "p", "o", 1);
    hi.confidence = 7.0;
    assert_eq!(memory::add_triple(&db, &hi).unwrap().unwrap().confidence, 1.0);
}

#[test]
fn add_triple_nests_inside_the_callers_transaction() {
    let mut db = db();
    {
        let tx = db.transaction().unwrap();
        memory::add_triple(&tx, &t(Some("ws"), "task:t1", "status", "ready", 1)).unwrap().unwrap();
        memory::add_triple(&tx, &t(Some("ws"), "task:t1", "status", "running", 2)).unwrap().unwrap();
        // a failing call inside the transaction undoes only itself
        assert!(memory::add_triple(&tx, &t(Some("ws"), "task:t1", "", "x", 3)).is_err());
        tx.rollback().unwrap();
    }
    assert!(live(&db, Some("ws"), None, None).is_empty(), "the caller's rollback takes everything");
    memory::add_triple(&db, &t(Some("ws"), "task:t1", "status", "ready", 1)).unwrap().unwrap();
    assert_eq!(live(&db, Some("ws"), None, None).len(), 1);
}

// ───────────────────────────── derive_triples ─────────────────────────────

fn ev(kind: EventKind) -> EventRow {
    EventRow { id: 42, ts: 1_700_000_000_000, agent_id: Some("agent-id-1".into()), task_id: Some("task-id-1".into()), run_id: None, kind }
}

fn task(key: &str, status: TaskStatus, assignee: Option<&str>, deps: &[&str]) -> Task {
    Task {
        id: format!("id-{key}"),
        plan_id: Some("plan".into()),
        key: key.into(),
        title: "title".into(),
        goal: "goal".into(),
        acceptance: vec![],
        write_scope: vec![],
        verify: vec![],
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        status,
        assignee: assignee.map(String::from),
        agent_hint: None,
        account_hint: None,
        consultant: None,
        fresh_session: false,
        branch: None,
        worktree: None,
        attempt: 1,
        status_reason: None,
        summary: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn facts(v: &[Triple]) -> Vec<String> {
    let mut f: Vec<String> = v.iter().map(|t| format!("{} {} {}", t.subject, t.predicate, t.object)).collect();
    f.sort();
    f
}

fn tool_started(activity: ActivityKind, path: Option<&str>, paths: &[&str]) -> EventKind {
    EventKind::ToolStarted {
        call_id: "c1".into(),
        name: "Edit".into(),
        activity,
        target: Target { path: path.map(String::from), paths: paths.iter().map(|p| p.to_string()).collect(), ..Target::default() },
        input: serde_json::Value::Null,
        confidence: "observed".into(),
    }
}

#[test]
fn edits_creates_and_deletes_become_edited_facts() {
    for activity in [ActivityKind::Edit, ActivityKind::Create, ActivityKind::Delete] {
        let e = ev(tool_started(activity, Some("src/a.rs"), &["./src/b.rs"]));
        let f = facts(&derive_triples(Some("ws"), &e, Some("Ada"), Some("t1")));
        assert_eq!(
            f,
            [
                "agent:Ada edited file:src/a.rs",
                "agent:Ada edited file:src/b.rs",
                "task:t1 touches file:src/a.rs",
                "task:t1 touches file:src/b.rs"
            ],
            "{activity:?}"
        );
    }
    // reading is not a fact worth keeping
    assert!(derive_triples(None, &ev(tool_started(ActivityKind::Read, Some("src/a.rs"), &[])), Some("Ada"), Some("t1")).is_empty());
    assert!(derive_triples(None, &ev(tool_started(ActivityKind::Execute, None, &[])), Some("Ada"), Some("t1")).is_empty());
}

#[test]
fn derived_triples_carry_provenance() {
    let e = ev(EventKind::FilesChanged { call_id: None, paths: vec!["x.rs".into()], source: "git".into() });
    let v = derive_triples(Some("ws"), &e, Some("Ada"), None);
    assert!(!v.is_empty());
    for t in &v {
        assert_eq!(t.workspace_id.as_deref(), Some("ws"));
        assert_eq!(t.source, "observed");
        assert_eq!(t.source_event, Some(42));
        assert_eq!(t.valid_from, 1_700_000_000_000);
        assert_eq!(t.valid_to, None);
        assert_eq!(t.confidence, 1.0);
    }
    // ephemeral events (id 0) have no source event
    let mut e0 = e.clone();
    e0.id = 0;
    assert!(derive_triples(None, &e0, Some("Ada"), None).iter().all(|t| t.source_event.is_none()));
}

#[test]
fn files_changed_commits_and_missing_names_fall_back_to_ids() {
    let e = ev(EventKind::Committed {
        sha: "abc".into(),
        files: vec!["a.rs".into(), ".git/x".into(), "".into(), "b.rs".into()],
        message: "m".into(),
    });
    assert_eq!(
        facts(&derive_triples(None, &e, None, None)),
        [
            "agent:agent-id-1 edited file:a.rs",
            "agent:agent-id-1 edited file:b.rs",
            "task:task-id-1 touches file:a.rs",
            "task:task-id-1 touches file:b.rs"
        ]
    );
    let mut anon = e.clone();
    anon.agent_id = None;
    assert_eq!(facts(&derive_triples(None, &anon, None, Some("t9"))), ["task:t9 touches file:a.rs", "task:t9 touches file:b.rs"]);
    let many = ev(EventKind::FilesChanged { call_id: None, paths: (0..500).map(|i| format!("f{i}.rs")).collect(), source: "git".into() });
    assert_eq!(derive_triples(None, &many, Some("A"), None).len(), 200, "100 paths, an agent fact and a task fact each");
}

#[test]
fn task_changes_yield_status_assignee_and_dependencies() {
    let tk = task("t2", TaskStatus::Running, Some("agent-id-1"), &["id-t1"]);
    let e = ev(EventKind::TaskChanged { task: tk.clone() });
    // without a resolver: the event's own agent is named, dependency ids stay ids
    let f = facts(&derive_triples(None, &e, Some("Ada"), Some("t2")));
    assert_eq!(f, ["task:t2 assigned_to agent:Ada", "task:t2 depends_on task:id-t1", "task:t2 status running"]);
    // with a resolver the dependency is readable
    let resolve = |id: &str| (id == "id-t1").then(|| "t1".to_string());
    let f = facts(&derive_triples_with(None, &e, Some("Ada"), Some("t2"), &resolve));
    assert!(f.contains(&"task:t2 depends_on task:t1".to_string()), "{f:?}");
    // no key passed: the task's own key is used
    let f = facts(&derive_triples(None, &ev(EventKind::TaskChanged { task: task("t7", TaskStatus::Done, None, &[]) }), None, None));
    assert_eq!(f, ["task:t7 status done"]);
}

#[test]
fn merge_verify_and_retrieval_facts() {
    let merged =
        derive_triples(None, &ev(EventKind::Merged { sha: "s".into(), branch: "codly/p1/integration".into() }), Some("Ada"), Some("t1"));
    assert_eq!(facts(&merged), ["task:t1 merged_into branch:codly/p1/integration"]);

    let ok = derive_triples(
        None,
        &ev(EventKind::Verified { command: "cargo   test -p core".into(), ok: true, exit_code: Some(0), tail: String::new() }),
        None,
        Some("t1"),
    );
    assert_eq!(facts(&ok), ["task:t1 verified_by cmd:cargo test -p core"]);
    let failed = derive_triples(
        None,
        &ev(EventKind::Verified { command: "cargo test".into(), ok: false, exit_code: Some(1), tail: String::new() }),
        None,
        Some("t1"),
    );
    assert!(failed.is_empty());

    let cit = |p: &str| Citation { id: "C1".into(), path: p.into(), start_line: 1, end_line: 9, symbol: None, why: vec![], score: 1.0 };
    let r = derive_triples(
        None,
        &ev(EventKind::Retrieval {
            query: "q".into(),
            confidence: Confidence::High,
            citations: vec![cit("a.rs"), cit("a.rs"), cit("b.rs")],
        }),
        Some("Ada"),
        Some("t1"),
    );
    assert_eq!(facts(&r), ["task:t1 relevant_file file:a.rs", "task:t1 relevant_file file:b.rs"]);
    assert!(r.iter().all(|t| t.confidence == 0.5), "retrieval is a heuristic: 0.5");
    let low = derive_triples(
        None,
        &ev(EventKind::Retrieval { query: "q".into(), confidence: Confidence::Low, citations: vec![cit("a.rs")] }),
        Some("Ada"),
        Some("t1"),
    );
    assert!(low.is_empty(), "a retrieval the index did not trust says nothing");
}

#[test]
fn unrelated_events_derive_nothing() {
    for k in [
        EventKind::TurnStarted { input: None },
        EventKind::Message { item_id: "i".into(), text: "hello".into(), phase: MessagePhase::Final },
        EventKind::Error { message: "x".into(), fatal: false },
        EventKind::Exited { code: Some(0) },
    ] {
        assert!(derive_triples(Some("ws"), &ev(k), Some("Ada"), Some("t1")).is_empty());
    }
}

#[test]
fn replaying_events_builds_the_expected_graph() {
    let db = db();
    let ws = Some("ws");
    let mut events = vec![
        ev(EventKind::TaskChanged { task: task("t1", TaskStatus::Ready, None, &[]) }),
        ev(EventKind::TaskChanged { task: task("t1", TaskStatus::Running, Some("agent-id-1"), &[]) }),
        ev(tool_started(ActivityKind::Edit, Some("src/a.rs"), &[])),
        ev(EventKind::FilesChanged { call_id: None, paths: vec!["src/a.rs".into()], source: "provider".into() }),
        ev(EventKind::Verified { command: "cargo test".into(), ok: true, exit_code: Some(0), tail: String::new() }),
        ev(EventKind::TaskChanged { task: task("t1", TaskStatus::Done, Some("agent-id-2"), &[]) }),
        ev(EventKind::Merged { sha: "s".into(), branch: "main".into() }),
    ];
    for (i, e) in events.iter_mut().enumerate() {
        e.id = i as i64 + 1;
        e.ts = 1000 * (i as i64 + 1);
    }
    let mut added = 0;
    for e in &events {
        let agent = if e.id == 6 { "Bo" } else { "Ada" };
        for tr in derive_triples(ws, e, Some(agent), Some("t1")) {
            if memory::add_triple(&db, &tr).unwrap().is_some() {
                added += 1;
            }
        }
    }
    assert_eq!(live(&db, ws, Some("task:t1"), Some("status")), [("task:t1".into(), "status".into(), "done".into())]);
    assert_eq!(live(&db, ws, Some("task:t1"), Some("assigned_to")), [("task:t1".into(), "assigned_to".into(), "agent:agent-id-2".into())]);
    assert_eq!(live(&db, ws, Some("file:src/a.rs"), None).len(), 2, "Ada edited it, t1 touched it; the duplicate event added nothing");
    assert_eq!(memory::triple_history(&db, ws, "task:t1", "status", 10).unwrap().len(), 3);
    assert!(added >= 8, "{added}");
    // what t1's status was while it ran
    let running = memory::triples_as_of(&db, ws, Some("task:t1"), Some("status"), 2500, 5).unwrap();
    assert_eq!(running[0].object, "running");
}
