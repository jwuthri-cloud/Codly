//! "New conversation" (`/new` in a 1:1 chat), with a fake agent in place of the CLI: the same
//! agent's next message starts a new session, in the same worktree, with its files and its
//! checkpoints as they were, and a note about restored files that waited is gone. The thread
//! and the record say so. Refused for an agent that is not a 1:1 agent, one on a task or with a
//! stopped one, one at work and one in your terminal; a refusal changes nothing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::Store;
use tokio::sync::{Notify, mpsc};

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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
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

/// What a turn does to a file of the agent's folder.
type FileEdit = (&'static str, &'static str);

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// How each run was asked to start, in order: "new", or "resume <session id>".
    starts: Vec<String>,
    /// How many runs Codly asked to shut down.
    shutdowns: usize,
    /// What the agent's next turn writes in its folder.
    edits: Vec<FileEdit>,
    /// Knob: a turn stays open until the agent is stopped.
    turns_stay_open: bool,
    /// Knob: the next turn writes its edits only after this is notified (once Codly has the
    /// checkpoint of the files before it).
    hold: Option<Arc<Notify>>,
}

/// A fake agent whose new sessions are "sess-1", "sess-2"…, which reports how full its
/// context is, and whose turns write their edits and end at once (or stay open, with the knob).
struct Fake(Arc<Mutex<Seen>>);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let seen = self.0.clone();
        Box::pin(async move {
            let session = {
                let mut s = seen.lock().unwrap();
                match &spec.session {
                    SessionMode::Resume { session_id } => {
                        s.starts.push(format!("resume {session_id}"));
                        session_id.clone()
                    }
                    SessionMode::New { .. } => {
                        s.starts.push("new".into());
                        format!("sess-{}", s.starts.iter().filter(|start| *start == "new").count())
                    }
                }
            };
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let (agent_id, run_id, cwd) = (spec.agent_id.clone(), spec.run_id.clone(), spec.cwd.clone());
            let send = move |kind| {
                let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some(session),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let handle = AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx };
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            let (stays_open, edits, hold) = {
                                let mut s = seen.lock().unwrap();
                                s.prompts.push(text.clone());
                                (s.turns_stay_open, std::mem::take(&mut s.edits), s.hold.take())
                            };
                            send(EventKind::TurnStarted { input: Some(text) });
                            if stays_open {
                                continue;
                            }
                            if let Some(gate) = hold {
                                gate.notified().await;
                            }
                            for (file, content) in edits {
                                std::fs::write(cwd.join(file), content).unwrap();
                            }
                            let usage = Usage { input_tokens: 1200, output_tokens: 300, context_used: Some(1500), context_window: Some(200_000), ..Default::default() };
                            send(EventKind::UsageUpdated { usage });
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => {
                            seen.lock().unwrap().shutdowns += 1;
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

async fn start(data: &Path, seen: &Arc<Mutex<Seen>>, repo: &Path, team: Vec<TeamMember>) -> Arc<Core> {
    let cfg = CoreConfig { data_dir: data.to_path_buf(), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(seen.clone()))),
        env: Some(core_env()),
        clis: Some(vec![cli(Provider::Claude), cli(Provider::Codex)]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let settings = Settings { lead_followups: false, worker_providers: vec![Provider::Claude], team, ..Default::default() };
    core.update_settings(settings).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    core
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn agent_now(core: &Core, id: &str) -> Agent {
    core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == id).expect("the agent is hired")
}

/// Send a message to the agent: once Codly has the checkpoint of the files before the turn,
/// the turn writes `edits`. Then wait until it is over and Codly has committed what it wrote.
async fn say(core: &Core, seen: &Arc<Mutex<Seen>>, id: &str, text: &str, edits: &[FileEdit]) {
    let points = async || core.agent_checkpoints(id).await.unwrap().checkpoints.len();
    let had = points().await;
    let gate = Arc::new(Notify::new());
    let turns = {
        let mut s = seen.lock().unwrap();
        s.edits = edits.to_vec();
        s.hold = Some(gate.clone());
        s.prompts.len() + 1
    };
    core.send_chat(&format!("agent:{id}"), text).await.unwrap();
    wait_for("the checkpoint before the turn", async || (points().await == had + 1).then_some(())).await;
    gate.notify_one();
    let wt = PathBuf::from(agent_now(core, id).await.worktree.expect("a 1:1 agent has a worktree"));
    wait_for("the turn to end, its files committed", async || {
        let idle = agent_now(core, id).await.status == AgentStatus::Idle;
        // Without optional locks: this `status` must not hold the index while Codly commits.
        let clean = git(&wt, &["--no-optional-locks", "status", "--porcelain=v1"]).is_empty();
        (seen.lock().unwrap().prompts.len() == turns && idle && clean).then_some(())
    })
    .await;
}

/// Everything said in the agent's conversation, as (who, what).
async fn said_to(core: &Core, id: &str) -> Vec<(String, String)> {
    let chats = core.snapshot().await.unwrap().chats;
    chats.into_iter().filter(|c| c.thread == format!("agent:{id}")).map(|c| (c.author, c.text)).collect()
}

/// The records of new conversations: (the agent the row is of, who decided, the summary, its detail).
async fn records(core: &Core) -> Vec<(Option<String>, String, String, serde_json::Value)> {
    let events = core.events(None, None, 2000).await.unwrap();
    events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::Decision { decision_type, method, summary, detail, .. } if decision_type == "conversation" => Some((e.agent_id, method, summary, detail)),
            _ => None,
        })
        .collect()
}

/// The notes about restored files that wait for the agent (checkpoints.rs keeps them in kv).
fn notes(store: &Store, id: &str) -> Vec<String> {
    store.kv_prefixed(&format!("checkpoint_note:{id}:")).unwrap().into_iter().map(|(_, note)| note).collect()
}

const LINE: &str = "starts without what was said above. Files and checkpoints are as they are.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_next_message_starts_a_new_session_of_the_same_agent() {
    for provider in [Provider::Claude, Provider::Codex] {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let core = start(&dir.path().join("data"), &seen, &repo, vec![]).await;
        let id = core.hire_agent(provider, None, None).await.unwrap().id;
        say(&core, &seen, &id, "write the notes", &[("notes.txt", "mine\n")]).await;
        let before = agent_now(&core, &id).await;
        assert_eq!(before.session_id.as_deref(), Some("sess-1"), "{provider:?}");
        assert_eq!(before.usage.context_used, Some(1500));
        let wt = PathBuf::from(before.worktree.clone().unwrap());
        let points = core.agent_checkpoints(&id).await.unwrap();
        assert_eq!(points.checkpoints.len(), 1, "{provider:?}: the checkpoint before the turn");

        core.new_conversation(&id).await.unwrap();
        // Its process went through the normal stop, and its session is forgotten.
        assert_eq!(seen.lock().unwrap().shutdowns, 1, "{provider:?}");
        let after = agent_now(&core, &id).await;
        assert_eq!(after.session_id, None, "{provider:?}");
        assert_eq!(after.usage.context_used, None, "{provider:?}: the context meter is empty");
        assert_eq!((&after.worktree, &after.model, after.role, after.provider), (&before.worktree, &before.model, Role::Solo, provider));
        // The thread says so, and the record keeps the old session, so it can be found again.
        let said = said_to(&core, &id).await;
        let line = ("codly".to_string(), format!("New conversation. {} {LINE}", before.name));
        assert_eq!(said.iter().filter(|s| **s == line).count(), 1, "{provider:?}: {said:?}");
        let recorded = records(&core).await;
        assert_eq!(recorded.len(), 1, "{provider:?}");
        let (agent, method, summary, detail) = &recorded[0];
        assert_eq!((agent.as_deref(), method.as_str()), (Some(id.as_str()), "human"));
        assert_eq!(summary, &format!("you started a new conversation with {}", before.name));
        assert_eq!((detail["agent_id"].as_str(), detail["session_id"].as_str()), (Some(id.as_str()), Some("sess-1")));
        // Its files and checkpoints are as they were.
        assert_eq!(std::fs::read_to_string(wt.join("notes.txt")).unwrap(), "mine\n");
        assert_eq!(core.agent_checkpoints(&id).await.unwrap(), points, "{provider:?}");

        // The next message starts a new session, in the same worktree.
        say(&core, &seen, &id, "next thing", &[]).await;
        assert_eq!(seen.lock().unwrap().starts, ["new", "new"], "{provider:?}");
        let now = agent_now(&core, &id).await;
        assert_eq!((now.session_id.as_deref(), now.worktree.as_deref()), (Some("sess-2"), before.worktree.as_deref()), "{provider:?}");
        shut_down(&core).await;
    }
}

