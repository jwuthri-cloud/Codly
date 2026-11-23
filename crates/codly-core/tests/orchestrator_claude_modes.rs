//! A Claude agent's mode, model, thinking effort, "compact now", context reading, MCP reconnect
//! and the stop of a background command end to end, with a fake agent in place of the CLI. The
//! fake answers session requests with the shapes Claude Code 2.1.311 was recorded to give
//! (docs/research/cli-claude-stream-json-2.1.311.md §2 and its part 2, §6, §18 and scenario
//! 13); the adapter's own tests replay the recordings themselves.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionRequest, SpawnSpec, permission_mode_changed};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::json;
use tokio::sync::{Notify, mpsc};

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
    /// "effort:<level>", "context", and the answer to a command it asked about:
    /// "allowed:<command>" or "denied:<command>".
    log: Vec<String>,
    /// The effort the session works with, once one was set (Claude Code starts with "medium").
    effort: Option<String>,
    /// Knob: the CLI drops the effort level it is asked for. It answers "success" all the same.
    drops_effort: bool,
    /// The run id of every spawn, in order.
    runs: Vec<String>,
    shutdowns: u32,
    /// Knob: the CLI refuses a mode switch.
    refuses_modes: bool,
    /// Ends the turn of a prompt with "HOLD-THE-TURN".
    release: Arc<Notify>,
}

impl Seen {
    fn prompts(&self) -> Vec<&str> {
        self.log.iter().filter_map(|l| l.strip_prefix("prompt:")).collect()
    }
}

