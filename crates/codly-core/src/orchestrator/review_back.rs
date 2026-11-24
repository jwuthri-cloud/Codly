//! Review comments on a finished plan become one more task of that plan.
//!
//! You read a finished plan's diff in the Changes room, write comments and press Send back. The
//! plan's branch, its worktree, its checks and the agents that wrote the files are all still
//! there, so the comments do not start a new request: they become one more task of the plan
//! (`send_back_plan`). Like a task that resolves a conflict with your branch
//! (`merge_conflict.rs`), it works in place, on the plan's own branch in the plan's worktree,
//! goes through the gate with the plan's checks, and lets the plan finish again. No merge is
//! made: what it changed is its own commits, and its reviewer reads those (`review_note`). If it
//! does not finish, the plan goes back to exactly where it was (`settle_resolution`).
//!
//! It goes to the agent that wrote most of the files the comments are on, while that agent is
//! still on the team and not in your terminal; else the scheduler picks one, as for every task.
//! It starts a new conversation in the plan's worktree: the agent's own one lives in another.
//!
//! A plan that is merged, kept, cancelled, not finished, or has a pull request is refused: its
//! comments are a new request, and the refusal says so (`NEW_REQUEST`).

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::json;

use super::merge_conflict::{self, NOT_COMMITTED, REVIEW, next_key};
use super::pull_request::{PrSource, PrState};
use super::Inner;
use crate::git::name_list;
use crate::model::*;

/// How a refusal ends when the comments belong in a new request. The Changes room opens one
/// then (src/lib/state/actions.ts, `NEW_REQUEST`).
const NEW_REQUEST: &str = "ask for the changes in a new request";

fn review_goal(comments: &str) -> String {
    format!(
        "The plan is finished, and the user reviewed its work. This worktree holds the plan's finished work, on the plan's \
         own branch. Make the changes the user asks for below, here, and keep the rest of the plan's work as it is. Then \
         make sure the project still builds and the verify commands pass.\n\
         Your write scope is the files the plan changed. If a change needs another file, make that change and name the file \
         in scope_exceptions in your report. If you do not make a change the user asked for, say why in your report.\n\
         Do not run git commands that change the repository (commit, merge, checkout, reset, stash): Codly commits your work \
         when you report. {NOT_COMMITTED}\n\n{comments}"
    )
}

/// The task that makes the changes you asked for in review, in the plan's own worktree.
/// `scope`: the files the plan changed; `assignee`: the agent that wrote the files the comments
/// are on, if it can take it.
#[allow(clippy::too_many_arguments)]
fn review_task(plan: &Plan, key: String, comments: &str, scope: Vec<String>, verify: Vec<String>, assignee: Option<Id>, branch: &str, wt: &Path) -> Task {
    let now = now_ms();
    Task {
        id: new_id(),
        plan_id: Some(plan.id.clone()),
        key,
        title: "Changes you asked for in review".into(),
        goal: review_goal(comments),
        acceptance: vec!["Each review comment is done, or the report says why not".into(), "The plan's verify commands pass".into()],
        write_scope: scope,
        verify,
        depends_on: vec![],
        status: TaskStatus::Pending,
        assignee,
        agent_hint: None,
        account_hint: None,
        consultant: None,
        // It works in the plan's worktree: the agent's own conversation lives in another.
        fresh_session: true,
        branch: Some(branch.into()),
        worktree: Some(wt.to_string_lossy().into_owned()),
        attempt: 0,
        status_reason: None,
        summary: None,
        created_at: now,
        updated_at: now,
    }
}

impl Inner {
    /// A plan whose review comments can become one more task of it: finished, not merged, not
    /// kept or cancelled, with its own branch and worktree, and no pull request open for it.
    /// Every refusal is a sentence for the user.
    fn plan_to_send_back(&self, plan_id: &str) -> Result<Plan> {
        let plan = self.plan(plan_id)?;
        if plan.merged_sha.is_some() {
            bail!("this plan is merged into your branch; {NEW_REQUEST}");
        }
        if plan.kept {
            bail!("you kept this plan on its own branch; {NEW_REQUEST}");
        }
        match plan.status {
            PlanStatus::Done => {}
            PlanStatus::Running | PlanStatus::Integrating => bail!("the plan is still running; send your comments again when it is done"),
            PlanStatus::Failed => bail!("the plan's checks failed when it finished, so it is not done; {NEW_REQUEST}"),
            PlanStatus::Cancelled => bail!("this plan was cancelled; {NEW_REQUEST}"),
            PlanStatus::Planning | PlanStatus::AwaitingApproval => bail!("the plan never ran, so there is no work to change"),
        }
        // As last read: a pull request you merged on its host and did not refresh reads as open.
        match self.pull_request(&PrSource::Plan(plan.id.clone()))?.map(|pr| pr.state) {
            Some(PrState::Open) => bail!("this plan has an open pull request: {NEW_REQUEST}"),
            Some(PrState::Merged) => bail!("this plan's pull request is merged: {NEW_REQUEST}"),
            Some(PrState::Closed) | None => {}
        }
        if plan.integration_branch.is_none() || plan.integration_worktree.is_none() {
            bail!("the plan has no branch and worktree of its own any more; {NEW_REQUEST}");
        }
        Ok(plan)
    }

