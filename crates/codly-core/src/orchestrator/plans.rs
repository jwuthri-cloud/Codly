//! Plans and tasks: the Lead's `codly_submit_plan`, running and cancelling plans, the
//! scheduler loop, starting worker runs, the user's board moves, crash recovery and plan
//! completion. Every choice is written as a Decision.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::planner::{self, PlanInput};
use super::runs::{RunReq, new_session};
use super::scheduler::{self, Action, AgentPick, SchedInput};
use super::{Inner, guard, lead, lock, names, prompts, row};
use crate::adapters::SessionMode;
use crate::memory::{self, MemoryView};
use crate::model::*;
use crate::store;

pub(super) async fn scheduler_loop(inner: Arc<Inner>) {
    loop {
        tokio::select! {
            _ = inner.kick.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        // Each pass runs as its own task so a panic fails that pass, not the loop.
        let me = inner.clone();
        match tokio::spawn(async move { me.schedule_pass().await }).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("scheduler pass failed: {e}"),
            Err(e) => tracing::error!("scheduler pass panicked: {e}"),
        }
    }
}

/// `codly/<short>/…`: a readable slug of the summary plus 4 random hex chars of the id
/// (UUIDv7's leading chars are a timestamp, so they would collide within a minute).
fn plan_short(p: &Plan) -> String {
    if let Some(s) = p.integration_branch.as_deref().and_then(|b| b.strip_prefix("codly/")).and_then(|b| b.strip_suffix("/integration")) {
        return s.to_string();
    }
    format!("{}-{}", names::slug(&p.summary, 24), &p.id[p.id.len().saturating_sub(4)..])
}

impl Inner {
    // ───────────── submit / run / cancel ─────────────

