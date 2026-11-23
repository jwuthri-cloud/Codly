//! The Codex questions at the Core's edge, with a fake agent in place of the CLI and a fixed
//! CLI list (so no CLI is asked): which model list a picker gets, and when "Compact now" is
//! refused before Codex hears of it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use tokio::sync::mpsc;

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

fn cli(provider: Provider) -> CliInfo {
    CliInfo {
        provider,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(provider),
        default_model: None,
    }
}

/// A fake agent: every prompt is one turn; a prompt with "slow" in it takes a second. It
/// records the prompts that reached it.
struct Fake(Arc<Mutex<Vec<String>>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let prompts = self.0.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("sess".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            prompts.lock().unwrap().push(text.clone());
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            if text.contains("slow") {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A started core with Claude and Codex installed and an open workspace.
async fn started(dir: &Path, prompts: &Arc<Mutex<Vec<String>>>) -> Arc<Core> {
    let repo = temp_repo(dir);
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(prompts.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![cli(Provider::Claude), cli(Provider::Codex)]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
}

/// The hire wizard and the model dialog say where their list comes from, so they may show
/// Codly's built-in Codex models when the app-server cannot be asked. The pickers that show
/// names only (the Lead's controls, the team and the settings) get the CLI's own list or the
/// error: a built-in name there would look like Codex's own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_catalog_falls_back_to_the_built_in_codex_models() {
    let dir = tempfile::tempdir().unwrap();
    let core = started(dir.path(), &Arc::default()).await;

    let catalog = core.model_catalog(Provider::Codex).await.unwrap();
    assert_eq!(catalog.source, ModelSource::BuiltIn);
    assert_eq!(catalog.problem.as_deref(), Some("the CLI list is fixed, so no CLI is asked"));
    assert!(!catalog.models.is_empty());

    let e = core.list_models(Provider::Codex).await.unwrap_err();
    assert_eq!(e.to_string(), "the CLI list is fixed, so no CLI is asked");
    // Claude has no list to ask for: its aliases are the list, in both
    assert_eq!(core.list_models(Provider::Claude).await.unwrap(), ["opus", "sonnet", "haiku", "fable"]);
    assert_eq!(core.model_catalog(Provider::Claude).await.unwrap().models.len(), 4);
    core.shutdown().await;
}

/// "Compact now" is for an agent that runs and is between turns (a Claude agent takes Claude
/// Code's own way, `orchestrator/controls.rs`; this is Codex's). Each refusal says why,
/// and a request that is refused leaves no trace in the History.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_now_is_refused_unless_a_codex_agent_is_idle() {
    let dir = tempfile::tempdir().unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let core = started(dir.path(), &prompts).await;
    let refusal = async |id: &str| core.compact_agent(id).await.unwrap_err().to_string();

    assert!(refusal("nobody").await.starts_with("unknown agent"));
    let codex = core.hire_agent(Provider::Codex, None, None).await.unwrap();
    assert_eq!(refusal(&codex.id).await, format!("{} is not running", codex.name));

    // running and idle: the first look passes. The question itself is not asked in this test
    // (the CLI list is fixed), which is where it stops.
    let thread = format!("agent:{}", codex.id);
    core.send_chat(&thread, "hello").await.unwrap();
    wait_for("the agent to go idle", async || {
        let a = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == codex.id)?;
        (prompts.lock().unwrap().len() == 1 && a.status == AgentStatus::Idle).then_some(())
    })
    .await;
    assert_eq!(refusal(&codex.id).await, "the CLI list is fixed, so no CLI is asked");

    // in a turn
    core.send_chat(&thread, "slow step").await.unwrap();
    assert_eq!(refusal(&codex.id).await, format!("{} is working. Compact it when its turn ends.", codex.name));

    let events = core.events(Some(&codex.id), None, 200).await.unwrap();
    assert!(
        !events.iter().any(|e| matches!(&e.kind, EventKind::Decision { summary, .. } if summary.contains("compact"))),
        "nothing was asked of Codex, so the History says nothing"
    );
    core.shutdown().await;
}
