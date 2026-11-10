//! The one event pipeline. Every `AdapterEvent` from every agent run goes through `ingest`,
//! in arrival order: persist (unless ephemeral) → broadcast → live-run bookkeeping → agent
//! projection → reactions → knowledge triples. Reactions that wait on anything (gates,
//! reviews, commits) are spawned, so a ten-minute verify never delays another agent's events.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::json;
use tokio::sync::mpsc;

use super::approvals::{self, PolicyCtx, Verdict};
use super::{Inner, guard, lead, lock, prompts, row, scheduler};
use crate::adapters::{AdapterEvent, AgentCmd};
use crate::{career, memory};
use crate::model::*;

pub(super) async fn run(inner: Arc<Inner>, mut rx: mpsc::UnboundedReceiver<AdapterEvent>) {
    while let Some(ev) = rx.recv().await {
        // A bug in one projection must never stop the pipeline for every agent.
        let ty = ev.kind.type_name();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.ingest(ev))).is_err() {
            tracing::error!("ingesting a {ty} event panicked; continuing");
        }
    }
}

/// Who an event belongs to.
struct Ctx {
    agent_id: Id,
    run_id: Id,
    role: Role,
    task_id: Option<Id>,
    ws: Option<Id>,
}

impl Inner {
    fn run_ctx(&self, agent_id: &str, run_id: &str) -> Ctx {
        if let Some(l) = lock(&self.live).get(agent_id).filter(|l| l.run_id == run_id) {
            return Ctx {
                agent_id: agent_id.into(),
                run_id: run_id.into(),
                role: l.role,
                task_id: l.task_id.clone(),
                ws: l.workspace_id.clone(),
            };
        }
        // Not (or no longer) live: the run row is written before the spawn, so it exists.
        let run = self.store.run(run_id).ok().flatten();
        Ctx {
            agent_id: agent_id.into(),
            run_id: run_id.into(),
            role: run.as_ref().map_or(Role::Worker, |r| r.role),
            task_id: run.and_then(|r| r.task_id),
            ws: self.store.agent_workspace(agent_id).ok().flatten(),
        }
    }

    fn ingest(self: &Arc<Self>, ev: AdapterEvent) {
        let ctx = self.run_ctx(&ev.agent_id, &ev.run_id);
        let mut r = row(Some(&ev.agent_id), ctx.task_id.as_deref(), Some(&ev.run_id), ev.kind);
        if let EventKind::ApprovalRequested { approval } = &mut r.kind
            && approval.task_id.is_none()
        {
            approval.task_id = ctx.task_id.clone(); // adapters do not know Codly's tasks
        }
        if r.kind.is_ephemeral() {
            r.workspace_id = ctx.ws.clone();
            let _ = self.bus.send(r);
            return;
        }
        match self.store.append_event(ctx.ws.as_deref(), &r) {
            Ok(id) => r.id = id,
            Err(e) => tracing::warn!("event not persisted ({}): {e}", r.kind.type_name()),
        }
        self.publish(ctx.ws.as_deref(), &r);
        self.track_live(&ctx, &r.kind);
        self.breaker_observe(&ctx.agent_id, &r.kind);
        match &r.kind {
            EventKind::TurnCompleted { .. } => self.control_turn_done(&ctx.agent_id),
            EventKind::ToolStarted { name, .. } => self.control_tool_started(&ctx.agent_id, name),
            _ => {}
        }
        if let Err(e) = self.project_agent(&ctx, &r.kind) {
            tracing::debug!("agent projection: {e}");
        }
        self.react(&ctx, &r);
    }

    /// Broadcast a persisted (or ephemeral) row and derive knowledge triples from it.
    pub(super) fn publish(&self, ws: Option<&str>, r: &EventRow) {
        let _ = self.bus.send(EventRow { workspace_id: ws.map(String::from), ..r.clone() });
        if r.id > 0 {
            self.derive_triples(ws, r);
            self.detect_artifacts(ws, r);
            self.track_career(r);
        }
    }