/// A note about restored files speaks of turns the old conversation had: it does not reach the
/// new one. The restore itself, and the way back from it, stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_about_restored_files_does_not_reach_the_new_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let core = start(&dir.path().join("data"), &seen, &repo, vec![]).await;
    let store = Store::open(&dir.path().join("data/codly.sqlite")).unwrap();
    let id = core.hire_agent(Provider::Claude, None, None).await.unwrap().id;
    let wt = PathBuf::from(agent_now(&core, &id).await.worktree.unwrap());
    say(&core, &seen, &id, "add the form", &[("a.txt", "1\n")]).await;
    say(&core, &seen, &id, "now break it", &[("a.txt", "2\n")]).await;
    core.restore_checkpoint(&id, 2).await.unwrap();
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "1\n");
    assert_eq!(notes(&store, &id).len(), 1, "a note waits for the agent's next prompt");
    let points = core.agent_checkpoints(&id).await.unwrap();

    core.new_conversation(&id).await.unwrap();
    assert!(notes(&store, &id).is_empty(), "the note is gone");
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "1\n", "the restored file stays");
    assert_eq!(core.agent_checkpoints(&id).await.unwrap(), points, "the checkpoints stay, with the way back");

    say(&core, &seen, &id, "go on", &[]).await;
    let prompt = seen.lock().unwrap().prompts.last().cloned().unwrap();
    assert!(prompt.contains("go on") && !prompt.contains("The user put"), "{prompt}");
    assert_eq!(seen.lock().unwrap().starts, ["new", "new"]);
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_for_an_agent_that_is_not_free_and_a_refusal_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let team = vec![TeamMember { name: "Ana".into(), role: "Backend".into(), ..Default::default() }];
    let core = start(&dir.path().join("data"), &seen, &repo, team).await;
    let no = async |id: &str| core.new_conversation(id).await.unwrap_err().to_string();

    // Not a 1:1 agent: a team member is a worker with its own conversation.
    let ana = wait_for("the team's agent", async || core.snapshot().await.unwrap().agents.into_iter().find(|a| a.member.is_some())).await;
    assert_eq!(no(&ana.id).await, "only a 1:1 agent can start a new conversation");
    assert!(no("nope").await.contains("unknown agent nope"));

    let ada = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    say(&core, &seen, &ada.id, "hello", &[]).await;
    // Kept, whatever is refused.
    let unchanged = async || {
        let now = agent_now(&core, &ada.id).await;
        assert_eq!(now.session_id.as_deref(), Some("sess-1"));
        assert!(!said_to(&core, &ada.id).await.iter().any(|(_, text)| text.ends_with(LINE)));
        assert!(records(&core).await.is_empty());
    };

    // In your terminal: its session is open there.
    wait_for("Ada to be between turns", async || (core.agent_takeover(&ada.id).await.unwrap().blocked.is_none()).then_some(())).await;
    core.take_over_agent(&ada.id).await.unwrap().takeover.expect("Ada is in your terminal");
    assert_eq!(no(&ada.id).await, format!("{} is in your terminal: give it back first", ada.name));
    unchanged().await;
    core.give_back_agent(&ada.id).await.unwrap();

    // At work: its turn runs, and nothing stops it.
    seen.lock().unwrap().turns_stay_open = true;
    core.send_chat(&format!("agent:{}", ada.id), "a long job").await.unwrap();
    wait_for("the turn to start", async || seen.lock().unwrap().prompts.iter().any(|p| p.ends_with("a long job")).then_some(())).await;
    let shutdowns = seen.lock().unwrap().shutdowns;
    assert_eq!(no(&ada.id).await, format!("{} is still working: stop its turn first", ada.name));
    assert_eq!(seen.lock().unwrap().shutdowns, shutdowns, "its turn goes on");
    unchanged().await;

    // On a task: the task has a conversation of its own, and its run goes on.
    let bo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let task = core.create_task("Write the greeting", "Create hello.txt containing hello").await.unwrap();
    core.assign_task(&task.id, Some(&bo.id)).await.unwrap();
    let on_task = format!("{} is on {}: start a new conversation when the task is done", bo.name, task.key);
    assert_eq!(no(&bo.id).await, on_task);
    wait_for("Bo's task to run", async || agent_now(&core, &bo.id).await.task_id.map(|_| ())).await;
    assert_eq!(no(&bo.id).await, on_task);
    // Stopped by you, the task is still Bo's: it waits for your Retry or your Cancel, not for its end.
    core.move_task(&task.id, TaskStatus::Blocked).await.unwrap();
    let stopped = format!("{} still has {} (blocked): retry it or cancel it first", bo.name, task.key);
    wait_for("Bo to let go of the stopped task", async || (no(&bo.id).await == stopped).then_some(())).await;

    // Stopped, Ada can start again.
    core.stop_agent(&ada.id).await.unwrap();
    wait_for("its process to end", async || agent_now(&core, &ada.id).await.run_id.is_none().then_some(())).await;
    core.new_conversation(&ada.id).await.unwrap();
    assert_eq!(agent_now(&core, &ada.id).await.session_id, None);
    shut_down(&core).await;
}
