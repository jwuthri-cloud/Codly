//! Checkpoints end to end, with a fake agent in place of the CLI: the files of a 1:1 agent's
//! worktree are kept before each turn, "Restore files to this point" puts them back and can be
//! undone, the agent is told with its next prompt, a running turn refuses the restore (also the
//! turn of another agent in the same worktree), and an agent that works in the project folder
//! itself has no checkpoints at all.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::hire::{HireManifest, SCHEMA};
use codly_core::model::*;
use codly_core::orchestrator::checkpoints::CheckpointKind;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::Store;
use tokio::sync::{Notify, mpsc};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
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

/// What a turn does to a file of the agent's folder: write it, or (None) delete it.
type FileEdit = (&'static str, Option<&'static str>);

#[derive(Default)]
struct Seen {
    /// Every prompt that reached the agent, in order.
    prompts: Vec<String>,
    /// What the agent's next turn does to its files.
    edits: Vec<FileEdit>,
    /// Turns that ended.
    done: usize,
}

/// A fake agent. A turn starts, waits for the test's `go`, edits files in its folder, ends.
struct Fake {
    seen: Arc<Mutex<Seen>>,
    go: Arc<Notify>,
}

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let (seen, go) = (self.seen.clone(), self.go.clone());
        Box::pin(async move {
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
                session_id: Some("sess".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            seen.lock().unwrap().prompts.push(text.clone());
                            send(EventKind::TurnStarted { input: Some(text) });
                            go.notified().await;
                            let edits = std::mem::take(&mut seen.lock().unwrap().edits);
                            for (file, content) in edits {
                                match content {
                                    Some(c) => std::fs::write(cwd.join(file), c).unwrap(),
                                    None => std::fs::remove_file(cwd.join(file)).unwrap(),
                                }
                            }
                            seen.lock().unwrap().done += 1;
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

struct Office {
    core: Arc<Core>,
    seen: Arc<Mutex<Seen>>,
    go: Arc<Notify>,
    repo: PathBuf,
}

async fn office(dir: &Path) -> Office {
    let repo = temp_repo(dir);
    let (seen, go) = (Arc::new(Mutex::new(Seen::default())), Arc::new(Notify::new()));
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake { seen: seen.clone(), go: go.clone() })),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    // The path as Codly knows it (git's own spelling of it).
    let repo = PathBuf::from(core.open_workspace(repo.to_str().unwrap()).await.unwrap().repo_path);
    Office { core, seen, go, repo }
}

impl Office {
    /// The agent's checkpoints as (number, kind, label, files that differ now).
    async fn points(&self, id: &str) -> Vec<(u32, CheckpointKind, String, Vec<String>)> {
        let view = self.core.agent_checkpoints(id).await.unwrap();
        assert_eq!(view.off, None);
        view.checkpoints.into_iter().map(|c| (c.n, c.kind, c.label, c.files)).collect()
    }

    /// Send `text` to the agent and wait until its turn has started and Codly has the
    /// checkpoint of the files before it. The turn then waits for `finish`.
    async fn start_turn(&self, id: &str, text: &str, edits: &[FileEdit]) {
        let had = self.points(id).await.len();
        self.seen.lock().unwrap().edits = edits.to_vec();
        self.core.send_chat(&format!("agent:{id}"), text).await.unwrap();
        wait_for("the checkpoint before the turn", async || (self.points(id).await.len() == had + 1).then_some(())).await;
    }

    /// Let the running turn edit its files and end, and wait until Codly has committed them.
    async fn finish(&self, id: &str, wt: &Path) {
        let turns = self.seen.lock().unwrap().done + 1;
        self.go.notify_one();
        wait_for("the end of the turn", async || {
            let idle = self.core.snapshot().await.unwrap().agents.into_iter().any(|a| a.id == id && a.status == AgentStatus::Idle);
            // Without optional locks: this `status` must not hold the index while Codly commits.
            let clean = git(wt, &["--no-optional-locks", "status", "--porcelain=v1"]).is_empty();
            (self.seen.lock().unwrap().done == turns && idle && clean).then_some(())
        })
        .await;
    }

    async fn turn(&self, id: &str, wt: &Path, text: &str, edits: &[FileEdit]) {
        self.start_turn(id, text, edits).await;
        self.finish(id, wt).await;
    }
}

fn files(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_go_back_to_before_a_turn_and_the_agent_is_told() {
    let dir = tempfile::tempdir().unwrap();
    let o = office(dir.path()).await;
    let agent = o.core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let (id, wt) = (agent.id.clone(), PathBuf::from(agent.worktree.clone().unwrap()));
    assert!(o.points(&id).await.is_empty(), "no turn yet");

    o.turn(&id, &wt, "add the form", &[("a.txt", Some("1\n")), ("b.txt", Some("b\n"))]).await;
    o.turn(&id, &wt, "now break it", &[("a.txt", Some("2\n")), ("README.md", None), ("c.txt", Some("c\n"))]).await;
    use CheckpointKind::{Restore, Turn};
    assert_eq!(
        o.points(&id).await,
        [
            (2, Turn, "now break it".to_string(), files(&["README.md", "a.txt", "c.txt"])),
            (1, Turn, "add the form".to_string(), files(&["README.md", "a.txt", "b.txt", "c.txt"])),
        ]
    );
    // Private refs of the repository, on no branch.
    let refs = git(&o.repo, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains(&format!("refs/codly/checkpoints/{id}/1\n")) && refs.contains(&format!("refs/codly/checkpoints/{id}/2\n")), "{refs}");
    assert_eq!(git(&o.repo, &["branch", "--contains", &format!("refs/codly/checkpoints/{id}/2")]), "");

    // Restore the files to before the second turn.
    let head = git(&wt, &["rev-parse", "HEAD"]);
    let done = o.core.restore_checkpoint(&id, 2).await.unwrap();
    assert_eq!((done.files.clone(), done.undo), (files(&["README.md", "a.txt", "c.txt"]), 3));
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "1\n", "a changed file is back");
    assert_eq!(std::fs::read_to_string(wt.join("README.md")).unwrap(), "# demo\n", "a deleted file is back");
    assert!(!wt.join("c.txt").exists(), "a file made later is gone");
    assert_eq!(std::fs::read_to_string(wt.join("b.txt")).unwrap(), "b\n", "the turn before is untouched");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), head, "files only: no commit moved");
    assert!(!o.repo.join("a.txt").exists() && o.repo.join("README.md").exists(), "your own checkout is never touched");
    assert_eq!(git(&o.repo, &["--no-optional-locks", "status", "--porcelain=v1"]), "");
    let points = o.points(&id).await;
    assert_eq!(points[0], (3, Restore, "now break it".to_string(), files(&["README.md", "a.txt", "c.txt"])), "the way back is the newest point");
    assert_eq!(points[1].3, Vec::<String>::new(), "the restored point is how the files are now");

    // It is on record, and in the conversation with the agent.
    let s = o.core.snapshot().await.unwrap();
    let said = format!("Put 3 files of {} back to before “now break it”.", agent.name);
    assert!(s.chats.iter().any(|c| c.thread == format!("agent:{id}") && c.author == "codly" && c.text.starts_with(&said)), "{:?}", s.chats);
    assert!(s.events.iter().any(|e| matches!(&e.kind, EventKind::Decision { decision_type, method, .. } if decision_type == "checkpoint" && method == "human")));

    // The agent still remembers its later turns, so its next prompt says what happened: once.
    o.turn(&id, &wt, "go on", &[]).await;
    o.turn(&id, &wt, "and on", &[]).await;
    {
        let seen = o.seen.lock().unwrap();
        let note = "[Codly] The user put the files in your worktree back to how they were before the turn that started with \
                    <untrusted>now break it</untrusted>. 3 files changed: <untrusted>README.md, a.txt, c.txt</untrusted>.";
        assert!(seen.prompts[2].starts_with(note) && seen.prompts[2].contains("\n\n[User] go on"), "{}", seen.prompts[2]);
        assert!(!seen.prompts[3].contains("put the files") && seen.prompts[3].starts_with("[User] and on"), "{}", seen.prompts[3]);
        assert!(!seen.prompts[1].contains("put the files"));
    }
    assert_eq!(git(&wt, &["show", "HEAD:a.txt"]), "1\n", "Codly's next commit in the worktree records the restored files");

    // Undo: the point taken just before the restore.
    let undone = o.core.restore_checkpoint(&id, done.undo).await.unwrap();
    assert_eq!(undone.files, files(&["README.md", "a.txt", "c.txt"]));
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "2\n");
    assert!(!wt.join("README.md").exists());
    assert_eq!(std::fs::read_to_string(wt.join("c.txt")).unwrap(), "c\n");

    // A turn that runs: no restore under the agent's hands.
    o.start_turn(&id, "work for a while", &[]).await;
    let e = o.core.restore_checkpoint(&id, 1).await.unwrap_err().to_string();
    assert_eq!(e, format!("{} is still working; restore files when its turn ends", agent.name));
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "2\n", "nothing changed");
    o.finish(&id, &wt).await;
    assert!(o.core.restore_checkpoint(&id, 999).await.unwrap_err().to_string().starts_with("that point is gone"));
    assert!(o.core.agent_checkpoints("nobody").await.is_err());
    o.core.shutdown().await;
}

