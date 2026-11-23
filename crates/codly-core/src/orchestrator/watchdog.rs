//! The watchdog: no agent sits "running" without end. Every `BEAT` it looks at each live run.
//!
//! - **A silent turn.** A run that is in a turn (or was sent a prompt that no turn took) and
//!   gave no event for `quiet_secs` (`quiet_tool_secs` while a tool call is open that gives
//!   no heartbeat: `Watch::limit_secs`) is flagged:
//!   "stalled: no output for N min" on its task, and a `Decision` the app notifies you of.
//!   Still silent at twice the limit, its turn is interrupted and it is asked to continue,
//!   once. Silent for one more limit after that, the run is stopped: a worker's task goes
//!   back in line (blocked from its third start on), a review ends without a verdict, a Lead
//!   or a 1:1 agent resumes with your next message. The same follows when the CLI does not
//!   stop at the interrupt and its adapter ends the process (`interrupts`). A run that waits
//!   for your answer to an approval or a question is not silent.
//! - **An idle worker.** A task's worker that sits between turns with nothing queued for
//!   `idle_secs`, on a task that is still running (you interrupted it, a control stopped its
//!   turn), is flagged the same way: "idle: …". Nothing is done to it: what is next is yours
//!   to say.
//!
//! Every adapter event of a run is a life sign, the live deltas too (`Inner::watchdog_seen`,
//! first thing in the ingest pipeline); an account's usage reading is not, since every agent
//! of the account gets it. Claude Code says every 30 s that a tool call still runs
//! (`ToolProgress`): such a call is not silent, however long it takes, while a CLI that hangs
//! in it says nothing and is. That heartbeat is recorded for a Bash call, a tool of an MCP
//! server and the `Agent` call that waits for a helper (`has_heartbeat`). So a Claude run
//! whose open calls are all of these kinds is silent after `quiet_secs`, like a turn with no
//! call open: 20 heartbeats that did not come. With any other tool open (WebFetch and
//! WebSearch are not recorded), and for every other CLI, which is not known to give such a
//! sign, the limit is the long `quiet_tool_secs`.
//! The clock is an `Instant`: a laptop that slept made no turn silent.
//! The Lead gets no continue prompt (its turns answer you or a worker): its turn is
//! interrupted, and the thread says so.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::interrupts::Asker;
use super::{Core, Inner, Live, lead, lock, row};
use crate::adapters::AdapterEvent;
use crate::model::*;

/// How often the watchdog looks.
const BEAT: Duration = Duration::from_secs(15);
/// A run that has been quiet this long is asked whether it waits for you (a store read).
const ASK_AFTER: Duration = Duration::from_secs(30);
/// How long a stopped run gets to exit before its work is put back in line (the adapters'
/// shutdown ladder takes up to ~13 s).
const EXIT_WAIT: Duration = Duration::from_secs(15);
/// A worker that went silent is restarted, until its task was started this many times.
const MAX_STARTS: i64 = 3;

/// Settings: when the watchdog steps in. A limit of 0 turns that check off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct WatchdogSettings {
    /// Seconds a turn may give no output before it is flagged. At twice this it is interrupted
    /// and asked to continue; silent for as long again, it is stopped.
    pub quiet_secs: u32,
    /// The same limit while a tool call is open: a build or a test suite can be quiet for long.
    pub quiet_tool_secs: u32,
    /// Seconds a task's worker may sit between turns, its task still running, before the task
    /// is flagged.
    pub idle_secs: u32,
}

impl Default for WatchdogSettings {
    fn default() -> Self {
        WatchdogSettings { quiet_secs: 600, quiet_tool_secs: 3600, idle_secs: 120 }
    }
}

/// What a task's `status_reason` starts with while the watchdog flags it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flag {
    Stalled,
    Idle,
}

impl Flag {
    fn prefix(self) -> &'static str {
        match self {
            Flag::Stalled => "stalled: ",
            Flag::Idle => "idle: ",
        }
    }
}

/// Does Claude Code say every 30 s that a call of this tool still runs (its `tool_progress`
/// line, `ToolProgress`)? Recorded on 2.1.311 for a Bash call, a tool of an MCP server and
/// the `Agent` call that waits for a helper (listed as `Task` by the CLI), once each
/// (docs/research/cli-claude-stream-json-2.1.311-part2.md §8 and -part3.md §3). Not recorded,
/// so not counted on: WebFetch, WebSearch and every other tool.
fn has_heartbeat(tool: &str) -> bool {
    matches!(tool, "Bash" | "Agent" | "Task") || tool.starts_with("mcp__")
}

/// A tool call that started and did not complete.
#[derive(Debug, Clone)]
struct OpenCall {
    id: String,
    /// The provider's name of the tool ("Bash", "mcp__linear__search").
    name: String,
    /// A helper of the agent made it (the event after its start said so).
    helpers: bool,
}

