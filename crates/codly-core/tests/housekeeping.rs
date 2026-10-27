//! Housekeeping through the real Core: the board sweep (archive file, search, thresholds from
//! the settings) and the default MCP servers handed to a new agent at spawn.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::housekeeping::archive::{ArchivePolicy, DAY_MS};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::mpsc;

/// Records every spec it is asked to start; the agent does nothing.
struct Recorder(Arc<Mutex<Vec<SpawnSpec>>>);

impl Spawner for Recorder {
    fn spawn(&self, spec: SpawnSpec, _events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
        self.0.lock().unwrap().push(spec);
        Box::pin(async move { Ok(handle) })
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

async fn start() -> (tempfile::TempDir, Arc<Core>, Arc<Mutex<Vec<SpawnSpec>>>) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "init"]);
    let claude = CliInfo {
        provider: Provider::Claude,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
    };
    let specs = Arc::new(Mutex::new(vec![]));
    let opts = StartOptions { spawner: Some(Arc::new(Recorder(specs.clone()))), env: Some(std::env::vars().collect()), clis: Some(vec![claude]), index: false };
    let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") }, opts).await.unwrap();
    core.open_workspace(&repo.to_string_lossy()).await.unwrap();
    (root, core, specs)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_task_is_archived_after_the_threshold_and_stays_searchable() {
    let (_root, core, _) = start().await;
    let t = core.create_task("Fix the login redirect", "Safari drops the session cookie on redirect").await.unwrap();
    let t = core.move_task(&t.id, TaskStatus::Cancelled).await.unwrap();
    let keep = core.create_task("Write the changelog", "Summarise the release").await.unwrap();

    // Fresh: nothing to do.
    let r = core.board_sweep().await.unwrap();
    assert_eq!((r.newly_archived, r.archived_ids.len(), r.flagged.len()), (0, 0, 0));

    // Three days later the finished task goes; the untouched one is not old enough to flag.
    let r = core.board_sweep_at(t.updated_at + 3 * DAY_MS).await.unwrap();
    assert_eq!(r.newly_archived, 1);
    assert_eq!(r.archived_ids, [t.id.clone()]);
    assert!(r.flagged.is_empty());
    assert_eq!(core.board_sweep_at(t.updated_at + 3 * DAY_MS).await.unwrap().newly_archived, 0, "once only");

    let hits = core.board_archive(Some("safari cookie")).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Fix the login redirect");
    assert!(core.board_archive(Some("changelog")).await.unwrap().is_empty());

    // 14 days untouched: flagged; 21: archived.
    let r = core.board_sweep_at(keep.updated_at + 15 * DAY_MS).await.unwrap();
    assert_eq!(r.flagged.len(), 1);
    assert_eq!(r.flagged[0].id, keep.id);
    assert_eq!(r.flagged[0].days, 15);
    let r = core.board_sweep_at(keep.updated_at + 22 * DAY_MS).await.unwrap();
    assert_eq!(r.newly_archived, 1);
    assert_eq!(core.board_archive(None).await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn thresholds_come_from_the_settings() {
    let (_root, core, _) = start().await;
    assert_eq!(core.board_policy(None).await.unwrap(), ArchivePolicy::default());
    let p = ArchivePolicy { done_days: 7, stale_flag_days: 30, stale_archive_days: 10, question_flag_days: 3 };
    let saved = core.board_policy(Some(p)).await.unwrap();
    assert_eq!((saved.done_days, saved.stale_flag_days, saved.stale_archive_days), (7, 30, 30), "archive never before flag");
    assert_eq!(core.board_policy(None).await.unwrap(), saved);

    let t = core.create_task("Old news", "x").await.unwrap();
    let t = core.move_task(&t.id, TaskStatus::Cancelled).await.unwrap();
    assert_eq!(core.board_sweep_at(t.updated_at + 3 * DAY_MS).await.unwrap().newly_archived, 0);
    assert_eq!(core.board_sweep_at(t.updated_at + 8 * DAY_MS).await.unwrap().newly_archived, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_agent_gets_the_safe_default_servers_and_no_credentialed_ones() {
    let (_root, core, specs) = start().await;
    core.send_chat("lead", "hello").await.unwrap();
    let spec = specs.lock().unwrap().first().cloned().expect("the Lead was spawned");
    let side = spec.raw_log.with_file_name("extra-mcp.json");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&side).unwrap()).unwrap();
    let names: Vec<&str> = doc.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["sequential-thinking", "time", "fetch", "context7", "filesystem", "git"]);
    let fs = &doc[4]["args"];
    assert_eq!(fs[fs.as_array().unwrap().len() - 1].as_str().unwrap(), spec.cwd.to_string_lossy());

    // The connection list agrees, and credentialed servers are off.
    let list = core.connections().await.unwrap();
    assert_eq!(list.iter().filter(|c| c.active).count(), 6);
    assert!(list.iter().filter(|c| c.id == "github" || c.id == "database" || c.id == "google-workspace").all(|c| !c.enabled && !c.active));

    // Switching one off applies to the next spawn; the running agent's file is untouched.
    core.update_connection("fetch", Some(false), None).await.unwrap();
    assert!(std::fs::read_to_string(&side).unwrap().contains("\"fetch\""));
    core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let hired = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.role == Role::Solo).unwrap();
    core.send_chat(&format!("agent:{}", hired.id), "hi").await.unwrap();
    let next = specs.lock().unwrap().last().cloned().unwrap();
    assert_ne!(next.run_id, spec.run_id);
    let text = std::fs::read_to_string(next.raw_log.with_file_name("extra-mcp.json")).unwrap();
    assert!(!text.contains("\"fetch\"") && text.contains("\"time\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_connections_and_slots_are_refused() {
    let (_root, core, _) = start().await;
    assert!(core.update_connection("nope", Some(true), None).await.is_err());
    let bad = codly_core::orchestrator::housekeeping::ConnectionSecret { env: "NOT_A_SLOT".into(), value: Some("x".into()) };
    assert!(core.update_connection("github", None, Some(bad)).await.is_err());
}
