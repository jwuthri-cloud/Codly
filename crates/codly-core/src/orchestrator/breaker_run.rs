//! Enforcement for the circuit breaker (policy: `breaker`). Events feed the tracker from the
//! ingest pipeline; a beat every `BEAT_MS` asks the policy what to do and does it with the
//! mechanisms that already exist: a prompt (rides the agent's next turn; Codex steers it into
//! the running turn), an interrupt, `stop_agent`. Every level change is a `Decision` on the
//! agent's timeline. The Lead is exempt: it only talks to you.
//!
//! A task's worker that is constrained is also paused ("stop after this step"), so the turn in
//! which it answers the note never reaches the gate as finished work; Resume sends it back to
//! the task. One you hold already gets no note to answer: it reads why on your Resume. A stop
//! says on the task that the breaker did it, and why.
//!
//! The budgets (tokens, cost) stop spend, not only steer it. What the team used is counted from
//! every usage report and saved with it, so the count is the same after a restart. Past a
//! budget the scheduler starts no task (`budget_hold`), only agents in a turn are steered and
//! constrained, and you are told once, with the way out: raise the budget or Reset.
//!
//! Inside a turn, a Claude run is held by Claude Code itself: it starts with what is left of
//! the cost budget as its own limit (`run_budget`). A turn that reaches it says why
//! (`run_limit_words`), and Codly ends that run, which can do no more work
//! (`run_limit_turn_end`): the next message starts a new one with what is left then.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::breaker::{self, Action, BEAT_MS, BreakerConfig, BreakerLevel, Spent, Today, Tracker, Transition, Trip};
use super::interrupts::Asker;
use super::retry::Failure;
use super::{Core, Inner, lock, row, scheduler};
use crate::adapters::claude;
use crate::model::*;

const KV_KEY: &str = "breaker";
/// What the team used since the last Reset, and today (`Saved`).
const KV_SPENT: &str = "breaker_spent";

/// The counts that outlive the app.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    floor: Spent,
    today: Today,
}

/// The settings and the signals, behind one lock.
#[derive(Default)]
pub(super) struct Runtime {
    cfg: BreakerConfig,
    tracker: Tracker,
    today: Today,
    /// A budget is used up, and you were told so.
    holding: bool,
    /// The live Claude runs that started with a limit of their own: run → (agent name, limit).
    limits: HashMap<Id, (String, f64)>,
    /// The runs whose turn reached that limit → what that turn says (`run_limit_words`).
    reached: HashMap<Id, String>,
}

impl Runtime {
    /// The saved settings (defaults when none, or when the saved JSON is from another version),
    /// and the saved counts.
    pub(super) fn load(store: &crate::store::Store) -> Runtime {
        let cfg = store.kv_get(KV_KEY).ok().flatten().and_then(|s| serde_json::from_str::<BreakerConfig>(&s).ok()).unwrap_or_default();
        let read = || -> anyhow::Result<Saved> {
            Ok(match store.kv_get(KV_SPENT)? {
                Some(s) => serde_json::from_str(&s)?,
                None => Saved::default(),
            })
        };
        let saved = read().unwrap_or_else(|e| {
            tracing::warn!("the saved budget count cannot be read, so it starts from zero: {e}");
            Saved::default()
        });
        let cfg = cfg.normalized();
        // Over a budget when you quit: you were told then, and the waiting tasks still say why.
        let holding = breaker::over_budget(&cfg, saved.floor).is_some();
        Runtime { cfg, tracker: Tracker::resumed(saved.floor), today: saved.today, holding, ..Runtime::default() }
    }

    fn saved(&self) -> Saved {
        Saved { floor: self.tracker.floor(), today: self.today.clone() }
    }
}