    /// Your review comments on a finished plan (`comments`: the message Send back built;
    /// `files`: the files they are on) become one more task of the plan, on its own branch, in
    /// its worktree. Returns the new task.
    pub(super) async fn send_back_plan(&self, plan_id: &str, comments: &str, files: &[String]) -> Result<Task> {
        let comments = comments.trim();
        if comments.is_empty() {
            bail!("there is no comment to send");
        }
        let plan = self.plan_to_send_back(plan_id)?;
        let branch = plan.integration_branch.clone().ok_or_else(|| anyhow!("the plan has no integration branch"))?;
        let wt = PathBuf::from(plan.integration_worktree.clone().ok_or_else(|| anyhow!("the plan has no integration worktree"))?);
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        // The plan's worktree, back if a cleanup removed it, with the project's dependencies.
        {
            let _g = self.repo_lock.lock().await;
            if self.git.rev_parse(&repo, &branch).await?.is_none() {
                bail!("the plan's branch {branch} was deleted; {NEW_REQUEST}");
            }
            let fresh = !wt.exists();
            self.git.worktree_add(&repo, &wt, &branch, &branch).await?;
            if fresh {
                self.mark_new_worktree(&wt)?;
            }
        }
        self.prepare_worktree(&ws, &wt, None, Some(&plan.thread)).await?;

        let _g = self.repo_lock.lock().await;
        // Read again under the lock: of two clicks, only the first makes a task. A resolution
        // that did not finish, and that the plan's last pass could not take back, goes first.
        let plan = self.settle_resolution(&self.plan_to_send_back(plan_id)?).await?;
        if let Some(open) = self.open_resolution(&plan)? {
            bail!("task {} works on this plan's branch now; send your comments again when it is done", open.key);
        }
        self.git.check_worktree(&repo, &wt).await?;
        // What is in the worktree and on no commit would become the task's work, and a task that
        // does not finish would take it away: Codly starts nothing over it.
        if let Some(what) = self.git.in_progress(&wt).await? {
            bail!("the plan's worktree ({}) is in the middle of {what}; finish or abort it there first", wt.display());
        }
        let uncommitted = self.git.changed_paths(&wt, &["HEAD"]).await?;
        if !uncommitted.is_empty() {
            bail!(
                "{} changed in the plan's worktree ({}) since its last commit; commit or undo that there first, then send your comments again",
                name_list(&uncommitted),
                wt.display()
            );
        }
        let before = self.git.head(&wt).await?.ok_or_else(|| anyhow!("{branch} has no commits"))?;
        let tasks = self.store.plan_tasks(&plan.id)?;
        let verify = self.plan_verify_commands(&plan)?;
        // Its write scope: the files the plan changed, measured from where it started, and the
        // files you commented on.
        let mut scope = match plan.base_commit.as_deref() {
            Some(base) => self.git.changed_paths(&repo, &[format!("{base}...{before}").as_str()]).await?,
            None => vec![],
        };
        for file in files {
            if !scope.contains(file) {
                scope.push(file.clone());
            }
        }
        let writer = self.wrote_most(&repo, &plan, &tasks, files).await?;
        // What worktree setup and the checks left there stays if the task does not finish.
        let untracked = self.git.untracked(&wt).await?;
        let task = review_task(&plan, next_key(&tasks, REVIEW), comments, scope, verify, writer.as_ref().map(|(a, _)| a.id.clone()), &branch, &wt);
        let ws_id = Some(ws.id.as_str());
        self.add_in_place(ws_id, &plan, &task, &before, untracked)?;
        let who = writer.as_ref().map(|(agent, wrote)| (agent.name.as_str(), wrote.key.as_str()));
        if let Err(e) = self.say_sent_back(ws_id, &plan, &task, who, comments, files) {
            // It never started: it does not wait in a finished plan. The record that would put
            // the branch back finds nothing to undo at the plan's next pass.
            if let Err(undo) = self.update_task(&task.id, |t| {
                t.status = TaskStatus::Cancelled;
                t.status_reason = Some("Codly could not start it".into());
                Ok(())
            }) {
                tracing::warn!("task {} of plan {} could not be cancelled after its start failed: {undo:#}", task.key, plan.id);
            }
            return Err(e);
        }
        drop(_g);
        self.kick();
        self.task(&task.id)
    }

