//! A 1:1 agent's whole diff for the Changes room (`Core::agent_diff`): its commits, its
//! uncommitted edits and its new files, from where its branch left yours, with the worktree and
//! its index as they were. An agent that works in your project folder itself shows its
//! uncommitted changes only, read without writing into your repository. No agent CLI runs: the
//! files are written by the test.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use codly_core::hire::{HireManifest, SCHEMA};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, StartOptions};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", &format!("write {file}")]);
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

/// A core on a repository with one commit, and the repository's path as Codly knows it.
async fn office(dir: &Path) -> (Arc<Core>, PathBuf) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    commit(&repo, "README.md", "# demo\n");
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: None, env: Some(std::env::vars().collect()), clis: Some(vec![claude()]), index: false };
    let core = Core::start_with(cfg, opts).await.unwrap();
    let repo = PathBuf::from(core.open_workspace(repo.to_str().unwrap()).await.unwrap().repo_path);
    (core, repo)
}

/// What `git status` says and the bytes of the index, read without taking the index lock.
fn state(wt: &Path) -> (String, Vec<u8>) {
    let status = git(wt, &["--no-optional-locks", "status", "--porcelain=v1", "--untracked-files=all"]);
    let index = PathBuf::from(git(wt, &["rev-parse", "--path-format=absolute", "--git-path", "index"]).trim());
    (status, std::fs::read(index).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_1_to_1_agents_diff_has_its_commits_its_edits_and_its_new_files() {
    let dir = tempfile::tempdir().unwrap();
    let (core, repo) = office(dir.path()).await;
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = PathBuf::from(agent.worktree.clone().unwrap());
    assert_eq!(core.agent_diff(&agent.id).await.unwrap(), "", "nothing changed yet");

    // Your own commit on main after the agent started is not its work.
    commit(&repo, "mine.txt", "yours\n");
    commit(&wt, "committed.txt", "from a commit\n");
    std::fs::write(wt.join("README.md"), "# demo\nedited, not committed\n").unwrap();
    std::fs::write(wt.join("new.txt"), "a new file\n").unwrap();
    let before = state(&wt);

    let diff = core.agent_diff(&agent.id).await.unwrap();
    assert!(diff.contains("+++ b/committed.txt") && diff.contains("+from a commit"), "{diff}");
    assert!(diff.contains("+++ b/README.md") && diff.contains("+edited, not committed"), "{diff}");
    assert!(diff.contains("+++ b/new.txt") && diff.contains("+a new file"), "{diff}");
    assert!(!diff.contains("mine.txt"), "your commit on main is not the agent's: {diff}");
    assert_eq!(state(&wt), before, "the worktree and its index are as they were");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_in_the_project_folder_shows_its_uncommitted_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (core, repo) = office(dir.path()).await;
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
    let agent = core.hire_from_manifest(manifest, false).await.unwrap();
    assert_eq!(agent.worktree.as_deref(), repo.to_str());

    commit(&repo, "done.txt", "committed already\n");
    commit(&repo, ".gitignore", "*.log\n");
    std::fs::write(repo.join("README.md"), "# demo\nshorter\n").unwrap();
    std::fs::create_dir_all(repo.join("notes")).unwrap();
    std::fs::write(repo.join("notes/new.md"), "a new note\n").unwrap();
    std::fs::write(repo.join("build.log"), "ignored\n").unwrap();
    let before = state(&repo);
    let objects = git(&repo, &["count-objects", "-v"]);

    let diff = core.agent_diff(&agent.id).await.unwrap();
    assert!(diff.contains("+++ b/README.md") && diff.contains("+shorter"), "{diff}");
    assert!(diff.contains("+++ b/notes/new.md") && diff.contains("+a new note"), "its new file is its work: {diff}");
    assert!(!diff.contains("done.txt"), "what HEAD has is not in it: {diff}");
    assert!(!diff.contains("build.log"), "an ignored file is not work: {diff}");
    assert_eq!(state(&repo), before, "your checkout and its index are as they were");
    assert_eq!(git(&repo, &["count-objects", "-v"]), objects, "no object was written into your repository");
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_whose_worktree_is_gone_has_an_empty_diff() {
    let dir = tempfile::tempdir().unwrap();
    let (core, repo) = office(dir.path()).await;
    let e = core.agent_diff("nope").await.unwrap_err().to_string();
    assert!(e.contains("unknown agent nope"), "{e}");
    let agent = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let wt = agent.worktree.clone().unwrap();
    git(&repo, &["worktree", "remove", "--force", &wt]);
    assert_eq!(core.agent_diff(&agent.id).await.unwrap(), "");
    core.shutdown().await;
}