/// What the watchdog remembers about one live run (`Live::watch`).
#[derive(Debug, Clone)]
pub(super) struct Watch {
    /// When the run's last event came, or when it was last seen waiting for you.
    last_event: Instant,
    /// Tool calls started and not completed in the current turn.
    open_tools: Vec<OpenCall>,
    /// Its silence is flagged ("stalled: …").
    flagged: bool,
    /// When the watchdog interrupted its turn. A turn that completes clears it: the next
    /// silence starts over with a flag.
    nudged: Option<Instant>,
    /// The continue prompt that follows that interrupt once the turn has ended, with the
    /// silence it reports.
    owes_continue: Option<Duration>,
    /// Its task is flagged as idle ("idle: …").
    idle_flagged: bool,
}

impl Watch {
    pub(super) fn new() -> Watch {
        Watch { last_event: Instant::now(), open_tools: Vec::new(), flagged: false, nudged: None, owes_continue: None, idle_flagged: false }
    }

    /// The seconds a busy run of `provider` may give no event. With no tool call open:
    /// `quiet_secs`. With one open: the long `quiet_tool_secs`, but for a Claude run whose
    /// own open calls all give a heartbeat while they run (`has_heartbeat`): a missing
    /// heartbeat is silence, so the plain limit holds. A helper's calls do not count for
    /// that: recorded, a Bash call inside a helper has no heartbeat of its own, and the
    /// `Agent` call that waits for the helper has. With only calls of helpers open (a helper
    /// that works in the background) nothing is known to give a sign: the long limit.
    fn limit_secs(&self, provider: Provider, cfg: &WatchdogSettings) -> u32 {
        if self.open_tools.is_empty() {
            return cfg.quiet_secs;
        }
        let mut own = self.open_tools.iter().filter(|call| !call.helpers).peekable();
        if provider == Provider::Claude && own.peek().is_some() && own.all(|call| has_heartbeat(&call.name)) {
            return cfg.quiet_secs;
        }
        cfg.quiet_tool_secs
    }

    /// An event of the run came. `queued`: the prompts it was sent that no turn has taken yet.
    fn seen(&mut self, kind: &EventKind, now: Instant, queued: u32) {
        // The account's usage reaches every agent on it: it says nothing about this run.
        if matches!(kind, EventKind::RateLimit { .. }) {
            return;
        }
        self.last_event = now;
        match kind {
            EventKind::ToolStarted { call_id, name, .. } => self.open_tools.push(OpenCall { id: call_id.clone(), name: name.clone(), helpers: false }),
            // The call that just started is a helper's (Claude Code: the event comes right
            // after the call's `ToolStarted`).
            EventKind::SubAgentProgress { tool_call_id: Some(id), .. } => {
                if let Some(call) = self.open_tools.iter_mut().find(|call| call.id == *id) {
                    call.helpers = true;
                }
            }
            EventKind::ToolCompleted { call_id, .. } => {
                // (a call whose start was not seen closes nothing)
                if let Some(at) = self.open_tools.iter().position(|call| call.id == *call_id) {
                    self.open_tools.remove(at);
                }
            }
            EventKind::TurnStarted { .. } => self.open_tools.clear(),
            EventKind::TurnCompleted { status, .. } => {
                self.open_tools.clear();
                // It finished what it was asked: its one chance is back. Not while a prompt is
                // still counted as waiting: a run that never takes it stays busy and silent, and
                // must reach its stop instead of being asked to continue without end.
                if *status == TurnStatus::Completed && queued == 0 {
                    self.nudged = None;
                }
            }
            _ => {}
        }
    }
}

/// What is next for a run that is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Wait,
    /// It gives output again: take the flag off.
    Clear,
    Flag,
    /// Interrupt its turn and ask it to continue.
    Nudge,
    Stop,
}

/// The policy for a busy run that has been quiet for `quiet`. `limit` is the silence allowed
/// (zero: the check is off), `since_nudge` how long ago its turn was interrupted, if it was.
fn step(quiet: Duration, limit: Duration, flagged: bool, since_nudge: Option<Duration>) -> Step {
    if limit.is_zero() || quiet < limit {
        return if flagged { Step::Clear } else { Step::Wait };
    }
    match since_nudge {
        // Interrupted, and a whole limit later it is silent again, or silent still.
        Some(since) if since >= limit => Step::Stop,
        Some(_) => Step::Wait,
        None if !flagged => Step::Flag,
        None if quiet >= limit * 2 => Step::Nudge,
        None => Step::Wait,
    }
}

