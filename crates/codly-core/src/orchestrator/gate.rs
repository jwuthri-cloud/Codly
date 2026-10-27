//! The gate (agents-backend §6.7–6.8), run when a worker's turn ends with nothing queued:
//! Codly commits the worktree → runs every verify command → checks the changed files against
//! the write scope → optionally asks a reviewer of another vendor → merges, serially, into the
//! plan's integration branch. Failures go back to the same worker session as feedback, at
//! most `MAX_FIX_ROUNDS` times. Every outcome is a Decision.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::scope::ScopeMatcher;
use super::{Inner, lock, names, prompts, row, runs::RunReq, runs::new_session, scheduler};
use crate::git::MergeOutcome;
use crate::model::*;

pub const MAX_FIX_ROUNDS: u32 = 2;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(600);
const TAIL_LINES: usize = 80;

pub(super) struct VerifyOut {
    pub ok: bool,
    pub code: Option<i64>,
    pub tail: String,
}

/// Why a task goes back to its worker.
enum Fix {
    Verify { cmd: String, code: Option<i64>, tail: String },
    Scope { files: Vec<String> },
    Markers { files: Vec<String> },
    Review { summary: String, issues: Vec<String>, unmet: Vec<String> },
    Conflict { files: Vec<String>, integration: String, sha: String },
}

impl Fix {
    fn short(&self) -> String {
        match self {
            Fix::Verify { cmd, code, .. } => {
                format!("`{}` failed ({})", prompts::clip(cmd, 50), code.map_or("timeout".into(), |c| format!("exit {c}")))
            }
            Fix::Scope { files } => format!("{} file(s) outside the write scope", files.len()),
            Fix::Markers { files } => format!("conflict markers left in {}", files.join(", ")),
            Fix::Review { .. } => "the reviewer requested changes".into(),
            Fix::Conflict { files, .. } => format!("merge conflict in {}", files.join(", ")),
        }
    }
}

impl Inner {
    pub(super) async fn gate(self: Arc<Self>, task_id: Id) {
        {
            let mut g = lock(&self.gating);
            if let Some(again) = g.get_mut(&task_id) {
                *again = true; // a turn ended mid-gate: look again when this gate is done
                return;
            }
            g.insert(task_id.clone(), false);
        }
        loop {
            if let Err(e) = self.gate_inner(&task_id).await {
                self.gate_error(&task_id, e).await;
            }
            let mut g = lock(&self.gating);
            if g.get(&task_id) == Some(&true) {
                g.insert(task_id.clone(), false);
                continue;
            }
            g.remove(&task_id);
            break;
        }
        self.kick();
    }

    async fn gate_error(self: &Arc<Self>, task_id: &str, e: anyhow::Error) {
        tracing::warn!("gate for {task_id} failed: {e}");
        if self.task(task_id).is_ok_and(|t| matches!(t.status, TaskStatus::Cancelled | TaskStatus::Done)) {
            return; // cancelled (or finished) while the gate ran: nothing to report
        }
        let reason = format!("blocked: the gate failed: {e}");
        let _ = self.update_task(task_id, |t| {
            t.status = TaskStatus::Blocked;
            t.status_reason = Some(prompts::clip(&reason, 300));
            Ok(())
        });
        let ws = self.store.task_workspace(task_id).ok().flatten();
        let _ = self.decide(ws.as_deref(), Some(task_id), "gate", prompts::clip(&reason, 200), json!({ "error": e.to_string() }));
        if let (Some(ws), Ok(t)) = (ws, self.task(task_id)) {
            self.lead_followup(&ws, &self.task_thread(&t), &format!("Task {} \"{}\" is {reason}.", t.key, t.title)).await;
        }
    }