/// A fake Claude. A prompt with "slow" takes 300 ms; one with "plan accepted" ends with the
/// CLI leaving plan mode by itself, as Claude Code does when its plan is accepted. The turn of
/// a prompt with "HOLD-THE-TURN" ends when the test says so (`Seen::release`), and one that
/// ends in "ASK-TO-RUN: <command>" asks to run that command and ends with the answer. Session
/// requests are answered during such a turn too, as the CLI answers them. In the turn of a
/// prompt with "LEAVE-RUNNING" its MCP servers report and it leaves a command running in the
/// background; one with "SAME-SERVERS" reports the servers where that turn left them, as a
/// new run does when it starts, and one with "DOCS-FAILS" reports one server that changed; one
/// with "MANY-STEPS" makes more events than a snapshot holds. A reconnect of the server
/// "takes-its-time" is answered after 11 s.
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
            let send = {
                let (agent_id, run_id) = (agent_id.clone(), run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            let release = {
                let mut s = seen.lock().unwrap();
                s.runs.push(spec.run_id.clone());
                s.release.clone()
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
                let done = || EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) };
                // A turn that waits for the test's word.
                let mut held = false;
                // The command of a turn that waits for its answer.
                let mut asked: Option<String> = None;
                loop {
                    let cmd = tokio::select! {
                        cmd = rx.recv() => match cmd {
                            Some(cmd) => cmd,
                            None => break,
                        },
                        _ = release.notified(), if held => {
                            held = false;
                            send(done());
                            continue;
                        }
                    };
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
                            let server = |name: &str, status, error: Option<&str>| EventKind::McpServer { name: name.into(), status, error: error.map(String::from) };
                            const REFUSED: Option<&str> = Some("Dynamic Client Registration rejected (HTTP 401)");
                            if text.contains("LEAVE-RUNNING") {
                                // Claude Code's whole list of background tasks, at each change.
                                let listed = |ids: &[&str], ended: &[&str]| EventKind::BackgroundTasks {
                                    tasks: ids.iter().map(|id| BackgroundTask { id: (*id).into(), kind: "command".into(), description: "npm run dev".into() }).collect(),
                                    ended: ended
                                        .iter()
                                        .map(|id| BackgroundEnded { id: (*id).into(), kind: "command".into(), description: "npm run dev".into(), status: "completed".into() })
                                        .collect(),
                                };
                                send(server("docs", McpServerStatus::Connected, None));
                                send(server("linear", McpServerStatus::Connecting, None));
                                send(server("linear", McpServerStatus::NeedsSignIn, REFUSED));
                                send(listed(&["tid-1"], &[]));
                                send(listed(&["tid-1", "tid-2"], &[]));
                                send(listed(&["tid-1"], &["tid-2"]));
                            }
                            if text.contains("SAME-SERVERS") {
                                send(server("docs", McpServerStatus::Connected, None));
                                send(server("linear", McpServerStatus::NeedsSignIn, REFUSED));
                            }
                            if text.contains("DOCS-FAILS") {
                                send(server("docs", McpServerStatus::Failed, Some("Connection closed")));
                            }
                            if text.contains("MANY-STEPS") {
                                for _ in 0..320 {
                                    send(EventKind::Unknown { raw_type: "step".into() });
                                }
                            }
                            if let Some((_, command)) = text.rsplit_once("ASK-TO-RUN: ") {
                                let command = command.trim().to_string();
                                let approval = Approval {
                                    id: new_id(),
                                    agent_id: agent_id.clone(),
                                    run_id: run_id.clone(),
                                    task_id: None,
                                    call_id: Some("c1".into()),
                                    subject: ApprovalSubject::Command { command: command.clone(), cwd: None },
                                    reason: None,
                                    choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
                                    risks: vec![],
                                    status: ApprovalStatus::Pending,
                                    decided_by: None,
                                    created_at: now_ms(),
                                };
                                send(EventKind::ApprovalRequested { approval });
                                asked = Some(command);
                            } else if text.contains("HOLD-THE-TURN") {
                                held = true;
                            } else {
                                send(done());
                            }
                        }
                        AgentCmd::Approval { decision, .. } => {
                            if let Some(command) = asked.take() {
                                let answer = if decision.choice == ApprovalChoice::Deny { "denied" } else { "allowed" };
                                seen.lock().unwrap().log.push(format!("{answer}:{command}"));
                                send(done());
                            }
                        }
                        AgentCmd::Session { request, reply } => {
                            if matches!(&request, SessionRequest::McpReconnect(server) if server == "takes-its-time") {
                                tokio::time::sleep(Duration::from_secs(11)).await;
                            }
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
                                    } else if name == "default" {
                                        Ok(json!({ "model": "claude-opus-5-5" }))
                                    } else {
                                        Ok(json!({ "model": format!("claude-{name}-5-5") }))
                                    }
                                }
                                SessionRequest::Effort(level) => {
                                    s.log.push(format!("effort:{level}"));
                                    if !s.drops_effort {
                                        s.effort = Some(level);
                                    }
                                    Ok(serde_json::Value::Null)
                                }
                                SessionRequest::Settings => Ok(json!({ "applied": { "effort": s.effort.as_deref().unwrap_or("medium") } })),
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
                                // (recorded: a server that still cannot connect answers with its reason)
                                SessionRequest::McpReconnect(server) => {
                                    s.log.push(format!("reconnect:{server}"));
                                    Err("Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}".to_string())
                                }
                                // (recorded: `{}`, whether or not a task ended)
                                SessionRequest::StopTask(task_id) => {
                                    s.log.push(format!("stop-task:{task_id}"));
                                    Ok(json!({}))
                                }
                            };
                            let _ = reply.send(answer);
                        }
                        AgentCmd::Shutdown => {
                            seen.lock().unwrap().shutdowns += 1;
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt => {}
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

/// Turns of the agent that Codly has seen start.
async fn turns_started(core: &Core, id: &str) -> usize {
    let events = core.events(Some(id), None, 500).await.unwrap();
    events.iter().filter(|e| matches!(e.kind, EventKind::TurnStarted { .. })).count()
}

/// The modes the CLI was asked for, in order, by the name the CLI gives them.
fn modes_asked(seen: &Arc<Mutex<Seen>>) -> Vec<String> {
    seen.lock().unwrap().log.iter().filter_map(|l| l.strip_prefix("mode:").map(String::from)).collect()
}

/// The last thing that reached the agent.
fn last_seen(seen: &Arc<Mutex<Seen>>) -> String {
    seen.lock().unwrap().log.last().cloned().unwrap_or_default()
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

    // Another model with another thinking effort: both change in place, the model first.
    let a = core.set_agent_model(&id, Some("opus:high".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus:high"));
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.log[s.log.len() - 2..], ["model:opus", "effort:high"], "{:?}", s.log);
        assert_eq!((s.runs.len(), s.shutdowns), (1, 0), "the same process goes on");
    }
    assert!(codly_said(&core, &id, "now works with opus and thinking effort high, in the same conversation").await);
    core.shutdown().await;
}

/// Does the agent's thread have a line of Codly's with this text?
async fn codly_said(core: &Core, id: &str, text: &str) -> bool {
    let events = core.events(Some(id), None, 500).await.unwrap();
    events.iter().any(|e| matches!(&e.kind, EventKind::Chat { message } if message.author == "codly" && message.text.contains(text)))
}

/// Before, another thinking effort and Claude Code's default model had no live request: both
/// stopped the process. Recorded since: `apply_flag_settings` sets the effort, `get_settings`
/// says which one the session works with, and `set_model` takes "default".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_claude_agent_changes_its_effort_and_takes_the_default_model_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let tail = |n: usize| {
        let s = seen.lock().unwrap();
        s.log[s.log.len() - n..].to_vec()
    };
    let model = async || agent(&core, &id).await.model;

    // Claude Code's default model: the agent shows the model the CLI named for it.
    core.set_agent_model(&id, Some("sonnet".into())).await.unwrap();
    let a = core.set_agent_model(&id, None).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(tail(2), ["model:sonnet", "model:default"]);
    assert!(codly_said(&core, &id, "now works with Claude Code's default model (claude-opus-5-5), in the same conversation").await);

    // A model with an effort, then another effort alone: the model is not asked for again.
    core.set_agent_model(&id, Some("sonnet:high".into())).await.unwrap();
    let a = core.set_agent_model(&id, Some("sonnet:low".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet:low"));
    assert_eq!(tail(3), ["model:sonnet", "effort:high", "effort:low"]);
    assert!(codly_said(&core, &id, "now works with thinking effort low, in the same conversation: no restart was needed.").await);

    // Claude Code answers "success" for a level it then drops. Codly reads the level back.
    // With another model too: the model changed, the effort did not, and the agent says so.
    seen.lock().unwrap().drops_effort = true;
    let e = format!("{:#}", core.set_agent_model(&id, Some("opus:max".into())).await.unwrap_err());
    assert!(e.contains("changed its model, but not its thinking effort") && e.contains("did not take the effort level \"max\""), "{e}");
    assert!(e.contains("it works with \"low\""), "{e}");
    assert_eq!(model().await.as_deref(), Some("opus:low"));
    assert_eq!(tail(2), ["model:opus", "effort:max"]);
    assert_eq!(seen.lock().unwrap().shutdowns, 0, "all of it in the same process");
    // The effort alone: the session keeps the one it has, so the agent restarts with the new
    // one, as it did before there was a live request. (A first version refused the switch,
    // which left no way to that effort but to stop the agent by hand.)
    let a = core.set_agent_model(&id, Some("opus:max".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus:max"));
    assert_eq!(tail(3), ["model:opus", "effort:max", "effort:max"], "the session was asked first");
    assert_eq!(seen.lock().unwrap().shutdowns, 1);
    assert!(!codly_said(&core, &id, "now works with thinking effort max").await, "nothing changed in place");
    seen.lock().unwrap().drops_effort = false;

    // A word that is no level of Claude Code is not sent to the session, and a level cannot
    // be taken away from it: those switches still restart.
    turn(&core, &id, "again").await;
    assert_eq!(seen.lock().unwrap().runs.len(), 2, "the next message starts the agent again");
    let asked = seen.lock().unwrap().log.len();
    let a = core.set_agent_model(&id, Some("opus".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus"));
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.log.len(), asked, "nothing was asked of the session: {:?}", s.log);
        assert_eq!(s.shutdowns, 2);
    }
    turn(&core, &id, "once more").await;
    core.set_agent_model(&id, Some("opus:minimal".into())).await.unwrap();
    {
        let s = seen.lock().unwrap();
        assert!(!s.log.iter().any(|l| l == "effort:minimal"), "{:?}", s.log);
        assert_eq!(s.shutdowns, 3);
    }
    core.shutdown().await;
}

/// Before, a model change was refused while a turn ran ("is still working"). Claude Code takes
/// `set_model` during a turn (recorded), so the switch reaches the session the turn runs in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_switch_reaches_the_live_session_during_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let run = agent(&core, &id).await.run_id;

    core.send_chat(&format!("agent:{id}"), "HOLD-THE-TURN").await.unwrap();
    wait_for("the turn to start", async || (turns_started(&core, &id).await == 2).then_some(())).await;

    let a = core.set_agent_model(&id, Some("sonnet".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet"));
    assert_eq!(last_seen(&seen), "model:sonnet", "the session got the model while its turn ran");
    assert_eq!(turns_done(&core, &id).await, 1, "the turn it was switched in is not over");
    assert_eq!(seen.lock().unwrap().shutdowns, 0);
    assert_eq!(agent(&core, &id).await.run_id, run, "the same run goes on");

    // The effort request is recorded between turns only: during one it is not sent.
    let e = core.set_agent_model(&id, Some("sonnet:high".into())).await.unwrap_err().to_string();
    assert!(e.contains("is still working: stop its turn first"), "{e}");
    assert_eq!(last_seen(&seen), "model:sonnet");
    assert_eq!(seen.lock().unwrap().shutdowns, 0);

    // The turn ends as any other, in the same process.
    seen.lock().unwrap().release.notify_one();
    settled(&core, &id, 2).await;
    assert_eq!(agent(&core, &id).await.run_id, run);
    assert_eq!(seen.lock().unwrap().runs.len(), 1);
    core.shutdown().await;
}

/// Before, a 1:1 agent on a task could not change its model ("is on a task": a restart would
/// strand the task). The live switch needs no restart, so the task's run takes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_1_to_1_agent_on_a_task_changes_its_model_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = hired_agent(dir.path(), &seen).await;

    // A task you give it: it works on it, in a turn that stays open.
    let task = core.create_task("Say hello", "HOLD-THE-TURN and say hello").await.unwrap();
    core.assign_task(&task.id, Some(&id)).await.unwrap();
    wait_for("the agent to work on its task", async || seen.lock().unwrap().prompts().iter().any(|p| p.contains("HOLD-THE-TURN")).then_some(())).await;
    let before = agent(&core, &id).await;
    assert_eq!(before.task_id.as_deref(), Some(task.id.as_str()));

    let a = core.set_agent_model(&id, Some("sonnet".into())).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet"));
    assert_eq!(last_seen(&seen), "model:sonnet");
    assert_eq!(seen.lock().unwrap().shutdowns, 0, "the task's run was not stopped");
    let after = agent(&core, &id).await;
    assert_eq!((after.run_id, after.task_id), (before.run_id, before.task_id), "the same run, still on its task");

    // So does Claude Code's default model (before, no request named it: the switch was refused).
    let a = core.set_agent_model(&id, None).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(last_seen(&seen), "model:default");
    // Another effort waits for the end of the turn, and a task's run is not restarted for it.
    let e = core.set_agent_model(&id, Some("opus:high".into())).await.unwrap_err().to_string();
    assert!(e.contains("is on a task"), "{e}");
    assert_eq!(last_seen(&seen), "model:default");
    assert_eq!(seen.lock().unwrap().shutdowns, 0);
    core.shutdown().await;
}

/// "Plan first" promises that nothing changes before you accept the plan. The approval rules
/// keep that promise for what the CLI asks about: full access allows a write for an agent that
/// asks for edits, and only reads for one that plans.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_that_plans_first_gets_only_reads_approved_until_its_plan_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    core.update_settings(Settings { approval_mode: ApprovalMode::Full, ..core.settings() }).await.unwrap();

    // Full access, "ask for edits": the rules allow the write.
    turn(&core, &id, "ASK-TO-RUN: touch notes.txt").await;
    assert_eq!(last_seen(&seen), "allowed:touch notes.txt");

    // Plan first: the same write waits for you, and the record says why.
    core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap();
    let turns = turns_done(&core, &id).await;
    core.send_chat(&format!("agent:{id}"), "ASK-TO-RUN: touch notes.txt").await.unwrap();
    let card = wait_for("the write to wait for you", async || core.snapshot().await.unwrap().approvals.into_iter().next()).await;
    wait_for("the broker to ask", async || {
        let events = core.events(None, None, 500).await.unwrap();
        events
            .iter()
            .any(|e| {
                matches!(&e.kind, EventKind::Decision { summary, detail, .. }
                if summary.starts_with("asking you") && summary.contains("its mode is \"plan first\"") && detail["approval_id"] == card.id.as_str())
            })
            .then_some(())
    })
    .await;
    assert!(last_seen(&seen).starts_with("prompt:"), "the agent has no answer yet");
    core.resolve_approval(&card.id, ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }).await.unwrap();
    settled(&core, &id, turns + 1).await;
    assert_eq!(last_seen(&seen), "denied:touch notes.txt");
    // A read is approved as before.
    turn(&core, &id, "ASK-TO-RUN: cat README.md").await;
    assert_eq!(last_seen(&seen), "allowed:cat README.md");

    // You accept its plan, Claude Code leaves plan mode by itself: full access counts again.
    turn(&core, &id, "plan accepted").await;
    wait_for("the agent's mode to follow the CLI", async || (agent(&core, &id).await.permission_mode == Some(PermissionMode::Ask)).then_some(())).await;
    turn(&core, &id, "ASK-TO-RUN: touch notes.txt").await;
    assert_eq!(last_seen(&seen), "allowed:touch notes.txt");
    core.shutdown().await;
}

/// In "accept edits" Claude Code does not ask about edits or file-writing shell commands in the
/// worktree (recorded), so no approval rule sees them. Where your permissions are "ask first"
/// every change asks you: the mode is refused there, and ends when you choose them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_edits_is_refused_and_ends_where_every_change_asks_you() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");
    let mode = async || agent(&core, &id).await.permission_mode;

    // Balanced, the default: the agent may accept edits.
    core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap();
    // "Ask first" in Settings: its live session goes back to "ask for edits" at once, on record.
    core.update_settings(Settings { approval_mode: ApprovalMode::Strict, ..core.settings() }).await.unwrap();
    assert_eq!(modes_asked(&seen), ["acceptEdits", "default"]);
    assert_eq!(mode().await, Some(PermissionMode::Ask));
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
            if decision_type == "agent_control" && summary.contains("asks for edits again") && summary.contains("\"Ask first\""))),
        "the change is on record"
    );
    // Refused while every change asks you, before the CLI is asked. The other modes are not.
    let e = core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap_err().to_string();
    assert!(e.contains("permissions are \"Ask first\"") && e.contains("cannot accept edits by itself"), "{e}");
    assert_eq!(modes_asked(&seen).len(), 2);
    core.set_agent_permission_mode(&id, PermissionMode::Plan).await.unwrap();
    core.set_agent_permission_mode(&id, PermissionMode::Ask).await.unwrap();

    // The conversation's own permissions come before the Settings ones.
    let own = |mode: ApprovalMode| RequestPrefs { approval_mode: Some(mode), ..Default::default() };
    core.set_request_prefs(&thread, own(ApprovalMode::Balanced)).await.unwrap();
    core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap();

    // A session that does not leave the mode: the switch keeps saying what is true, the
    // timeline says why, and the next run starts in "ask for edits" and is asked for no mode.
    seen.lock().unwrap().refuses_modes = true;
    core.set_request_prefs(&thread, own(ApprovalMode::Strict)).await.unwrap();
    assert_eq!(mode().await, Some(PermissionMode::AcceptEdits));
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Error { message, fatal: false }
            if message.contains("did not leave the mode \"accept edits\"") && message.contains("the session cannot change its mode"))),
        "{events:?}"
    );
    stop(&core, &id).await;
    seen.lock().unwrap().refuses_modes = false;
    let asked = modes_asked(&seen).len();
    turn(&core, &id, "again").await;
    assert_eq!(mode().await, Some(PermissionMode::Ask));
    assert_eq!(modes_asked(&seen).len(), asked);

    // An agent that rests has no session to switch: its switch goes back with your permissions.
    core.set_request_prefs(&thread, own(ApprovalMode::Balanced)).await.unwrap();
    core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap();
    stop(&core, &id).await;
    let asked = modes_asked(&seen).len();
    core.set_request_prefs(&thread, own(ApprovalMode::Strict)).await.unwrap();
    assert_eq!(mode().await, Some(PermissionMode::Ask));
    assert_eq!(modes_asked(&seen).len(), asked);
    core.shutdown().await;
}