/// "45 s", "10 min": a silence, in the words a task card shows.
fn span(d: Duration) -> String {
    match d.as_secs() {
        s if s < 120 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

/// Is `agent_id` the worker of this task, with the task still its to work on?
pub(super) fn working(t: &Task, agent_id: &str) -> bool {
    matches!(t.status, TaskStatus::Running | TaskStatus::ChangesRequested) && t.assignee.as_deref() == Some(agent_id)
}

/// What an agent reads after the watchdog interrupted its silent turn.
fn continue_prompt(role: Role, task: Option<&Task>, quiet: &str) -> String {
    const HANG: &str = "If a command did not return, do not run it the same way again.";
    match (role, task) {
        (Role::Worker, Some(t)) => format!(
            "[Codly] Your last turn on task {} \"{}\" gave no output for {quiet}, so Codly interrupted it. {HANG} Run `git status` \
             and `git diff` in your worktree to see where you are, then continue toward the acceptance criteria. Do not redo \
             work that is already there, and do not commit. Call codly_submit_report when done.",
            t.key, t.title
        ),
        (Role::Reviewer, Some(t)) => format!(
            "[Codly] Your review of task {} gave no output for {quiet}, so Codly interrupted it. {HANG} Continue the review, \
             then call codly_submit_review with your verdict, criteria and issues.",
            t.key
        ),
        _ => format!("[Codly] Your last turn gave no output for {quiet}, so Codly interrupted it. {HANG} Continue from where you stopped."),
    }
}

/// One live run, as a beat saw it (or `on_exit`, for a run its adapter ended).
#[derive(Debug, Clone)]
pub(super) struct Look {
    pub(super) agent_id: Id,
    pub(super) run_id: Id,
    pub(super) role: Role,
    provider: Provider,
    pub(super) task_id: Option<Id>,
    pub(super) ws: Option<Id>,
    turn_active: bool,
    /// In a turn, or sent a prompt that no turn has taken yet.
    busy: bool,
    /// A failed turn's continue prompt is owed (`Live::continue_owed`), or the gate waits for
    /// the run's background work (background.rs): Codly is about to act.
    retrying: bool,
    /// How long ago its last event came.
    quiet: Duration,
    watch: Watch,
}

impl Look {
    pub(super) fn of(agent_id: &str, l: &Live, now: Instant) -> Look {
        Look {
            agent_id: agent_id.to_string(),
            run_id: l.run_id.clone(),
            role: l.role,
            provider: l.provider,
            task_id: l.task_id.clone(),
            ws: l.workspace_id.clone(),
            turn_active: l.turn_active,
            busy: l.turn_active || l.queued > 0,
            retrying: l.continue_owed.is_some() || l.background.holds_gate(),
            quiet: now.duration_since(l.watch.last_event),
            watch: l.watch.clone(),
        }
    }

    /// The task this run works on as its worker.
    fn work(&self) -> Option<&str> {
        self.task_id.as_deref().filter(|_| self.role == Role::Worker)
    }
}

pub(super) async fn run(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(BEAT).await;
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        // A bug in one look must not end the watch over every agent.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.watchdog_beat())).is_err() {
            tracing::error!("the watchdog beat panicked; continuing");
        }
    }
}

impl Inner {
    /// Called by the ingest pipeline for every adapter event, before anything else: any event
    /// of a run is a life sign.
    pub(super) fn watchdog_seen(&self, ev: &AdapterEvent) {
        if let Some(l) = lock(&self.live).get_mut(&ev.agent_id).filter(|l| l.run_id == ev.run_id) {
            l.watch.seen(&ev.kind, Instant::now(), l.queued);
        }
    }

    /// One look at every live run: flag, interrupt and continue, stop.
    pub(super) fn watchdog_beat(self: &Arc<Self>) {
        let cfg = self.settings().watchdog;
        let now = Instant::now();
        let looks: Vec<Look> = lock(&self.live)
            .iter()
            // A run that starts or stops has limits of its own (the handshake, the shutdown ladder),
            // and so has one that its adapter is ending after an interrupt.
            .filter(|(_, l)| !l.stopping && l.starting.is_none() && !l.interrupt.gave_up())
            .map(|(id, l)| Look::of(id, l, now))
            .collect();
        for r in looks {
            match r.watch.owes_continue {
                // Its interrupted turn has ended: ask it to continue.
                Some(quiet) if !r.turn_active => self.send_continue(&r, quiet),
                _ if r.busy => self.watch_turn(&cfg, now, &r),
                _ => self.watch_idle(&cfg, &r),
            }
        }
    }

