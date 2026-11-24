//! A chat row says which message of an agent it shows, with a fake agent in place of the CLI:
//! the row Codly posts for a message carries that message's item id (two equal replies are two
//! rows that name two messages), every other row carries none, and a row stored before rows
//! had the id still loads.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use serde_json::Value;
use tokio::sync::mpsc;

/// `Core::shutdown`, and every run exited when it was told to stop. A fake agent that ignores
/// the stop fails here, in its own test: else the shutdown's last resort kills every process
/// group this test process started, the check commands of the tests beside it too.
async fn shut_down(core: &Core) {
    core.shutdown().await;
    assert_eq!(core.runs_given_up(), 0, "a run outlived the shutdown: its fake did not exit when told to stop");
}

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

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
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

/// What the fake agent says in a turn that speaks.
const REPLY: &str = "All done.";
/// What it gives as the result of a turn in which it said nothing.
const RESULT: &str = "Done without a word.";

/// A fake agent: every prompt is one turn. It says `REPLY` as the message `msg-<n>` (n: the
/// turn's number), the same words each time. A prompt with "silently" in it says nothing: the
/// turn's result is its only text. It counts the turns that ended.
struct Fake(Arc<AtomicUsize>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let ended = self.0.clone();
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
                let mut n = 0;
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            n += 1;
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            let result = if text.contains("silently") {
                                RESULT
                            } else {
                                send(EventKind::Message { item_id: format!("msg-{n}"), text: REPLY.into(), phase: MessagePhase::Final });
                                REPLY
                            };
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: Some(result.into()), duration_ms: Some(1) });
                            ended.fetch_add(1, Ordering::SeqCst);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_chat_row_of_a_message_carries_its_item_id() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let ended = Arc::new(AtomicUsize::new(0));
    let cfg = CoreConfig { data_dir: dir.path().join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(ended.clone()))),
        env: Some(core_env()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let thread = format!("agent:{}", agent.id);

    // Three turns, one after the other: two that say the same words, one that says nothing.
    for (turn, text) in ["Is it done?", "Are you sure?", "Check it silently."].into_iter().enumerate() {
        core.send_chat(&thread, text).await.unwrap();
        wait_for("the turn to end", async || {
            let a = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == agent.id)?;
            (ended.load(Ordering::SeqCst) == turn + 1 && a.status == AgentStatus::Idle).then_some(())
        })
        .await;
    }

    let rows: Vec<ChatMessage> = wait_for("the three rows of the agent", async || {
        let rows: Vec<ChatMessage> = core.snapshot().await.unwrap().chats.into_iter().filter(|c| c.thread == thread).collect();
        (rows.iter().filter(|c| c.author == agent.id).count() == 3).then_some(rows)
    })
    .await;
    let of_agent = |text: &str| -> Vec<Option<&str>> {
        let mut ids: Vec<_> = rows.iter().filter(|c| c.author == agent.id && c.text == text).map(|c| c.item_id.as_deref()).collect();
        ids.sort();
        ids
    };
    // The same words twice: each row names its own message, so nothing has to compare words.
    assert_eq!(of_agent(REPLY), [Some("msg-1"), Some("msg-2")]);
    // The turn result that no message carried shows no message, and neither do your rows.
    assert_eq!(of_agent(RESULT), [None]);
    let yours: Vec<_> = rows.iter().filter(|c| c.author == "user").collect();
    assert_eq!(yours.len(), 3);
    assert!(yours.iter().all(|c| c.item_id.is_none()), "{yours:?}");

    // The chat event the UI gets says the same as the stored row.
    let events = core.events(None, None, 2000).await.unwrap();
    let mut sent: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Chat { message } if message.author == agent.id => Some(message.item_id.as_deref()),
            _ => None,
        })
        .collect();
    sent.sort();
    assert_eq!(sent, [None, Some("msg-1"), Some("msg-2")]);
    shut_down(&core).await;
}

#[test]
fn a_chat_row_stored_before_rows_had_the_item_id_still_loads() {
    let stored = r#"{"id":"c1","thread":"lead","author":"a1","text":"All done.","attachment":null,"ts":5}"#;
    let row: ChatMessage = serde_json::from_str(stored).unwrap();
    assert_eq!(row.item_id, None);
    // A row without the id is written as it always was; one with it adds the field.
    assert_eq!(serde_json::to_value(&row).unwrap(), serde_json::from_str::<Value>(stored).unwrap());
    let named = ChatMessage { item_id: Some("msg-1".into()), ..row };
    assert_eq!(serde_json::to_value(&named).unwrap()["item_id"], "msg-1");
}
