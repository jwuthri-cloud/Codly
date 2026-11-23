//! What Codly's commits leave out is always on record. `Git::commit_work` commits an agent's
//! work without the new paths that are not work: a huge new folder or file, and a copy Codly
//! made that the branch does not ignore (`git.rs`). The files stay in the worktree and a
//! merge goes on without them, so every commit Codly makes of an agent's work says what it
//! left: the gate's commit of a task and the merge it concludes there, a solo agent's commit
//! after a turn and before a merge, a push or a conflict resolution, a team member's
//! leftovers, the merge of your branch into a plan's.
//!
//! The record is a Decision of type `commit`: in the task's record, in the agent's when the
//! work is no task's, in the office's for a plan's branch. It is said once per task (or agent)
//! and worktree, and again when the paths change. kv `left_out:<task or agent>:<worktree>`
//! holds the paths said last: while there are any, and for a task until it is done.
//!
//! Who else hears it is the gate's choice (`gate.rs`): the task's worker, once, while a turn
//! of its own can still change what is merged; the Lead, when no worker turn follows.

use std::path::Path;

use anyhow::Result;
use serde_json::json;

use super::{Inner, prompts, row};
use crate::git::{Commit, LeftOut};
use crate::model::*;

/// `owner`: the task's id, the agent's when the work is no task's, empty for a plan's branch.
fn key(owner: &str, wt: &Path) -> String {
    format!("left_out:{owner}:{}", wt.display())
}

/// What you can do about the paths in `left`, for the record. A copy of Codly's is not to
/// delete or to commit: the agent's commands in the worktree need it, and it can hold your
/// local settings.
fn what_to_do(left: &[LeftOut]) -> &'static str {
    let copied = left.iter().filter(|l| matches!(l, LeftOut::Copied { .. })).count();
    if copied == 0 {
        "It is still in the worktree: add it to .gitignore or delete it, or commit it yourself if it is work."
    } else if copied == left.len() {
        "It is still in the worktree, and the agent's commands need it there: add it to .gitignore, and do not delete or commit it."
    } else {
        "They are still in the worktree. What Codly copied in: add it to .gitignore, and do not delete or commit it (the agent's \
         commands need it there). The rest: add it to .gitignore or delete it, or commit it yourself if it is work."
    }
}

impl Inner {
    /// `Git::commit_work` in `wt` of work that is no task's, with what it left out on record
    /// (`note_left_out`, which the other arguments are for). The commit, None when nothing
    /// changed.
    pub(super) async fn commit_noted(&self, ws: Option<&str>, agent: Option<&str>, who: &str, wt: &Path, message: &str) -> Result<Option<Commit>> {
        let snap = self.git.commit_work(wt, message).await?;
        self.note_left_out(ws, agent, None, who, wt, &snap.left_out)?;
        Ok(snap.commit)
    }

    /// Say what Codly's commit in `wt` left out (`Git::commit_work`), in the task's record,
    /// or in the agent's when it works alone. Every later commit leaves the same paths out,
    /// so it is said once per task (or agent), and again when the paths change (not when a
    /// folder only gets more files, or a file grows). True = said now.
    pub(super) fn note_left_out(&self, ws: Option<&str>, agent: Option<&str>, task: Option<&Task>, who: &str, wt: &Path, left: &[LeftOut]) -> Result<bool> {
        let key = key(task.map(|t| t.id.as_str()).or(agent).unwrap_or_default(), wt);
        let said = self.store.kv_get(&key)?;
        if left.is_empty() {
            // Nothing is left out any more: a path that comes back is news again.
            if said.is_some() {
                self.store.kv_write(&[(key, None)])?;
            }
            return Ok(false);
        }
        let paths = json!(left.iter().map(LeftOut::path).collect::<Vec<_>>());
        if said == Some(paths.to_string()) {
            return Ok(false);
        }
        self.store.kv_set(&key, &paths.to_string())?;
        let list = left.iter().map(LeftOut::to_string).collect::<Vec<_>>().join(", ");
        let summary = format!("{who}: Codly did not commit {list}. {} {}", prompts::left_out_why(left), what_to_do(left));
        let detail = json!({ "plan_id": task.and_then(|t| t.plan_id.clone()), "left_out": paths, "worktree": wt.to_string_lossy() });
        let kind = EventKind::Decision { id: new_id(), decision_type: "commit".into(), method: "rule".into(), summary, detail };
        self.emit(ws, row(agent, task.map(|t| t.id.as_str()), None, kind))?;
        Ok(true)
    }