    fn watch_turn(self: &Arc<Self>, cfg: &WatchdogSettings, now: Instant, r: &Look) {
        if r.watch.idle_flagged {
            self.watch_unflag(r, Flag::Idle); // it is at work again
        }
        let limit = Duration::from_secs(u64::from(r.watch.limit_secs(r.provider, cfg)));
        // Waiting for you is not silence: the clock starts again when you answer.
        let waits = !limit.is_zero() && r.quiet >= limit.min(ASK_AFTER) && self.waits_for_you(r);
        if waits {
            self.watch_mark(r, |w| w.last_event = now);
        }
        let quiet = if waits { Duration::ZERO } else { r.quiet };
        match step(quiet, limit, r.watch.flagged, r.watch.nudged.map(|at| now.duration_since(at))) {
            Step::Wait => {}
            Step::Clear => self.watch_unflag(r, Flag::Stalled),
            Step::Flag => self.flag_stalled(r),
            Step::Nudge => self.watch_nudge(r, now),
            Step::Stop => {
                tokio::spawn(self.clone().stop_stalled(r.clone()));
            }
        }
    }

    fn watch_idle(&self, cfg: &WatchdogSettings, r: &Look) {
        if r.watch.flagged {
            self.watch_unflag(r, Flag::Stalled); // the silent turn ended
        }
        let limit = Duration::from_secs(u64::from(cfg.idle_secs));
        let idle = self.idles_on_its_task(r);
        if r.watch.idle_flagged && !idle {
            self.watch_unflag(r, Flag::Idle);
        } else if !r.watch.idle_flagged && idle && !limit.is_zero() && r.quiet >= limit {
            self.flag_idle(r);
        }
    }

    /// Does the run wait for your answer to an approval (a question card is one too)?
    fn waits_for_you(&self, r: &Look) -> bool {
        self.store.pending_approvals_for_run(&r.run_id).is_ok_and(|waiting| !waiting.is_empty())
    }

    /// Is this a task's worker that sits between turns while nothing of Codly's is about to
    /// move the task: no gate runs, no retry is owed, and you do not hold it?
    fn idles_on_its_task(&self, r: &Look) -> bool {
        let Some(task_id) = r.work() else { return false };
        if r.retrying || r.watch.owes_continue.is_some() {
            return false;
        }
        let in_gate = lock(&self.gating).contains_key(task_id);
        !in_gate && !self.control_holding(&r.agent_id) && self.task(task_id).is_ok_and(|t| working(&t, &r.agent_id))
    }

    /// Change what the watchdog remembers about the run it looked at. False: that run is gone
    /// or is stopping.
    fn watch_mark(&self, r: &Look, f: impl FnOnce(&mut Watch)) -> bool {
        match lock(&self.live).get_mut(&r.agent_id).filter(|l| l.run_id == r.run_id && !l.stopping) {
            Some(l) => {
                f(&mut l.watch);
                true
            }
            None => false,
        }
    }

    /// Say `reason` on the task the run works on, while the task is still its to work on.
    fn watch_flag_task(&self, r: &Look, reason: String) {
        let Some(task_id) = r.work() else { return };
        let _ = self.update_task(task_id, |t| {
            if working(t, &r.agent_id) {
                t.status_reason = Some(reason);
            }
            Ok(())
        });
    }

    /// Take a flag off: the watchdog's note, and its reason on the task if that is still there.
    fn watch_unflag(&self, r: &Look, flag: Flag) {
        self.watch_mark(r, |w| match flag {
            Flag::Stalled => w.flagged = false,
            Flag::Idle => w.idle_flagged = false,
        });
        let Some(task_id) = r.work() else { return };
        let _ = self.update_task(task_id, |t| {
            if working(t, &r.agent_id) && t.status_reason.as_deref().is_some_and(|reason| reason.starts_with(flag.prefix())) {
                t.status_reason = None;
            }
            Ok(())
        });
    }

    /// "Ada", and " on t1" when the run has a task.
    fn watch_who(&self, r: &Look) -> (String, String) {
        let name = self.agent(&r.agent_id).map_or_else(|_| "An agent".into(), |a| a.name);
        let on = r.task_id.as_deref().and_then(|id| self.task(id).ok()).map(|t| format!(" on {}", t.key)).unwrap_or_default();
        (name, on)
    }

    /// Every step is a decision on the agent's timeline. The app notifies you of the ones that
    /// need a look (`action`: flagged, idle, interrupted, stopped; not "continued").
    fn watchdog_decision(&self, r: &Look, action: &str, summary: String) {
        let detail = json!({ "agent_id": r.agent_id, "action": action, "quiet_secs": r.quiet.as_secs(), "tool_open": !r.watch.open_tools.is_empty() });
        let kind = EventKind::Decision { id: new_id(), decision_type: "watchdog".into(), method: "rule".into(), summary, detail };
        if let Err(e) = self.emit(r.ws.as_deref(), row(Some(&r.agent_id), r.task_id.as_deref(), Some(&r.run_id), kind)) {
            tracing::warn!("watchdog event not recorded: {e}");
        }
    }

    /// Where the agent talks: its task's request thread, the Lead's own, or its 1:1 chat.
    fn watch_thread(&self, agent: &Agent, task: Option<&Task>) -> String {
        match (task, agent.role) {
            (Some(t), _) => self.task_thread(t),
            (None, Role::Lead) => lead::thread_of(agent).to_string(),
            _ => format!("agent:{}", agent.id),
        }
    }

