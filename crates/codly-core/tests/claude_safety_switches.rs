//! Settings → Safety, the two Claude Code switches, through the real Core: "Sandbox Claude's
//! shell" (`Settings.claude_sandbox`) and "Use my shell's Claude sign-in"
//! (`Settings.claude_shell_auth`). Both are off by default and reach the agents that start
//! after the change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
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

/// A Core on a small repo whose login shell selects Bedrock for Claude and, like a build
/// started from inside Claude Code, also carries a variable of that parent session.
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
        install: codly_core::env::install_hint(Provider::Claude),
        default_model: None,
    };
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert("CLAUDE_CODE_USE_BEDROCK".into(), "1".into());
    env.insert("CLAUDE_CODE_ENTRYPOINT".into(), "cli".into());
    let specs = Arc::new(Mutex::new(vec![]));
    let opts = StartOptions { spawner: Some(Arc::new(Recorder(specs.clone()))), env: Some(env), clis: Some(vec![claude]), index: false };
    let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") }, opts).await.unwrap();
    core.open_workspace(&repo.to_string_lossy()).await.unwrap();
    (root, core, specs)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_claude_switches_are_off_by_default_and_reach_the_next_agent() {
    let (_root, core, specs) = start().await;
    assert!(!core.settings().claude_sandbox && !core.settings().claude_shell_auth);

    // Off: no sandbox asked for, and the shell's Bedrock switch is taken out with the rest.
    core.send_chat("lead", "hello").await.unwrap();
    let before = specs.lock().unwrap().last().cloned().expect("the Lead was started");
    assert!(!before.sandbox_shell);
    assert!(!before.env.keys().any(|k| k.starts_with("CLAUDE_CODE_USE_") || k == "CLAUDE_CODE_ENTRYPOINT"), "{:?}", before.env.keys());

    // On: the agent started after the change has both. A parent session's variable still goes.
    core.update_settings(Settings { claude_sandbox: true, claude_shell_auth: true, ..core.settings() }).await.unwrap();
    let hired = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    core.send_chat(&format!("agent:{}", hired.id), "hi").await.unwrap();
    let after = specs.lock().unwrap().last().cloned().unwrap();
    assert_ne!(after.run_id, before.run_id);
    assert_eq!((after.role, after.read_only), (Role::Solo, false));
    assert!(after.sandbox_shell);
    assert_eq!(after.env.get("CLAUDE_CODE_USE_BEDROCK").map(String::as_str), Some("1"));
    assert!(!after.env.contains_key("CLAUDE_CODE_ENTRYPOINT"));
    let args = codly_core::adapters::claude::build_args(&after, "s", None);
    let at = args.iter().position(|a| a == "--settings").expect("the sandbox is passed to the CLI");
    let settings: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
    assert_eq!(settings["sandbox"]["enabled"], serde_json::json!(true));
}
