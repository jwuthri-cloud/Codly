//! The add-agent wizard's last step: a reviewed hire manifest becomes a 1:1 agent, with fake
//! agents standing in for the CLIs in a temp git repo. Checks isolation on and off, the
//! resumed session, the avatar and colour, the standing goal on every prompt, and that
//! nothing a manifest says can skip Codly's own checks.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SessionMode, SpawnSpec};
use codly_core::hire::{HireManifest, SCHEMA};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
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
    let out = Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).output().unwrap();
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

/// Records every spawn (cwd, session) and every prompt a fake agent receives.
#[derive(Default)]
struct Log {
    spawns: Vec<(PathBuf, SessionMode)>,
    prompts: Vec<String>,
}

struct FakeSpawner(Arc<Mutex<Log>>);

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let log = self.0.clone();
        Box::pin(async move {
            log.lock().unwrap().spawns.push((spec.cwd.clone(), spec.session.clone()));
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
            let send = move |kind| {
                let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("fake-session".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            log.lock().unwrap().prompts.push(text.clone());
                            send(EventKind::TurnStarted { input: Some(text) });
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        _ => {}
                    }
                }
            });
            Ok(AgentHandle { agent_id: spec.agent_id.clone(), run_id: spec.run_id.clone(), tx })
        })
    }
}

async fn start(root: &Path) -> (Arc<Core>, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(Log::default()));
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts =
        StartOptions { spawner: Some(Arc::new(FakeSpawner(log.clone()))), env: Some(core_env()), clis: Some(vec![claude()]), index: false };
    (Core::start_with(cfg, opts).await.unwrap(), log)
}