/// An agent in "accept edits" moves to an office where every change asks you. Its first run
/// there starts in "ask for edits", the mode a run starts in: its CLI is asked for no mode, and
/// the agent's switch says so, on record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_edits_does_not_follow_an_agent_into_an_office_where_every_change_asks_you() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let thread = format!("agent:{id}");

    // In this office the conversation has permissions of its own, so the agent may accept
    // edits although the Settings say "Ask first".
    core.set_request_prefs(&thread, RequestPrefs { approval_mode: Some(ApprovalMode::Balanced), ..Default::default() }).await.unwrap();
    core.update_settings(Settings { approval_mode: ApprovalMode::Strict, ..core.settings() }).await.unwrap();
    core.set_agent_permission_mode(&id, PermissionMode::AcceptEdits).await.unwrap();
    assert_eq!(modes_asked(&seen), ["acceptEdits"]);

    // Another office, where the conversation has none: there the Settings count.
    let there = core.open_workspace(temp_repo(&dir.path().join("there")).to_str().unwrap()).await.unwrap();
    core.open_workspace(dir.path().join("repo").to_str().unwrap()).await.unwrap();
    core.transfer_agent(&id, &there.id).await.unwrap();
    core.open_workspace(&there.repo_path).await.unwrap();
    assert_eq!(agent(&core, &id).await.permission_mode, Some(PermissionMode::AcceptEdits), "it has not run there yet");

    turn(&core, &id, "hello from the new office").await;
    assert_eq!(agent(&core, &id).await.permission_mode, Some(PermissionMode::Ask));
    assert_eq!(seen.lock().unwrap().runs.len(), 2);
    assert_eq!(modes_asked(&seen), ["acceptEdits"], "the new run was asked for no mode");
    let events = core.events(Some(&id), None, 200).await.unwrap();
    assert!(
        events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
            if decision_type == "agent_control" && summary.contains("asks for edits again") && summary.contains("\"Ask first\""))),
        "the change is on record in the new office: {events:?}"
    );
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