    /// Files an agent wrote that are worth viewing (images, HTML, SVG, docs, Codex image
    /// generation) become `artifact_added` events. Re-announced when the file changes size.
    fn detect_artifacts(&self, ws: Option<&str>, r: &EventRow) {
        let (paths, caption) = match &r.kind {
            EventKind::FilesChanged { paths, .. } | EventKind::Committed { files: paths, .. } => (paths.clone(), None),
            _ => return,
        };
        let cwd = r.agent_id.as_deref().and_then(|a| {
            lock(&self.live).get(a).map(|l| l.cwd.clone()).or_else(|| self.store.agent(a).ok().flatten()?.worktree.map(Into::into))
        });
        for p in paths {
            let Some((kind, mime)) = ArtifactKind::for_path(&p) else { continue };
            let abs = match (std::path::Path::new(&p).is_absolute(), &cwd) {
                (true, _) => std::path::PathBuf::from(&p),
                (false, Some(c)) => c.join(&p),
                (false, None) => continue,
            };
            let Ok(meta) = std::fs::metadata(&abs) else { continue };
            if !meta.is_file() || meta.len() > 64 << 20 {
                continue;
            }
            let key = format!("{}:{}", abs.display(), meta.len());
            if !lock(&self.artifacts_seen).insert(key) {
                continue;
            }
            let display = cwd
                .as_ref()
                .and_then(|c| abs.strip_prefix(c).ok())
                .map(|rel| rel.to_string_lossy().into_owned())
                .unwrap_or_else(|| abs.to_string_lossy().into_owned());
            let title = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| display.clone());
            // Snapshot it: task worktrees are removed after merge, and generated images can be
            // anywhere. The copy is what the UI reads.
            let id = new_id();
            let dir = self.cfg.data_dir.join("artifacts");
            let ext = abs.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            let copy = dir.join(format!("{id}.{ext}"));
            if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&abs, &copy)) {
                tracing::debug!("artifact not copied: {e}");
                continue;
            }
            let abs = copy;
            let artifact = Artifact {
                id,
                kind,
                mime: mime.into(),
                path: display,
                abs_path: abs.to_string_lossy().into_owned(),
                title,
                bytes: meta.len() as i64,
                agent_id: r.agent_id.clone(),
                task_id: r.task_id.clone(),
                caption: caption.clone(),
                created_at: now_ms(),
            };
            if let Err(e) = self.emit(ws, row(r.agent_id.as_deref(), r.task_id.as_deref(), r.run_id.as_deref(), EventKind::ArtifactAdded { artifact })) {
                tracing::debug!("artifact not recorded: {e}");
            }
        }
    }

    fn track_live(&self, ctx: &Ctx, kind: &EventKind) {
        let mut live = lock(&self.live);
        let Some(l) = live.get_mut(&ctx.agent_id).filter(|l| l.run_id == ctx.run_id) else { return };
        match kind {
            EventKind::TurnStarted { .. } => {
                l.turn_active = true;
                l.queued = l.queued.saturating_sub(1);
                l.turn_messages = 0;
                l.last_text = None;
            }
            EventKind::Message { text, .. } if !text.trim().is_empty() => {
                l.last_text = Some(text.clone());
                l.turn_messages += 1;
            }
            EventKind::TurnCompleted { .. } => {
                if !l.turn_active {
                    // A turn whose start we never saw still consumed its prompt.
                    l.queued = l.queued.saturating_sub(1);
                }
                l.turn_active = false;
                l.turns += 1;
            }
            _ => {}
        }
    }

    /// Keep the agent row (status, activity, usage, session) in step with its events.
    fn project_agent(&self, ctx: &Ctx, kind: &EventKind) -> anyhow::Result<()> {
        let relevant = matches!(
            kind,
            EventKind::SessionStarted { .. }
                | EventKind::StatusChanged { .. }
                | EventKind::TurnStarted { .. }
                | EventKind::ToolStarted { .. }
                | EventKind::ToolCompleted { .. }
                | EventKind::ApprovalRequested { .. }
                | EventKind::ApprovalResolved { .. }
                | EventKind::UsageUpdated { .. }
                | EventKind::RateLimit { .. }
                | EventKind::Compacting { .. }
                | EventKind::Retrying { .. }
                | EventKind::TurnCompleted { .. }
                | EventKind::Error { fatal: true, .. }
        );
        if !relevant {
            return Ok(());
        }
        let run_id = ctx.run_id.clone();
        self.update_agent(&ctx.agent_id, |a| {
            if a.run_id.as_deref() != Some(run_id.as_str()) && !matches!(kind, EventKind::SessionStarted { .. }) {
                return; // a late event from a previous run
            }
            match kind {
                EventKind::SessionStarted { session_id, model, .. } => {
                    if let Some(s) = session_id {
                        a.session_id = Some(s.clone());
                    }
                    // A member keeps its configured model (with its effort suffix) for the next run.
                    if model.is_some() && a.member.is_none() {
                        a.model = model.clone();
                    }
                    if a.status == AgentStatus::Starting {
                        a.status = AgentStatus::Idle;
                    }
                }
                EventKind::StatusChanged { status } => {
                    a.status = *status;
                    if *status == AgentStatus::Idle {
                        a.activity = None;
                        a.activity_label = None;
                    }
                }
                EventKind::TurnStarted { .. } => {
                    a.status = AgentStatus::Thinking;
                    a.activity = Some(ActivityKind::Thinking);
                    a.activity_label = None;
                }
                EventKind::ToolStarted { activity, target, .. } => {
                    a.status = AgentStatus::Working;
                    a.activity = Some(*activity);
                    a.activity_label = Some(crate::adapters::classify::activity_label(*activity, target));
                }
                EventKind::ToolCompleted { .. } => {
                    a.activity = Some(ActivityKind::Thinking);
                    a.activity_label = None;
                }
                EventKind::ApprovalRequested { .. } => {
                    a.status = AgentStatus::WaitingApproval;
                    a.activity = Some(ActivityKind::AwaitApproval);
                }
                EventKind::ApprovalResolved { .. } if a.status == AgentStatus::WaitingApproval => {
                    a.status = AgentStatus::Working;
                    a.activity = Some(ActivityKind::Thinking);
                }
                EventKind::UsageUpdated { usage } => a.usage = usage.clone(),
                EventKind::RateLimit { used_fraction, status, .. } => {
                    let limited = status.as_deref().is_some_and(|s| matches!(s, "rejected" | "limited" | "exceeded"))
                        || used_fraction.is_some_and(|f| f >= 1.0);
                    if limited {
                        a.status = AgentStatus::RateLimited;
                        a.activity = Some(ActivityKind::RateLimited);
                    }
                }
                EventKind::Compacting { .. } => a.activity = Some(ActivityKind::Compacting),
                EventKind::Retrying { .. } => a.activity = Some(ActivityKind::Retrying),
                EventKind::TurnCompleted { .. } => {
                    a.status = AgentStatus::Idle;
                    a.activity = None;
                    a.activity_label = None;
                }
                EventKind::Error { fatal: true, .. } => a.status = AgentStatus::Failed,
                _ => {}
            }
        })?;
        Ok(())
    }

    fn react(self: &Arc<Self>, ctx: &Ctx, r: &EventRow) {
        let ws = ctx.ws.as_deref();
        // A Lead speaks in the request thread it serves; everyone else in their 1:1 thread.
        let thread = || match ctx.role {
            Role::Lead => self.agent(&ctx.agent_id).map_or_else(|_| default_lead_thread(), |a| lead::thread_of(&a).to_string()),
            _ => format!("agent:{}", ctx.agent_id),
        };
        match &r.kind {
            EventKind::SessionStarted { session_id: Some(s), .. } => {
                let _ = self.store.set_run_session(&ctx.run_id, s);
            }
            EventKind::Message { text, .. } if !text.trim().is_empty() => {
                // A Lead turn answering a worker's codly_ask_lead links back to the asker.
                let reply_to = (ctx.role == Role::Lead).then(|| lock(&self.leads).reply_to(&ctx.agent_id)).flatten();
                let _ = self.chat(ws, &thread(), &ctx.agent_id, text, reply_to.map(|id| json!({ "reply_to": id })));
            }
            EventKind::TurnCompleted { status, result_text, .. } => {
                let (had_message, last) = lock(&self.live)
                    .get(&ctx.agent_id)
                    .filter(|l| l.run_id == ctx.run_id)
                    .map_or((true, None), |l| (l.turn_messages > 0, l.last_text.clone()));
                if !had_message && let Some(t) = result_text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let _ = self.chat(ws, &thread(), &ctx.agent_id, t, None);
                }
                let reply = last.or_else(|| result_text.clone());
                self.on_turn_completed(ctx, *status, reply);
            }
            EventKind::StatusChanged { status: AgentStatus::RateLimited } => {
                // Codex usage errors and Claude's rate_limit result report only this: park the
                // account until a reading says otherwise.
                if let Some(account) = self.event_account(&ctx.agent_id, &ctx.run_id) {
                    self.record_usage(&account, "limit", Some(1.0), None, Some("rejected"));
                }
            }
            EventKind::RateLimit { window, used_fraction, resets_at, status } => {
                // Quota is per sign-in: the account this run uses.
                if let Some(account) = self.event_account(&ctx.agent_id, &ctx.run_id) {
                    let used = self.record_usage(&account, window, *used_fraction, *resets_at, status.as_deref());
                    if used.is_none_or(|u| u < scheduler::PARK_AT) {
                        self.kick(); // an account may have come back
                    }
                }
            }
            EventKind::ApprovalRequested { approval } => self.broker(ctx, approval),
            EventKind::Exited { code } => self.on_exit(ctx, *code),
            _ => {}
        }
    }

    fn on_turn_completed(self: &Arc<Self>, ctx: &Ctx, status: TurnStatus, reply: Option<String>) {
        if ctx.role != Role::Lead {
            self.forward_answers(&ctx.agent_id, &ctx.run_id, reply.as_deref());
        }
        match ctx.role {
            Role::Lead => {
                let targets = lock(&self.leads).turn_done(&ctx.agent_id);
                if let Some(reply) = reply.filter(|r| !r.trim().is_empty()) {
                    for w in targets {
                        let msg = format!("[Codly] Answer from the lead:\n{}", prompts::untrusted("Lead", &reply, 4000));
                        if let Err(e) = self.send_prompt(&w, msg) {
                            tracing::debug!("lead answer not forwarded: {e}");
                        }
                    }
                }
                self.pump_leads(); // an idle Lead can make room for a request that waits
            }
            Role::Worker if ctx.task_id.is_some() && self.hit_limit(ctx) => self.hand_off(ctx),
            Role::Worker if ctx.task_id.is_some() => {
                // Gate only when the worker is really done: not interrupted, and no queued
                // prompt (a user steer, a fix round) is about to start another turn.
                if status != TurnStatus::Interrupted && self.is_quiet(&ctx.agent_id) && !self.control_holding(&ctx.agent_id) {
                    let me = self.clone();
                    let task = ctx.task_id.clone().unwrap_or_default();
                    tokio::spawn(async move { me.gate(task).await });
                }
            }
            Role::Reviewer if ctx.task_id.is_some() => {
                if self.is_quiet(&ctx.agent_id) {
                    let me = self.clone();
                    let (task, run) = (ctx.task_id.clone().unwrap_or_default(), ctx.run_id.clone());
                    tokio::spawn(async move { me.review_done(task, run).await });
                }
            }
            Role::Solo => {
                let me = self.clone();
                let agent = ctx.agent_id.clone();
                let ws = ctx.ws.clone();
                tokio::spawn(async move { me.solo_snapshot(agent, ws).await });
            }
            _ => {}
        }
    }

    fn provider_of(&self, agent_id: &str) -> Option<Provider> {
        lock(&self.live).get(agent_id).map(|l| l.provider).or_else(|| self.store.agent(agent_id).ok().flatten().map(|a| a.provider))
    }

    /// The agent is rate limited, or its account is on a full (100%) window.
    fn hit_limit(&self, ctx: &Ctx) -> bool {
        if self.agent(&ctx.agent_id).is_ok_and(|a| a.status == AgentStatus::RateLimited) {
            return true;
        }
        let Some(account) = self.event_account(&ctx.agent_id, &ctx.run_id) else { return false };
        let windows = lock(&self.quota).get(&account).cloned().unwrap_or_default();
        super::accounts::load(&windows, now_ms()).used.is_some_and(|u| u >= 1.0)
    }

    /// A worker hit its usage limit mid-task: no gate on half-done work. Stop it and put the
    /// task back to Ready, unassigned, so the next free provider continues in the same worktree.
    fn hand_off(self: &Arc<Self>, ctx: &Ctx) {
        let task_id = ctx.task_id.clone().unwrap_or_default();
        let Ok(t) = self.task(&task_id) else { return };
        if !matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
            return;
        }
        self.stop_run(&ctx.agent_id); // stopping: on_exit will not call task_run_lost
        let provider = self.provider_of(&ctx.agent_id).map_or("The provider", |p| p.label());
        let reason = format!(
            "{provider} hit its usage limit; handing off to the next free provider. Continue from the work already in this worktree."
        );
        // status_reason reaches a resumed session; a fresh one (another provider) gets this.
        lock(&self.pending_feedback).entry(task_id.clone()).or_insert_with(|| format!("[Codly] {reason}"));
        let _ = self.update_task(&task_id, |t| {
            t.status = TaskStatus::Ready;
            t.assignee = None;
            t.status_reason = Some(reason.clone());
            Ok(())
        });
        let _ = self.update_agent(&ctx.agent_id, |a| {
            if a.task_id.as_deref() == Some(task_id.as_str()) {
                a.task_id = None;
            }
        });
        let ws = self.store.task_workspace(&task_id).ok().flatten();
        let _ = self.decide(ws.as_deref(), Some(&task_id), "retry", reason, json!({ "plan_id": t.plan_id, "provider": provider }));
        self.kick();
    }

    /// Codly commits a solo agent's worktree after each turn (C7: agents never commit).
    async fn solo_snapshot(self: Arc<Self>, agent_id: Id, ws: Option<Id>) {
        let (cwd, prompt) = match lock(&self.live).get(&agent_id) {
            Some(l) => (l.cwd.clone(), l.last_prompt.clone()),
            None => return,
        };
        let Ok(agent) = self.agent(&agent_id) else { return };
        let what = prompt.map(|p| prompts::clip(p.trim_start_matches("[User]").trim(), 60)).unwrap_or_else(|| "turn".into());
        let message = format!("codly(solo/{}): {what}", agent.name);
        let repo = ws.as_deref().and_then(|w| self.store.workspace(w).ok().flatten()).map(|w| std::path::PathBuf::from(w.repo_path));
        let commit = {
            let _g = self.repo_lock.lock().await;
            match repo {
                Some(repo) => match self.git.check_worktree(&repo, &cwd).await {
                    Ok(()) => self.git.commit_all(&cwd, &message).await,
                    Err(e) => Err(e),
                },
                None => Err(anyhow::anyhow!("no workspace")),
            }
        };
        match commit {
            Ok(Some(c)) => {
                let _ = self.emit(
                    ws.as_deref(),
                    row(Some(&agent_id), None, agent.run_id.as_deref(), EventKind::Committed { sha: c.sha, files: c.files, message }),
                );
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("solo snapshot for {} failed: {e}", agent.name),
        }
    }

    /// The approval broker: L0 rules first; every automatic outcome is a decision.
    fn broker(&self, ctx: &Ctx, approval: &Approval) {
        let ws = ctx.ws.as_deref();
        if self.control_deny(&ctx.run_id, ctx.task_id.as_deref(), ws, &ctx.agent_id, approval) {
            return; // tools are blocked for this agent
        }
        let (cwd, read_only) = lock(&self.live).get(&ctx.agent_id).map_or((None, true), |l| (Some(l.cwd.clone()), l.read_only));
        let task = ctx.task_id.as_deref().and_then(|t| self.store.task(t).ok().flatten());
        // The request the run works for (a Lead's own, a task's plan's) or a 1:1 chat may set its own permissions.
        let thread = match (&task, ctx.role) {
            (Some(t), _) => Some(self.task_thread(t)),
            (None, Role::Lead) => self.agent(&ctx.agent_id).ok().map(|a| lead::thread_of(&a).to_string()),
            (None, Role::Solo) => Some(format!("agent:{}", ctx.agent_id)),
            _ => None,
        };
        let verify = task.map(|t| t.verify).unwrap_or_default();
        let base = self.approval_mode_for(ws, thread.as_deref());
        let trusts = |r| {
            !read_only
                && self.agent(&ctx.agent_id).is_ok_and(|a| matches!(a.role, Role::Worker | Role::Solo) && career::trusts(&a.career, r))
        };
        let mode = approvals::with_trust(base, trusts(Rank::Senior));
        let pctx = PolicyCtx { mode, worktree: cwd.as_deref(), verify: &verify, read_only, edits: trusts(Rank::Junior) };
        let verdict = match approvals::evaluate(approval, &pctx) {
            Verdict::Allow { choice, rule } if mode != base => Verdict::Allow { choice, rule: format!("{rule} (trusted Senior)") },
            v => v,
        };
        let subject = serde_json::to_value(&approval.subject).unwrap_or_default();
        let detail =
            |rule: &str| json!({ "approval_id": approval.id, "mode": mode, "rule": rule, "subject": subject, "read_only": read_only });
        match verdict {
            Verdict::Allow { choice, rule } => {
                let decision = ApprovalDecision { choice, message: None, answers: None };
                let sent = lock(&self.live)
                    .get(&ctx.agent_id)
                    .filter(|l| l.run_id == ctx.run_id)
                    .is_some_and(|l| l.handle.send(AgentCmd::Approval { approval_id: approval.id.clone(), decision }));
                if !sent {
                    return;
                }
                let _ = self.emit(
                    ws,
                    row(
                        Some(&ctx.agent_id),
                        ctx.task_id.as_deref(),
                        Some(&ctx.run_id),
                        EventKind::ApprovalResolved {
                            approval_id: approval.id.clone(),
                            status: ApprovalStatus::Approved,
                            by: "policy".into(),
                        },
                    ),
                );
                let _ = self.decide(ws, ctx.task_id.as_deref(), "approval_policy", format!("auto-allowed {rule}"), detail(&rule));
                let _ = self.update_agent(&ctx.agent_id, |a| {
                    if a.status == AgentStatus::WaitingApproval {
                        a.status = AgentStatus::Working;
                        a.activity = Some(ActivityKind::Thinking);
                    }
                });
            }
            Verdict::Ask { reason } => {
                let _ = self.decide(ws, ctx.task_id.as_deref(), "approval_policy", format!("asking you: {reason}"), detail(&reason));
            }
        }
    }

    fn on_exit(self: &Arc<Self>, ctx: &Ctx, code: Option<i64>) {
        let removed = {
            let mut live = lock(&self.live);
            match live.get(&ctx.agent_id) {
                Some(l) if l.run_id == ctx.run_id => live.remove(&ctx.agent_id),
                _ => None,
            }
        };
        if let (Some(l), Some(m)) = (&removed, self.mcp.get()) {
            m.revoke(&l.token);
        }
        if removed.is_some() {
            self.control_clear(&ctx.agent_id); // controls do not outlive the process
        }
        let stopping = removed.as_ref().is_none_or(|l| l.stopping);
        // It exited during its handshake: `spawn_run` owns this start and fails it, and its
        // caller may already be starting the next one (resume, then a new session).
        let starting = removed.as_ref().is_some_and(|l| l.starting.is_some());
        let shutting = self.shutting_down.load(Ordering::SeqCst);
        let status = if shutting {
            "lost" // resumed by session id on the next start
        } else if !starting && (stopping || code == Some(0)) {
            "completed"
        } else {
            "failed"
        };
        let _ = self.store.set_run_status(&ctx.run_id, status);
        let ws = ctx.ws.as_deref();
        for a in self.store.pending_approvals_for_run(&ctx.run_id).unwrap_or_default() {
            let _ = self.emit(
                ws,
                row(
                    Some(&ctx.agent_id),
                    a.task_id.as_deref(),
                    Some(&ctx.run_id),
                    EventKind::ApprovalResolved { approval_id: a.id, status: ApprovalStatus::Expired, by: "codly".into() },
                ),
            );
        }
        if shutting {
            return;
        }
        let unexpected = !stopping && code != Some(0);
        let run_id = ctx.run_id.clone();
        let _ = self.update_agent(&ctx.agent_id, |a| {
            if a.run_id.as_deref() == Some(run_id.as_str()) {
                a.run_id = None;
                a.activity = None;
                a.activity_label = None;
                if a.status != AgentStatus::Offline {
                    a.status = if unexpected { AgentStatus::Failed } else { AgentStatus::Idle };
                }
            }
        });
        if let Some(l) = &removed {
            self.answers_lost(&ctx.agent_id, &l.answers);
        }
        if ctx.role == Role::Lead {
            if removed.is_some() {
                lock(&self.leads).forget(&ctx.agent_id);
            }
            self.pump_leads(); // a Lead slot freed up
        }
        if !stopping && !starting && let Some(task_id) = ctx.task_id.clone() {
            let me = self.clone();
            let (role, agent_id, run) = (ctx.role, ctx.agent_id.clone(), ctx.run_id.clone());
            tokio::spawn(async move { me.task_run_lost(task_id, role, agent_id, run, code).await });
        }
        self.kick();
    }

    /// A task's worker or reviewer exited without Codly asking it to.
    async fn task_run_lost(self: Arc<Self>, task_id: Id, role: Role, agent_id: Id, run_id: Id, code: Option<i64>) {
        let Ok(t) = self.task(&task_id) else { return };
        let ws = self.store.task_workspace(&task_id).ok().flatten();
        match role {
            Role::Reviewer if t.status == TaskStatus::Reviewing => self.review_done(task_id, run_id).await,
            Role::Worker if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) => {
                let code = code.map_or("signal".to_string(), |c| c.to_string());
                let give_up = t.attempt >= 3;
                let reason = if give_up {
                    format!("blocked: the worker exited unexpectedly {} times (last exit {code}); check the CLI and its run log", t.attempt)
                } else {
                    format!("the worker exited unexpectedly ({code}); restarting it")
                };
                let _ = self.update_task(&task_id, |t| {
                    t.status = if give_up { TaskStatus::Blocked } else { TaskStatus::Ready };
                    t.status_reason = Some(reason.clone());
                    Ok(())
                });
                let _ = self.update_agent(&agent_id, |a| {
                    if a.task_id.as_deref() == Some(task_id.as_str()) {
                        a.task_id = None;
                    }
                });
                let _ = self.decide(
                    ws.as_deref(),
                    Some(&task_id),
                    "retry",
                    reason.clone(),
                    json!({ "plan_id": t.plan_id, "exit": code, "attempt": t.attempt }),
                );
                if give_up && let Some(ws) = ws {
                    self.lead_followup(&ws, &self.task_thread(&t), &format!("Task {} \"{}\" is {reason}.", t.key, t.title)).await;
                }
                self.kick();
            }
            _ => {}
        }
    }

    fn derive_triples(&self, ws: Option<&str>, r: &EventRow) {
        if matches!(r.kind, EventKind::TripleAdded { .. } | EventKind::MemoryAdded { .. } | EventKind::Decision { .. }) {
            return;
        }
        let agent_name = r.agent_id.as_deref().and_then(|id| self.label(id, true));
        let task_key = r.task_id.as_deref().and_then(|id| self.label(id, false));
        let rel = self.relative_paths(ws, r);
        let resolve = |id: &str| self.label(id, true).or_else(|| self.label(id, false));
        let derived = guard("memory::derive_triples", || {
            Ok(memory::derive_triples_with(ws, &rel, agent_name.as_deref(), task_key.as_deref(), &resolve))
        });
        let triples = match derived {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!("{e}");
                return;
            }
        };
        for mut t in triples {
            if t.source_event.is_none() {
                t.source_event = Some(r.id);
            }
            if let Err(e) = self.add_triple(ws, t, r.agent_id.as_deref()) {
                tracing::debug!("triple not stored: {e}");
            }
        }
    }

    /// The event with file paths made relative to the agent's worktree (or the repo), so
    /// knowledge-graph file entities are the same across worktrees: `file:src/app.ts`.
    fn relative_paths(&self, ws: Option<&str>, r: &EventRow) -> EventRow {
        let mut roots: Vec<String> = Vec::new();
        if let Some(a) = r.agent_id.as_deref() {
            if let Some(l) = lock(&self.live).get(a) {
                roots.push(l.cwd.to_string_lossy().into_owned());
            }
            if let Some(wt) = self.store.agent(a).ok().flatten().and_then(|a| a.worktree) {
                roots.push(wt);
            }
        }
        // The event's own repo: another office may be the open one.
        let open = self.workspace().filter(|w| Some(w.id.as_str()) == ws);
        if let Some(ws) = open.or_else(|| ws.and_then(|id| self.store.workspace(id).ok().flatten())) {
            roots.push(ws.repo_path);
        }
        let rel = |p: &str| -> String {
            roots
                .iter()
                .find_map(|root| p.strip_prefix(root.trim_end_matches('/')).and_then(|rest| rest.strip_prefix('/')))
                .unwrap_or(p)
                .to_string()
        };
        let mut out = r.clone();
        match &mut out.kind {
            EventKind::ToolStarted { target, .. } => {
                target.path = target.path.as_deref().map(rel);
                target.paths = target.paths.iter().map(|p| rel(p)).collect();
            }
            EventKind::FilesChanged { paths, .. } | EventKind::Committed { files: paths, .. } => {
                *paths = paths.iter().map(|p| rel(p)).collect();
            }
            _ => {}
        }
        out
    }

    /// Agent name / task key for readable entities, cached (neither changes in practice).
    /// Task keys repeat across plans ("t1"), so plan tasks get a plan suffix: `t1@b835`.
    fn label(&self, id: &str, agent: bool) -> Option<String> {
        if let Some(l) = lock(&self.labels).get(id) {
            return Some(l.clone());
        }
        let l = if agent {
            self.store.agent(id).ok().flatten().map(|a| a.name)
        } else {
            self.store.task(id).ok().flatten().map(|t| match t.plan_id {
                Some(p) => format!("{}@{}", t.key, &p[p.len().saturating_sub(4)..]),
                None => t.key,
            })
        }?;
        lock(&self.labels).insert(id.to_string(), l.clone());
        Some(l)
    }
}