    fn flag_stalled(&self, r: &Look) {
        if !self.watch_mark(r, |w| w.flagged = true) {
            return;
        }
        let quiet = span(r.quiet);
        self.watch_flag_task(r, format!("{}no output for {quiet}", Flag::Stalled.prefix()));
        let (name, on) = self.watch_who(r);
        self.watchdog_decision(r, "flagged", format!("{name} gave no output for {quiet}{on}. Codly interrupts its turn if it stays silent."));
    }

    fn flag_idle(&self, r: &Look) {
        if !self.watch_mark(r, |w| w.idle_flagged = true) {
            return;
        }
        self.watch_flag_task(r, format!("{}the worker stopped without finishing; message it to continue, or stop it", Flag::Idle.prefix()));
        let (name, on) = self.watch_who(r);
        self.watchdog_decision(r, "idle", format!("{name} stopped without finishing{on}. Message it to continue, or stop it."));
    }

    /// Still silent at twice the limit: interrupt the turn. The continue prompt follows at the
    /// beat that sees the turn ended (`send_continue`), so no adapter folds it into the turn
    /// that is being interrupted. An adapter whose CLI ignores the interrupt ends the process:
    /// that exit comes back here (`stalled_run_gave_up`), as the watchdog asked.
    fn watch_nudge(&self, r: &Look, now: Instant) {
        let to_continue = r.role != Role::Lead;
        let sent = match lock(&self.live).get_mut(&r.agent_id).filter(|l| l.run_id == r.run_id && !l.stopping) {
            Some(l) => {
                l.watch.nudged = Some(now);
                l.watch.owes_continue = to_continue.then_some(r.quiet);
                l.send_interrupt(Asker::Watchdog { quiet: r.quiet })
            }
            None => return,
        };
        if !sent {
            return; // its process is gone
        }
        let quiet = span(r.quiet);
        let (name, on) = self.watch_who(r);
        if to_continue {
            self.watch_flag_task(r, format!("{}no output for {quiet}; Codly interrupted the turn and asks the worker to continue", Flag::Stalled.prefix()));
            self.watchdog_decision(r, "continued", format!("{name} gave no output for {quiet}{on}. Codly interrupted its turn and asks it to continue."));
            return;
        }
        // The Lead's turns answer you or a worker: Codly does not write one for it.
        self.watchdog_decision(r, "interrupted", format!("{name} gave no output for {quiet}. Codly interrupted its turn."));
        if let Ok(agent) = self.agent(&r.agent_id) {
            let text = format!("{name} gave no output for {quiet}, so Codly interrupted its turn. Send your message again.");
            let _ = self.chat(r.ws.as_deref(), &self.watch_thread(&agent, None), "codly", &text, None);
        }
    }

    fn send_continue(&self, r: &Look, quiet: Duration) {
        if !self.watch_mark(r, |w| w.owes_continue = None) {
            return;
        }
        let task = r.task_id.as_deref().and_then(|id| self.task(id).ok());
        // No longer its task to change (you moved it meanwhile): nothing to continue.
        if r.work().is_some() && !task.as_ref().is_some_and(|t| working(t, &r.agent_id)) {
            return;
        }
        if let Err(e) = self.send_prompt(&r.agent_id, continue_prompt(r.role, task.as_ref(), &span(quiet))) {
            tracing::debug!("the silent agent was not asked to continue: {e}");
        }
    }

    /// Silent again, or still, a whole limit after the interrupt: stop the run and put its
    /// work back in line. `stop_run` marks the run as stopping, so its exit starts nothing by
    /// itself (`on_exit` leaves a stopping run's task alone): what is next is written here,
    /// once the process is gone and the agent is free to start again.
    async fn stop_stalled(self: Arc<Self>, r: Look) {
        if !self.watch_mark(&r, |_| {}) {
            return; // it exited meanwhile, or something else stops it
        }
        self.stop_run_if(&r.agent_id, |l| l.run_id == r.run_id);
        let live = || lock(&self.live).get(&r.agent_id).is_some_and(|l| l.run_id == r.run_id);
        let until = Instant::now() + EXIT_WAIT;
        while live() && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if self.shutting_down.load(Ordering::SeqCst) {
            return; // quitting: the next start resumes the work
        }
        self.stalled_run_ended(r).await;
    }

    /// The CLI did not stop at the watchdog's interrupt of a turn that was silent for `quiet`,
    /// so its adapter ended the process (`interrupts`). What is next is the same as for a run
    /// the watchdog stops itself. The restart is its one more chance, in place of the
    /// continue prompt that an interrupted turn would get.
    pub(super) async fn stalled_run_gave_up(self: Arc<Self>, r: Look, quiet: Duration) {
        self.stalled_run_ended(Look { quiet, ..r }).await;
    }