/// Your acts on a live agent, as the History shows them.
async fn your_acts(core: &Core, id: &str) -> Vec<String> {
    let mut events = core.events(Some(id), None, 500).await.unwrap();
    events.sort_by_key(|e| e.id);
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type, method, summary, .. } if decision_type == "agent_control" && method == "human" => Some(summary),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reconnect_reaches_the_live_session_and_its_answer_is_yours() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let name = agent(&core, &id).await.name;

    // The server still does not connect: Claude Code's own reason is the error.
    let e = core.reconnect_agent_mcp(&id, " linear ").await.unwrap_err().to_string();
    assert_eq!(e, "Claude Code answered: Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}");
    assert_eq!(last_seen(&seen), "reconnect:linear");
    assert_eq!(your_acts(&core, &id).await, [format!("asked {name} to reconnect its MCP server linear")]);

    // No server named: nothing reaches the CLI.
    let e = core.reconnect_agent_mcp(&id, "  ").await.unwrap_err().to_string();
    assert_eq!(e, "which MCP server?");
    assert_eq!(last_seen(&seen), "reconnect:linear");

    stop(&core, &id).await;
    let e = core.reconnect_agent_mcp(&id, "linear").await.unwrap_err().to_string();
    assert!(e.contains("is not running"), "{e}");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_background_command_is_stopped_through_the_live_session() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let name = agent(&core, &id).await.name;

    core.stop_agent_task(&id, "tid-2").await.unwrap();
    assert_eq!(last_seen(&seen), "stop-task:tid-2");
    assert_eq!(your_acts(&core, &id).await, [format!("asked {name} to stop a command it left running in the background")]);
    let e = core.stop_agent_task(&id, "").await.unwrap_err().to_string();
    assert_eq!(e, "which background command?");

    stop(&core, &id).await;
    let e = core.stop_agent_task(&id, "tid-2").await.unwrap_err().to_string();
    assert!(e.contains("is not running"), "{e}");
    core.shutdown().await;
}

