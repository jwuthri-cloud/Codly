//! Transfers between offices (PROGRESSION.md §4): an agent moves to another repo you opened
//! before with its career, and leaves trust, friends, mentor and its session behind.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

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
        // Workers you keep between tasks (a finished consultant would be let go at start).
        kept: true,
        away_until: None,
        away_since: None,
        tired_since: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transfer_keeps_the_career_and_leaves_the_repo_behind() {
    let dir = tempfile::tempdir().unwrap();
    let (here, there) = (temp_repo(dir.path(), "here"), temp_repo(dir.path(), "there"));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    // "here" opened last: the core reopens it at start, as `open_workspace` does below.
    let b = workspace(&store, &there);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let a = workspace(&store, &here);

    // Dee: a Senior with a record, trusted, friends with Bo, mentored by Bo, in a conversation.
    let bo = agent("Bo", Role::Worker);
    let mut dee = agent("Dee", Role::Worker);
    dee.session_id = Some("sess-1".into());
    dee.worktree = Some(here.join("wt").to_string_lossy().into_owned());
    dee.career = Career {
        xp: 300,
        rank: Rank::Senior,
        traits: vec!["perfectionist".into()],
        milestones: vec![Milestone { at: 1, text: "First merge".into() }],
        skills: [("Rust".to_string(), 40)].into(),
        trust: Rank::Senior,
        friends: [(bo.id.clone(), 3)].into(),
        mentor: Some(bo.id.clone()),
        ..Default::default()
    };
    let lead = Agent { thread: Some("lead".into()), ..agent("Lead", Role::Lead) };
    let busy = agent("Cy", Role::Worker);
    let clash = agent("Eve", Role::Solo);
    for x in [&bo, &dee, &lead, &busy, &clash] {
        put(&store, &a, x);
    }
    put(&store, &b, &agent("eve", Role::Worker));
    drop(store);

    let core = start(dir.path()).await;
    core.open_workspace(here.to_str().unwrap()).await.unwrap();

    // The picker: the other office, with its crew; never the open one.
    let offices = core.offices().unwrap();
    assert_eq!(offices.iter().map(|o| (o.id.as_str(), o.name.as_str(), o.crew)).collect::<Vec<_>>(), [(b.id.as_str(), "there", 1)]);

    // Refused: the Lead, an agent on a task, a name taken there, the same office, an unknown one.
    let no = |r: anyhow::Result<Agent>| r.unwrap_err().to_string();
    assert!(no(core.transfer_agent(&lead.id, &b.id).await).contains("Lead"));
    // Cy has a card on the board.
    let card = core.create_task("Fix the build", "make it green").await.unwrap();
    core.assign_task(&card.id, Some(&busy.id)).await.unwrap();
    let e = no(core.transfer_agent(&busy.id, &b.id).await);
    assert!(e.contains("mid-task"), "{e}");
    assert!(no(core.transfer_agent(&clash.id, &b.id).await).contains("already works in there"));
    assert!(no(core.transfer_agent(&dee.id, &a.id).await).contains("already works here"));
    assert!(no(core.transfer_agent(&dee.id, "nowhere").await).contains("no such office"));

    let before = core.snapshot().await.unwrap().agents.into_iter().find(|x| x.id == dee.id).unwrap();
    let moved = core.transfer_agent(&dee.id, &b.id).await.unwrap();
    // Kept: id, XP, rank, skills, traits, look, milestones (plus the move).
    assert_eq!(moved.id, dee.id);
    assert_eq!((moved.career.xp, moved.career.rank), (300, Rank::Senior));
    assert_eq!((&moved.career.traits, &moved.career.skills, &moved.look_seed), (&before.career.traits, &before.career.skills, &before.look_seed));
    assert_eq!(moved.career.stamina, before.career.stamina);
    let texts: Vec<&str> = moved.career.milestones.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts.first(), Some(&"First merge"));
    assert_eq!(texts.last(), Some(&"Moved to there"));
    // Left behind: trust, friends, mentor, the session and the worktree.
    assert_eq!(moved.career.trust, Rank::Intern);
    assert!(moved.career.friends.is_empty() && moved.career.mentor.is_none());
    assert_eq!((moved.session_id.as_deref(), moved.worktree.as_deref()), (None, None));

    // It left this office and works in the other one.
    assert!(!core.snapshot().await.unwrap().agents.iter().any(|x| x.id == dee.id));
    let offices = core.offices().unwrap();
    assert_eq!(offices[0].crew, 2);
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    assert_eq!(store.agent_workspace(&dee.id).unwrap().as_deref(), Some(b.id.as_str()));
    assert!(store.agents(&b.id).unwrap().iter().any(|x| x.id == dee.id && x.session_id.is_none()));
    // Both offices record it.
    let moves = |ws: &str| {
        store
            .events(Some(ws), None, None, 500)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.kind {
                EventKind::Decision { decision_type, summary, .. } if decision_type == "transfer" => Some(summary),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(moves(&a.id), ["Dee moved to there"]);
    assert_eq!(moves(&b.id), ["Dee moved in from here"]);

    // It's gone from here: transferring it again from this office is refused.
    assert!(no(core.transfer_agent(&dee.id, &b.id).await).contains("doesn't work in this office"));
    core.shutdown().await;
}

/// The Neighbourhood map walks agents at home down the street: every other office lists who is
/// at home now (never the Lead, never one back already), with when it left and when it's back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offices_list_who_is_at_home() {
    let dir = tempfile::tempdir().unwrap();
    let (here, there) = (temp_repo(dir.path(), "here"), temp_repo(dir.path(), "there"));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    let b = workspace(&store, &there);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let a = workspace(&store, &here);
    let now = now_ms();
    let ada = Agent { away_until: Some(now + 600_000), away_since: Some(now - 1_000), ..agent("Ada", Role::Worker) };
    let back = Agent { away_until: Some(now - 1_000), ..agent("Bo", Role::Worker) };
    put(&store, &b, &ada);
    put(&store, &b, &back);
    put(&store, &b, &agent("Cy", Role::Worker));
    let dee = agent("Dee", Role::Worker);
    put(&store, &a, &dee);
    drop(store);

    let core = start(dir.path()).await;
    core.open_workspace(here.to_str().unwrap()).await.unwrap();
    let offices = core.offices().unwrap();
    assert_eq!(offices.len(), 1);
    assert_eq!(offices[0].home, [AwayAgent { id: ada.id.clone(), name: "Ada".into(), color: 3, since: Some(now - 1_000), until: now + 600_000 }]);

    // Sent home here: it left now; a longer stay keeps when it left; back home clears it.
    let gone = core.set_agent_home(&dee.id, Some(now + 300_000)).unwrap();
    let since = gone.away_since.expect("left now");
    assert!(since >= now);
    let longer = core.set_agent_home(&dee.id, Some(now + 900_000)).unwrap();
    assert_eq!(longer.away_since, Some(since));
    let in_again = core.set_agent_home(&dee.id, None).unwrap();
    assert_eq!((in_again.away_until, in_again.away_since), (None, None));
    core.shutdown().await;
}

