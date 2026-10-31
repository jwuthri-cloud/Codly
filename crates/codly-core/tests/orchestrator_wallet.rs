//! The wallet through the Core: the developer panel's cheats are refused until Developer mode
//! is on, and once used they flag the wallet for good.

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

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    repo
}

/// A worker with `xp`, in a repo of its own (another office: the wallet counts it too).
fn seed_worker(data: &Path, xp: i64) -> Id {
    let store = Store::open(&data.join("codly.sqlite")).unwrap();
    let ws = Workspace {
        id: new_id(),
        repo_path: "/elsewhere/other".into(),
        name: "other".into(),
        default_branch: "main".into(),
        current_branch: None,
        head: None,
        dirty: false,
        created_at: now_ms(),
    };
    store.upsert_workspace(&ws).unwrap();
    let id = new_id();
    let agent = Agent {
        id: id.clone(),
        name: "Ada".into(),
        provider: Provider::Claude,
        model: None,
        role: Role::Worker,
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
        look_seed: id.clone(),
        thread: None,
        member: None,
        title: None,
        consultant: false,
        kept: false,
        away_until: None,
        tired_since: None,
        archived: false,
        usage: Usage::default(),
        career: Career { xp, ..Default::default() },
        created_at: now_ms(),
        updated_at: now_ms(),
    };
    let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind: EventKind::AgentChanged { agent } };
    store.append_event(Some(&ws.id), &row).unwrap();
    id
}

async fn start(root: &Path) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: None, env: Some(std::env::vars().collect()), clis: Some(vec![]), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cheats_need_developer_mode_and_flag_the_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let ada = seed_worker(&dir.path().join("data"), 40);
    let core = start(dir.path()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let w = core.wallet().unwrap();
    assert_eq!((w.earned, w.balance, w.cheated), (40, 40, false), "another repo's XP is in the wallet");
    let piece = Placed { uid: "s1".into(), item: "kenney.loungeDesignSofa".into(), x: 1.0, y: 0.0, z: 5.0, rot: 0.0 };
    assert!(core.place(piece.clone()).unwrap_err().to_string().contains("not enough coins"));

    // Off by default: every cheat is refused.
    assert!(!core.settings().developer_mode);
    for err in [core.dev_grant(1000).map(|_| ()), core.dev_give_xp(&ada, 250).map(|_| ()), core.dev_reset_office().map(|_| ())] {
        assert!(err.unwrap_err().to_string().contains("Developer mode"));
    }
    assert!(!core.wallet().unwrap().cheated);

    core.update_settings(Settings { developer_mode: true, ..core.settings() }).await.unwrap();
    let w = core.dev_grant(1000).unwrap();
    assert_eq!((w.granted, w.balance, w.cheated), (1000, 1040, true));
    assert!(core.dev_grant(0).is_err());
    let shop = core.place(piece).unwrap();
    assert_eq!(shop.placed.len(), 1);
    let a = core.dev_give_xp(&ada, 250).unwrap();
    assert_eq!((a.career.xp, a.career.rank), (290, Rank::Senior), "the rank follows");
    assert_eq!(a.career.milestones.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Cheat: +250 XP", "Promoted to Senior"]);
    let shop = core.dev_reset_office().unwrap();
    assert!(shop.placed.is_empty());
    assert_eq!((shop.wallet.spent, shop.wallet.balance), (0, 1290));

    // Developer mode off again: still flagged.
    core.update_settings(Settings { developer_mode: false, ..core.settings() }).await.unwrap();
    assert!(core.dev_grant(1000).is_err());
    assert!(core.wallet().unwrap().cheated);
    core.shutdown().await;
}