    /// `codly_submit_plan`: validate, persist Plan + Tasks + decisions atomically, post the
    /// plan card in the calling Lead's request thread, and auto-run when the user allows it.
    /// Errors go back to the Lead verbatim.
    pub(super) async fn submit_plan(self: &Arc<Self>, lead_id: &str, args: Value) -> Result<Value> {
        let ws = match self.store.agent_workspace(lead_id)? {
            Some(id) => self.store.workspace(&id)?.ok_or_else(|| anyhow!("unknown workspace"))?,
            None => self.require_ws()?,
        };
        let thread = self.agent(lead_id).map_or_else(|_| default_lead_thread(), |a| lead::thread_of(&a).to_string());
        let input: PlanInput = serde_json::from_value(args).map_err(|e| {
            anyhow!(
                "Plan rejected: the arguments do not match the schema ({e}). Send {{summary, tasks: [{{id, title, goal, \
                 acceptance_criteria, write_scope, depends_on, verify}}]}}."
            )
        })?;
        let valid = planner::validate(input).map_err(|e| anyhow!(e))?;
        let settings = self.settings();
        let now = now_ms();

        // A request's new plan replaces its own unstarted one, never another request's.
        for old in self.store.plans(&ws.id, 50)?.into_iter().filter(|p| p.status == PlanStatus::AwaitingApproval && p.thread == thread) {
            self.cancel_plan_inner(&old, "superseded by a newer plan from the Lead").await?;
        }

        let plan_id = new_id();
        let plan = Plan {
            id: plan_id.clone(),
            workspace_id: ws.id.clone(),
            thread: thread.clone(),
            request: self.store.last_chat_by(Some(&ws.id), &thread, "user")?.unwrap_or_default(),
            summary: valid.plan.summary.clone(),
            assumptions: valid.plan.assumptions.clone(),
            open_questions: valid.plan.open_questions.clone(),
            status: PlanStatus::AwaitingApproval,
            base_commit: None,
            integration_branch: None,
            integration_worktree: None,
            merged_sha: None,
            kept: false,
            created_at: now,
            updated_at: now,
        };
        let ids: HashMap<String, Id> = valid.plan.tasks.iter().map(|t| (t.id.clone(), new_id())).collect();
        let tasks: Vec<Task> = valid
            .plan
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| Task {
                id: ids[&t.id].clone(),
                plan_id: Some(plan_id.clone()),
                key: t.id.clone(),
                title: t.title.clone(),
                goal: t.goal.clone(),
                acceptance: t.acceptance_criteria.clone(),
                write_scope: t.write_scope.clone(),
                verify: t.verify.clone(),
                depends_on: t.depends_on.iter().filter_map(|d| ids.get(d).cloned()).collect(),
                status: TaskStatus::Pending,
                assignee: None,
                agent_hint: t.hint(),
                account_hint: None,
                branch: None,
                worktree: None,
                attempt: 0,
                status_reason: None,
                summary: None,
                created_at: now + i as i64,
                updated_at: now + i as i64,
            })
            .collect();
        let auto = settings.auto_run_plans && valid.plan.open_questions.is_empty();
        let plan_json = serde_json::to_value(&valid.plan)?;
        let rows = self.store.tx(|tx| {
            let mut out = vec![];
            let mut push = |r: EventRow| -> Result<()> {
                let mut r = r;
                r.id = store::insert_event(tx, Some(&ws.id), &r)?;
                out.push(r);
                Ok(())
            };
            push(row(None, None, None, EventKind::PlanChanged { plan: plan.clone() }))?;
            store::set_plan_json(tx, &plan_id, &plan_json)?;
            for t in &tasks {
                push(row(None, Some(&t.id), None, EventKind::TaskChanged { task: t.clone() }))?;
            }
            let mut decisions: Vec<(String, Value)> = valid.fixes.iter().map(|f| (f.summary.clone(), f.detail.clone())).collect();
            decisions.push((
                format!(
                    "accepted plan \"{}\": {} task{}",
                    prompts::clip(&plan.summary, 60),
                    tasks.len(),
                    if tasks.len() == 1 { "" } else { "s" }
                ),
                json!({ "rule": "validated", "tasks": tasks.iter().map(|t| &t.key).collect::<Vec<_>>(), "fixes": valid.fixes.len() }),
            ));
            if settings.auto_run_plans && !auto {
                decisions.push(("not auto-running: the plan has open questions for you".into(), json!({ "rule": "open_questions" })));
            }
            for (summary, mut detail) in decisions {
                detail["plan_id"] = json!(plan_id);
                push(row(
                    None,
                    None,
                    None,
                    EventKind::Decision { id: new_id(), decision_type: "plan_validate".into(), method: "rule".into(), summary, detail },
                ))?;
            }
            Ok(out)
        })?;
        for r in &rows {
            self.publish(Some(&ws.id), r);
        }
        let n = tasks.len();
        let text = format!(
            "Plan ready: {} ({n} task{}).{}",
            plan.summary,
            if n == 1 { "" } else { "s" },
            if auto { " Running it now." } else { " Review it on the board, then click Run plan." }
        );
        self.chat(Some(&ws.id), &thread, "codly", &text, Some(json!({ "plan_id": plan_id })))?;
        if auto {
            let me = self.clone();
            let id = plan_id.clone();
            let ws_id = ws.id.clone();
            tokio::spawn(async move {
                if let Err(e) = me.run_plan(&id).await {
                    let _ =
                        me.chat(Some(&ws_id), &thread, "codly", &format!("Could not start the plan: {e}"), Some(json!({ "plan_id": id })));
                }
            });
        }
        let key_of: HashMap<&Id, &String> = tasks.iter().map(|t| (&t.id, &t.key)).collect();
        Ok(json!({
            "plan_id": plan_id,
            "status": if auto { "running" } else { "awaiting_approval" },
            "tasks": tasks.iter().map(|t| json!({
                "id": t.key, "title": t.title,
                "depends_on": t.depends_on.iter().filter_map(|d| key_of.get(d)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "codly_adjustments": valid.fixes.iter().map(|f| &f.summary).collect::<Vec<_>>(),
            "next": if auto { "Codly is running the plan. Tell the user briefly what it will do." }
                    else { "Tell the user briefly what the plan does; they start it with Run plan." },
        }))
    }

    pub(super) async fn run_plan(self: &Arc<Self>, plan_id: &str) -> Result<()> {
        let plan = self.plan(plan_id)?;
        match plan.status {
            PlanStatus::AwaitingApproval | PlanStatus::Planning => {}
            PlanStatus::Running | PlanStatus::Integrating => return Ok(()),
            s => bail!("the plan is already {}", store::enum_str(&s)),
        }
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let short = plan_short(&plan);
        let branch = format!("codly/{short}/integration");
        let wt = self.worktree_root(&ws).join(&short).join("integration");
        let (base, fresh) = {
            let _g = self.repo_lock.lock().await;
            let head = self
                .git
                .head(&repo)
                .await?
                .ok_or_else(|| anyhow!("the repository has no commits yet; make a first commit, then run the plan"))?;
            let base = match plan.base_commit.clone() {
                Some(b) if self.git.branch_exists(&repo, &branch).await? => b,
                _ => head,
            };
            if !self.git.branch_exists(&repo, &branch).await? {
                self.git.create_branch(&repo, &branch, &base).await?;
            }
            let fresh = !wt.exists();
            self.git.worktree_add(&repo, &wt, &branch, &base).await?;
            (base, fresh)
        };
        if fresh {
            self.bootstrap_worktree(&ws, &repo, &wt, None).await; // integration checks need deps too
        }
        let wt_s = wt.to_string_lossy().into_owned();
        self.update_plan(plan_id, |p| {
            if !matches!(p.status, PlanStatus::AwaitingApproval | PlanStatus::Planning) {
                bail!("the plan was changed meanwhile");
            }
            p.status = PlanStatus::Running;
            p.base_commit = Some(base.clone());
            p.integration_branch = Some(branch.clone());
            p.integration_worktree = Some(wt_s.clone());
            Ok(())
        })?;
        let current = self.git.current_branch(&repo).await.ok().flatten().unwrap_or_else(|| "HEAD".into());
        self.decide(
            Some(&ws.id),
            None,
            "schedule",
            format!("plan started on {branch} from {current} @ {}", &base[..base.len().min(10)]),
            json!({ "plan_id": plan_id, "base": base, "integration_branch": branch, "worktree": wt_s }),
        )?;
        if self.git.dirty(&repo, false).await.unwrap_or(false) {
            self.chat(
                Some(&ws.id),
                &plan.thread,
                "codly",
                &format!(
                    "Note: your checkout has uncommitted changes. Agents work from {} and will not see them.",
                    &base[..base.len().min(10)]
                ),
                Some(json!({ "plan_id": plan_id })),
            )?;
        }
        self.kick();
        Ok(())
    }

    pub(super) async fn cancel_plan(self: &Arc<Self>, plan_id: &str) -> Result<()> {
        let plan = self.plan(plan_id)?;
        self.cancel_plan_inner(&plan, "cancelled by you").await?;
        self.kick();
        Ok(())
    }

    async fn cancel_plan_inner(&self, plan: &Plan, why: &str) -> Result<()> {
        if matches!(plan.status, PlanStatus::Done | PlanStatus::Cancelled) {
            return Ok(());
        }
        self.update_plan(&plan.id, |p| {
            p.status = PlanStatus::Cancelled;
            Ok(())
        })?;
        for t in self.store.plan_tasks(&plan.id)? {
            if matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled) {
                continue;
            }
            self.release_task_agents(&t);
            self.update_task(&t.id, |t| {
                t.status = TaskStatus::Cancelled;
                t.status_reason = Some(format!("plan {why}"));
                Ok(())
            })?;
        }
        // Branches and worktrees are kept: never delete unmerged work.
        self.decide(
            Some(&plan.workspace_id),
            None,
            "schedule",
            format!("plan \"{}\" {why}", prompts::clip(&plan.summary, 60)),
            json!({ "plan_id": plan.id }),
        )?;
        Ok(())
    }

    /// Stop the worker and reviewer runs of a task and free those agents.
    fn release_task_agents(&self, t: &Task) {
        let agents = self.store.task_workspace(&t.id).ok().flatten().and_then(|ws| self.store.agents(&ws).ok()).unwrap_or_default();
        for a in agents.iter().filter(|a| {
            a.task_id.as_deref() == Some(t.id.as_str()) || (scheduler::is_active(t.status) && t.assignee.as_deref() == Some(a.id.as_str()))
        }) {
            self.stop_run(&a.id);
            let _ = self.update_agent(&a.id, |a| a.task_id = None);
        }
        lock(&self.fix_rounds).remove(&t.id);
    }

    // ───────────── scheduling ─────────────

    /// ponytail: schedules the open workspace only; tasks already running elsewhere still
    /// finish their gates. Loop over workspaces with running plans if users juggle repos.
    async fn schedule_pass(self: &Arc<Self>) -> Result<()> {
        let Some(ws) = self.workspace() else { return Ok(()) };
        let _g = self.sched_lock.lock().await;
        let tasks = self.store.tasks(&ws.id)?;
        let plans = self.store.plans(&ws.id, 100)?;
        let agents = self.store.agents(&ws.id)?;
        let clis = self.clis();
        let settings = self.settings();
        let busy: HashSet<Id> = lock(&self.live).keys().cloned().collect();
        let accounts = self.account_states();
        let actions = scheduler::schedule(&SchedInput {
            tasks: &tasks,
            plans: &plans,
            agents: &agents,
            clis: &clis,
            settings: &settings,
            busy: &busy,
            accounts: &accounts,
        });
        let by_id: HashMap<&str, &Task> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
        for a in actions {
            match a {
                Action::MakeReady { task_id } => {
                    self.update_task(&task_id, |t| {
                        if t.status == TaskStatus::Pending {
                            t.status = TaskStatus::Ready;
                            t.status_reason = None;
                        }
                        Ok(())
                    })?;
                }
                Action::Wait { task_id, reason, detail } => {
                    if by_id.get(task_id.as_str()).and_then(|t| t.status_reason.as_deref()) != Some(reason.as_str()) {
                        self.update_task(&task_id, |t| {
                            t.status_reason = Some(reason.clone());
                            Ok(())
                        })?;
                        self.decide(Some(&ws.id), Some(&task_id), "schedule", reason, detail)?;
                    }
                }
                Action::Start { task_id, provider, account, agent, summary, detail } => {
                    if let Err(e) = self.claim(&ws, &task_id, provider, account, agent, summary, detail) {
                        tracing::warn!("could not start task {task_id}: {e}");
                    }
                }
            }
        }
        for p in plans.iter().filter(|p| p.status == PlanStatus::Running) {
            let ts: Vec<&Task> = tasks.iter().filter(|t| t.plan_id.as_deref() == Some(&p.id)).collect();
            if !ts.is_empty() && ts.iter().all(|t| t.status == TaskStatus::Done) {
                let me = self.clone();
                let id = p.id.clone();
                tokio::spawn(async move { me.complete_plan(&id).await });
            }
        }
        Ok(())
    }

    /// Persist the scheduler's choice before any side effect, then start the run.
    #[allow(clippy::too_many_arguments)]
    fn claim(
        self: &Arc<Self>,
        ws: &Workspace,
        task_id: &str,
        provider: Provider,
        account: String,
        pick: AgentPick,
        summary: String,
        detail: Value,
    ) -> Result<()> {
        let agent = match pick {
            AgentPick::Existing(id) => self.agent(&id)?,
            AgentPick::New { name, color } => self.create_agent(&ws.id, &name, provider, Role::Worker, color, None)?,
        };
        let mut why = None; // why it was not running: tells a resumed session what happened
        self.update_task(task_id, |t| {
            if !matches!(t.status, TaskStatus::Ready | TaskStatus::Pending) {
                bail!("task {} is {} now", t.key, store::enum_str(&t.status));
            }
            why = t.status_reason.take();
            t.status = TaskStatus::Running;
            t.assignee = Some(agent.id.clone());
            t.attempt += 1;
            Ok(())
        })?;
        self.update_agent(&agent.id, |a| a.task_id = Some(task_id.to_string()))?;
        self.decide(Some(&ws.id), Some(task_id), "route", summary, detail)?;
        let me = self.clone();
        let (t, a) = (task_id.to_string(), agent.id.clone());
        tokio::spawn(async move { me.start_worker(t, a, account, why).await });
        Ok(())
    }

    async fn start_worker(self: Arc<Self>, task_id: Id, agent_id: Id, account: String, why: Option<String>) {
        let Err(e) = self.try_start_worker(&task_id, &agent_id, &account, why.as_deref()).await else { return };
        let reason = format!("blocked: could not start the worker: {e}");
        let _ = self.update_task(&task_id, |t| {
            if t.status == TaskStatus::Running {
                t.status = TaskStatus::Blocked;
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        let _ = self.update_agent(&agent_id, |a| {
            if a.task_id.as_deref() == Some(task_id.as_str()) {
                a.task_id = None;
            }
        });
        let ws = self.store.task_workspace(&task_id).ok().flatten();
        let _ = self.decide(ws.as_deref(), Some(&task_id), "schedule", reason.clone(), json!({ "error": e.to_string() }));
        if let (Some(ws), Ok(t)) = (ws, self.task(&task_id)) {
            self.lead_followup(&ws, &self.task_thread(&t), &format!("Task {} \"{}\" is {reason}.", t.key, t.title)).await;
        }
        self.kick();
    }

    async fn try_start_worker(self: &Arc<Self>, task_id: &str, agent_id: &str, account: &str, why: Option<&str>) -> Result<()> {
        let task = self.task(task_id)?;
        let agent = self.agent(agent_id)?;
        let ws_id = self.store.task_workspace(task_id)?.ok_or_else(|| anyhow!("task without workspace"))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let plan = task.plan_id.as_deref().map(|p| self.plan(p)).transpose()?;
        let (branch, wt, start) = match &plan {
            Some(p) => {
                let short = plan_short(p);
                let integ = p.integration_branch.clone().ok_or_else(|| anyhow!("the plan has no integration branch"))?;
                (format!("codly/{short}/{}", task.key), self.worktree_root(&ws).join(&short).join(&task.key), integ)
            }
            None => (format!("codly/adhoc/{}", task.key), self.worktree_root(&ws).join("adhoc").join(&task.key), "HEAD".to_string()),
        };
        let fresh = !wt.exists();
        {
            let _g = self.repo_lock.lock().await;
            self.git.worktree_add(&repo, &wt, &branch, &start).await?;
        }
        if fresh {
            self.bootstrap_worktree(&ws, &repo, &wt, Some(task_id)).await;
        }
        if self.store.task_commits(task_id)?.0.is_none() {
            let base = self.git.rev_parse(&repo, &start).await?;
            self.store.set_task_commits(task_id, base.as_deref(), None)?;
        }
        let wt_s = wt.to_string_lossy().into_owned();
        let task = self.update_task(task_id, |t| {
            t.branch = Some(branch.clone());
            t.worktree = Some(wt_s.clone());
            Ok(())
        })?;
        // Same agent, same task, a previous session on this account: resume it (crash, stop,
        // retry). A session only exists in its account's config home; on another account the
        // worker starts over in a new session (the worktree keeps its work).
        let primary = self.primary_account(agent.provider);
        let prev = self.store.latest_task_run(task_id, Some(Role::Worker))?.filter(|r| {
            r.agent_id == agent.id
                && r.provider == agent.provider
                && r.session_id.is_some()
                && r.account.clone().or_else(|| primary.clone()).as_deref().is_none_or(|a| a == account)
        });
        let feedback = lock(&self.pending_feedback).remove(task_id);
        let req = |session: SessionMode| RunReq {
            agent: agent.clone(),
            role: Role::Worker,
            cwd: wt.clone(),
            read_only: false,
            task_id: Some(task_id.to_string()),
            session,
            system_prompt: prompts::WORKER_SYSTEM.to_string(),
            workspace_id: Some(ws.id.clone()),
            account: Some(account.to_string()),
        };
        let resumed = match prev.and_then(|r| r.session_id) {
            Some(session_id) => match self.spawn_run(req(SessionMode::Resume { session_id })).await {
                Ok(_) => true,
                Err(e) => {
                    tracing::warn!("resuming {} on {} failed ({e}); starting a new session", agent.name, task.key);
                    self.spawn_run(req(new_session(agent.provider))).await?;
                    false
                }
            },
            None => {
                self.spawn_run(req(new_session(agent.provider))).await?;
                false
            }
        };
        let prompt = match (resumed, feedback) {
            (true, Some(f)) => f,
            (true, None) => prompts::continuation(&task, why),
            (false, f) => {
                let brief = self.worker_brief(&ws, &task, plan.as_ref(), &agent).await;
                match f {
                    Some(f) => format!("{brief}\n\n{f}"),
                    None => brief,
                }
            }
        };
        self.send_prompt(agent_id, prompt)
    }

    /// Give a new worktree what git does not carry (CRITIQUE G9): the checkout's installed
    /// dependencies and local env files, as APFS clones (`cp -c`: copy-on-write, near-free).
    /// Only paths the repo ignores are copied, so none of them can end up in a Codly commit.
    /// ponytail: a fixed list; per-repo setup scripts and port blocks come when users ask.
    async fn bootstrap_worktree(&self, ws: &Workspace, repo: &Path, wt: &Path, task_id: Option<&str>) {
        const SEED: &[&str] = &["node_modules", ".venv", "venv", ".env", ".env.local", ".env.development.local", ".env.test.local"];
        let mut copied = vec![];
        for rel in SEED {
            let (src, dst) = (repo.join(rel), wt.join(rel));
            if !src.exists() || dst.exists() || !self.git.is_ignored(repo, rel).await.unwrap_or(false) {
                continue;
            }
            let cp = tokio::process::Command::new("/bin/cp")
                .arg("-cR")
                .arg(&src)
                .arg(&dst)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output();
            match tokio::time::timeout(Duration::from_secs(120), cp).await {
                Ok(Ok(out)) if out.status.success() => copied.push(rel.to_string()),
                Ok(Ok(out)) => {
                    tracing::info!("could not clone {rel} into {}: {}", wt.display(), String::from_utf8_lossy(&out.stderr).trim())
                }
                _ => tracing::info!("cloning {rel} into {} failed or timed out", wt.display()),
            }
        }
        if !copied.is_empty() {
            let summary = format!("seeded the new worktree with {} from your checkout (APFS clones, git-ignored)", copied.join(", "));
            let _ = self.decide(Some(&ws.id), task_id, "context", summary, json!({ "worktree": wt.to_string_lossy(), "copied": copied }));
        }
    }

    /// Task brief + pointer pack (logged as a Retrieval event) + memory pack + what the
    /// dependencies did.
    async fn worker_brief(&self, ws: &Workspace, task: &Task, plan: Option<&Plan>, agent: &Agent) -> String {
        let agents = self.store.agents(&ws.id).unwrap_or_default();
        let deps: Vec<(String, String, String, Option<String>)> = task
            .depends_on
            .iter()
            .filter_map(|d| self.store.task(d).ok().flatten())
            .map(|d| {
                let who = d
                    .assignee
                    .as_deref()
                    .and_then(|a| agents.iter().find(|x| x.id == a))
                    .map_or_else(|| "a worker".into(), |a| a.name.clone());
                (d.key, d.title, who, d.summary)
            })
            .collect();
        let query = format!("{} {}", task.title, task.goal);
        let pointer_pack = match self.pointer_pack(&query).await {
            Some((text, result)) => {
                let _ = self.emit(
                    Some(&ws.id),
                    row(
                        Some(&agent.id),
                        Some(&task.id),
                        None,
                        EventKind::Retrieval {
                            query: prompts::clip(&query, 300),
                            confidence: result.confidence,
                            citations: result.citations,
                        },
                    ),
                );
                Some(text)
            }
            None => None,
        };
        let mut task_ids = vec![task.id.clone()];
        task_ids.extend(task.depends_on.iter().cloned());
        let view = MemoryView { workspace_id: Some(&ws.id), agent_id: Some(&agent.id), task_ids: &task_ids };
        let memory_pack = match self.store.with(|db| guard("memory", || memory::memory_pack(db, &view, &task.title, &task.goal))) {
            Ok(p) => Some(prompts::clip(&p, prompts::WORKER_MEMORY_BUDGET)),
            Err(e) => {
                tracing::debug!("memory pack: {e}");
                self.notes_fallback(ws, &task.depends_on, &agents)
            }
        };
        prompts::task_brief(&prompts::Brief { task, plan, deps, pointer_pack, memory_pack })
    }

    /// Blackboard notes of the dependency tasks, read from events (used while the memory
    /// module cannot build a pack).
    fn notes_fallback(&self, ws: &Workspace, deps: &[Id], agents: &[Agent]) -> Option<String> {
        let notes: Vec<String> = self
            .store
            .memory_events(Some(&ws.id), 200)
            .ok()?
            .into_iter()
            .filter(|m| m.kind == MemoryKind::Note && m.scope == MemoryScope::Task && m.scope_id.as_ref().is_some_and(|s| deps.contains(s)))
            .take(8)
            .map(|m| {
                let who = agents.iter().find(|a| a.id == m.author).map_or_else(|| m.author.clone(), |a| a.name.clone());
                format!("- note from {who}:\n{}", prompts::untrusted(&format!("note from {who}"), &m.text, 500))
            })
            .collect();
        (!notes.is_empty()).then(|| notes.join("\n"))
    }

    // ───────────── the board (user moves) ─────────────

    pub(super) fn create_task(&self, title: &str, goal: &str) -> Result<Task> {
        let ws = self.require_ws()?;
        let (title, goal) = (title.trim(), goal.trim());
        if title.is_empty() {
            bail!("a task needs a title");
        }
        let id = new_id();
        let now = now_ms();
        let task = Task {
            key: format!("a{}", &id[id.len() - 5..]),
            id,
            plan_id: None,
            title: title.into(),
            goal: if goal.is_empty() { title.into() } else { goal.into() },
            acceptance: vec![],
            write_scope: vec![],
            verify: vec![],
            depends_on: vec![],
            status: TaskStatus::Pending,
            assignee: None,
            agent_hint: None,
            account_hint: None,
            branch: None,
            worktree: None,
            attempt: 0,
            status_reason: None,
            summary: None,
            created_at: now,
            updated_at: now,
        };
        self.emit(Some(&ws.id), row(None, Some(&task.id), None, EventKind::TaskChanged { task: task.clone() }))?;
        Ok(task)
    }

    pub(super) async fn move_task(&self, task_id: &str, to: TaskStatus) -> Result<Task> {
        let t = self.task(task_id)?;
        let from = t.status;
        if from == to {
            return Ok(t);
        }
        let plan = t.plan_id.as_deref().map(|p| self.plan(p)).transpose()?;
        let plan_running = plan.as_ref().is_none_or(|p| p.status == PlanStatus::Running);
        let deps_done = t.depends_on.iter().all(|d| self.store.task(d).ok().flatten().is_some_and(|d| d.status == TaskStatus::Done));
        use TaskStatus::*;
        match (from, to) {
            (Done, _) => bail!("{} is done; done tasks cannot move", t.key),
            (Pending | Blocked | Failed | Cancelled, Ready) => {
                if !plan_running {
                    bail!("run the plan first");
                }
                if !deps_done {
                    bail!("{} still waits for its dependencies", t.key);
                }
            }
            // Plan tasks start as soon as their dependencies are merged; only ad-hoc tasks
            // have a backlog (the scheduler would make a plan task ready again at once).
            (Ready, Pending) if t.plan_id.is_none() => {}
            (s, Cancelled) if !matches!(s, Cancelled) => self.release_task_agents(&t),
            _ => bail!("cannot move {} from {} to {}", t.key, store::enum_str(&from), store::enum_str(&to)),
        }
        if to == Ready {
            lock(&self.fix_rounds).remove(task_id); // a retry gets fresh fix rounds
        }
        let t = self.update_task_as(task_id, true, |t| {
            t.status = to;
            t.status_reason = (to == Cancelled).then(|| "cancelled by you".to_string());
            Ok(())
        })?;
        let ws = self.store.task_workspace(task_id)?;
        self.decide_by(
            ws.as_deref(),
            Some(task_id),
            "schedule",
            "human",
            format!("you moved {} from {} to {}", t.key, store::enum_str(&from), store::enum_str(&to)),
            json!({ "plan_id": t.plan_id }),
        )?;
        self.kick();
        Ok(t)
    }

    pub(super) fn assign_task(&self, task_id: &str, agent_id: Option<&str>) -> Result<Task> {
        let t = self.task(task_id)?;
        if scheduler::is_active(t.status) {
            bail!("{} is running; stop it before reassigning", t.key);
        }
        if matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled) {
            bail!("{} is {}", t.key, store::enum_str(&t.status));
        }
        let agent = agent_id.map(|id| self.agent(id)).transpose()?;
        if let Some(a) = &agent
            && !matches!(a.role, Role::Worker | Role::Solo)
        {
            bail!("{} is the {}; only workers and solo agents take tasks", a.name, store::enum_str(&a.role));
        }
        let t = self.update_task(task_id, |t| {
            t.assignee = agent.as_ref().map(|a| a.id.clone());
            // Ad-hoc tasks run when assigned.
            if t.plan_id.is_none() && t.status == TaskStatus::Pending && agent.is_some() {
                t.status = TaskStatus::Ready;
            }
            Ok(())
        })?;
        let ws = self.store.task_workspace(task_id)?;
        let summary = match &agent {
            Some(a) => format!("you assigned {} to {}", t.key, a.name),
            None => format!("you unassigned {}", t.key),
        };
        self.decide_by(ws.as_deref(), Some(task_id), "route", "human", summary, json!({ "plan_id": t.plan_id }))?;
        self.kick();
        Ok(t)
    }

    // ───────────── crash recovery ─────────────

    /// Runs left `starting`/`running` by a crash become `lost`; their pending approvals
    /// expire; their tasks go back to ready, keeping the assignee, so the scheduler resumes
    /// the same agent with the stored session id.
    pub(super) async fn recover(&self) {
        if let Err(e) = self.recover_inner().await {
            tracing::warn!("crash recovery incomplete: {e}");
        }
    }

    async fn recover_inner(&self) -> Result<()> {
        for r in self.store.runs_with_status(&["starting", "running"])? {
            self.store.set_run_status(&r.id, "lost")?;
            let ws = self.store.agent_workspace(&r.agent_id)?;
            for a in self.store.pending_approvals_for_run(&r.id)? {
                self.emit(
                    ws.as_deref(),
                    row(
                        Some(&r.agent_id),
                        a.task_id.as_deref(),
                        Some(&r.id),
                        EventKind::ApprovalResolved { approval_id: a.id, status: ApprovalStatus::Expired, by: "codly".into() },
                    ),
                )?;
            }
            let session = r.session_id.as_deref().map(|s| format!(" (session {})", &s[..s.len().min(8)])).unwrap_or_default();
            self.decide(
                ws.as_deref(),
                r.task_id.as_deref(),
                "retry",
                format!("a {} run was lost when Codly stopped{session}; it will be resumed", r.provider.label()),
                json!({ "run_id": r.id, "session_id": r.session_id }),
            )?;
        }
        let active = [TaskStatus::Running, TaskStatus::Verifying, TaskStatus::Reviewing, TaskStatus::ChangesRequested, TaskStatus::Merging];
        for t in self.store.tasks_with_status(&active)? {
            self.update_task(&t.id, |t| {
                t.status = TaskStatus::Ready;
                t.status_reason = Some("resuming after Codly restarted".into());
                Ok(())
            })?;
        }
        for a in self.store.all_agents()? {
            let busy = !matches!(a.status, AgentStatus::Idle | AgentStatus::Offline | AgentStatus::Done);
            if busy || a.run_id.is_some() || a.task_id.is_some() {
                self.update_agent(&a.id, |a| {
                    if !matches!(a.status, AgentStatus::Offline) {
                        a.status = AgentStatus::Idle;
                    }
                    a.run_id = None;
                    a.task_id = None;
                    a.activity = None;
                    a.activity_label = None;
                })?;
            }
        }
        for p in self.store.plans_with_status(&[PlanStatus::Integrating])? {
            self.update_plan(&p.id, |p| {
                p.status = PlanStatus::Running;
                Ok(())
            })?;
        }
        for p in self.store.plans_with_status(&[PlanStatus::Running])? {
            if let Some(wt) = p.integration_worktree.as_deref().map(Path::new).filter(|w| w.exists())
                && let Ok(true) = self.git.abort_merge(wt).await
            {
                self.decide(Some(&p.workspace_id), None, "merge", "aborted a merge left half-done by a crash", json!({ "plan_id": p.id }))?;
            }
        }
        Ok(())
    }

    // ───────────── plan completion ─────────────

    async fn complete_plan(self: Arc<Self>, plan_id: &str) {
        let Ok(plan) = self.update_plan(plan_id, |p| {
            if p.status != PlanStatus::Running {
                bail!("not running");
            }
            p.status = PlanStatus::Integrating;
            Ok(())
        }) else {
            return;
        };
        if let Err(e) = self.complete_plan_inner(&plan).await {
            tracing::warn!("completing plan {plan_id}: {e}");
            let _ = self.update_plan(plan_id, |p| {
                p.status = PlanStatus::Failed;
                Ok(())
            });
            let _ = self.chat(
                Some(&plan.workspace_id),
                &plan.thread,
                "codly",
                &format!("Finishing the plan failed: {e}"),
                Some(json!({ "plan_id": plan_id })),
            );
        }
    }

    async fn complete_plan_inner(self: &Arc<Self>, plan: &Plan) -> Result<()> {
        let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let ws_id = Some(ws.id.as_str());
        let checks: Vec<String> = self.store.plan_json(&plan.id)?["integration_checks"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let wt = plan.integration_worktree.as_deref().map(PathBuf::from);
        let mut failed = vec![];
        if let Some(wt) = &wt {
            for cmd in &checks {
                let v = self.run_verify(wt, cmd).await;
                self.emit(
                    ws_id,
                    row(None, None, None, EventKind::Verified { command: cmd.clone(), ok: v.ok, exit_code: v.code, tail: v.tail }),
                )?;
                if !v.ok {
                    failed.push(cmd.clone());
                }
            }
        }
        let status = if failed.is_empty() { PlanStatus::Done } else { PlanStatus::Failed };
        let tasks = self.store.plan_tasks(&plan.id)?;
        let branch = plan.integration_branch.clone().unwrap_or_default();
        let files = match (&wt, plan.base_commit.as_deref()) {
            (Some(wt), Some(base)) => self.git.changed_files(wt, base).await.map(|f| f.len()).unwrap_or(0),
            _ => 0,
        };
        let target = self.git.current_branch(Path::new(&ws.repo_path)).await.ok().flatten().unwrap_or_else(|| ws.default_branch.clone());
        let n = tasks.len();
        let text = if failed.is_empty() {
            format!(
                "Plan done: \"{}\". {n} task{} merged into {branch} ({files} file{} changed). Review the diff, then merge it into {target}.",
                plan.summary,
                if n == 1 { "" } else { "s" },
                if files == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "Plan \"{}\" finished, but integration checks failed: {}. The work is on {branch}; ask the Lead for a fix plan or merge it anyway.",
                plan.summary,
                failed.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ")
            )
        };
        self.chat(ws_id, &plan.thread, "codly", &text, Some(json!({ "plan_id": plan.id })))?;
        self.decide(
            ws_id,
            None,
            "merge",
            prompts::clip(&text, 200),
            json!({ "plan_id": plan.id, "files": files, "failed_checks": failed }),
        )?;
        self.update_plan(&plan.id, |p| {
            p.status = status;
            Ok(())
        })?;
        self.lead_followup(&ws.id, &plan.thread, &text).await;
        Ok(())
    }
}
