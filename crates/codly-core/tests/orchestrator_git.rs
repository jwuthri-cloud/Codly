//! git.rs against throwaway repositories.

use std::path::{Path, PathBuf};
use std::process::Command;

use codly_core::git::{Git, MergeOutcome};

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo on `main` with one commit; returns (tempdir guard, repo path).
fn repo(identity: bool) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    if identity {
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
    }
    std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "init"]);
    (dir, repo)
}

#[tokio::test]
async fn reads() {
    let (_d, repo) = repo(true);
    let git = Git::from_process_env();
    assert!(git.is_repo(&repo).await);
    assert!(!git.is_repo(repo.parent().unwrap()).await);
    assert_eq!(std::fs::canonicalize(git.toplevel(&repo.join(".")).await.unwrap()).unwrap(), std::fs::canonicalize(&repo).unwrap());
    assert_eq!(git.current_branch(&repo).await.unwrap().as_deref(), Some("main"));
    assert_eq!(git.default_branch(&repo).await.unwrap(), "main");
    let head = git.head(&repo).await.unwrap().unwrap();
    assert_eq!(head.len(), 40);
    assert!(!git.dirty(&repo, true).await.unwrap());
    std::fs::write(repo.join("untracked.txt"), "x").unwrap();
    assert!(git.dirty(&repo, true).await.unwrap());
    assert!(!git.dirty(&repo, false).await.unwrap(), "untracked files alone are not tracked changes");
    std::fs::write(repo.join("a.txt"), "changed").unwrap();
    assert!(git.dirty(&repo, false).await.unwrap());
    assert_eq!(git.ls_files(&repo).await.unwrap(), vec!["a.txt"]);
}