/// After a task went on to another worker, the first worker's row still shows the worktree the
/// second one works in. Restore is about the worktree: the first worker's points put no file
/// back under the second one's turn or under Codly's checks there, and after a restore the
/// agent that works there is told too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_restore_under_another_agent_at_work_in_the_same_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let o = office(dir.path()).await;
    let ada = o.core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let bo = o.core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = PathBuf::from(bo.worktree.clone().unwrap());
    o.turn(&bo.id, &wt, "add the form", &[("a.txt", Some("1\n"))]).await;

    // That state, made by hand: Ada's row shows Bo's worktree, and the point before Bo's
    // turn is one of hers too.
    let store = Store::open(&dir.path().join("data/codly.sqlite")).unwrap();
    let mut row = store.agent(&ada.id).unwrap().unwrap();
    row.worktree = bo.worktree.clone();
    store.with(|db| codly_core::store::save_live_agent(db, &row)).unwrap();
    git(&o.repo, &["update-ref", &format!("refs/codly/checkpoints/{}/1", ada.id), &format!("refs/codly/checkpoints/{}/1", bo.id)]);
    assert_eq!(o.points(&ada.id).await, [(1, CheckpointKind::Turn, "add the form".to_string(), files(&["a.txt"]))]);

    // Bo's turn runs: Ada is at rest, the worktree is not.
    o.start_turn(&bo.id, "go on", &[]).await;
    let e = o.core.restore_checkpoint(&ada.id, 1).await.unwrap_err().to_string();
    assert_eq!(e, format!("{} is working in this worktree; restore files when its turn ends", bo.name));
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "1\n", "nothing changed");
    o.finish(&bo.id, &wt).await;

    // Codly checks a task in that worktree, whoever it is assigned to.
    let ws = store.agent_workspace(&bo.id).unwrap().unwrap();
    let mut task = o.core.create_task("Check the form", "").await.unwrap();
    let mut set = |status| {
        task.status = status;
        task.worktree = bo.worktree.clone();
        let kind = EventKind::TaskChanged { task: task.clone() };
        store.append_event(Some(ws.as_str()), &EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: Some(task.id.clone()), run_id: None, workspace_id: None, kind }).unwrap();
    };
    set(TaskStatus::Verifying);
    let e = o.core.restore_checkpoint(&ada.id, 1).await.unwrap_err().to_string();
    assert!(e.starts_with("Codly is checking task ") && e.ends_with("; restore files when the checks are done"), "{e}");
    assert!(wt.join("a.txt").exists(), "nothing changed");
    set(TaskStatus::Cancelled);

    // At rest: the files go back, and Bo, who works there, reads of it in its next prompt.
    let done = o.core.restore_checkpoint(&ada.id, 1).await.unwrap();
    assert_eq!(done.files, files(&["a.txt"]));
    assert!(!wt.join("a.txt").exists());
    o.turn(&bo.id, &wt, "and on", &[]).await;
    let seen = o.seen.lock().unwrap();
    let told = seen.prompts.last().unwrap();
    let note = "[Codly] The user put the files in your worktree back to how they were before the turn that started with \
                <untrusted>add the form</untrusted>. 1 file changed: <untrusted>a.txt</untrusted>.";
    assert!(told.starts_with(note) && told.contains("\n\n[User] and on"), "{told}");
    drop(seen);
    o.core.shutdown().await;
}