    /// A run that was silent for `r.quiet` is gone, stopped by the watchdog or ended by its
    /// adapter: put its work back in line, and say so.
    async fn stalled_run_ended(self: Arc<Self>, r: Look) {
        let Ok(agent) = self.agent(&r.agent_id) else { return };
        let quiet = span(r.quiet);
        let task = r.task_id.as_deref().and_then(|id| self.task(id).ok());
        match (r.role, task) {
            (Role::Worker, Some(t)) => self.requeue_stalled(&r, &agent, &t, &quiet).await,
            (Role::Reviewer, Some(t)) => {
                let summary = format!("{} gave no output for {quiet} on its review of {} and did not recover. Codly stopped it.", agent.name, t.key);
                self.watchdog_decision(&r, "stopped", summary);
                // As for a reviewer that exited by itself: the review ends without a verdict.
                self.clone().review_done(t.id, r.run_id.clone()).await;
            }
            (_, task) => {
                let text = format!("{} gave no output for {quiet} and did not recover, so Codly stopped it. Your next message starts it again.", agent.name);
                self.watchdog_decision(&r, "stopped", text.clone());
                let _ = self.chat(r.ws.as_deref(), &self.watch_thread(&agent, task.as_ref()), "codly", &text, None);
            }
        }
        self.kick();
    }

    /// The task of a worker the watchdog stopped goes back to Ready with its worker, which
    /// resumes its session; from its `MAX_STARTS`th start on it is blocked for you instead.
    async fn requeue_stalled(self: &Arc<Self>, r: &Look, agent: &Agent, t: &Task, quiet: &str) {
        let give_up = t.attempt >= MAX_STARTS;
        let reason = if give_up {
            format!("blocked: the worker gave no output for {quiet} and restarts did not help ({} starts); check the CLI and its run log, then Retry", t.attempt)
        } else {
            format!("the worker gave no output for {quiet}; Codly stopped it and restarts it")
        };
        // Checked in the write: moved, stopped or cancelled by you meanwhile, it stays as it is.
        let mut moved = false;
        let _ = self.update_task(&t.id, |t| {
            moved = working(t, &agent.id);
            if moved {
                t.status = if give_up { TaskStatus::Blocked } else { TaskStatus::Ready };
                t.status_reason = Some(reason.clone());
            }
            Ok(())
        });
        if !moved {
            return;
        }
        let _ = self.update_agent(&agent.id, |a| {
            if a.task_id.as_deref() == Some(t.id.as_str()) {
                a.task_id = None;
            }
        });
        let next = if give_up { "blocked its task" } else { "restarts its task" };
        self.watchdog_decision(r, "stopped", format!("{} gave no output for {quiet} on {} and did not recover. Codly stopped it and {next}.", agent.name, t.key));
        if give_up && let Some(ws) = r.ws.as_deref() {
            self.lead_followup(ws, &self.task_thread(t), &format!("Task {} \"{}\" is {reason}.", t.key, t.title)).await;
        }
    }
}