/// What Settings shows next to the limits.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BreakerStatus {
    /// What every agent used together since the last Reset: the budgets are checked against it.
    pub used: Spent,
    /// What every agent used today (your local day).
    pub today: Spent,
    /// Why no new task starts: the budget that is used up. None: work goes on.
    pub hold: Option<String>,
    pub agents: Vec<AgentBreaker>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentBreaker {
    pub agent_id: Id,
    pub level: BreakerLevel,
    pub work_tokens: i64,
}

pub(super) async fn beat_loop(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(Duration::from_millis(BEAT_MS as u64)).await;
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        inner.breaker_beat().await;
    }
}

impl Inner {
    /// Called by the ingest pipeline for every persisted event.
    pub(super) fn breaker_observe(&self, agent_id: &str, kind: &EventKind) {
        let now = now_ms();
        let saved = {
            let mut rt = lock(&self.breaker);
            let added = rt.tracker.observe(agent_id, now, kind);
            if added == Spent::default() {
                return;
            }
            rt.today.add(&crate::career::day_of(now), added);
            rt.saved()
        };
        // Saved with every usage report: a restart (or a crash) does not give the budget back.
        if let Err(e) = self.breaker_save(&saved) {
            tracing::warn!("budget count not saved: {e}");
        }
    }

    fn breaker_save(&self, saved: &Saved) -> anyhow::Result<()> {
        self.store.kv_set(KV_SPENT, &serde_json::to_string(saved)?)
    }

    /// Why no new task starts: the budget the team is over. The scheduler asks at every pass.
    pub(super) fn budget_hold(&self) -> Option<String> {
        let rt = lock(&self.breaker);
        breaker::over_budget(&rt.cfg, rt.tracker.floor()).map(|b| b.hold())
    }

    /// The limit of its own a run starts with: for a Claude run, what is left of the cost budget
    /// (`SpawnSpec::max_budget_usd`). Kept with the run, to know the turn that reaches it.
    pub(super) fn run_budget(&self, run_id: &str, agent_name: &str, provider: Provider) -> Option<f64> {
        if provider != Provider::Claude {
            return None;
        }
        let mut rt = lock(&self.breaker);
        let rest = breaker::rest_usd(&rt.cfg, rt.tracker.floor())?;
        rt.limits.insert(run_id.to_string(), (agent_name.to_string(), rest));
        Some(rest)
    }

    /// The words of a turn that reached its run's own limit (the error it reports, then its
    /// end), before they are on record. When the team's budget is now used up: that budget and
    /// the way out. Else what comes next for this run's `role` (`task`: the run works on a
    /// task): a task's worker and reviewer are not told to send a message, since Codly moves
    /// their task on (`run_limit_turn_end`), and a Lead's turn may have been started by Codly
    /// (a worker's question, a follow-up), not by a message of yours. Any other agent: the
    /// run's own words, which ask you to send your message again.
    pub(super) fn run_limit_words(&self, run_id: &str, role: Role, task: bool, kind: &mut EventKind) {
        let said = match kind {
            EventKind::Error { message, .. } => message,
            EventKind::TurnCompleted { status: TurnStatus::Failed, result_text: Some(text), .. } => text,
            _ => return,
        };
        let mut rt = lock(&self.breaker);
        let Some((agent, limit)) = rt.limits.get(run_id).cloned() else { return };
        if *said != claude::run_limit_reached(&agent, limit) {
            return;
        }
        // The error and the turn's end say the same, even if the count moved in between.
        let words = match rt.reached.get(run_id) {
            Some(words) => words.clone(),
            None => {
                let reached = format!("This run of {agent} reached the limit it started with ({limit:.2} USD of your budget were left then).");
                let words = match (breaker::over_budget(&rt.cfg, rt.tracker.floor()), role) {
                    (Some(budget), _) => budget.reached(),
                    (None, Role::Worker) if task => format!("{reached} Codly ends this run; the task's card says what comes next."),
                    (None, Role::Reviewer) if task => format!("{reached} Its review stops here: re-run the checks of the task to review it again."),
                    (None, Role::Lead) => format!("{reached} The next message to {agent} starts a new run with what is left now."),
                    (None, _) => said.clone(),
                };
                rt.reached.insert(run_id.to_string(), words.clone());
                words
            }
        };
        *said = words;
    }