/// An agent hired with Git isolation off edits your own checkout: Codly keeps no checkpoint
/// of it, says why, and never puts a file back there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_in_the_project_folder_has_no_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let o = office(dir.path()).await;
    let manifest = HireManifest {
        schema: SCHEMA.into(),
        name: "Ravi".into(),
        avatar: Some("moss".into()),
        color: Some(5),
        title: Some("Docs".into()),
        provider: "claude".into(),
        model: None,
        command: None,
        isolate: false,
        resume_session: None,
        description: "Tidies the docs.".into(),
        goal: "Keep the README short.".into(),
        traits: vec![],
        career: None,
        memory: None,
    };
    let agent = o.core.hire_from_manifest(manifest, false).await.unwrap();
    assert_eq!(agent.worktree.as_deref(), o.repo.to_str());

    o.seen.lock().unwrap().edits = vec![("notes.txt", Some("mine\n"))];
    o.core.send_chat(&format!("agent:{}", agent.id), "tidy up").await.unwrap();
    wait_for("the turn to start", async || (o.seen.lock().unwrap().prompts.len() == 1).then_some(())).await;
    o.go.notify_one();
    wait_for("the end of the turn", async || {
        let idle = o.core.snapshot().await.unwrap().agents.into_iter().any(|a| a.id == agent.id && a.status == AgentStatus::Idle);
        (o.seen.lock().unwrap().done == 1 && idle).then_some(())
    })
    .await;

    let view = o.core.agent_checkpoints(&agent.id).await.unwrap();
    assert!(view.checkpoints.is_empty());
    let off = view.off.unwrap();
    assert!(off.starts_with("Ravi works in your project folder itself (Git isolation is off)"), "{off}");
    let e = o.core.restore_checkpoint(&agent.id, 1).await.unwrap_err().to_string();
    assert_eq!(e, off);
    assert_eq!(std::fs::read_to_string(o.repo.join("notes.txt")).unwrap(), "mine\n");
    o.core.shutdown().await;
    assert_eq!(git(&o.repo, &["for-each-ref", "refs/codly"]), "", "no checkpoint was ever taken of your checkout");
}
