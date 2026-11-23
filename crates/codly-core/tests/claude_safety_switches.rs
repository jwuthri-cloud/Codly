//! Settings → Safety, the two Claude Code switches, through the real Core: "Sandbox Claude's
//! shell" (`Settings.claude_sandbox`) and "Use my shell's Claude sign-in"
//! (`Settings.claude_shell_auth`). Both are off by default and reach the agents that start
//! after the change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codly_core::adapters::claude::SandboxNotOn;
use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::hire::{HireManifest, SCHEMA};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::mpsc;

/// Records every spec it is asked to start; the agent does nothing. With `no_sandbox`, a run
/// that asks for the shell sandbox is not started: what the Claude adapter answers when the
/// CLI's sandbox is not on.
struct Recorder {
    specs: Arc<Mutex<Vec<SpawnSpec>>>,
    no_sandbox: bool,
}

impl Spawner for Recorder {
    fn spawn(&self, spec: SpawnSpec, _events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        self.specs.lock().unwrap().push(spec.clone());
        if self.no_sandbox && spec.sandbox_shell && !spec.read_only {
            let refused = anyhow::Error::new(SandboxNotOn("Claude Code did not turn its sandbox on, so Codly did not start this agent".into()));
            return Box::pin(async move { Err(refused) });
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let handle = AgentHandle { agent_id: spec.agent_id, run_id: spec.run_id, tx };
        Box::pin(async move { Ok(handle) })
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A Core on a small repo whose login shell selects Bedrock for Claude and, like a build
/// started from inside Claude Code, also carries a variable of that parent session.
/// `no_sandbox`: Claude Code's sandbox is not on there (`Recorder`).
async fn start(no_sandbox: bool) -> (tempfile::TempDir, Arc<Core>, Arc<Mutex<Vec<SpawnSpec>>>) {
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
    let opts = StartOptions { spawner: Some(Arc::new(Recorder { specs: specs.clone(), no_sandbox })), env: Some(env), clis: Some(vec![claude]), index: false };
    let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") }, opts).await.unwrap();
    core.open_workspace(&repo.to_string_lossy()).await.unwrap();
    (root, core, specs)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_claude_switches_are_off_by_default_and_reach_the_next_agent() {
    let (_root, core, specs) = start(false).await;
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
    let args = codly_core::adapters::claude::build_args(&after, "s", None, &[]);
    let at = args.iter().position(|a| a == "--settings").expect("the sandbox is passed to the CLI");
    let settings: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
    assert_eq!(settings["sandbox"]["enabled"], serde_json::json!(true));
    // The sandbox is a floor under the approval rules, not a way around them: commands still ask.
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], serde_json::json!(false));
}

/// A 1:1 agent whose sandbox is not on is not started, and it keeps its conversation: nothing
/// is wrong with the session, and a new one would not start either. With the switch off
/// again, it goes on where it was.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_whose_sandbox_is_not_on_keeps_its_conversation() {
    let (_root, core, specs) = start(true).await;
    let manifest = HireManifest {
        schema: SCHEMA.into(),
        name: "Cleo".into(),
        avatar: None,
        color: None,
        title: None,
        provider: Provider::Claude.as_str().into(),
        model: None,
        command: None,
        isolate: true,
        resume_session: Some("earlier-session".into()),
        description: "Helps with the repo.".into(),
        goal: String::new(),
        traits: vec![],
        career: None,
        memory: None,
    };
    let agent = core.hire_from_manifest(manifest, false).await.unwrap();
    let thread = format!("agent:{}", agent.id);
    let resumes = |spec: &SpawnSpec| matches!(&spec.session, SessionMode::Resume { session_id } if session_id == "earlier-session");

    core.update_settings(Settings { claude_sandbox: true, ..core.settings() }).await.unwrap();
    let refused = core.send_chat(&thread, "hi").await.unwrap_err();
    assert!(refused.is::<SandboxNotOn>() && refused.to_string().contains("did not turn its sandbox on"), "{refused:#}");
    let kept = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == agent.id).unwrap().session_id;
    assert_eq!(kept.as_deref(), Some("earlier-session"));
    {
        let tried = specs.lock().unwrap();
        assert_eq!(tried.len(), 1, "no new session was tried in its place");
        assert!(tried[0].sandbox_shell && resumes(&tried[0]), "{:?}", tried[0].session);
    }

    core.update_settings(Settings { claude_sandbox: false, ..core.settings() }).await.unwrap();
    core.send_chat(&thread, "hi").await.unwrap();
    let last = specs.lock().unwrap().last().cloned().unwrap();
    assert!(!last.sandbox_shell && resumes(&last), "{:?}", last.session);
}