    /// The task is done: Codly commits nothing for it any more, so what its record said last
    /// is not kept.
    pub(super) fn forget_left_out(&self, task_id: &str) -> Result<()> {
        let rows = self.store.kv_prefixed(&key(task_id, Path::new("")))?;
        self.store.kv_write(&rows.into_iter().map(|(key, _)| (key, None)).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::git::MAX_NEW_FOLDER_FILES;
    use crate::orchestrator::{Core, CoreConfig, StartOptions};

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A core with one open repo of one commit.
    async fn start(root: &Path) -> (Arc<Core>, Workspace) {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("app.txt"), "app\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        (core, ws)
    }

    /// A solo agent with a worktree on its own branch.
    async fn solo(core: &Core, ws: &Workspace, name: &str) -> (Agent, PathBuf) {
        let inner = &core.inner;
        let wt = inner.worktree_root(ws).join("solo").join(name);
        inner.git.worktree_add(Path::new(&ws.repo_path), &wt, &format!("codly/solo/{name}"), "main").await.unwrap();
        let agent = inner.create_agent(&ws.id, name, Provider::Claude, Role::Solo, 0, None).unwrap();
        let agent = inner.update_agent(&agent.id, |a| a.worktree = Some(wt.to_string_lossy().into_owned())).unwrap();
        (agent, wt)
    }

    /// A dependency folder that the repo does not ignore: one file more than a commit takes.
    fn fill(folder: &Path) {
        for i in 0..=MAX_NEW_FOLDER_FILES {
            let sub = folder.join(format!("m{}", i % 40));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join(format!("{i}.js")), "x").unwrap();
        }
    }

    /// (summary, detail) of the `commit` Decisions on record, oldest first.
    fn said(core: &Core, ws: &Workspace) -> Vec<(String, serde_json::Value)> {
        let mut events = core.inner.store.latest_events(Some(&ws.id), 300).unwrap();
        events.sort_by_key(|e| e.id);
        let commit = |e: EventRow| match e.kind {
            EventKind::Decision { decision_type, summary, detail, .. } if decision_type == "commit" => Some((summary, detail)),
            _ => None,
        };
        events.into_iter().filter_map(commit).collect()
    }

    fn rows(core: &Core) -> Vec<(String, String)> {
        core.inner.store.kv_prefixed("left_out:").unwrap()
    }

    /// Merge, from a solo agent's Git tab, commits what its last turn left. The commit leaves
    /// a dependency folder out, and the agent's record says so: once for the same folder, and
    /// no row is kept once the folder is gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_solo_agents_merge_says_what_its_commit_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let repo = PathBuf::from(&ws.repo_path);
        let (ada, wt) = solo(&core, &ws, "ada").await;
        std::fs::write(wt.join("notes.txt"), "from the agent\n").unwrap();
        fill(&wt.join("node_modules"));

        core.merge_agent(&ada.id).await.unwrap();
        assert!(repo.join("notes.txt").exists() && !repo.join("node_modules").exists(), "the work is merged, the folder is not");
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "?? node_modules/", "it stays in the worktree, untracked");
        let record = said(&core, &ws);
        assert_eq!(record.len(), 1, "{record:?}");
        assert!(record[0].0.starts_with("ada: Codly did not commit node_modules/ (2001 new files). A new folder with more than 2000 files"), "{}", record[0].0);
        assert!(record[0].0.ends_with("add it to .gitignore or delete it, or commit it yourself if it is work."), "{}", record[0].0);
        assert_eq!(record[0].1["left_out"], json!(["node_modules/"]));
        assert_eq!(rows(&core), [(key(&ada.id, &wt), "[\"node_modules/\"]".to_string())]);

        // The same folder at the next commit is not news.
        std::fs::write(wt.join("more.txt"), "more\n").unwrap();
        core.merge_agent(&ada.id).await.unwrap();
        assert!(repo.join("more.txt").exists());
        assert_eq!(said(&core, &ws).len(), 1);

