//! A Claude agent's mode, model, "compact now" and context reading end to end, with a fake
//! agent in place of the CLI. The fake answers session requests with the shapes Claude Code
//! 2.1.311 was recorded to give (docs/research/cli-claude-stream-json-2.1.311.md §2); the
//! adapter's own tests replay the recordings themselves.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionRequest, SpawnSpec, permission_mode_changed};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::json;
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

fn claude() -> CliInfo {
    CliInfo {
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
    }
}

#[derive(Default)]
struct Seen {
    /// What reached the agent, in order: "prompt:<text>", "mode:<cli name>", "model:<name>",
    /// "context".
    log: Vec<String>,
    /// The run id of every spawn, in order.
    runs: Vec<String>,
    shutdowns: u32,
    /// Knob: the CLI refuses a mode switch.
    refuses_modes: bool,
}

impl Seen {
    fn prompts(&self) -> Vec<&str> {
        self.log.iter().filter_map(|l| l.strip_prefix("prompt:")).collect()
    }
}

/// A fake Claude. A prompt with "slow" takes 300 ms; one with "plan accepted" ends with the
/// CLI leaving plan mode by itself, as Claude Code does when its plan is accepted.
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let send = {
                let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            seen.lock().unwrap().runs.push(spec.run_id.clone());
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
                            seen.lock().unwrap().log.push(format!("prompt:{text}"));
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            if text.contains("slow") {
                                tokio::time::sleep(Duration::from_millis(300)).await;
                            }
                            if text.contains("plan accepted") {
                                send(permission_mode_changed(PermissionMode::Ask));
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Session { request, reply } => {
                            let mut s = seen.lock().unwrap();
                            let answer = match request {
                                SessionRequest::PermissionMode(mode) => {
                                    s.log.push(format!("mode:{}", mode.cli()));
                                    if s.refuses_modes { Err("the session cannot change its mode".to_string()) } else { Ok(json!({ "mode": mode.cli() })) }
                                }
                                SessionRequest::Model(name) => {
                                    s.log.push(format!("model:{name}"));
                                    if name == "no-such-model" {
                                        Err(format!("Model '{name}' not found"))
                                    } else {
                                        Ok(json!({ "model": format!("claude-{name}-5-5") }))
                                    }
                                }
                                SessionRequest::ContextUsage => {
                                    s.log.push("context".into());
                                    Ok(json!({
                                        "categories": [
                                            { "id": "messages", "name": "Messages", "tokens": 1442, "kind": "used" },
                                            { "id": "free_space", "name": "Free space", "tokens": 998558, "kind": "free" },
                                        ],
                                        "totalTokens": 1442, "maxTokens": 1000000, "percentage": 1,
                                        "autoCompactThreshold": 967000, "isAutoCompactEnabled": true,
                                    }))
                                }
                            };
                            let _ = reply.send(answer);
                        }
                        AgentCmd::Shutdown => {
                            seen.lock().unwrap().shutdowns += 1;
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } => {}
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

async fn agent(core: &Core, id: &str) -> Agent {
    core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == id).expect("the agent")
}

/// A started core with one hired 1:1 Claude agent, not running yet.
async fn hired_agent(dir: &Path, seen: &Arc<Mutex<Seen>>) -> (Arc<Core>, String) {
    let repo = temp_repo(dir);
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    (core, agent.id)
}

/// Turns of the agent that Codly has seen end.
async fn turns_done(core: &Core, id: &str) -> usize {
    let events = core.events(Some(id), None, 500).await.unwrap();
    events.iter().filter(|e| matches!(e.kind, EventKind::TurnCompleted { .. })).count()
}

/// Wait until Codly has seen `turns` turns of the agent end and the agent is idle again.
async fn settled(core: &Core, id: &str, turns: usize) {
    wait_for("the turn to end", async || (turns_done(core, id).await >= turns).then_some(())).await;
    wait_for("the agent to go idle", async || (agent(core, id).await.status == AgentStatus::Idle).then_some(())).await;
}

/// Send a message and wait until its turn is over.
async fn turn(core: &Core, id: &str, text: &str) {
    let before = turns_done(core, id).await;
    core.send_chat(&format!("agent:{id}"), text).await.unwrap();
    settled(core, id, before + 1).await;
}

/// A started core with one hired agent that is live (a first message spawned it).
async fn live_agent(dir: &Path, seen: &Arc<Mutex<Seen>>) -> (Arc<Core>, String) {
    let (core, id) = hired_agent(dir, seen).await;
    turn(&core, &id, "hello").await;
    (core, id)
}

/// Kill the agent's process and wait until it is gone.
async fn stop(core: &Core, id: &str) {
    core.stop_agent(id).await.unwrap();
    wait_for("the agent to stop", async || agent(core, id).await.run_id.is_none().then_some(())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mode_switches_the_live_session_and_is_set_again_on_the_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    assert_eq!(agent(&core, &id).await.permission_mode, None, "never chosen: it asks for edits");

    // The live session switches, with no restart, and the agent keeps the mode.
    let a = core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap();
    assert_eq!(a.permission_mode, Some(PermissionMode::Plan));
    assert_eq!(seen.lock().unwrap().log.last().map(String::as_str), Some("mode:plan"));
    {
        let s = seen.lock().unwrap();
        assert_eq!((s.runs.len(), s.shutdowns), (1, 0), "the same process goes on");
    }
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
            if decision_type == "agent_control" && summary.contains("plan first"))),
        "the switch is on record"
    );
    // The same mode again asks the CLI nothing.
    core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap();
    assert_eq!(seen.lock().unwrap().log.iter().filter(|l| l.starts_with("mode:")).count(), 1);

    // A new run starts in the CLI's own mode: Codly sets the agent's mode before its first prompt.
    stop(&core, &id).await;
    turn(&core, &id, "second run").await;
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.runs.len(), 2);
        let at = |what: &str| s.log.iter().rposition(|l| l.starts_with(what)).unwrap_or_else(|| panic!("no {what} in {:?}", s.log));
        assert_eq!(s.log.iter().filter(|l| *l == "mode:plan").count(), 2, "{:?}", s.log);
        assert!(at("mode:plan") < at("prompt:"), "the mode is on before the prompt is read: {:?}", s.log);
    }

    // Claude Code leaves plan mode by itself when its plan is accepted: the switch follows.
    turn(&core, &id, "plan accepted").await;
    wait_for("the agent's mode to follow the CLI", async || (agent(&core, &id).await.permission_mode == Some(PermissionMode::Ask)).then_some(())).await;
    // "Ask for edits" is the mode a run starts in: the next run is asked nothing.
    stop(&core, &id).await;
    turn(&core, &id, "third run").await;
    assert_eq!(seen.lock().unwrap().log.iter().filter(|l| l.starts_with("mode:")).count(), 2);

    assert!(core.set_agent_permission_mode("nobody", PermissionMode::Plan).await.is_err());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mode_chosen_while_the_agent_rests_is_set_when_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = hired_agent(dir.path(), &seen).await;

    let a = core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap();
    assert_eq!(a.permission_mode, Some(PermissionMode::AcceptEdits));
    assert!(seen.lock().unwrap().log.is_empty(), "no process to ask");
    turn(&core, &id, "hello").await;
    let log = seen.lock().unwrap().log.clone();
    assert_eq!(log[0], "mode:acceptEdits", "{log:?}");
    assert!(log[1].starts_with("prompt:") && log[1].ends_with("hello"), "{log:?}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mode_the_cli_refuses_is_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen { refuses_modes: true, ..Default::default() }));
    let (core, id) = live_agent(dir.path(), &seen).await;

    // Live: the switch fails with the CLI's own words and nothing changes.
    let e = core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap_err().to_string();
    assert!(e.contains("the session cannot change its mode"), "{e}");
    assert_eq!(agent(&core, &id).await.permission_mode, None);

    // At the start of a run: the run goes on in "ask for edits", and the thread says so.
    stop(&core, &id).await;
    core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap();
    turn(&core, &id, "again").await;
    wait_for("the mode to be given up", async || agent(&core, &id).await.permission_mode.is_none().then_some(())).await;
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Error { message, fatal: false }
            if message.contains("could not go back to the mode \"plan first\"") && message.contains("It asks before every edit instead"))),
        "{events:?}"
    );
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_claude_agent_changes_its_model_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    let a = core.set_agent_model(&id, Some("sonnet".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet"));
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.log.last().map(String::as_str), Some("model:sonnet"));
        assert_eq!((s.runs.len(), s.shutdowns), (1, 0), "the same process goes on");
    }
    assert!(agent(&core, &id).await.run_id.is_some());
    let events = core.events(Some(&id), None, 200).await.unwrap();
    let said = events.iter().any(|e| matches!(&e.kind, EventKind::Chat { message }
        if message.author == "codly" && message.text.contains("now works with sonnet") && message.text.contains("no restart was needed")));
    assert!(said, "the thread says that the model changed in place");

    // A model the CLI does not know: its words, and the agent keeps its model.
    let e = core.set_agent_model(&id, Some("no-such-model".into())).await.unwrap_err().to_string();
    assert!(e.contains("Model 'no-such-model' not found"), "{e}");
    assert_eq!(agent(&core, &id).await.model.as_deref(), Some("sonnet"));

    // Another reasoning effort is set when the process starts: that switch still restarts.
    let a = core.set_agent_model(&id, Some("opus:high".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus:high"));
    {
        let s = seen.lock().unwrap();
        assert!(!s.log.iter().any(|l| l.starts_with("model:opus")), "{:?}", s.log);
        assert_eq!(s.shutdowns, 1);
    }
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_now_is_a_turn_of_its_own_between_turns() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    core.compact_agent(&id).await.unwrap();
    settled(&core, &id, 2).await;
    assert_eq!(seen.lock().unwrap().prompts()[1], "/compact", "the command alone, with nothing around it");

    // Not into a running turn, and not while the agent is paused.
    core.send_chat(&format!("agent:{id}"), "slow step").await.unwrap();
    wait_for("the slow turn to start", async || (seen.lock().unwrap().prompts().len() == 3).then_some(())).await;
    let e = core.compact_agent(&id).await.unwrap_err().to_string();
    assert!(e.contains("is still working"), "{e}");
    settled(&core, &id, 3).await;
    core.stop_agent_after_step(&id).await.unwrap();
    let e = core.compact_agent(&id).await.unwrap_err().to_string();
    assert!(e.contains("is paused"), "{e}");
    assert_eq!(seen.lock().unwrap().prompts().len(), 3);

    stop(&core, &id).await;
    let e = core.compact_agent(&id).await.unwrap_err().to_string();
    assert!(e.contains("is not running"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_context_reading_comes_from_the_live_session() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    let usage = core.agent_context_usage(&id).await.unwrap();
    assert_eq!((usage.used_tokens, usage.max_tokens, usage.auto_compact_at), (1442, 1_000_000, Some(967_000)));
    assert_eq!(usage.parts, vec![ContextPart { name: "Messages".into(), tokens: 1442 }]);
    assert_eq!(seen.lock().unwrap().log.last().map(String::as_str), Some("context"));

    stop(&core, &id).await;
    let e = core.agent_context_usage(&id).await.unwrap_err().to_string();
    assert!(e.contains("is not running"), "{e}");
    core.shutdown().await;
}