/// Claude Code answers a reconnect when its try to connect is over, and a server can take
/// longer to start than the 10 s the other session requests have: its answer is still yours.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reconnect_waits_for_a_server_that_is_slow_to_connect() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    let e = core.reconnect_agent_mcp(&id, "takes-its-time").await.unwrap_err().to_string();
    assert_eq!(e, "Claude Code answered: Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}");

    stop(&core, &id).await;
    core.shutdown().await;
}

/// The rows of a snapshot's `session_events`, as "<server> <state>" and, for a list of
/// background tasks, "runs <task ids>; ended <task ids>".
fn session_lines(rows: &[EventRow]) -> Vec<String> {
    let ids = |ids: Vec<&str>| if ids.is_empty() { "nothing".to_string() } else { ids.join(" ") };
    rows.iter()
        .map(|r| match &r.kind {
            EventKind::McpServer { name, status, .. } => format!("{name} {status:?}"),
            EventKind::BackgroundTasks { tasks, ended } => {
                format!("runs {}; ended {}", ids(tasks.iter().map(|t| t.id.as_str()).collect()), ids(ended.iter().map(|t| t.id.as_str()).collect()))
            }
            other => format!("{other:?}"),
        })
        .collect()
}

fn is_session_row(e: &EventRow) -> bool {
    matches!(e.kind, EventKind::McpServer { .. } | EventKind::BackgroundTasks { .. })
}

