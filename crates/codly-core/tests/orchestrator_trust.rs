//! Trust is a safety setting, not a game reward: you set it per agent whatever its rank, XP
//! never changes it, only an agent that writes on its own can be trusted, and every change is
//! a decision on the agent.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use codly_core::career::trusts;
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, StartOptions};
use codly_core::store::Store;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn temp_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    // As `open_workspace` keeps it, so the seeded workspace is the one it opens.
    std::fs::canonicalize(&repo).unwrap()
}

fn workspace(store: &Store, repo: &Path) -> Workspace {
    let name = repo.file_name().unwrap().to_string_lossy().into_owned();
    let ws = Workspace {
        id: new_id(),
        repo_path: repo.to_string_lossy().into_owned(),
        name,
        default_branch: "main".into(),
        current_branch: None,
        head: None,
        dirty: false,
        created_at: now_ms(),
    };
    store.upsert_workspace(&ws).unwrap();
    ws
}

/// A new hire: an Intern with no XP and no trust.
fn agent(name: &str, role: Role) -> Agent {
    let id = new_id();
    Agent {
        id: id.clone(),
        name: name.into(),
        provider: Provider::Claude,
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
        color: 3,
        look_seed: format!("{id}#hair=2"),
        thread: None,
        member: None,
        title: None,
        consultant: false,
        // Yours between tasks (a finished consultant would be let go at start).
        kept: true,
        away_until: None,
        away_since: None,
        tired_since: None,
        permission_mode: None,
        archived: false,
        usage: Usage::default(),
        career: Career::default(),
        created_at: now_ms(),
        updated_at: now_ms(),
    }
}

fn put(store: &Store, ws: &Workspace, a: &Agent) {
    let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind: EventKind::AgentChanged { agent: a.clone() } };
    store.append_event(Some(&ws.id), &row).unwrap();
}

async fn start(root: &Path) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: None, env: Some(std::env::vars().collect()), clis: Some(vec![]), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

/// The career the core has saved for an agent of the open office.
async fn saved(core: &Core, id: &str) -> Career {
    core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == id).unwrap().career
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn you_set_trust_whatever_the_rank_and_every_change_is_a_decision() {
    let dir = tempfile::tempdir().unwrap();
    let here = temp_repo(dir.path(), "here");
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    let ws = workspace(&store, &here);
    let ana = agent("Ana", Role::Worker);
    let sol = agent("Sol", Role::Solo);
    let lead = Agent { thread: Some("lead".into()), ..agent("Lead", Role::Lead) };
    let rae = agent("Rae", Role::Reviewer);
    for x in [&ana, &sol, &lead, &rae] {
        put(&store, &ws, x);
    }
    drop(store);

    let core = start(dir.path()).await;
    core.open_workspace(here.to_str().unwrap()).await.unwrap();
    assert_eq!(saved(&core, &ana.id).await.trust, Rank::Intern, "the default is no extra trust");

    // An Intern, trusted all the way: its rank is no cap.
    let a = core.trust_agent(&ana.id, Rank::Staff).unwrap();
    assert_eq!((a.career.rank, a.career.trust), (Rank::Intern, Rank::Staff));
    assert!(trusts(&saved(&core, &ana.id).await, Rank::Staff));
    // The same level again changes nothing (and records nothing, see below).
    assert_eq!(core.trust_agent(&ana.id, Rank::Staff).unwrap().career.trust, Rank::Staff);
    // Lower it, then take it all back; a 1:1 agent can be trusted too.
    assert_eq!(core.trust_agent(&ana.id, Rank::Junior).unwrap().career.trust, Rank::Junior);
    assert_eq!(core.trust_agent(&ana.id, Rank::Intern).unwrap().career.trust, Rank::Intern);
    assert_eq!(core.trust_agent(&sol.id, Rank::Senior).unwrap().career.trust, Rank::Senior);

    // The developer panel's cheat promotes Ana to Staff: its trust stays where you left it.
    core.update_settings(Settings { developer_mode: true, ..core.settings() }).await.unwrap();
    let cheated = core.dev_give_xp(&ana.id, 5_000).unwrap();
    assert_eq!((cheated.career.rank, cheated.career.trust), (Rank::Staff, Rank::Intern));
    assert!(!trusts(&saved(&core, &ana.id).await, Rank::Junior));

    // Refused: the Lead and a reviewer (they always ask), and an agent that does not exist.
    let no = |r: anyhow::Result<Agent>| r.unwrap_err().to_string();
    assert!(no(core.trust_agent(&lead.id, Rank::Junior)).contains("only a worker or a 1:1 agent can be trusted"));
    assert!(no(core.trust_agent(&rae.id, Rank::Staff)).contains("Rae always asks"));
    assert!(no(core.trust_agent("nobody", Rank::Junior)).contains("unknown agent"));
    assert_eq!(saved(&core, &rae.id).await.trust, Rank::Intern);
    // Taking trust back is never refused.
    assert_eq!(core.trust_agent(&lead.id, Rank::Intern).unwrap().career.trust, Rank::Intern);

    // Each change is a decision by you, on the agent, that says what it may do from now on.
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    let mut events = store.events(Some(&ws.id), None, None, 500).unwrap();
    events.sort_by_key(|e| e.id);
    let changes: Vec<_> = events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type, method, summary, detail, .. } if decision_type == "trust" => Some((e.agent_id, method, summary, detail)),
            _ => None,
        })
        .collect();
    let by_you = |a: &Agent, summary: &str, from: &str, to: &str| {
        (Some(a.id.clone()), "human".to_string(), summary.to_string(), serde_json::json!({ "agent": a.id, "name": a.name, "from": from, "to": to }))
    };
    assert_eq!(
        changes,
        [
            by_you(&ana, "you trust Ana to work inside its own worktree without asking, and to merge without the cross-vendor review", "intern", "staff"),
            by_you(&ana, "you trust Ana to edit files inside its own worktree without asking", "staff", "junior"),
            by_you(&ana, "you took back your trust in Ana: it asks like every other agent", "junior", "intern"),
            by_you(&sol, "you trust Sol to work inside its own worktree without asking", "intern", "senior"),
        ]
    );
    core.shutdown().await;
}