    /// A turn that reached its run's own limit is over, and the run can do no more work (every
    /// later turn fails at once): Codly ends it, and moves its task on.
    /// - A task's worker: its task waits for a new run with what is left then, never for the
    ///   gate. Past the budget it waits for the budget (`worker_waits_for_budget`), and that
    ///   stop is no failure of the worker's: only the scheduler's hold bounds its restarts.
    ///   With budget left (raised while the turn ran) it fails the way a worker does
    ///   (`worker_failed`), and that counted row stops a restart loop.
    /// - A task's reviewer: the review stops, and the task waits for you (`review_stopped`).
    /// - Any other run just ends: the next message starts a new one.
    pub(super) fn run_limit_turn_end(self: &Arc<Self>, agent_id: &str, run_id: &str, role: Role, task_id: Option<&str>, kind: &EventKind) {
        if !matches!(kind, EventKind::TurnCompleted { status: TurnStatus::Failed, .. }) {
            return;
        }
        let (limit, over) = {
            let mut rt = lock(&self.breaker);
            if rt.reached.remove(run_id).is_none() {
                return;
            }
            let Some((_, limit)) = rt.limits.remove(run_id) else { return };
            (limit, breaker::over_budget(&rt.cfg, rt.tracker.floor()))
        };
        let moved = match (role, task_id, over) {
            (Role::Worker, Some(task_id), Some(budget)) => self.worker_waits_for_budget(task_id, agent_id, &budget.hold()),
            (Role::Worker, Some(task_id), None) => {
                let what = format!("the worker's run reached the limit it started with ({limit:.2} USD of your budget)");
                self.worker_failed(task_id, agent_id, Failure::Limit(what))
            }
            (Role::Reviewer, Some(task_id), _) => {
                let reason = match over {
                    Some(budget) => format!("blocked: the review stopped: {}; raise the budget, then re-run the checks", budget.reason()),
                    None => format!("blocked: the reviewer's run reached the limit it started with ({limit:.2} USD of your budget); re-run the checks to review it again"),
                };
                self.review_stopped(task_id, agent_id, run_id, reason)
            }
            _ => false,
        };
        if !moved {
            self.stop_run_if(agent_id, |l| l.run_id == run_id);
        }
    }