#[tokio::test]
async fn worktree_commit_merge_cycle() {
    let (d, repo) = repo(false); // no identity: Codly's fallback identity must kick in
    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    let git = Git::new("git".into(), env);
    let base = git.head(&repo).await.unwrap().unwrap();
    let wt_root = d.path().join("worktrees");

    git.create_branch(&repo, "codly/p/integration", &base).await.unwrap();
    let integ = wt_root.join("integration");
    git.worktree_add(&repo, &integ, "codly/p/integration", &base).await.unwrap();
    // Idempotent re-add.
    git.worktree_add(&repo, &integ, "codly/p/integration", &base).await.unwrap();

    let t1 = wt_root.join("t1");
    git.worktree_add(&repo, &t1, "codly/p/t1", "codly/p/integration").await.unwrap();
    assert_eq!(git.commit_all(&t1, "nothing").await.unwrap(), None, "no changes → no commit");

    std::fs::write(t1.join("new.txt"), "hello\n").unwrap();
    std::fs::write(t1.join("a.txt"), "ONE\ntwo\nthree\n").unwrap();
    let c = git.commit_all(&t1, "codly(t1): snapshot").await.unwrap().unwrap();
    assert_eq!(c.files, vec!["a.txt", "new.txt"]);
    let author = sh(&t1, &["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(author, "Codly <codly@localhost>");
    assert_eq!(git.changed_files(&t1, "codly/p/integration").await.unwrap(), vec!["a.txt", "new.txt"]);
    assert!(git.diff(&repo, &base, "codly/p/t1").await.unwrap().contains("+hello"));

    // Removing an unmerged worktree is refused; the branch is never deleted.
    let err = git.worktree_remove(&repo, &t1, Some("codly/p/integration"), false).await.unwrap_err();
    assert!(err.to_string().contains("not merged"), "{err}");

    let check = git.merge_tree_check(&repo, "codly/p/integration", "codly/p/t1").await.unwrap();
    assert!(check.clean, "{check:?}");
    let MergeOutcome::Merged { sha } = git.merge(&integ, "codly/p/t1", "codly(t1): merge", true).await.unwrap() else {
        panic!("expected a merge")
    };
    assert_eq!(git.head(&integ).await.unwrap().unwrap(), sha);
    assert!(matches!(git.merge(&integ, "codly/p/t1", "again", true).await.unwrap(), MergeOutcome::AlreadyMerged { .. }));

    git.worktree_remove(&repo, &t1, Some("codly/p/integration"), false).await.unwrap();
    assert!(!t1.exists());
    assert!(git.branch_exists(&repo, "codly/p/t1").await.unwrap());
    let wts = git.worktrees(&repo).await.unwrap();
    assert_eq!(wts.len(), 2, "{wts:?}");
    assert!(wts.iter().any(|w| w.branch.as_deref() == Some("codly/p/integration")));
}

#[tokio::test]
async fn conflicts_are_detected_aborted_and_resolvable() {
    let (d, repo) = repo(true);
    let git = Git::from_process_env();
    let base = git.head(&repo).await.unwrap().unwrap();
    let root = d.path().join("wt");
    let integ = root.join("integration");
    git.worktree_add(&repo, &integ, "integ", &base).await.unwrap();
    let a = root.join("a");
    let b = root.join("b");
    git.worktree_add(&repo, &a, "task-a", "integ").await.unwrap();
    git.worktree_add(&repo, &b, "task-b", "integ").await.unwrap();
    std::fs::write(a.join("a.txt"), "A\ntwo\nthree\n").unwrap();
    std::fs::write(b.join("a.txt"), "B\ntwo\nthree\n").unwrap();
    git.commit_all(&a, "a").await.unwrap().unwrap();
    git.commit_all(&b, "b").await.unwrap().unwrap();
    assert!(matches!(git.merge(&integ, "task-a", "merge a", true).await.unwrap(), MergeOutcome::Merged { .. }));

    let check = git.merge_tree_check(&repo, "integ", "task-b").await.unwrap();
    assert!(!check.clean);
    assert_eq!(check.conflicts, vec!["a.txt"]);
    assert!(check.messages.contains("CONFLICT"), "{check:?}");

    let head_before = git.head(&integ).await.unwrap();
    let MergeOutcome::Conflict { files, .. } = git.merge(&integ, "task-b", "merge b", true).await.unwrap() else {
        panic!("expected a conflict")
    };
    assert_eq!(files, vec!["a.txt"]);
    assert!(!git.in_merge(&integ).await, "the conflicting merge must be aborted");
    assert_eq!(git.head(&integ).await.unwrap(), head_before);
    assert!(!git.dirty(&integ, true).await.unwrap());

    // Resolution flow: Codly starts the merge in the task worktree, the agent edits, Codly commits.
    let conflicted = git.merge_for_resolution(&b, "integ").await.unwrap();
    assert_eq!(conflicted, vec!["a.txt"]);
    assert!(std::fs::read_to_string(b.join("a.txt")).unwrap().contains("<<<<<<<"));
    std::fs::write(b.join("a.txt"), "AB\ntwo\nthree\n").unwrap();
    let c = git.commit_all(&b, "resolve").await.unwrap().unwrap();
    assert_eq!(sh(&b, &["rev-list", "--parents", "-n", "1", &c.sha]).split(' ').count(), 3, "a merge commit");
    assert!(git.merge_tree_check(&repo, "integ", "task-b").await.unwrap().clean);
    assert!(matches!(git.merge(&integ, "task-b", "merge b", true).await.unwrap(), MergeOutcome::Merged { .. }));
    assert_eq!(std::fs::read_to_string(integ.join("a.txt")).unwrap(), "AB\ntwo\nthree\n");
}

#[tokio::test]
async fn dirty_worktrees_are_kept_unless_forced() {
    let (d, repo) = repo(true);
    let git = Git::from_process_env();
    let base = git.head(&repo).await.unwrap().unwrap();
    let wt = d.path().join("wt");
    git.worktree_add(&repo, &wt, "w", &base).await.unwrap();
    std::fs::write(wt.join("scratch.txt"), "wip").unwrap();
    let err = git.worktree_remove(&repo, &wt, None, false).await.unwrap_err();
    assert!(err.to_string().contains("uncommitted"), "{err}");
    assert!(wt.exists());
    git.worktree_remove(&repo, &wt, None, true).await.unwrap();
    assert!(!wt.exists());
    // A deleted-but-registered worktree is pruned and can be re-added.
    git.worktree_add(&repo, &wt, "w", &base).await.unwrap();
    std::fs::remove_dir_all(&wt).unwrap();
    git.worktree_add(&repo, &wt, "w", &base).await.unwrap();
    assert!(wt.join("a.txt").exists());
}

#[tokio::test]
async fn user_hooks_do_not_run_on_codly_commits() {
    let (_d, repo) = repo(true);
    let git = Git::from_process_env();
    let hook = repo.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    assert!(git.commit_all(&repo, "snapshot").await.unwrap().is_some());
}