fn manifest(name: &str) -> HireManifest {
    HireManifest {
        schema: SCHEMA.into(),
        name: name.into(),
        avatar: Some("moss".into()),
        color: Some(5),
        title: Some("Docs".into()),
        provider: "claude".into(),
        model: None,
        command: None,
        isolate: true,
        resume_session: None,
        description: "Keeps the docs true.".into(),
        goal: "Never let the README drift from the code.".into(),
        traits: vec![],
        career: None,
        memory: None,
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reviewed_manifest_becomes_an_isolated_agent_with_a_standing_goal() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    // Installed deps, ignored by a committed rule: git does not put them in a new worktree.
    std::fs::write(repo.join(".gitignore"), "node_modules/\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "ignore rules"]);
    std::fs::create_dir_all(repo.join("node_modules/dep")).unwrap();
    std::fs::write(repo.join("node_modules/dep/index.js"), "module.exports = 1;\n").unwrap();
    let (core, log) = start(dir.path()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let mut m = manifest("Mina");
    m.resume_session = Some("c84b6b3f-0a5e".into());
    let a = core.hire_from_manifest(m, false).await.unwrap();
    assert_eq!((a.name.as_str(), a.role, a.color, a.look_seed.as_str(), a.title.as_deref()), ("Mina", Role::Solo, 5, "moss", Some("Docs")));
    assert_eq!(a.session_id.as_deref(), Some("c84b6b3f-0a5e"));
    let wt = PathBuf::from(a.worktree.clone().unwrap());
    assert!(wt.to_string_lossy().contains("/solo/") && wt.join("README.md").exists(), "own worktree: {}", wt.display());

    core.send_chat(&format!("agent:{}", a.id), "check the docs").await.unwrap();
    wait_until("the prompt", || !log.lock().unwrap().prompts.is_empty()).await;
    {
        let l = log.lock().unwrap();
        let p = &l.prompts[0];
        assert!(p.contains("Your standing goal: Never let the README drift") && p.contains("Your role: Keeps the docs true."), "{p}");
        assert!(p.ends_with("[User] check the docs"), "{p}");
        assert!(matches!(&l.spawns[0].1, SessionMode::Resume { session_id } if session_id == "c84b6b3f-0a5e"), "{:?}", l.spawns[0].1);
        assert!(l.spawns[0].0.to_string_lossy().contains("/solo/"));
    }
    assert!(wt.join("node_modules/dep/index.js").exists(), "the worktree is made ready at the agent's first message");
    // The goal is on every prompt, not just the first.
    core.send_chat(&format!("agent:{}", a.id), "and again").await.unwrap();
    wait_until("the second prompt", || log.lock().unwrap().prompts.len() == 2).await;
    assert!(log.lock().unwrap().prompts[1].contains("Your standing goal"));
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_isolation_the_agent_works_in_the_project_folder_and_is_told_so() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (core, log) = start(dir.path()).await;
    let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let mut m = manifest("Ravi");
    m.isolate = false;
    let a = core.hire_from_manifest(m, false).await.unwrap();
    assert_eq!(a.worktree.as_deref(), Some(ws.repo_path.as_str()));
    core.send_chat(&format!("agent:{}", a.id), "tidy up").await.unwrap();
    wait_until("the prompt", || !log.lock().unwrap().prompts.is_empty()).await;
    let l = log.lock().unwrap();
    assert_eq!(l.spawns[0].0, PathBuf::from(&ws.repo_path));
    assert!(l.prompts[0].contains("not in a separate worktree"), "{}", l.prompts[0]);
    drop(l);
    // No worktree was created for it.
    assert!(!std::fs::read_dir(dir.path().join("data")).unwrap().flatten().any(|e| e.path().join("solo").exists()));
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_core_checks_again_whatever_the_wizard_said() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (core, _log) = start(dir.path()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let mut bad = manifest("Mina");
    bad.schema = "evil@9".into();
    assert!(core.hire_from_manifest(bad, true).await.is_err());

    let mut custom = manifest("Cus");
    custom.provider = "custom".into();
    custom.command = Some("my-agent --acp".into());
    let e = core.hire_from_manifest(custom.clone(), false).await.unwrap_err().to_string();
    assert!(e.contains("custom commands"), "{e}");
    custom.command = Some("my-agent; rm -rf ~".into());
    let e = core.hire_from_manifest(custom, false).await.unwrap_err().to_string();
    assert!(e.contains("confirm"), "unconfirmed shell metacharacters stop here: {e}");

    let mut codex = manifest("Cody");
    codex.provider = "codex".into();
    let e = core.hire_from_manifest(codex, false).await.unwrap_err().to_string();
    assert!(e.contains("not installed"), "{e}");

    core.hire_from_manifest(manifest("Mina"), false).await.unwrap();
    let e = core.hire_from_manifest(manifest("mina"), false).await.unwrap_err().to_string();
    assert!(e.contains("already exists"), "{e}");
    // Nothing half-made was left behind by the refusals.
    assert_eq!(core.snapshot().await.unwrap().agents.iter().filter(|a| a.role == Role::Solo).count(), 1);
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exported_agent_comes_back_with_its_career_traits_and_memory() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (core, log) = start(dir.path()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();

    let mut m = manifest("Mina");
    m.traits = vec!["perfectionist".into(), "nightowl".into()];
    let a = core.hire_from_manifest(m, false).await.unwrap();
    assert_eq!(a.career.traits, vec!["perfectionist".to_string(), "nightowl".to_string()]);
    core.save_memory_file(&a.id, "# Mina\n- The docs build with `make docs`.\n").await.unwrap();

    // A behaviour trait reaches the prompt; a personality-only one does not.
    core.send_chat(&format!("agent:{}", a.id), "check the docs").await.unwrap();
    wait_until("the prompt", || !log.lock().unwrap().prompts.is_empty()).await;
    let p = log.lock().unwrap().prompts[0].clone();
    assert!(p.contains("Perfectionist: Before you say you are done") && !p.contains("Night owl"), "{p}");

    // Export, then import into another repo: same colleague, same record.
    let json = core.export_agent(&a.id).unwrap();
    let mut back: HireManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.schema, codly_core::hire::AGENT_SCHEMA);
    let mut career = back.career.clone().unwrap();
    career.xp = 300; // as if it had worked a while
    back.career = Some(career);
    shut_down(&core).await;

    let dir2 = tempfile::tempdir().unwrap();
    let repo2 = temp_repo(dir2.path());
    let (core2, _log2) = start(dir2.path()).await;
    core2.open_workspace(repo2.to_str().unwrap()).await.unwrap();
    let b = core2.hire_from_manifest(back, false).await.unwrap();
    assert_eq!((b.name.as_str(), b.career.rank, b.career.traits.len()), ("Mina", Rank::Senior, 2));
    assert!(core2.memory_file(&b.id).await.unwrap().text.contains("make docs"));
    shut_down(&core2).await;
}

/// What a career adds (the traits' work rules, the track record) is said once in a run, on its
/// first prompt: on every later message it only costs tokens. The standing goal stays on every
/// prompt, and a new run is told again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_career_block_is_on_the_first_prompt_of_a_run_only() {
    let dir = tempfile::tempdir().unwrap();
    let repo = temp_repo(dir.path());
    let (core, log) = start(dir.path()).await;
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let mut m = manifest("Mina");
    m.traits = vec!["perfectionist".into()];
    let a = core.hire_from_manifest(m, false).await.unwrap();
    let thread = format!("agent:{}", a.id);
    let block = "[Who you are at work]\nPerfectionist: Before you say you are done";

    core.send_chat(&thread, "check the docs").await.unwrap();
    wait_until("the first prompt", || log.lock().unwrap().prompts.len() == 1).await;
    core.send_chat(&thread, "and the changelog").await.unwrap();
    wait_until("the second prompt", || log.lock().unwrap().prompts.len() == 2).await;
    {
        let l = log.lock().unwrap();
        assert_eq!(l.spawns.len(), 1, "both messages went to one run");
        assert!(l.prompts[0].contains(block), "{}", l.prompts[0]);
        assert!(!l.prompts[1].contains("Who you are at work") && !l.prompts[1].contains("Perfectionist"), "{}", l.prompts[1]);
        assert!(l.prompts.iter().all(|p| p.contains("Your standing goal")), "the standing goal is on every prompt");
    }

    // A new run of the agent starts without it: its first prompt says it again.
    core.stop_agent(&a.id).await.unwrap();
    let mut ended = false;
    for _ in 0..200 {
        ended = core.snapshot().await.unwrap().agents.iter().any(|x| x.id == a.id && x.run_id.is_none());
        if ended {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(ended, "the stopped run ended");
    core.send_chat(&thread, "one more thing").await.unwrap();
    wait_until("the third prompt", || log.lock().unwrap().prompts.len() == 3).await;
    let l = log.lock().unwrap();
    assert_eq!(l.spawns.len(), 2, "a new run");
    assert!(l.prompts[2].contains(block), "{}", l.prompts[2]);
    drop(l);
    shut_down(&core).await;
}