/// A server's event comes when its state changes and the list of background tasks when it
/// changes, and a snapshot has only the workspace's newest events. A run that went on for
/// long after them still has its servers and its last list in a fresh snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_snapshot_has_the_servers_and_the_background_work_of_a_long_run() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;

    // While the snapshot's own events still hold them, they are not given twice.
    turn(&core, &id, "LEAVE-RUNNING").await;
    let snap = core.snapshot().await.unwrap();
    assert_eq!(snap.events.iter().filter(|e| is_session_row(e)).count(), 6, "three states of two servers, three lists");
    assert_eq!(session_lines(&snap.session_events), Vec::<String>::new());

    // The run goes on, and its events push them out of the snapshot's events: each server's
    // last state and the run's last list come with the snapshot. The lists before it are
    // history.
    turn(&core, &id, "MANY-STEPS").await;
    let snap = core.snapshot().await.unwrap();
    assert!(!snap.events.iter().any(is_session_row), "the snapshot's events no longer hold them");
    assert_eq!(session_lines(&snap.session_events), ["docs Connected", "linear NeedsSignIn", "runs tid-1; ended tid-2"]);
    let run = agent(&core, &id).await.run_id;
    assert!(run.is_some() && snap.session_events.iter().all(|r| r.run_id == run && r.agent_id.as_deref() == Some(id.as_str())));
    let ids: Vec<i64> = snap.session_events.iter().map(|r| r.id).collect();
    assert!(ids.is_sorted() && ids.iter().all(|i| *i > 0 && *i < snap.events[0].id), "{ids:?} before {}", snap.events[0].id);

    // The process is gone, and its servers and its background work with it.
    stop(&core, &id).await;
    assert_eq!(session_lines(&core.snapshot().await.unwrap().session_events), Vec::<String>::new());
    core.shutdown().await;
}

