//! App exit with an agent that ignores the shutdown ladder: `Core::shutdown` still returns in
//! about 6 s and the agent's whole process group is gone.
//!
//! This test must stay alone in its file. `StuckSpawner` ignores the stop on purpose, so the
//! shutdown's last resort runs, and that kills every process group led by a child of this
//! process: in one test binary all tests share the process, so a test beside it would have its
//! check commands and agents killed too. The other test files' fakes all end when told to stop,
//! and their shutdown helper fails a test whose run outlives the shutdown.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::mpsc;

/// Your own git settings (`~/.gitconfig`, the system file) reach no git of this test: neither
/// the core's (`core_env`) nor the test's own, which commits as a fixed author.
const NO_GIT_CONFIG: [(&str, &str); 2] = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")];

/// Who the test's own commits are by, whatever the repository says.
const TEST_AUTHOR: [(&str, &str); 4] =
    [("GIT_AUTHOR_NAME", "Test"), ("GIT_AUTHOR_EMAIL", "test@example.com"), ("GIT_COMMITTER_NAME", "Test"), ("GIT_COMMITTER_EMAIL", "test@example.com")];

/// The environment the core runs with: this process's, without your git settings.
fn core_env() -> std::collections::HashMap<String, String> {
    std::env::vars().chain(NO_GIT_CONFIG.map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

/// Stands in for a CLI that hangs on exit: a real child in its own process group (with a
/// background child of its own), and an actor that never acts on `Shutdown`.
struct StuckSpawner {
    pgid: Arc<Mutex<Option<i32>>>,
}

impl Spawner for StuckSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let pgid = self.pgid.clone();
        Box::pin(async move {
            let mut child = tokio::process::Command::new("/bin/sh").args(["-c", "sleep 60 & sleep 60"]).process_group(0).spawn()?;
            *pgid.lock().unwrap() = child.id().map(|p| p as i32);
            let ev = |kind| AdapterEvent { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), kind };
            let _ = events.send(ev(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("stuck".into()),
                billing: Billing::Subscription,
                tools: vec![],
            }));
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let exited = ev(EventKind::Exited { code: None });
            tokio::spawn(async move {
                let _ = child.wait().await;
                let _ = events.send(exited);
            });
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_kills_agents_that_outlive_their_ladder() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "init"]);

    let pgid = Arc::new(Mutex::new(None));
    let claude = CliInfo {
        provider: Provider::Claude,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(Provider::Claude),
        default_model: None,
    };
    let cfg = CoreConfig { data_dir: root.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(StuckSpawner { pgid: pgid.clone() })),
        env: Some(core_env()),
        clis: Some(vec![claude]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(&repo.to_string_lossy()).await.unwrap();
    core.send_chat("lead", "hello").await.unwrap();
    let pg = pgid.lock().unwrap().expect("the Lead was spawned");
    assert!(nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pg), None).is_ok());

    let started = Instant::now();
    core.shutdown().await;
    assert!(started.elapsed() < Duration::from_secs(7), "shutdown took {:?}", started.elapsed());
    assert_eq!(core.runs_given_up(), 1, "the shutdown gave up on the stuck run");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pg), None).is_err(), "the agent's process group outlived the app");
}