    async fn gate_inner(self: &Arc<Self>, task_id: &str) -> Result<()> {
        let task = self.task(task_id)?;
        if !matches!(task.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
            return Ok(());
        }
        let worker = task.assignee.clone().ok_or_else(|| anyhow!("the task has no worker"))?;
        if !self.is_quiet(&worker) {
            return Ok(()); // another turn started meanwhile; its end runs the gate
        }
        let turns = self.turns_done(&worker);
        let ws_id = self.store.task_workspace(task_id)?.ok_or_else(|| anyhow!("task without workspace"))?;
        let ws = Some(ws_id.as_str());
        let wt = PathBuf::from(task.worktree.as_deref().ok_or_else(|| anyhow!("the task has no worktree"))?);
        let run = self.store.latest_task_run(task_id, Some(Role::Worker))?;
        let report = run.as_ref().and_then(|r| r.report.clone()).unwrap_or(Value::Null);

        // 1. The worker says it cannot finish.
        let reported = report["status"].as_str().unwrap_or("");
        if matches!(reported, "blocked" | "failed") {
            let summary = report["summary"].as_str().unwrap_or("no details").to_string();
            let status = if reported == "blocked" { TaskStatus::Blocked } else { TaskStatus::Failed };
            let reason = format!("{reported} (worker): {}", prompts::clip(&summary, 240));
            self.update_task(task_id, |t| {
                t.status = status;
                t.status_reason = Some(reason.clone());
                t.summary = Some(summary.clone());
                Ok(())
            })?;
            self.decide(
                ws,
                Some(task_id),
                "gate",
                format!("{}: {reason}", task.key),
                json!({ "plan_id": task.plan_id, "report": report }),
            )?;
            self.release_worker(&worker, task_id);
            let who = self.agent(&worker).map(|a| a.name).unwrap_or_else(|_| "worker".into());
            self.lead_followup(&ws_id, &self.task_thread(&task), &prompts::worker_stopped(&task, reported, &who, &summary)).await;
            return Ok(());
        }

        // 2. Codly commits (agents never do).
        self.update_task(task_id, |t| {
            t.status = TaskStatus::Verifying;
            t.status_reason = Some("Codly is checking the work".into());
            Ok(())
        })?;
        let repo = PathBuf::from(self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?.repo_path);
        self.git.check_worktree(&repo, &wt).await?;
        if self.git.in_merge(&wt).await {
            let unmerged = self.git.unmerged_files(&wt).await.unwrap_or_default();
            let markers = files_with_markers(&wt, &unmerged);
            if !markers.is_empty() {
                return self.fix_round(&task, &worker, Fix::Markers { files: markers }).await;
            }
        }
        let message = format!("codly({}): {}", task.key, prompts::clip(&task.title, 60));
        let commit = {
            let _g = self.repo_lock.lock().await;
            self.git.commit_all(&wt, &message).await?
        };
        if let Some(c) = commit {
            self.store.set_task_commits(task_id, None, Some(&c.sha))?;
            self.emit(
                ws,
                row(
                    Some(&worker),
                    Some(task_id),
                    run.as_ref().map(|r| r.id.as_str()),
                    EventKind::Committed { sha: c.sha, files: c.files, message },
                ),
            )?;
        }

        // 3. Verify, in the worktree, as Codly (the worker's own claims are advisory).
        let mut results: Vec<(String, bool, Option<i64>)> = vec![];
        for cmd in &task.verify {
            let v = self.run_verify(&wt, cmd).await;
            self.emit(
                ws,
                row(
                    Some(&worker),
                    Some(task_id),
                    None,
                    EventKind::Verified { command: cmd.clone(), ok: v.ok, exit_code: v.code, tail: v.tail.clone() },
                ),
            )?;
            results.push((cmd.clone(), v.ok, v.code));
            if !v.ok {
                return self.fix_round(&task, &worker, Fix::Verify { cmd: cmd.clone(), code: v.code, tail: v.tail }).await;
            }
        }

        // 4. Scope: changed files ⊆ write_scope ∪ declared exceptions.
        let plan = task.plan_id.as_deref().map(|p| self.plan(p)).transpose()?;
        let base_ref = match plan.as_ref().and_then(|p| p.integration_branch.clone()) {
            Some(b) => b,
            None => self.store.task_commits(task_id)?.0.ok_or_else(|| anyhow!("the task has no base commit"))?,
        };
        let files = self.git.changed_files(&wt, &base_ref).await?;
        let matcher = ScopeMatcher::new(&task.write_scope).map_err(|e| anyhow!(e))?;
        let exceptions: Vec<String> = report["scope_exceptions"]
            .as_array()
            .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let outside: Vec<String> = matcher.outside(&files).into_iter().cloned().collect();
        let (declared, undeclared): (Vec<String>, Vec<String>) =
            outside.into_iter().partition(|f| exceptions.iter().any(|e| e.contains(f.as_str())));
        if !undeclared.is_empty() {
            return self.fix_round(&task, &worker, Fix::Scope { files: undeclared }).await;
        }
        let mut summary = format!(
            "{}: verify passed ({} command{}), {} file{} changed within scope",
            task.key,
            results.len(),
            if results.len() == 1 { "" } else { "s" },
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
        if !declared.is_empty() {
            summary.push_str(&format!(" plus {} declared exception(s)", declared.len()));
        }
        self.decide(
            ws,
            Some(task_id),
            "gate",
            summary,
            json!({ "plan_id": task.plan_id, "verify": results, "files": files, "declared_exceptions": declared }),
        )?;

        // 5. Optional cross-vendor review; it resumes the gate from review_done().
        if self.settings().review_gate && !files.is_empty() {
            let notes = report["notes_for_reviewer"].as_str().map(String::from);
            return self.start_review(&task, &ws_id, &base_ref, &results, &declared, notes.as_deref()).await;
        }

        // 6. Merge queue — unless the worker got a new message meanwhile (a user steer):
        // merging now would shut it down mid-turn. Its next turn end re-runs the gate.
        if !self.is_quiet(&worker) || self.turns_done(&worker) != turns {
            self.update_task(task_id, |t| {
                t.status = TaskStatus::Running;
                t.status_reason = Some("the worker got a new message; re-checking when it finishes".into());
                Ok(())
            })?;
            return Ok(());
        }
        self.merge_task(task_id, 0).await
    }

    /// Send the work back to the same worker session, or fail the task after
    /// `MAX_FIX_ROUNDS` rounds.
    async fn fix_round(self: &Arc<Self>, task: &Task, worker: &str, fix: Fix) -> Result<()> {
        let ws_id = self.store.task_workspace(&task.id)?;
        let ws = ws_id.as_deref();
        let round = {
            let mut m = lock(&self.fix_rounds);
            let r = m.entry(task.id.clone()).or_insert(0);
            *r += 1;
            *r
        };
        let what = fix.short();
        if round > MAX_FIX_ROUNDS {
            let reason = format!("failed: {what} after {MAX_FIX_ROUNDS} fix rounds");
            self.update_task(&task.id, |t| {
                t.status = TaskStatus::Failed;
                t.status_reason = Some(reason.clone());
                Ok(())
            })?;
            self.decide(
                ws,
                Some(&task.id),
                "gate",
                format!("{}: {reason}", task.key),
                json!({ "plan_id": task.plan_id, "rounds": MAX_FIX_ROUNDS }),
            )?;
            self.release_worker(worker, &task.id);
            lock(&self.fix_rounds).remove(&task.id);
            if let Some(ws) = ws_id.as_deref() {
                self.lead_followup(
                    ws,
                    &self.task_thread(task),
                    &format!(
                        "Task {} \"{}\" {reason}. The branch {} is kept.",
                        task.key,
                        task.title,
                        task.branch.as_deref().unwrap_or("?")
                    ),
                )
                .await;
            }
            return Ok(());
        }
        let (status, prompt) = match &fix {
            Fix::Verify { cmd, code, tail } => (TaskStatus::Running, prompts::verify_failed(cmd, *code, tail, round, MAX_FIX_ROUNDS)),
            Fix::Scope { files } => {
                let refs: Vec<&String> = files.iter().collect();
                (TaskStatus::Running, prompts::scope_violation(&refs, &task.write_scope, round, MAX_FIX_ROUNDS))
            }
            Fix::Markers { files } => (TaskStatus::Running, prompts::conflict_markers(files, round, MAX_FIX_ROUNDS)),
            Fix::Review { summary, issues, unmet } => {
                (TaskStatus::ChangesRequested, prompts::review_feedback(summary, issues, unmet, round, MAX_FIX_ROUNDS))
            }
            Fix::Conflict { files, integration, sha } => {
                (TaskStatus::ChangesRequested, prompts::merge_conflict(files, integration, sha, round, MAX_FIX_ROUNDS))
            }
        };
        let reason = format!("fix round {round}/{MAX_FIX_ROUNDS}: {what}");
        self.update_task(&task.id, |t| {
            t.status = status;
            t.status_reason = Some(reason.clone());
            Ok(())
        })?;
        self.decide(ws, Some(&task.id), "gate", format!("{}: {reason}", task.key), json!({ "plan_id": task.plan_id, "round": round }))?;
        if self.is_live(worker) {
            self.send_prompt(worker, prompt)?;
        } else {
            // The worker exited; the next run of this task resumes its session with the feedback.
            lock(&self.pending_feedback).insert(task.id.clone(), prompt);
            self.update_task(&task.id, |t| {
                t.status = TaskStatus::Ready;
                Ok(())
            })?;
            self.kick();
        }
        Ok(())
    }

    /// Shut a worker's run down and free the agent for the next task.
    fn release_worker(&self, agent_id: &str, task_id: &str) {
        self.stop_run(agent_id);
        let _ = self.update_agent(agent_id, |a| {
            if a.task_id.as_deref() == Some(task_id) {
                a.task_id = None;
            }
        });
    }

    /// Run one verify command in a worktree: `/bin/sh -c`, scrubbed env, own process group,
    /// 10 minute timeout, last 80 lines of combined output kept.
    pub(super) async fn run_verify(&self, wt: &Path, cmd: &str) -> VerifyOut {
        verify_in(wt, cmd, verify_env(&self.base_env), VERIFY_TIMEOUT).await
    }

    // ───────────── review ─────────────

    async fn start_review(
        self: &Arc<Self>,
        task: &Task,
        ws_id: &str,
        base_ref: &str,
        results: &[(String, bool, Option<i64>)],
        declared: &[String],
        notes: Option<&str>,
    ) -> Result<()> {
        let ws = self.store.workspace(ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let agents = self.store.agents(ws_id)?;
        let worker_provider = task.assignee.as_deref().and_then(|a| agents.iter().find(|x| x.id == a)).map(|a| a.provider);
        let settings = self.settings();
        let tasks = self.store.tasks(ws_id)?;
        let idle = |a: &Agent| {
            a.status != AgentStatus::Offline
                && !self.is_live(&a.id)
                && a.task_id.as_deref().and_then(|t| tasks.iter().find(|x| x.id == t)).is_none_or(|t| !scheduler::is_active(t.status))
        };
        // A reviewer on the team first (another vendor than the author's if possible).
        let members: Vec<&Agent> =
            agents.iter().filter(|a| a.role == Role::Reviewer && a.member.is_some() && self.cli_usable(a.provider) && idle(a)).collect();
        let member = members.iter().find(|a| Some(a.provider) != worker_provider).or_else(|| members.first()).copied();
        let order: Vec<Provider> = settings.worker_providers.iter().copied().chain(Provider::ALL).collect();
        let other = order.iter().copied().find(|p| Some(*p) != worker_provider && self.cli_usable(*p));
        let (provider, why) = match (other, worker_provider) {
            _ if member.is_some() => (member.map_or(Provider::Claude, |m| m.provider), "on the team".to_string()),
            (Some(p), Some(w)) => (p, format!("another vendor than {}", w.label())),
            (Some(p), None) => (p, "first available".to_string()),
            (None, Some(w)) if self.cli_usable(w) => (w, "no second vendor is signed in: same vendor, clean context".to_string()),
            _ => {
                self.decide(
                    Some(ws_id),
                    Some(&task.id),
                    "review",
                    format!("{}: review skipped, no reviewer CLI available", task.key),
                    json!({ "plan_id": task.plan_id }),
                )?;
                return self.merge_task(&task.id, 0).await;
            }
        };
        let free = |a: &&Agent| a.role == Role::Reviewer && a.member.is_none() && a.provider == provider && idle(a);
        let reviewer = match member.or_else(|| agents.iter().find(free)) {
            Some(a) => a.clone(),
            None => {
                let name = names::next_name(&agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>());
                let color = names::next_color(&agents.iter().map(|a| a.color).collect::<Vec<_>>());
                self.create_agent(ws_id, &name, provider, Role::Reviewer, color, None)?
            }
        };
        let branch = task.branch.clone().ok_or_else(|| anyhow!("the task has no branch"))?;
        let diff = self.git.diff(Path::new(&ws.repo_path), base_ref, &branch).await?;
        self.update_task(&task.id, |t| {
            t.status = TaskStatus::Reviewing;
            t.status_reason = Some(format!("review by {} ({})", reviewer.name, provider.label()));
            Ok(())
        })?;
        self.decide(
            Some(ws_id),
            Some(&task.id),
            "review",
            format!("{}: reviewer {} · {} ({why})", task.key, provider.label(), reviewer.name),
            json!({ "plan_id": task.plan_id, "provider": provider, "agent": reviewer.name }),
        )?;
        let wt = PathBuf::from(task.worktree.clone().unwrap_or_default());
        let req = RunReq {
            agent: reviewer.clone(),
            role: Role::Reviewer,
            cwd: wt,
            read_only: true,
            task_id: Some(task.id.clone()),
            // Reviewers run in the author's worktree, so even a team member starts fresh.
            session: new_session(provider),
            system_prompt: match self.team_member(&reviewer) {
                Some(m) => prompts::member_system(prompts::REVIEWER_SYSTEM, &m),
                None => prompts::REVIEWER_SYSTEM.to_string(),
            },
            workspace_id: Some(ws_id.to_string()),
            account: None, // a new session: the account with the most quota left
        };
        if let Err(e) = self.spawn_run(req).await {
            self.decide(
                Some(ws_id),
                Some(&task.id),
                "review",
                format!("{}: review skipped, the reviewer could not start: {e}", task.key),
                json!({ "plan_id": task.plan_id }),
            )?;
            return self.merge_task(&task.id, 0).await;
        }
        let brief = prompts::review_brief(&prompts::ReviewBrief {
            task,
            diff: &diff,
            verify: results,
            worker_notes: notes,
            scope_exceptions: declared,
        });
        self.send_prompt(&reviewer.id, brief)
    }

    /// After the reviewer's turn (or its exit): read its verdict and continue the gate.
    pub(super) async fn review_done(self: Arc<Self>, task_id: Id, run_id: Id) {
        if let Err(e) = self.review_done_inner(&task_id, &run_id).await {
            self.gate_error(&task_id, e).await;
        }
        self.kick();
    }

    async fn review_done_inner(self: &Arc<Self>, task_id: &str, run_id: &str) -> Result<()> {
        let task = self.task(task_id)?;
        if task.status != TaskStatus::Reviewing {
            return Ok(());
        }
        let run = self.store.run(run_id)?.ok_or_else(|| anyhow!("unknown reviewer run"))?;
        let reviewer = run.agent_id.clone();
        let ws = self.store.task_workspace(task_id)?;
        let review = run.report.clone().or_else(|| self.last_text(&reviewer).and_then(|t| verdict_in_text(&t)));
        let Some(review) = review else {
            let nudges = {
                let mut m = lock(&self.review_nudges);
                let n = m.entry(run_id.to_string()).or_insert(0);
                *n += 1;
                *n
            };
            if nudges <= 1 && self.is_live(&reviewer) {
                return self.send_prompt(
                    &reviewer,
                    "[Codly] You ended without a verdict. Call codly_submit_review now with your verdict, criteria and issues.".into(),
                );
            }
            lock(&self.review_nudges).remove(run_id);
            self.release_worker(&reviewer, task_id);
            self.emit(
                ws.as_deref(),
                row(
                    Some(&reviewer),
                    Some(task_id),
                    Some(run_id),
                    EventKind::Reviewed { verdict: "none".into(), summary: "the reviewer gave no verdict".into() },
                ),
            )?;
            self.decide(
                ws.as_deref(),
                Some(task_id),
                "review",
                format!("{}: no verdict from the reviewer; merging on verify and scope alone", task.key),
                json!({ "plan_id": task.plan_id }),
            )?;
            return self.merge_task(task_id, 0).await;
        };
        lock(&self.review_nudges).remove(run_id);
        self.release_worker(&reviewer, task_id);
        let r = judge(&review);
        self.emit(
            ws.as_deref(),
            row(
                Some(&reviewer),
                Some(task_id),
                Some(run_id),
                EventKind::Reviewed { verdict: r.verdict.clone(), summary: r.summary.clone() },
            ),
        )?;
        let mut summary = format!("{}: review verdict {}", task.key, r.verdict);
        if r.downgraded {
            summary.push_str(" (downgraded from approve: unmet criteria or blocker/major issues)");
        }
        self.decide(
            ws.as_deref(),
            Some(task_id),
            "review",
            summary,
            json!({ "plan_id": task.plan_id, "review": review, "downgraded": r.downgraded }),
        )?;
        let worker = task.assignee.clone().unwrap_or_default();
        match r.verdict.as_str() {
            "approve" => self.merge_task(task_id, 0).await,
            "reject" => {
                let reason = format!("failed: the reviewer rejected it: {}", prompts::clip(&r.summary, 200));
                self.update_task(task_id, |t| {
                    t.status = TaskStatus::Failed;
                    t.status_reason = Some(reason.clone());
                    Ok(())
                })?;
                self.release_worker(&worker, task_id);
                if let Some(ws) = ws {
                    self.lead_followup(&ws, &self.task_thread(&task), &prompts::review_rejected(&task, &r.summary)).await;
                }
                Ok(())
            }
            _ => self.fix_round(&task, &worker, Fix::Review { summary: r.summary, issues: r.issues, unmet: r.unmet }).await,
        }
    }

    // ───────────── merge queue ─────────────

    /// Merge one task into its plan's integration branch (serial: one merge at a time).
    /// On conflict, Codly starts merging integration into the task worktree and the worker
    /// resolves the markers; its next gate concludes that merge.
    async fn merge_task(self: &Arc<Self>, task_id: &str, depth: u8) -> Result<()> {
        let guard = self.merge_lock.lock().await;
        let task = self.task(task_id)?;
        let ws_id = self.store.task_workspace(task_id)?.ok_or_else(|| anyhow!("task without workspace"))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let Some(plan) = task.plan_id.as_deref().map(|p| self.plan(p)).transpose()? else {
            drop(guard);
            // ponytail: an ad-hoc task's result is its own branch (codly/adhoc/<key>); add a
            // "merge task" command when users want it on their branch from the board.
            return self.finish_task(&task, None).await;
        };
        if matches!(plan.status, PlanStatus::Cancelled | PlanStatus::Failed) {
            return Ok(());
        }
        let integ = plan.integration_branch.clone().ok_or_else(|| anyhow!("the plan has no integration branch"))?;
        let integ_wt = PathBuf::from(plan.integration_worktree.clone().ok_or_else(|| anyhow!("the plan has no integration worktree"))?);
        let branch = task.branch.clone().ok_or_else(|| anyhow!("the task has no branch"))?;
        self.update_task(task_id, |t| {
            t.status = TaskStatus::Merging;
            t.status_reason = Some(format!("merging into {integ}"));
            Ok(())
        })?;
        let pre = match self.git.merge_tree_check(&repo, &integ, &branch).await {
            Ok(c) if !c.clean => Some(c.conflicts),
            Ok(_) => None,
            Err(e) => {
                tracing::debug!("merge-tree pre-check unavailable ({e}); trying the merge");
                None
            }
        };
        let conflicts = match pre {
            Some(files) => files,
            None => {
                let message = format!("codly({}): {}", task.key, prompts::clip(&task.title, 60));
                let out = {
                    let _g = self.repo_lock.lock().await;
                    self.git.merge(&integ_wt, &branch, &message, true).await?
                };
                match out {
                    MergeOutcome::Merged { sha } | MergeOutcome::AlreadyMerged { sha } => {
                        let ev = self.emit(
                            Some(&ws_id),
                            row(
                                task.assignee.as_deref(),
                                Some(task_id),
                                None,
                                EventKind::Merged { sha: sha.clone(), branch: integ.clone() },
                            ),
                        )?;
                        drop(guard);
                        return self.finish_task(&task, Some((sha, ev.id))).await;
                    }
                    MergeOutcome::Conflict { files, .. } => files,
                }
            }
        };
        drop(guard);
        let sha = self.git.rev_parse(&repo, &integ).await?.unwrap_or_default();
        self.decide(
            Some(&ws_id),
            Some(task_id),
            "merge",
            format!("{}: conflicts with {integ} in {}; the worker resolves them", task.key, conflicts.join(", ")),
            json!({ "plan_id": task.plan_id, "files": conflicts, "integration_sha": sha }),
        )?;
        let wt = PathBuf::from(task.worktree.clone().ok_or_else(|| anyhow!("the task has no worktree"))?);
        let started = {
            let _g = self.repo_lock.lock().await;
            self.git.check_worktree(&repo, &wt).await?;
            self.git.merge_for_resolution(&wt, &integ).await?
        };
        if started.is_empty() && depth == 0 {
            // It merged cleanly inside the worktree after all: conclude and retry once.
            let _g = self.repo_lock.lock().await;
            self.git.commit_all(&wt, &format!("codly({}): merge {integ}", task.key)).await?;
            drop(_g);
            return Box::pin(self.merge_task(task_id, depth + 1)).await;
        }
        let worker = task.assignee.clone().unwrap_or_default();
        let files = if started.is_empty() { conflicts } else { started };
        self.fix_round(&task, &worker, Fix::Conflict { files, integration: integ, sha }).await
    }

    async fn finish_task(self: &Arc<Self>, task: &Task, merged: Option<(String, i64)>) -> Result<()> {
        let ws_id = self.store.task_workspace(&task.id)?;
        let ws = ws_id.as_deref();
        let worker = task.assignee.clone().unwrap_or_default();
        let report = self.store.latest_task_run(&task.id, Some(Role::Worker))?.and_then(|r| r.report);
        let summary = report
            .as_ref()
            .and_then(|r| r["summary"].as_str().map(String::from))
            .or_else(|| self.last_text(&worker))
            .map(|s| prompts::clip(s.trim(), 1200))
            .unwrap_or_else(|| "(no summary)".into());
        // Clean up before announcing Done: "done" then means nothing is left running.
        self.release_worker(&worker, &task.id);
        lock(&self.fix_rounds).remove(&task.id);
        lock(&self.pending_feedback).remove(&task.id);
        // A merged, clean worktree is disposable; anything else is kept.
        if merged.is_some()
            && let (Some(wt), Some(plan_id)) = (task.worktree.as_deref(), task.plan_id.as_deref())
            && let Ok(plan) = self.plan(plan_id)
            && let (Some(integ), Some(ws)) = (plan.integration_branch.as_deref(), ws.and_then(|w| self.store.workspace(w).ok().flatten()))
            && !self.is_member_worktree(&ws, Path::new(wt))
        {
            let _g = self.repo_lock.lock().await;
            if let Err(e) = self.git.worktree_remove(Path::new(&ws.repo_path), Path::new(wt), Some(integ), false).await {
                tracing::info!("keeping worktree {wt}: {e}");
            }
        }
        let task = self.update_task(&task.id, |t| {
            t.status = TaskStatus::Done;
            t.status_reason = None;
            t.summary = Some(summary.clone());
            Ok(())
        })?;
        let where_ = match &merged {
            Some((sha, _)) => format!("merged at {}", &sha[..sha.len().min(10)]),
            None => format!("done on {}", task.branch.as_deref().unwrap_or("its branch")),
        };
        self.decide(
            ws,
            Some(&task.id),
            "merge",
            format!("{}: {where_}", task.key),
            json!({ "plan_id": task.plan_id, "sha": merged.as_ref().map(|m| &m.0) }),
        )?;
        self.learn(ws, &task, &worker, &summary, merged.as_ref().map(|m| m.1));
        self.kick();
        Ok(())
    }

    /// Zero-LLM learning (memory-knowledge §7): a Summary of every finished task and, when a
    /// verify failure was later fixed, a Lesson naming the failure and the fix.
    fn learn(&self, ws: Option<&str>, task: &Task, worker: &str, summary: &str, source: Option<i64>) {
        let author = if worker.is_empty() { "codly" } else { worker };
        let text = format!("{} {}: {summary}", task.key, task.title);
        let meta = MemoryMeta { title: task.title.clone(), tags: vec![task.key.clone()], refs: vec![format!("task:{}", task.id)] };
        if let Err(e) = self.add_memory_meta(ws, MemoryScope::Project, None, MemoryKind::Summary, &text, author, source, 0.5, meta) {
            tracing::debug!("summary memory: {e}");
        }
        let verified = self.store.task_events(&task.id, "verified").unwrap_or_default();
        let failed = verified.iter().find_map(|e| match &e.kind {
            EventKind::Verified { command, ok: false, tail, .. } => Some((e.id, command.clone(), first_error_line(tail))),
            _ => None,
        });
        if let Some((event_id, cmd, err)) = failed {
            let text = format!("{}: `{cmd}` failed first ({err}); fixed by: {}", task.key, prompts::clip(summary, 400));
            let meta = MemoryMeta { title: format!("`{cmd}` failed first"), tags: vec![task.key.clone(), "verify".into()], refs: vec![format!("task:{}", task.id)] };
            if let Err(e) = self.add_memory_meta(ws, MemoryScope::Project, None, MemoryKind::Lesson, &text, author, Some(event_id), 0.6, meta) {
                tracing::debug!("lesson memory: {e}");
            }
        }
    }
}

/// `/bin/sh -c cmd` in `wt` with `env`, in its own process group; the group is killed when
/// the command ends (or times out).
async fn verify_in(wt: &Path, cmd: &str, env: Vec<(String, String)>, timeout: Duration) -> VerifyOut {
    let mut c = tokio::process::Command::new("/bin/sh");
    c.arg("-c")
        .arg(cmd)
        .current_dir(wt)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = match c.spawn() {
        Ok(c) => c,
        Err(e) => return VerifyOut { ok: false, code: None, tail: format!("could not start /bin/sh: {e}") },
    };
    let tail = Arc::new(Mutex::new(Tail::default()));
    let mut pumps = vec![];
    if let Some(out) = child.stdout.take() {
        pumps.push(tokio::spawn(pump(out, tail.clone())));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(tokio::spawn(pump(err, tail.clone())));
    }
    let pid = child.id();
    let status = tokio::time::timeout(timeout, child.wait()).await;
    // Always, not only on timeout: `npm run dev &` or a watcher the command left behind
    // would outlive the gate (and hold the output pipes open).
    if let Some(pid) = pid.filter(|p| *p > 1) {
        let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
    }
    let (ok, code) = match status {
        Ok(Ok(s)) => (s.success(), s.code().map(i64::from)),
        Ok(Err(e)) => {
            lock(&tail).push_line(format!("[codly] wait failed: {e}"));
            (false, None)
        }
        Err(_) => {
            let _ = child.wait().await; // reap the killed leader
            lock(&tail).push_line(format!("[codly] killed after {} minutes", timeout.as_secs() / 60));
            (false, None)
        }
    };
    for p in pumps {
        let _ = tokio::time::timeout(Duration::from_secs(5), p).await;
    }
    let tail = lock(&tail).text();
    VerifyOut { ok, code, tail }
}

/// The child env for verify commands: the base env minus agent/provider secrets and
/// session markers, non-interactive (CI=1 stops watch modes and prompts).
fn verify_env(base: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    const DROP_PREFIX: &[&str] = &["ANTHROPIC_", "OPENAI_", "CLAUDE_CODE_", "CLAUDE_AGENT_SDK_", "CODLY_", "CODEX_"];
    const DROP: &[&str] = &["CLAUDECODE", "GEMINI_API_KEY", "GOOGLE_API_KEY", "OPENROUTER_API_KEY"];
    let mut env: Vec<(String, String)> = base
        .iter()
        .filter(|(k, _)| !DROP.contains(&k.as_str()) && !DROP_PREFIX.iter().any(|p| k.starts_with(p)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    env.retain(|(k, _)| !matches!(k.as_str(), "CI" | "TERM" | "NO_COLOR" | "GIT_TERMINAL_PROMPT"));
    env.extend(
        [("CI", "1"), ("TERM", "dumb"), ("NO_COLOR", "1"), ("GIT_TERMINAL_PROMPT", "0")].map(|(k, v)| (k.to_string(), v.to_string())),
    );
    env
}

/// The last `TAIL_LINES` lines of a stream, each clipped, built from raw chunks so a giant
/// line without a newline cannot grow memory.
#[derive(Default)]
struct Tail {
    lines: VecDeque<String>,
    partial: Vec<u8>,
}

impl Tail {
    fn push_line(&mut self, l: String) {
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(l);
    }

    fn feed(&mut self, chunk: &[u8]) {
        for &b in chunk {
            if b == b'\n' {
                let l = String::from_utf8_lossy(&self.partial).into_owned();
                self.partial.clear();
                self.push_line(prompts::clip(&l, 400));
            } else if self.partial.len() < 2000 {
                self.partial.push(b);
            }
        }
    }

    fn text(&mut self) -> String {
        if !self.partial.is_empty() {
            let l = String::from_utf8_lossy(&std::mem::take(&mut self.partial)).into_owned();
            self.push_line(prompts::clip(&l, 400));
        }
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

async fn pump(mut r: impl tokio::io::AsyncRead + Unpin, tail: Arc<Mutex<Tail>>) {
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => lock(&tail).feed(&buf[..n]),
        }
    }
}

fn files_with_markers(wt: &Path, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(wt.join(f)).is_ok_and(|s| {
                s.lines().any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "<<<<<<<" || l == ">>>>>>>")
            })
        })
        .cloned()
        .collect()
}

fn first_error_line(tail: &str) -> String {
    let pick = tail
        .lines()
        .map(str::trim)
        .find(|l| {
            let l = l.to_ascii_lowercase();
            ["error", "fail", "panic", "assert", "exception", "cannot", "not found"].iter().any(|k| l.contains(k))
        })
        .or_else(|| tail.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .unwrap_or("no output");
    prompts::clip(pick, 160)
}

/// A reviewer that answered in text instead of calling codly_submit_review: take the last
/// JSON object in its message that has a "verdict".
fn verdict_in_text(text: &str) -> Option<Value> {
    let mut found = None;
    for (i, _) in text.match_indices('{') {
        let mut de = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
        if let Some(Ok(v)) = de.next()
            && v.get("verdict").and_then(Value::as_str).is_some()
        {
            found = Some(v);
        }
    }
    found
}

pub(super) struct Judged {
    pub verdict: String,
    pub summary: String,
    pub issues: Vec<String>,
    pub unmet: Vec<String>,
    pub downgraded: bool,
}

/// Codly's deterministic reading of a review: approve requires every criterion met and no
/// blocker or major issue; anything else is downgraded to request_changes. Nits never block.
pub(super) fn judge(review: &Value) -> Judged {
    let mut verdict = review["verdict"].as_str().unwrap_or("request_changes").to_string();
    if !matches!(verdict.as_str(), "approve" | "request_changes" | "reject") {
        verdict = "request_changes".into();
    }
    let unmet: Vec<String> = review["criteria"]
        .as_array()
        .map(|a| a.iter().filter(|c| c["met"].as_bool() == Some(false)).filter_map(|c| c["criterion"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut serious = false;
    let issues: Vec<String> = review["issues"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    let sev = i["severity"].as_str().unwrap_or("minor");
                    if sev == "nit" {
                        return None;
                    }
                    serious |= matches!(sev, "blocker" | "major");
                    let at = match (i["file"].as_str(), i["line"].as_i64()) {
                        (Some(f), Some(l)) => format!(" {f}:{l}"),
                        (Some(f), None) => format!(" {f}"),
                        _ => String::new(),
                    };
                    let fix = i["suggested_fix"].as_str().map(|f| format!(" (fix: {f})")).unwrap_or_default();
                    Some(format!("[{sev}]{at} {}{fix}", i["description"].as_str().unwrap_or("")))
                })
                .collect()
        })
        .unwrap_or_default();
    let downgraded = verdict == "approve" && (!unmet.is_empty() || serious);
    if downgraded {
        verdict = "request_changes".into();
    }
    Judged { verdict, summary: review["summary"].as_str().unwrap_or("").to_string(), issues, unmet, downgraded }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_needs_met_criteria_and_no_major_issues() {
        let ok = judge(&json!({ "verdict": "approve", "summary": "fine", "criteria": [{ "criterion": "a", "met": true }],
            "issues": [{ "severity": "nit", "description": "spacing" }] }));
        assert_eq!(ok.verdict, "approve");
        assert!(ok.issues.is_empty());
        let unmet = judge(&json!({ "verdict": "approve", "summary": "", "criteria": [{ "criterion": "a", "met": false }] }));
        assert_eq!(unmet.verdict, "request_changes");
        assert!(unmet.downgraded);
        let major = judge(
            &json!({ "verdict": "approve", "summary": "", "issues": [{ "severity": "major", "file": "x.rs", "line": 3, "description": "bug" }] }),
        );
        assert_eq!(major.verdict, "request_changes");
        assert_eq!(major.issues, vec!["[major] x.rs:3 bug"]);
    }

    /// Reviewer-verified: a verify command's background child survived the gate.
    #[tokio::test]
    async fn verify_kills_what_the_command_left_running() {
        let dir = tempfile::tempdir().unwrap();
        let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
        let started = std::time::Instant::now();
        let v = verify_in(dir.path(), "sleep 30 & echo $! > bg.pid; echo done", env.clone(), Duration::from_secs(60)).await;
        assert!(v.ok && v.tail.contains("done"), "{}", v.tail);
        assert!(started.elapsed() < Duration::from_secs(3), "the background child held the output open");
        let pid: i32 = std::fs::read_to_string(dir.path().join("bg.pid")).unwrap().trim().parse().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err(), "the background sleep survived");
        let v = verify_in(dir.path(), "sleep 30", env, Duration::from_millis(200)).await;
        assert!(!v.ok && v.code.is_none() && v.tail.contains("killed"));
    }

    #[test]
    fn verdict_from_text_and_tail_helpers() {
        let v = verdict_in_text("Done. {\"verdict\": \"approve\", \"summary\": \"ok\"} bye").unwrap();
        assert_eq!(v["verdict"], "approve");
        assert!(verdict_in_text("no json here").is_none());
        let mut t = Tail::default();
        for i in 0..100 {
            t.feed(format!("line {i}\n").as_bytes());
        }
        t.feed(b"partial");
        let text = t.text();
        assert!(text.starts_with("line 21"), "{text}");
        assert!(text.ends_with("partial"));
        assert_eq!(first_error_line("ok\nerror: boom\nmore"), "error: boom");
        assert_eq!(first_error_line("a\nb\n"), "b");
    }
}
