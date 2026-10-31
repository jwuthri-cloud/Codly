//! Triggers through the Core facade: the config persists across a restart, the UI cannot move
//! the clocks Codly owns, and the network gate follows the saved mode.

use std::path::PathBuf;

use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, StartOptions};
use codly_core::triggers::*;

async fn start(dir: &std::path::Path) -> std::sync::Arc<Core> {
    let cfg = CoreConfig { data_dir: dir.to_path_buf(), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: None, env: Some(std::env::vars().collect()), clis: Some(Vec::<CliInfo>::new()), index: false };
    Core::start_with(cfg, opts).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn triggers_persist_and_gate_follows_the_mode() {
    let dir = tempfile::tempdir().unwrap();
    let core = start(dir.path()).await;

    let fresh = core.triggers().await.unwrap();
    assert!(fresh.schedules.is_empty());
    assert_eq!(core.network_gate(NetworkAction::Read, "ana").await, Gate::Deny);

    let mut cfg = fresh;
    cfg.schedules.push(Schedule { name: "Standup".into(), prompt: "review every agent".into(), every_minutes: 60, last_fired_at: Some(1), ..Default::default() });
    cfg.network.mode = NetworkMode::Converse;
    let saved = core.set_triggers(cfg).await.unwrap();
    assert!(!saved.schedules[0].id.is_empty());
    assert_eq!(saved.schedules[0].last_fired_at, None, "the UI cannot set a clock");
    assert_eq!(core.network_gate(NetworkAction::Message, "ana").await, Gate::Allow);
    assert_eq!(core.network_gate(NetworkAction::Delegate, "ana").await, Gate::Deny);
    core.shutdown().await;

    let again = start(dir.path()).await;
    let loaded = again.triggers().await.unwrap();
    assert_eq!(loaded.schedules, saved.schedules);
    assert_eq!(loaded.network.mode, NetworkMode::Converse);
    again.shutdown().await;
}