        // The folder goes: nothing is kept about it, and one that comes back is said again.
        std::fs::remove_dir_all(wt.join("node_modules")).unwrap();
        std::fs::write(wt.join("last.txt"), "last\n").unwrap();
        core.merge_agent(&ada.id).await.unwrap();
        assert!(rows(&core).is_empty(), "{:?}", rows(&core));
        fill(&wt.join("node_modules"));
        let commit = core.inner.commit_noted(Some(&ws.id), Some(&ada.id), &ada.name, &wt, "codly(solo/ada): again").await.unwrap();
        assert_eq!(commit, None, "nothing but the folder is new");
        assert_eq!(said(&core, &ws).len(), 2);
        core.shutdown().await;
    }

    /// A team member's worktree goes on to the member's next task: what the last task left
    /// there is committed on its branch first, and the member's record says what that commit
    /// left out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_members_leftovers_commit_says_what_it_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        let ana = inner.create_agent(&ws.id, "Ana", Provider::Claude, Role::Worker, 0, None).unwrap();
        let ana = inner.update_agent(&ana.id, |a| a.member = Some("ana".into())).unwrap();
        let (wt, _) = inner.switch_member_worktree(&ws, &ana, "t1", "codly/p1/t1", "HEAD").await.unwrap();
        std::fs::write(wt.join("t1.txt"), "half done\n").unwrap();
        fill(&wt.join("node_modules"));

        let (_, note) = inner.switch_member_worktree(&ws, &ana, "t2", "codly/p1/t2", "HEAD").await.unwrap();
        assert!(note.as_deref().is_some_and(|n| n.contains("leftovers on codly/p1/t1")), "{note:?}");
        assert_eq!(sh(&wt, &["ls-tree", "-r", "--name-only", "codly/p1/t1"]), "app.txt\nt1.txt", "the leftovers, without the folder");
        assert_eq!(sh(&wt, &["status", "--porcelain"]), "?? node_modules/", "the folder stays, untracked");
        let record = said(&core, &ws);
        assert_eq!(record.len(), 1, "{record:?}");
        assert!(record[0].0.starts_with("Ana: Codly did not commit node_modules/ (2001 new files)."), "{}", record[0].0);
        core.shutdown().await;
    }

    /// A task's rows go when the task is done (`finish_task`); another task's and an agent's
    /// stay.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_done_task_keeps_no_row() {
        let dir = tempfile::tempdir().unwrap();
        let (core, ws) = start(dir.path()).await;
        let inner = &core.inner;
        let (ada, wt) = solo(&core, &ws, "ada").await;
        let task = |key: &str, status: TaskStatus| Task {
            id: new_id(),
            plan_id: None,
            key: key.into(),
            title: "Add notes".into(),
            goal: "Add notes".into(),
            acceptance: vec![],
            write_scope: vec![],
            verify: vec![],
            depends_on: vec![],
            status,
            assignee: None,
            agent_hint: None,
            account_hint: None,
            consultant: None,
            fresh_session: false,
            branch: Some("codly/solo/ada".into()),
            worktree: Some(wt.to_string_lossy().into_owned()),
            attempt: 0,
            status_reason: None,
            summary: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        // t1 is at its merge, the step that ends a task.
        let (t1, t2) = (task("t1", TaskStatus::Merging), task("t2", TaskStatus::Blocked));
        for t in [&t1, &t2] {
            inner.emit(Some(&ws.id), row(None, Some(&t.id), None, EventKind::TaskChanged { task: t.clone() })).unwrap();
        }
        let left = [LeftOut::Copied { path: ".env".into() }, LeftOut::File { path: "dump.bin".into(), bytes: 30 << 20 }];
        // A team member's worktree goes from task to task: each task says it in its own record.
        assert!(inner.note_left_out(Some(&ws.id), None, Some(&t1), "t1", &wt, &left).unwrap());
        assert!(inner.note_left_out(Some(&ws.id), None, Some(&t1), "t1", Path::new("/elsewhere"), &left[..1]).unwrap());
        assert!(inner.note_left_out(Some(&ws.id), None, Some(&t2), "t2", &wt, &left).unwrap());
        assert!(inner.note_left_out(Some(&ws.id), Some(&ada.id), None, "ada", &wt, &left).unwrap());
        assert!(!inner.note_left_out(Some(&ws.id), None, Some(&t1), "t1", &wt, &left).unwrap(), "said once");
        let record = said(&core, &ws);
        assert_eq!(record.len(), 4, "{record:?}");
        // Each kind in the list has its reason.
        assert!(record[0].0.starts_with("t1: Codly did not commit .env (copied in by Codly), dump.bin (30 MB). A new folder with more than"), "{}", record[0].0);
        assert!(record[0].0.contains("What Codly copied into the worktree"), "{}", record[0].0);
        assert!(!record[1].0.contains("25 MB") && record[1].0.contains("What Codly copied into the worktree"), "{}", record[1].0);
        // A copy is not to delete or to commit; a path that is too big can be both.
        assert!(record[0].0.ends_with("The rest: add it to .gitignore or delete it, or commit it yourself if it is work."), "{}", record[0].0);
        assert!(record[0].0.contains("What Codly copied in: add it to .gitignore, and do not delete or commit it"), "{}", record[0].0);
        assert!(record[1].0.ends_with("add it to .gitignore, and do not delete or commit it."), "{}", record[1].0);
        assert!(!record[1].0.contains("or delete it") && !record[1].0.contains("commit it yourself"), "{}", record[1].0);
        assert_eq!(rows(&core).len(), 4);

        inner.finish_task(&t1, None).await.unwrap();
        assert_eq!(inner.task(&t1.id).unwrap().status, TaskStatus::Done);
        let kept: Vec<String> = rows(&core).into_iter().map(|(key, _)| key).collect();
        let mut expected = vec![key(&t2.id, &wt), key(&ada.id, &wt)];
        expected.sort();
        assert_eq!(kept, expected);
        core.shutdown().await;
    }
}