/// Every run reports each of its MCP servers when it starts. A server that stands where the
/// agent's last row has it gets no new row: the open window still gets the event, a snapshot
/// during that run still has the server, and a server that changed gets its one row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_run_with_the_same_servers_writes_no_rows_and_still_shows_them() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (core, id) = live_agent(dir.path(), &seen).await;
    let server_rows = async || {
        let mut rows = core.events(Some(&id), None, 500).await.unwrap();
        rows.retain(|e| matches!(e.kind, EventKind::McpServer { .. }));
        rows.sort_by_key(|e| e.id);
        rows
    };

    // The first run: every state of its servers is on record.
    turn(&core, &id, "LEAVE-RUNNING").await;
    let first_run = agent(&core, &id).await.run_id;
    assert_eq!(session_lines(&server_rows().await), ["docs Connected", "linear Connecting", "linear NeedsSignIn"]);
    stop(&core, &id).await;

    // A second run, and its servers stand as the first run left them.
    let mut window = core.subscribe();
    turn(&core, &id, "SAME-SERVERS").await;
    let second_run = agent(&core, &id).await.run_id;
    assert!(second_run.is_some() && second_run != first_run);
    assert_eq!(server_rows().await.len(), 3, "no new row: nothing changed");
    // The open window got them all the same, as events that are in no log (id 0).
    let shown: Vec<EventRow> = std::iter::from_fn(|| window.try_recv().ok()).filter(|e| matches!(e.kind, EventKind::McpServer { .. })).collect();
    assert_eq!(session_lines(&shown), ["docs Connected", "linear NeedsSignIn"]);
    assert!(shown.iter().all(|e| e.id == 0 && e.run_id == second_run), "{shown:?}");
    // And a snapshot taken now has the servers of this run.
    let snap = core.snapshot().await.unwrap();
    assert_eq!(session_lines(&snap.session_events), ["docs Connected", "linear NeedsSignIn"]);
    assert!(snap.session_events.iter().all(|r| r.id == 0 && r.run_id == second_run && r.agent_id.as_deref() == Some(id.as_str())));
    assert!(!snap.events.iter().any(|e| e.run_id == second_run && matches!(e.kind, EventKind::McpServer { .. })));

    // A server that changes its state gets exactly one row, and the snapshot's events hold it.
    turn(&core, &id, "DOCS-FAILS").await;
    let rows = server_rows().await;
    assert_eq!(session_lines(&rows[3..]), ["docs Failed"]);
    assert_eq!(rows[3].run_id, second_run);
    let snap = core.snapshot().await.unwrap();
    assert_eq!(session_lines(&snap.session_events), ["linear NeedsSignIn"], "the server that did not change");
    // The same state again, in the same run: said once.
    turn(&core, &id, "DOCS-FAILS").await;
    assert_eq!(server_rows().await.len(), 4);
    stop(&core, &id).await;
    core.shutdown().await;
}