impl Core {
    /// Look at every live run now, as the watchdog does every 15 s (the tests do not wait for it).
    pub fn watchdog_beat(&self) {
        self.inner.watchdog_beat();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(600);

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn a_silent_turn_is_flagged_then_interrupted_then_stopped() {
        // Output within the limit: nothing, and a flag from before comes off.
        assert_eq!(step(secs(599), LIMIT, false, None), Step::Wait);
        assert_eq!(step(secs(5), LIMIT, true, None), Step::Clear);
        // Silent for the limit: flagged, once.
        assert_eq!(step(secs(600), LIMIT, false, None), Step::Flag);
        assert_eq!(step(secs(900), LIMIT, true, None), Step::Wait);
        // Silent for twice the limit: interrupted and asked to continue.
        assert_eq!(step(secs(1200), LIMIT, true, None), Step::Nudge);
        // The interrupt gets a whole limit to show an effect, then the run is stopped.
        assert_eq!(step(secs(1215), LIMIT, true, Some(secs(15))), Step::Wait);
        assert_eq!(step(secs(1800), LIMIT, true, Some(secs(600))), Step::Stop);
        // It continued and went silent again: stopped at the limit, with no second interrupt.
        assert_eq!(step(secs(60), LIMIT, false, Some(secs(75))), Step::Wait);
        assert_eq!(step(secs(600), LIMIT, false, Some(secs(615))), Step::Stop);
    }

    #[test]
    fn a_limit_of_zero_turns_the_check_off() {
        assert_eq!(step(secs(86_400), Duration::ZERO, false, None), Step::Wait);
        assert_eq!(step(secs(86_400), Duration::ZERO, true, Some(secs(86_400))), Step::Clear);
    }

    #[test]
    fn every_event_of_a_run_is_a_life_sign_but_the_accounts_usage() {
        let start = Instant::now();
        let mut w = Watch { last_event: start, ..Watch::new() };
        let later = start + secs(30);
        w.seen(&EventKind::RateLimit { window: "5h".into(), used_fraction: Some(0.4), resets_at: None, status: None }, later, 0);
        assert_eq!(w.last_event, start, "every agent of the account gets this reading");
        w.seen(&EventKind::TextDelta { item_id: "m".into(), text: "x".into() }, later, 0);
        assert_eq!(w.last_event, later, "a live delta counts, although it is never stored");
    }

    /// A tool call starts.
    fn started(call_id: &str, name: &str) -> EventKind {
        EventKind::ToolStarted { call_id: call_id.into(), name: name.into(), activity: ActivityKind::Execute, target: Target::default(), input: json!({}), confidence: "observed".into() }
    }

    fn completed(call_id: &str) -> EventKind {
        EventKind::ToolCompleted { call_id: call_id.into(), ok: true, output: None, exit_code: None, duration_ms: None, tests_passed: None, tests_failed: None }
    }

    /// A run that started these tool calls (id, tool) at `start` and gave no event since.
    fn with_open(start: Instant, calls: &[(&str, &str)]) -> Watch {
        let mut w = Watch { last_event: start, ..Watch::new() };
        for (id, name) in calls {
            w.seen(&started(id, name), start, 0);
        }
        w
    }

    /// The silence the run may keep, with the default settings: 10 minutes, or 60.
    fn limit(w: &Watch, provider: Provider) -> Duration {
        secs(w.limit_secs(provider, &WatchdogSettings::default()).into())
    }

    const PLAIN: Duration = Duration::from_secs(600);
    const LONG: Duration = Duration::from_secs(3600);

    /// A build that runs for hours under Claude Code is never flagged: the CLI's heartbeat for
    /// the open call (every 30 s) starts the clock again each time, and the call stays open.
    /// So its limit is the plain one: when the heartbeats stop, the CLI hangs.
    #[test]
    fn a_long_command_that_says_it_still_runs_is_not_silent() {
        let start = Instant::now();
        let mut w = with_open(start, &[("c1", "Bash")]);
        assert_eq!(limit(&w, Provider::Claude), PLAIN);
        let mut now = start;
        while now < start + LONG * 3 {
            now += secs(30);
            assert_eq!(step(now.duration_since(w.last_event), limit(&w, Provider::Claude), w.flagged, None), Step::Wait);
            w.seen(&EventKind::ToolProgress { call_id: "c1".into() }, now, 0);
        }
        assert_eq!(w.open_tools.len(), 1, "a heartbeat ends no call");
        // The heartbeats stop (the CLI hangs): flagged 10 minutes later, not an hour.
        assert_eq!(step(PLAIN - secs(1), limit(&w, Provider::Claude), w.flagged, None), Step::Wait);
        assert_eq!(step((now + PLAIN).duration_since(w.last_event), limit(&w, Provider::Claude), w.flagged, None), Step::Flag);
    }

    /// The tools of Claude Code whose calls have a recorded heartbeat keep the plain limit: an
    /// MCP tool that hangs is flagged after 10 minutes, and one that still runs never is.
    /// A tool with no recorded heartbeat (WebFetch) keeps the long limit, also beside a call
    /// that has one, and so does every call of another CLI.
    #[test]
    fn a_hung_tool_call_of_a_claude_run_is_silent_after_the_plain_limit() {
        let start = Instant::now();
        // An MCP tool that never returns, and says nothing.
        let mut w = with_open(start, &[("c1", "mcp__linear__search")]);
        assert_eq!(limit(&w, Provider::Claude), PLAIN);
        assert_eq!(step(PLAIN - secs(1), limit(&w, Provider::Claude), false, None), Step::Wait);
        assert_eq!(step(PLAIN, limit(&w, Provider::Claude), false, None), Step::Flag);
        // The same call with its heartbeat every 30 s, for three hours: never flagged.
        let mut now = start;
        while now < start + LONG * 3 {
            now += secs(30);
            assert_eq!(step(now.duration_since(w.last_event), limit(&w, Provider::Claude), w.flagged, None), Step::Wait);
            w.seen(&EventKind::ToolProgress { call_id: "c1".into() }, now, 0);
        }
        for tool in ["Bash", "Agent", "Task", "mcp__codly__codly_submit_report"] {
            assert_eq!(limit(&with_open(start, &[("c1", tool)]), Provider::Claude), PLAIN, "{tool}");
        }

        // No recorded heartbeat: the long limit holds, alone and beside a command.
        let quiet: [&[(&str, &str)]; 3] = [&[("c1", "WebFetch")], &[("c1", "Bash"), ("c2", "WebSearch")], &[("c1", "Read")]];
        for calls in quiet {
            let w = with_open(start, calls);
            assert_eq!(limit(&w, Provider::Claude), LONG, "{calls:?}");
            assert_eq!(step(PLAIN, limit(&w, Provider::Claude), false, None), Step::Wait, "{calls:?}");
            assert_eq!(step(LONG, limit(&w, Provider::Claude), false, None), Step::Flag, "{calls:?}");
        }
        // When that call returns, the command's heartbeat is the sign again.
        let mut w = with_open(start, &[("c1", "Bash"), ("c2", "WebFetch")]);
        w.seen(&completed("c2"), start, 0);
        assert_eq!(limit(&w, Provider::Claude), PLAIN);
        w.seen(&completed("c1"), start, 0);
        assert!(w.open_tools.is_empty());

        // Another CLI is not known to say that a call still runs: the long limit, whatever
        // the tool is called. With no call open every CLI has the plain one.
        for provider in Provider::ALL.into_iter().filter(|p| *p != Provider::Claude) {
            for tool in ["commandExecution", "Bash", "mcp__linear__search"] {
                let w = with_open(start, &[("c1", tool)]);
                assert_eq!(limit(&w, provider), LONG, "{provider:?} {tool}");
                assert_eq!(step(PLAIN, limit(&w, provider), false, None), Step::Wait, "{provider:?} {tool}");
            }
            assert_eq!(limit(&with_open(start, &[]), provider), PLAIN, "{provider:?}");
        }
        assert_eq!(limit(&with_open(start, &[]), Provider::Claude), PLAIN);
    }

    /// A helper's own calls do not decide the limit. While the agent's `Agent` call waits for
    /// the helper, that call's heartbeat is the sign, whatever the helper runs (recorded: a
    /// Bash call inside a helper has no heartbeat of its own). A helper that works on in the
    /// background has no call of the agent's open: nothing is known to give a sign there.
    #[test]
    fn a_helpers_own_calls_do_not_decide_the_limit() {
        let start = Instant::now();
        let helpers = |call: &str| EventKind::SubAgentProgress { call_id: "a1".into(), name: None, step: None, tool_call_id: Some(call.into()), tool_uses: None, tokens: None, report: None };
        let mut w = with_open(start, &[("a1", "Agent"), ("h1", "WebFetch")]);
        assert_eq!(limit(&w, Provider::Claude), LONG, "not yet known as the helper's call");
        w.seen(&helpers("h1"), start, 0);
        assert_eq!(limit(&w, Provider::Claude), PLAIN, "the Agent call's heartbeat stands for the helper's call");
        // The `Agent` call returned and the helper works on: only its own call is open.
        w.seen(&completed("a1"), start, 0);
        assert_eq!(limit(&w, Provider::Claude), LONG);
        w.seen(&completed("h1"), start, 0);
        assert!(w.open_tools.is_empty());
        assert_eq!(limit(&w, Provider::Claude), PLAIN);
    }

    #[test]
    fn a_turn_that_completes_lets_the_next_silence_start_over() {
        let mut w = Watch::new();
        let now = Instant::now();
        w.nudged = Some(now);
        w.seen(&started("c1", "Bash"), now, 0);
        w.seen(&started("c2", "Read"), now, 0);
        let ended = |status| EventKind::TurnCompleted { status, result_text: None, duration_ms: None };
        // The interrupted turn itself ends: the run is still on its one chance.
        w.seen(&ended(TurnStatus::Interrupted), now, 0);
        assert!(w.nudged.is_some());
        assert!(w.open_tools.is_empty(), "no tool call outlives its turn");
        // A turn completes, but a prompt it was sent is still not taken: no new chance yet.
        w.seen(&ended(TurnStatus::Completed), now, 1);
        assert!(w.nudged.is_some());
        w.seen(&ended(TurnStatus::Completed), now, 0);
        assert!(w.nudged.is_none());
    }

    #[test]
    fn a_silence_reads_in_seconds_then_minutes() {
        assert_eq!(span(secs(45)), "45 s");
        assert_eq!(span(secs(119)), "119 s");
        assert_eq!(span(secs(600)), "10 min");
        assert_eq!(span(secs(3725)), "62 min");
    }

    /// A settings file written before the watchdog existed still loads, with the defaults.
    #[test]
    fn settings_saved_before_the_watchdog_load_with_its_defaults() {
        let mut saved = serde_json::to_value(Settings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("watchdog");
        let loaded: Settings = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded.watchdog, WatchdogSettings { quiet_secs: 600, quiet_tool_secs: 3600, idle_secs: 120 });
        // A part of it only: the rest keeps its default.
        let partial: WatchdogSettings = serde_json::from_value(json!({ "quiet_secs": 60 })).unwrap();
        assert_eq!(partial, WatchdogSettings { quiet_secs: 60, ..Default::default() });
    }
}