    /// A worker's run stopped at the team's budget: its task goes back to Ready, still its
    /// worker's (the next run resumes its session), with the budget's hold as its reason, and
    /// its run ends. False: the task is not this worker's to move any more (you stopped or
    /// cancelled it, or it is past its worker's part).
    fn worker_waits_for_budget(self: &Arc<Self>, task_id: &str, agent_id: &str, hold: &str) -> bool {
        let mut mine = false;
        let written = self.update_task(task_id, |t| {
            mine = matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) && t.assignee.as_deref() == Some(agent_id);
            if mine {
                t.status = TaskStatus::Ready;
                t.status_reason = Some(hold.to_string());
            }
            Ok(())
        });
        if !mine || written.is_err() {
            return false;
        }
        self.release_worker(agent_id, task_id);
        true
    }

    /// The reviewer's run of the review under way stopped at its limit, before a verdict: the
    /// review ends here, and the task waits for you, Blocked with `reason`, its worker and
    /// reviewer released. Merged without its review, the work would skip the review; nudged,
    /// the reviewer would find its run ending. False: the task is not in this review any more,
    /// or the run is not the one of the review under way (an earlier round's run that was
    /// told to stop).
    fn review_stopped(self: &Arc<Self>, task_id: &str, reviewer: &str, run_id: &str, reason: String) -> bool {
        if !lock(&self.live).get(reviewer).is_some_and(|l| l.run_id == run_id && !l.stopping) {
            return false;
        }
        let Ok(task) = self.task(task_id) else { return false };
        let mut mine = false;
        let written = self.update_task(task_id, |t| {
            mine = t.status == TaskStatus::Reviewing;
            if mine {
                t.status = TaskStatus::Blocked;
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        if !mine || written.is_err() {
            return false;
        }
        let ws = self.store.task_workspace(task_id).ok().flatten();
        let _ = self.decide(ws.as_deref(), Some(task_id), "review", format!("{}: {reason}", task.key), json!({ "plan_id": task.plan_id }));
        self.release_worker(reviewer, task_id);
        if let Some(worker) = task.assignee.as_deref() {
            self.release_worker(worker, task_id);
        }
        if let Some(ws) = ws {
            let me = self.clone();
            let (thread, text) = (self.task_thread(&task), format!("Task {} \"{}\" is {reason}.", task.key, task.title));
            tokio::spawn(async move { me.lead_followup(&ws, &thread, &text).await });
        }
        self.kick();
        true
    }

    /// Say it once when a budget is used up, and once when work goes on again: a line in every
    /// conversation with a running plan (in the open office's first conversation when no plan
    /// runs), and a decision, which the app also shows as a notification.
    fn budget_announce(&self) {
        let (over, was) = {
            let mut rt = lock(&self.breaker);
            let over = breaker::over_budget(&rt.cfg, rt.tracker.floor());
            (over, std::mem::replace(&mut rt.holding, over.is_some()))
        };
        let (summary, next, detail) = match (over, was) {
            (Some(budget), false) => (
                format!("Budget reached: {}. No new task starts.", budget.reason()),
                " Agents in a turn are asked to stop. To go on, raise the budget or press Reset counters in Settings → Autonomy & Budgets.",
                json!({ "action": "budget", "level": BreakerLevel::Constrained, "trip": budget.kind(), "reason": budget.reason() }),
            ),
            (None, true) => (
                "The budget has room again: waiting tasks start.".to_string(),
                " An agent the circuit breaker paused still waits for your Resume.",
                json!({ "action": "budget_cleared", "level": BreakerLevel::Healthy }),
            ),
            _ => return,
        };
        let mut threads: Vec<(Id, String)> = match self.store.plans_with_status(&[PlanStatus::Running]) {
            Ok(plans) => plans.into_iter().map(|p| (p.workspace_id, p.thread)).collect(),
            Err(e) => {
                tracing::warn!("running plans not read for the budget message: {e}");
                vec![]
            }
        };
        threads.sort();
        threads.dedup();
        if threads.is_empty()
            && let Some(ws) = self.workspace()
        {
            threads.push((ws.id, default_lead_thread()));
        }
        for (ws, thread) in &threads {
            if let Err(e) = self.chat(Some(ws.as_str()), thread, "codly", &format!("{summary}{next}"), None) {
                tracing::warn!("budget message not posted: {e}");
            }
        }
        if let Err(e) = self.decide(None, None, "breaker", summary, detail) {
            tracing::warn!("budget event not recorded: {e}");
        }
    }

    /// One look at the floor: decide, enforce, record.
    pub(super) async fn breaker_beat(&self) {
        self.budget_announce();
        // The limits of runs that are over.
        let runs: HashSet<Id> = lock(&self.live).values().map(|l| l.run_id.clone()).collect();
        {
            let mut rt = lock(&self.breaker);
            rt.limits.retain(|run, _| runs.contains(run));
            rt.reached.retain(|run, _| runs.contains(run));
        }
        let leads: HashSet<Id> = lock(&self.live).iter().filter(|(_, l)| l.role == Role::Lead).map(|(id, _)| id.clone()).collect();
        let transitions = {
            let mut rt = lock(&self.breaker);
            let cfg = rt.cfg.clone();
            rt.tracker.beat(&cfg, now_ms(), &|id| leads.contains(id))
        };
        for t in transitions {
            self.breaker_enforce(t).await;
        }
    }

    async fn breaker_enforce(&self, t: Transition) {
        let Ok(agent) = self.agent(&t.agent) else { return };
        let ws = self.store.agent_workspace(&t.agent).ok().flatten();
        // The task it holds (as `stop_agent` reads it), and whether it is that task's worker.
        let held = agent.task_id.clone().or_else(|| lock(&self.live).get(&t.agent).and_then(|l| l.task_id.clone()));
        let task = held.and_then(|id| self.task(&id).ok());
        let work = task.as_ref().filter(|x| scheduler::is_active(x.status) && x.assignee.as_deref() == Some(t.agent.as_str()));
        // Send first, record after: a note that could not be delivered is said so in the event.
        let delivered = match (t.action, &t.trip) {
            (Action::Steer, Some(trip)) => self.send_prompt(&t.agent, breaker::note(BreakerLevel::Steering, trip)).is_ok(),
            (Action::Constrain, Some(trip)) => match work {
                // A task's worker is paused too.
                Some(task) => self.breaker_pause(ws.as_deref(), &agent, task, trip),
                None => {
                    let interrupted = self.interrupt_run(&t.agent, Asker::Breaker);
                    interrupted && self.send_prompt(&t.agent, breaker::note(BreakerLevel::Constrained, trip)).is_ok()
                }
            },
            (Action::Stop, trip) => {
                let why = trip.as_ref().map(|x| format!(": {}", x.reason())).unwrap_or_default();
                let stopped = self.stop_agent_as(&t.agent, &format!("the circuit breaker{why}")).await.is_ok();
                // Its task is blocked now: say so where you follow that task.
                if let Some(task) = task.as_ref().filter(|x| self.task(&x.id).is_ok_and(|now| now.status == TaskStatus::Blocked)) {
                    let text =
                        format!("The circuit breaker stopped {} on {} \"{}\"{why}. The task is blocked until you retry it.", agent.name, task.key, task.title);
                    let _ = self.chat(ws.as_deref(), &self.task_thread(task), "codly", &text, None);
                }
                stopped
            }
            _ => true,
        };
        let name = &agent.name;
        let (verb, why) = (
            match t.action {
                Action::Steer => "steered",
                Action::Constrain => "constrained",
                Action::Stop => "stopped",
                Action::None => "recovered",
            },
            t.trip.as_ref().map(|x| x.reason()),
        );
        let summary = match (&why, t.action) {
            (Some(w), _) => format!("Circuit breaker {verb} {name}: {w}"),
            (None, _) => format!("Circuit breaker: {name} is {} again", t.to.label()),
        };
        let detail = json!({
            "level": t.to,
            "from": t.from,
            "action": verb,
            "trip": t.trip.as_ref().map(|x| x.kind()),
            "reason": why,
            "delivered": delivered,
            "hard_stop": lock(&self.breaker).cfg.hard_stop,
        });
        let kind = EventKind::Decision { id: new_id(), decision_type: "breaker".into(), method: "rule".into(), summary, detail };
        if let Err(e) = self.emit(ws.as_deref(), row(Some(&t.agent), agent.task_id.as_deref(), agent.run_id.as_deref(), kind)) {
            tracing::warn!("breaker event not recorded: {e}");
        }
    }

    /// Constrain a task's worker. The turn that answers the note is a summary, not finished
    /// work: the worker is paused before its turn is interrupted, so no turn that ends from
    /// here on reaches the gate, and the note goes past the pause. A worker you hold already
    /// gets no note: held, it would be answered after your Resume, and that answer would reach
    /// the gate. Its turn is interrupted, and it reads why when you resume it. Your Resume
    /// sends the worker back to its task. True = the worker got the note, or will read why on
    /// your Resume.
    fn breaker_pause(&self, ws: Option<&str>, agent: &Agent, task: &Task, trip: &Trip) -> bool {
        let (interrupted, held) = {
            // Under the `live` lock: an exit (which clears the controls) is before or after, not between.
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(&agent.id) else { return false };
            let mut controls = lock(&self.controls);
            let control = controls.entry(agent.id.clone()).or_default();
            let held = control.holding();
            // The note is its next turn: the pause holds from now and shows when a turn ends.
            control.request_stop(true);
            (l.send_interrupt(Asker::Breaker), held)
        };
        if !interrupted {
            return false; // its process is gone: the exit clears the pause
        }
        let note = (!held).then(|| self.deliver_prompt(&agent.id, breaker::note(BreakerLevel::Constrained, trip)));
        // Held until the resume (holding it also tells the UI about the pause).
        let resume = self.send_prompt(&agent.id, breaker::resume_note(held.then_some(trip)));
        if let Err(e) = &resume {
            tracing::debug!("breaker resume note not held: {e}");
        }
        let why = trip.reason();
        let reason = format!("paused by the circuit breaker: {why}; resume {} to continue", agent.name);
        let _ = self.update_task(&task.id, |t| {
            if matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) {
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        let text = format!(
            "The circuit breaker paused {} on {} \"{}\": {why}. Its work is not checked while it is paused. Resume {} to let it continue, or stop it.",
            agent.name, task.key, task.title, agent.name
        );
        let _ = self.chat(ws, &self.task_thread(task), "codly", &text, None);
        note.unwrap_or(resume).is_ok()
    }
}

impl Core {
    pub async fn breaker_config(&self) -> anyhow::Result<BreakerConfig> {
        Ok(lock(&self.inner.breaker).cfg.clone())
    }

    pub async fn set_breaker_config(&self, cfg: BreakerConfig) -> anyhow::Result<BreakerConfig> {
        let cfg = cfg.normalized();
        self.inner.store.kv_set(KV_KEY, &serde_json::to_string(&cfg)?)?;
        lock(&self.inner.breaker).cfg = cfg.clone();
        // A raised budget lets waiting tasks start now (not at the scheduler's next slow tick);
        // a lowered one may hold them.
        self.inner.budget_announce();
        self.inner.kick();
        Ok(cfg)
    }

    /// Start the floor's counts and every level over (running agents keep running). Today's
    /// total stays: it is what was used, not what a budget is checked against.
    pub async fn reset_breaker(&self) -> anyhow::Result<()> {
        let saved = {
            let mut rt = lock(&self.inner.breaker);
            rt.tracker.reset();
            rt.saved()
        };
        self.inner.breaker_save(&saved)?;
        self.inner.budget_announce();
        self.inner.kick();
        Ok(())
    }

    /// Look at the floor now, as the beat loop does every `BEAT_MS` (the tests do not wait for it).
    pub async fn breaker_beat(&self) {
        self.inner.breaker_beat().await;
    }

    pub async fn breaker_status(&self) -> anyhow::Result<BreakerStatus> {
        let rt = lock(&self.inner.breaker);
        let agents = rt.tracker.snapshot().into_iter().map(|(agent_id, level, work_tokens)| AgentBreaker { agent_id, level, work_tokens }).collect();
        let used = rt.tracker.floor();
        Ok(BreakerStatus {
            used,
            today: rt.today.on(&crate::career::day_of(now_ms())),
            hold: breaker::over_budget(&rt.cfg, used).map(|b| b.reason()),
            agents,
        })
    }

    /// What a plan's tasks used so far: the total of every worker and reviewer run on them.
    /// The Lead's own turns are not in it (they belong to the conversation, not to one plan).
    pub async fn plan_spent(&self, plan_id: &str) -> anyhow::Result<Spent> {
        let store = &self.inner.store;
        self.inner.plan(plan_id)?;
        let mut total = Spent::default();
        for task in store.plan_tasks(plan_id)? {
            total.add(store.task_run_usage(&task.id)?.iter().map(Spent::of).sum());
        }
        Ok(total)
    }
}