    /// The records of a plan sent back with review comments, and then the plan runs again: it
    /// finishes when the new task is done. `who`: the agent the task is for, and the task of the
    /// plan it did.
    fn say_sent_back(&self, ws_id: Option<&str>, plan: &Plan, task: &Task, who: Option<(&str, &str)>, comments: &str, files: &[String]) -> Result<()> {
        let summary = match who {
            Some((name, _)) => format!("you sent the plan back with review comments: task {} for {name}", task.key),
            None => format!("you sent the plan back with review comments: task {}", task.key),
        };
        let detail = json!({ "plan_id": plan.id, "files": files, "assignee": task.assignee });
        self.decide_by(ws_id, Some(&task.id), "review", "human", summary, detail)?;
        // What you asked is in the plan's thread, where you can read it again.
        self.chat(ws_id, &plan.thread, "user", comments, None)?;
        let text = match who {
            Some((name, wrote)) => format!(
                "Your review comments are task {} of this plan, for {name} (who wrote {wrote}): it works in the plan's worktree, \
                 in a new conversation. Then the plan's checks run again and you can merge it.",
                task.key
            ),
            None => format!(
                "Your review comments are task {} of this plan: the next free agent takes it and works in the plan's worktree. \
                 Then the plan's checks run again and you can merge it.",
                task.key
            ),
        };
        self.chat(ws_id, &plan.thread, "codly", &text, None)?;
        // It builds on what it built on before: its diff is still the plan's work only.
        self.update_plan(&plan.id, |p| {
            p.status = PlanStatus::Running;
            Ok(())
        })
        .map(drop)
    }

    /// The agent that did the plan's finished task which changed most of `files` (by the
    /// commits Codly recorded for each task), and that task. None when no such task changed one
    /// of them, or its agent is gone, let go, or in your terminal: then the scheduler picks.
    async fn wrote_most(&self, repo: &Path, plan: &Plan, tasks: &[Task], files: &[String]) -> Result<Option<(Agent, Task)>> {
        let mut best: Option<(usize, &Task)> = None;
        for t in tasks.iter().filter(|t| t.status == TaskStatus::Done && merge_conflict::in_place(t, plan).is_none()) {
            let (Some(base), Some(head)) = self.store.task_commits(&t.id)? else { continue };
            let changed = match self.git.changed_paths(repo, &[base.as_str(), head.as_str()]).await {
                Ok(changed) => changed,
                Err(e) => {
                    tracing::warn!("what task {} changed cannot be read, so it is not asked to make the review's changes: {e:#}", t.key);
                    continue;
                }
            };
            let n = files.iter().filter(|f| changed.contains(f)).count();
            if n > 0 && best.is_none_or(|(most, _)| n > most) {
                best = Some((n, t));
            }
        }
        let Some((_, wrote)) = best else { return Ok(None) };
        let Some(agent) = wrote.assignee.as_deref().map(|id| self.store.agent(id)).transpose()?.flatten() else { return Ok(None) };
        if agent.archived || self.in_terminal_ids().contains(&agent.id) {
            return Ok(None);
        }
        Ok(Some((agent, wrote.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_review_task_works_in_the_plans_worktree_with_your_comments() {
        let plan: Plan = serde_json::from_value(json!({
            "id": "p1", "workspace_id": "w1", "thread": "lead", "request": "", "summary": "Add search", "assumptions": [],
            "open_questions": [], "status": "done", "base_commit": "abc", "integration_branch": "codly/p/integration",
            "integration_worktree": "/wt/integration", "created_at": 0, "updated_at": 0,
        }))
        .unwrap();
        let comments = "Review comments (1):\n\n1. `b.txt` (the whole file)\n   Say it twice.";
        let wt = Path::new("/wt/integration");
        let t = review_task(&plan, "review".into(), comments, vec!["b.txt".into()], vec!["npm test".into()], Some("a2".into()), "codly/p/integration", wt);
        assert_eq!((t.title.as_str(), t.plan_id.as_deref(), t.assignee.as_deref()), ("Changes you asked for in review", Some("p1"), Some("a2")));
        assert_eq!((t.branch.as_deref(), t.worktree.as_deref(), t.fresh_session), (Some("codly/p/integration"), Some("/wt/integration"), true));
        assert_eq!((&t.write_scope, &t.verify, t.status), (&vec!["b.txt".to_string()], &vec!["npm test".to_string()], TaskStatus::Pending));
        assert!(t.goal.starts_with("The plan is finished, and the user reviewed its work.") && t.goal.ends_with(comments), "{}", t.goal);
        assert_eq!(t.acceptance[0], "Each review comment is done, or the report says why not");
        // It works on the plan's own branch, like a resolution, and a second one gets a key of its own.
        assert!(merge_conflict::in_place(&t, &plan).is_some());
        assert_eq!(next_key(std::slice::from_ref(&t), REVIEW), "review-2");
    }
}