#[tokio::test]
async fn a_break_together_makes_friends_leads_aside() {
    let dir = tempfile::tempdir().unwrap();
    let here = temp_repo(dir.path(), "here");
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    let ws = workspace(&store, &here);
    let (ada, bo, lead) = (agent("Ada", Role::Worker), agent("Bo", Role::Worker), agent("Lea", Role::Lead));
    for a in [&ada, &bo, &lead] {
        put(&store, &ws, a);
    }
    drop(store);

    let core = start(dir.path()).await;
    core.open_workspace(here.to_str().unwrap()).await.unwrap();
    core.note_hangout(&ada.id, &bo.id);
    core.note_hangout(&bo.id, &ada.id);
    core.note_hangout(&ada.id, &lead.id);
    let agents = core.snapshot().await.unwrap().agents;
    let friends = |id: &str| agents.iter().find(|a| a.id == id).unwrap().career.friends.clone();
    assert_eq!(friends(&ada.id).get(&bo.id), Some(&2));
    assert_eq!(friends(&bo.id).get(&ada.id), Some(&2));
    assert!(!friends(&ada.id).contains_key(&lead.id), "the Lead keeps out of it");
}

/// "Let go…": an agent you hired becomes a ghost you can bring back; never mid-task, never the Lead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn letting_an_agent_go_makes_a_ghost_you_can_bring_back() {
    let dir = tempfile::tempdir().unwrap();
    let here = temp_repo(dir.path(), "here");
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let store = Store::open(&dir.path().join("data").join("codly.sqlite")).unwrap();
    let ws = workspace(&store, &here);
    let (hana, cy) = (agent("Hana", Role::Solo), agent("Cy", Role::Worker));
    let lead = Agent { thread: Some("lead".into()), ..agent("Lead", Role::Lead) };
    for a in [&hana, &cy, &lead] {
        put(&store, &ws, a);
    }
    drop(store);

    let core = start(dir.path()).await;
    core.open_workspace(here.to_str().unwrap()).await.unwrap();
    let no = |r: anyhow::Result<Agent>| r.unwrap_err().to_string();
    assert!(no(core.let_go(&lead.id).await).contains("Lead"));
    let card = core.create_task("Fix the build", "make it green").await.unwrap();
    core.assign_task(&card.id, Some(&cy.id)).await.unwrap();
    assert!(no(core.let_go(&cy.id).await).contains("is on"));

    let gone = core.let_go(&hana.id).await.unwrap();
    assert!(gone.archived);
    assert!(!core.snapshot().await.unwrap().agents.iter().any(|a| a.id == hana.id));
    assert!(core.ghosts().unwrap().iter().any(|g| g.id == hana.id));
    assert!(no(core.let_go(&hana.id).await).contains("doesn't work in this office"));

    let back = core.bring_back(&hana.id).unwrap();
    assert!(!back.archived && back.role == Role::Solo);
    core.shutdown().await;
}
