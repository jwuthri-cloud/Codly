//! The one event pipeline. Every `AdapterEvent` from every agent run goes through `ingest`,
//! in arrival order: persist (unless ephemeral) → broadcast → live-run bookkeeping → agent
//! projection → reactions → knowledge triples. Reactions that wait on anything (gates,
//! reviews, commits) are spawned, so a ten-minute verify never delays another agent's events.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;

use super::approvals::{self, PolicyCtx, Verdict};
use super::retry::{self, Failure};
use super::{Inner, guard, lead, lock, prompts, read, row, scheduler, write};
use crate::adapters::{AdapterEvent, AgentCmd};
use crate::{career, memory};
use crate::model::*;

/// How long Codly waits before it asks a worker whose turn failed to continue, one wait per
/// retry. The CLI retried the provider by itself before it gave up, so these are long enough
/// for a short outage to pass. After the last one, the next failed turn blocks the task.
pub(super) const TURN_RETRY_WAITS: [Duration; 2] = [Duration::from_secs(15), Duration::from_secs(60)];

/// Is `agent_id` the worker of this task, and may its turns still change it? Also in the
/// task's gate: you can message its worker there. Not in the merge: that work was checked.
fn works_on(t: &Task, agent_id: &str) -> bool {
    scheduler::is_active(t.status) && t.status != TaskStatus::Merging && t.assignee.as_deref() == Some(agent_id)
}

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
    /// Not live, and its row says `failed`: a start Codly gave up on (`spawn_failed`), whose CLI
    /// may still come up and exit. The other writer of `failed` is `on_exit`, at a run's last event.
    dead: bool,
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
                dead: false,
            };
        }
        // Not (or no longer) live: the run row is written before the spawn, so it exists.
        let run = self.store.run(run_id).ok().flatten();
        Ctx {
            agent_id: agent_id.into(),
            run_id: run_id.into(),
            role: run.as_ref().map_or(Role::Worker, |r| r.role),
            dead: run.as_ref().is_some_and(|r| r.status == "failed"),
            task_id: run.and_then(|r| r.task_id),
            ws: self.store.agent_workspace(agent_id).ok().flatten(),
        }
    }

    fn ingest(self: &Arc<Self>, ev: AdapterEvent) {
        self.watchdog_seen(&ev); // any event is a life sign, the live deltas below too
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
        // Only what is on record is shown (as `emit` does): a row the store refused would be a card
        // nobody can answer, gone at the next resync. The run's bookkeeping below still follows it.
        match self.store.append_event(ctx.ws.as_deref(), &r) {
            Ok(id) => {
                r.id = id;
                self.publish(ctx.ws.as_deref(), &r);
            }
            Err(e) => tracing::warn!("event not persisted ({}): {e}", r.kind.type_name()),
        }
        if ctx.dead {
            return; // on record, and nothing else: the agent and the row keep what the failure wrote
        }
        self.track_live(&ctx, &r.kind);
        self.breaker_observe(&ctx.agent_id, &r.kind);
        match &r.kind {
            EventKind::TurnCompleted { .. } => self.control_turn_done(&ctx.agent_id),
            EventKind::ToolStarted { name, .. } => self.control_tool_started(&ctx.agent_id, name),
            EventKind::TurnStarted { .. } => self.checkpoint_turn(&ctx.agent_id, &ctx.run_id), // the files before the turn
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
        l.interrupt.seen(kind);
        match kind {
            EventKind::TurnStarted { .. } => {
                l.turn_active = true;
                l.queued = l.queued.saturating_sub(1);
                l.turn_messages = 0;
                l.last_text = None;
                l.last_error = None;
            }
            // Delivered into the running turn: no turn of its own will take it off the count.
            EventKind::PromptSteered { .. } => l.queued = l.queued.saturating_sub(1),
            EventKind::Message { text, .. } if !text.trim().is_empty() => {
                l.last_text = Some(text.clone());
                l.turn_messages += 1;
            }
            EventKind::Error { message, .. } => l.last_error = Some(message.clone()),
            EventKind::TurnCompleted { status, .. } => {
                if !l.turn_active {
                    // A turn whose start we never saw still consumed its prompt.
                    l.queued = l.queued.saturating_sub(1);
                }
                l.turn_active = false;
                l.turns += 1;
                match status {
                    TurnStatus::Completed => l.failed_turns = 0,
                    TurnStatus::Failed | TurnStatus::MaxTurns => l.failed_turns += 1,
                    TurnStatus::Interrupted => {}
                }
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
                    // A 1:1 agent's session is its conversation with you: a task it takes (a
                    // Worker run, in the task's worktree) has its own, kept on the run.
                    if let Some(s) = session_id
                        && (a.role != Role::Solo || ctx.role == Role::Solo)
                    {
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
                    .map_or((true, None), |l| {
                        // A failed turn whose result is the error it just reported has said
                        // why: the thread shows that error, and the result does not repeat it
                        // as the agent's words. Any other result text is still posted.
                        let told = *status == TurnStatus::Failed && result_text.is_some() && l.last_error == *result_text;
                        (l.turn_messages > 0 || told, l.last_text.clone())
                    });
                if !had_message && let Some(t) = result_text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let _ = self.chat(ws, &thread(), &ctx.agent_id, t, None);
                }
                let reply = last.or_else(|| result_text.clone());
                self.on_turn_completed(ctx, *status, reply, result_text.as_deref());
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

    fn on_turn_completed(self: &Arc<Self>, ctx: &Ctx, status: TurnStatus, reply: Option<String>, result_text: Option<&str>) {
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
            Role::Worker if ctx.task_id.is_some() => match status {
                // Gate only when the worker is really done: its turn completed, and no queued
                // prompt (a user steer, a fix round) is about to start another turn. A turn that
                // completed while a failed turn's continue prompt is still owed answered something
                // else (a message from you, a note): the worker goes on with its work instead.
                TurnStatus::Completed => {
                    let task = ctx.task_id.clone().unwrap_or_default();
                    // Its worker runs: a failure after this turn is the first of a new row (`retry.rs`).
                    lock(&self.retries).forget(&task);
                    if self.is_quiet(&ctx.agent_id) && !self.continue_failed_turn(&ctx.agent_id) && !self.control_holding(&ctx.agent_id) {
                        let me = self.clone();
                        tokio::spawn(async move { me.gate(task).await });
                    }
                }
                // An error or the turn limit ended it: the work is not finished, so no gate.
                TurnStatus::Failed | TurnStatus::MaxTurns => self.turn_failed(ctx, status, result_text),
                // Whoever interrupted it (you, a control, the circuit breaker) says what is next.
                TurnStatus::Interrupted => {}
            },
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
    /// A team member keeps its task: the work is in its own worktree, on the task's branch,
    /// which no other worktree can check out. The task waits for the member's quota.
    fn hand_off(self: &Arc<Self>, ctx: &Ctx) {
        let task_id = ctx.task_id.clone().unwrap_or_default();
        let Ok(t) = self.task(&task_id) else { return };
        if !matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
            return;
        }
        self.stop_run(&ctx.agent_id); // stopping: on_exit will not call task_run_lost
        let provider = self.provider_of(&ctx.agent_id).map_or("The provider", |p| p.label());
        let member = self.agent(&ctx.agent_id).is_ok_and(|a| a.member.is_some());
        let next = if member { "waiting for its quota" } else { "handing off to the next free provider" };
        let reason = format!("{provider} hit its usage limit; {next}. Continue from the work already in this worktree.");
        // status_reason reaches a resumed session; a fresh one (another provider) gets this,
        // after what already waits for the task's next run (your answer, the gate's feedback).
        if let Err(e) = self.add_feedback(&task_id, &format!("[Codly] {reason}")) {
            tracing::warn!("the hand-off note of task {task_id} was not kept: {e}");
        }
        let moved = self.update_task(&task_id, |t| {
            t.status = TaskStatus::Ready;
            if !member {
                t.assignee = None;
            }
            t.status_reason = Some(reason.clone());
            Ok(())
        });
        let _ = self.update_agent(&ctx.agent_id, |a| {
            if a.task_id.as_deref() == Some(task_id.as_str()) {
                a.task_id = None;
            }
        });
        if moved.is_err() {
            return; // stopped or cancelled by you meanwhile: that stands, and nothing is retried
        }
        let ws = self.store.task_workspace(&task_id).ok().flatten();
        let _ = self.decide(ws.as_deref(), Some(&task_id), "retry", reason, json!({ "plan_id": t.plan_id, "provider": provider }));
        self.kick();
    }

    /// A worker's turn ended on an error or at its turn limit. Its work is not finished, so it
    /// never reaches the gate. While the run lives, Codly asks the same session to continue,
    /// once per wait in `turn_retry_waits`; the next failed turn after those blocks the task
    /// with the error. The continue prompt is owed from here (`Live::continue_owed`): the
    /// wait sends it, or `continue_failed_turn` when something else comes first. A run that is
    /// exiting (a crash ends its turn first) is left to `task_run_lost`, which restarts the
    /// worker.
    fn turn_failed(self: &Arc<Self>, ctx: &Ctx, status: TurnStatus, result_text: Option<&str>) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return; // quitting ends turns too: the next start resumes the task
        }
        let task_id = ctx.task_id.clone().unwrap_or_default();
        let run = lock(&self.live).get(&ctx.agent_id).filter(|l| l.run_id == ctx.run_id && !l.stopping).map(|l| {
            // Claude gives a failed turn's error as the turn's result; the others send an Error first
            // (their result is the agent's last message).
            let result = result_text.filter(|r| l.provider == Provider::Claude && !r.trim().is_empty());
            (l.failed_turns, l.turns, result.map(String::from).or_else(|| l.last_error.clone()))
        });
        let Some((failures, turns, error)) = run else { return };
        let Ok(t) = self.task(&task_id) else { return };
        if !works_on(&t, &ctx.agent_id) {
            return;
        }
        let what = match error.map(|e| e.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|e| !e.is_empty()) {
            _ if status == TurnStatus::MaxTurns => "it reached its turn limit".to_string(),
            Some(e) => prompts::clip(&e, 200),
            None => "the CLI gave no reason".to_string(),
        };
        // A full session fails every next turn the same way: the task starts again in a new
        // one, as a failure of its worker (`retry.rs`). Not for a task in its gate: that
        // worker goes on below.
        if status != TurnStatus::MaxTurns
            && retry::context_full(&what)
            && self.worker_failed(&task_id, &ctx.agent_id, Failure::ContextFull(what.clone()))
        {
            return;
        }
        let ws = ctx.ws.as_deref();
        let waits = read(&self.turn_retry_waits).clone();
        let detail = json!({ "plan_id": t.plan_id, "turn": status, "error": what, "failed_turns": failures });
        let Some(wait) = waits.get(failures.saturating_sub(1) as usize).copied() else {
            let reason = format!("blocked: the worker's turn failed {failures} times in a row ({what}); fix the cause, then Retry");
            // Checked in the write: stopped or cancelled by you meanwhile, or in its merge, it stays as it is.
            let mut blocked = false;
            let _ = self.update_task(&task_id, |t| {
                blocked = works_on(t, &ctx.agent_id);
                if blocked {
                    t.status = TaskStatus::Blocked;
                    t.status_reason = Some(reason.clone());
                }
                Ok(())
            });
            if !blocked {
                return;
            }
            // Its worker goes (stopping: on_exit will not call task_run_lost), and a reviewer that
            // was reading it: a Retry starts from the worker's session, with fresh fix rounds.
            self.release_task_agents(&t);
            let _ = self.decide(ws, Some(&task_id), "retry", reason, detail);
            if let Some(ws) = ctx.ws.clone() {
                let who = self.agent(&ctx.agent_id).map_or_else(|_| "the worker".into(), |a| a.name);
                let (me, thread) = (self.clone(), self.task_thread(&t));
                let text = prompts::turns_failed(&t, &who, failures, &what);
                tokio::spawn(async move { me.lead_followup(&ws, &thread, &text).await });
            }
            self.kick();
            return;
        };
        match lock(&self.live).get_mut(&ctx.agent_id).filter(|l| l.run_id == ctx.run_id) {
            Some(l) => l.continue_owed = Some(prompts::turn_failed(&t, &what)),
            None => return, // the run exited meanwhile: the restart continues the task
        }
        let reason = format!("the worker's turn failed ({what}); Codly asks it to continue (try {failures} of {})", waits.len());
        let _ = self.update_task(&task_id, |t| {
            if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        let _ = self.decide(ws, Some(&task_id), "retry", reason, detail);
        let me = self.clone();
        let (agent_id, run_id) = (ctx.agent_id.clone(), ctx.run_id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            // Still this run, and nothing happened since: no turn ended, runs or is queued. A
            // turn that came meanwhile (a message from you, a note) sends the prompt when it
            // completes. A run that exited meanwhile was a crash: the restart continues the task.
            let idle = lock(&me.live)
                .get(&agent_id)
                .is_some_and(|l| l.run_id == run_id && !l.stopping && l.turns == turns && !l.turn_active && l.queued == 0);
            if idle {
                me.continue_failed_turn(&agent_id);
            }
        });
    }

    /// Ask a worker to go on with the work its failed turn left, if that prompt is still owed
    /// (it is taken: it goes out once). True = it was owed, so the worker's last turn is not
    /// its finish and the gate must not run. A worker you hold keeps the prompt for your Resume.
    pub(super) fn continue_failed_turn(&self, agent_id: &str) -> bool {
        let owed = lock(&self.live).get_mut(agent_id).and_then(|l| Some((l.continue_owed.take()?, l.task_id.clone()?)));
        let Some((prompt, task_id)) = owed else { return false };
        // No longer its task to change (you moved it, or it is in its merge): nothing to continue.
        if !self.task(&task_id).is_ok_and(|t| works_on(&t, agent_id)) {
            return false;
        }
        if let Err(e) = self.send_prompt(agent_id, prompt) {
            tracing::debug!("the worker was not asked to continue: {e}");
        }
        true
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
            Verdict::Allow { choice, rule } if mode != base => {
                Verdict::Allow { choice, rule: format!("{rule} (you trust it to work on its own)") }
            }
            v => v,
        };
        // What the rules would ask you about, a rule you saved for this repo may answer.
        let verdict = self.with_saved_rules(ws, approval, &pctx, verdict);
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
        // Its adapter ended it because the turn did not stop after an interrupt: Codly's doing
        // too, and no crash (interrupts.rs). A run Codly also asked to stop is the stopper's.
        let gave_up = removed.as_ref().filter(|l| !l.stopping).and_then(|l| l.gave_up(&ctx.agent_id));
        let ended = stopping || gave_up.is_some();
        // It exited during its handshake: `spawn_run` owns this start and fails it, and its
        // caller may already be starting the next one (resume, then a new session).
        let starting = removed.as_ref().is_some_and(|l| l.starting.is_some());
        let shutting = self.shutting_down.load(Ordering::SeqCst);
        let status = if shutting {
            "lost" // resumed by session id on the next start
        } else if !starting && (ended || code == Some(0)) {
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
        let unexpected = !ended && code != Some(0);
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
        if let Some(gave_up) = gave_up {
            // Who asked for the interrupt says what is next.
            let me = self.clone();
            tokio::spawn(async move { me.interrupt_gave_up(gave_up).await });
        } else if !stopping && !starting && let Some(task_id) = ctx.task_id.clone() {
            let me = self.clone();
            let (role, agent_id, run) = (ctx.role, ctx.agent_id.clone(), ctx.run_id.clone());
            let error = removed.and_then(|l| l.last_error);
            tokio::spawn(async move { me.task_run_lost(task_id, role, agent_id, run, code, error).await });
        }
        self.kick();
    }

    /// A task's worker or reviewer exited without Codly asking it to. `error`: the last error
    /// its CLI reported.
    async fn task_run_lost(self: Arc<Self>, task_id: Id, role: Role, agent_id: Id, run_id: Id, code: Option<i64>, error: Option<String>) {
        let Ok(t) = self.task(&task_id) else { return };
        match role {
            Role::Reviewer if t.status == TaskStatus::Reviewing => self.review_done(task_id, run_id).await,
            // The worker starts again after a wait; its third failure in a row blocks the task.
            Role::Worker => {
                self.worker_failed(&task_id, &agent_id, Failure::Exit { code, error });
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

impl super::Core {
    /// The waits before Codly asks a worker whose turn failed to continue, one per retry
    /// (`TURN_RETRY_WAITS` until set; the tests shorten them).
    pub fn set_turn_retry_waits(&self, waits: Vec<Duration>) {
        *write(&self.inner.turn_retry_waits) = waits;
    }
}
