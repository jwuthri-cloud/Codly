//! Native notifications: which events are worth one, what it says and what it opens.
//!
//! They come from Rust: the webview may be suspended while hidden, and an approval must never
//! wait silently. `Notifier::notice` turns an event into a notice. Notices of one kind that
//! come within a few seconds show as one notification (`Groups`). Settings → Notifications has
//! one switch per kind, and one for "only when Codly is not in front" (`shows`).
//!
//! The notification plugin does not report a click on the desktop. A click brings Codly to the
//! front, so when the window comes to the front while a notification is still on screen, the UI
//! is told what that notification names and opens it (`Shown`, the `open-notice` event, and
//! src/lib/state/openNotice.ts). That is a guess: the UI never leaves your office on it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::Core;
use codly_core::blockers::{BlockerSeverity, BlockerStatus};
use codly_core::model::{Approval, ApprovalSubject, EventKind, EventRow, PlanStatus, Task, TaskStatus};
use codly_core::notifications::NotificationSettings;
use codly_core::orchestrator::resume::Standing;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, sleep_until};

/// Notices of one kind that come within this time of the first show as one notification.
const GROUP_WINDOW: Duration = Duration::from_secs(3);
/// The lines one notification shows for the notices it stands for; the rest is counted.
const GROUP_LINES: usize = 3;
/// Coming to Codly within this time of a notification opens what it names: a banner stays on
/// screen for about 5 seconds, and a click on it needs a moment to bring the window forward.
/// Later there was no banner to click: you are back for your own reasons, and the screen stays
/// where you left it.
const OPEN_WITHIN: Duration = Duration::from_secs(8);
/// The event that tells the UI what to open (payload: `Target`).
const OPEN_EVENT: &str = "open-notice";

/// The kinds of notification: each has its switch in Settings → Notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An agent waits for your approval, or a plan waits for your Run.
    Approval,
    /// An agent asks you a question.
    Question,
    PlanDone,
    /// A plan failed, or cannot go on by itself.
    PlanFailed,
    /// A task is blocked, or failed.
    TaskBlocked,
    /// An agent gives no output or sits idle, or Codly held it back or stopped it.
    AgentStalled,
    Budget,
    /// A solo agent's branch has new work to merge.
    MergeReady,
}

impl Kind {
    /// The title of the one notification that stands for `n` notices of this kind.
    fn many(self, n: usize) -> String {
        match self {
            Kind::Approval => format!("{n} approvals wait for you"),
            Kind::Question => format!("{n} questions wait for you"),
            Kind::PlanDone => format!("{n} plans finished"),
            Kind::PlanFailed => format!("{n} plans stopped"),
            Kind::TaskBlocked => format!("{n} tasks stopped"),
            Kind::AgentStalled => format!("{n} agents need a look"),
            Kind::Budget => format!("{n} budgets reached"),
            Kind::MergeReady => format!("{n} agents have work ready to merge"),
        }
    }
}

/// What a notification names, for the UI to open (`NoticeTarget` in src/lib/state/openNotice.ts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    /// The office it is in (None: not in one office).
    pub workspace_id: Option<String>,
    #[serde(flatten)]
    pub what: What,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    /// An approval card, in the conversation that shows it.
    Approval { id: String, agent_id: String },
    /// The questions that wait for your answer.
    Questions,
    /// A plan, in the conversation of its request.
    Plan { id: String },
    Task { id: String },
    Agent { id: String },
    /// Settings → Safety, where the budgets are.
    Budgets,
}

/// One thing to tell you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// What the body says when several notices share one notification: one line that says who
    /// or what by itself.
    line: String,
    /// What a click opens (None: the event did not say whose it is).
    pub target: Option<Target>,
}

impl Notice {
    /// A notice whose body already says who or what by itself.
    fn new(kind: Kind, title: &str, body: String, target: Option<Target>) -> Notice {
        Notice { kind, title: title.to_string(), line: body.clone(), body, target }
    }
}

/// The names of an office and of an agent (`Core::names_for`).
pub type Names<'a> = &'a dyn Fn(Option<&str>, Option<&str>) -> (Option<String>, Option<String>);

/// An approval that has no answer yet.
struct Asked {
    approval: Approval,
    /// You were told of it.
    told: bool,
}

/// Event → notice. It remembers what it told you, so each thing is told once, however many
/// events show it, and again only after it ended.
#[derive(Default)]
pub struct Notifier {
    /// Approvals not answered yet. The broker allows most of them by itself right away: only
    /// one it leaves to you ("asking you: …") is worth a notification.
    asked: HashMap<String, Asked>,
    /// Each plan's status as last seen: only a change of it is told.
    plans: HashMap<String, PlanStatus>,
    /// Tasks that stopped (Codly blocked them, or they failed) and were told.
    stopped: HashMap<String, TaskStatus>,
}

impl Notifier {
    /// A notifier that starts with Codly. What stands since before the last quit (a task that
    /// is still blocked, a plan that still waits for you) was told then: it is told again
    /// only after it ended and came back.
    pub fn knowing(standing: &Standing) -> Notifier {
        Notifier {
            asked: HashMap::new(),
            plans: standing.plans.iter().map(|plan| (plan.id.clone(), plan.status)).collect(),
            stopped: standing.tasks.iter().filter(|task| stopped(task)).map(|task| (task.id.clone(), task.status)).collect(),
        }
    }

    /// The notice for this event, if it is worth one. Call it for every event: it must see a
    /// stop end to tell the next one.
    pub fn notice(&mut self, ev: &EventRow, names: Names) -> Option<Notice> {
        let ws = ev.workspace_id.as_deref();
        let at = |what: What| Target { workspace_id: ev.workspace_id.clone(), what };
        match &ev.kind {
            EventKind::ApprovalRequested { approval } => {
                self.asked.insert(approval.id.clone(), Asked { approval: approval.clone(), told: false });
                None
            }
            EventKind::ApprovalResolved { approval_id, .. } => {
                self.asked.remove(approval_id);
                None
            }
            EventKind::Decision { decision_type, summary, detail, .. } => match (decision_type.as_str(), detail["action"].as_str()) {
                ("approval_policy", _) if summary.starts_with("asking you") => {
                    let asked = self.asked.get_mut(detail["approval_id"].as_str()?)?;
                    if std::mem::replace(&mut asked.told, true) {
                        return None;
                    }
                    let approval = &asked.approval;
                    let (kind, what) = match &approval.subject {
                        ApprovalSubject::Command { command, .. } => (Kind::Approval, format!("run `{}`", truncate(command, 60))),
                        ApprovalSubject::FileChange { paths, .. } => (Kind::Approval, format!("change {}", files(paths.len()))),
                        ApprovalSubject::Question { .. } => (Kind::Question, "ask you a question".to_string()),
                        // A command in Claude Code's sandbox wants a host: said as that, not by the request's tool name.
                        ApprovalSubject::Tool { name, input } => match codly_core::adapters::claude::sandbox_host(name, input) {
                            Some("") => (Kind::Approval, "use the network".to_string()),
                            Some(host) => (Kind::Approval, format!("reach {}", truncate(host, 60))),
                            None => (Kind::Approval, format!("use {name}")),
                        },
                        ApprovalSubject::Permissions { .. } => (Kind::Approval, "get more permissions".to_string()),
                    };
                    // Who, and in which office: it may not be the one on screen.
                    let (office, agent) = names(ws, Some(&approval.agent_id));
                    let agent = agent.unwrap_or_else(|| "An agent".to_string());
                    Some(Notice {
                        kind,
                        title: in_office(office, format!("{agent} needs you")),
                        body: format!("It wants to {what}."),
                        line: format!("{agent} wants to {what}"),
                        target: Some(at(What::Approval { id: approval.id.clone(), agent_id: approval.agent_id.clone() })),
                    })
                }
                // The circuit breaker held an agent back or stopped it. Or the watchdog saw an
                // agent that gives no output or sits idle on its task, or it ended a silent turn
                // or run. (Asking an agent to continue needs no look from you.)
                ("breaker", Some("constrained" | "stopped")) | ("watchdog", Some("flagged" | "idle" | "interrupted" | "stopped")) => Some(Notice::new(
                    Kind::AgentStalled,
                    "An agent needs a look",
                    truncate(summary, 160),
                    ev.agent_id.clone().map(|id| at(What::Agent { id })),
                )),
                // A budget is used up: no new task starts until you raise it or reset the count.
                ("breaker", Some("budget")) => Some(Notice::new(
                    Kind::Budget,
                    "Budget reached",
                    truncate(summary.strip_prefix("Budget reached: ").unwrap_or(summary), 160),
                    Some(Target { workspace_id: None, what: What::Budgets }),
                )),
                // A plan that cannot go on by itself (`stuck::tell_stuck`): its conversation
                // says the ways out. The summary reads `plan "…" cannot go on by itself; told
                // you the ways out`.
                ("schedule", _) if detail["stopped"].is_array() && detail["waiting"].is_array() => {
                    let said = summary.strip_prefix("plan ").unwrap_or(summary);
                    let said = said.split_once("; ").map_or(said, |(what, _)| what);
                    Some(Notice::new(
                        Kind::PlanFailed,
                        "Plan stuck",
                        truncate(said, 160),
                        detail["plan_id"].as_str().map(|id| at(What::Plan { id: id.to_string() })),
                    ))
                }
                _ => None,
            },
            // A new blocking question, or one that has waited for days.
            EventKind::BlockerChanged { blocker }
                if blocker.status == BlockerStatus::Open && blocker.severity == BlockerSeverity::Blocking && blocker.updated_at == blocker.reminded_at.unwrap_or(blocker.created_at) =>
            {
                let title = if blocker.reminded_at.is_some() { "Still waiting for your answer" } else { "An agent needs a decision" };
                let workspace_id = blocker.workspace_id.clone().or_else(|| ev.workspace_id.clone());
                Some(Notice::new(Kind::Question, title, truncate(&blocker.title, 120), Some(Target { workspace_id, what: What::Questions })))
            }
            EventKind::PlanChanged { plan } => {
                if self.plans.insert(plan.id.clone(), plan.status) == Some(plan.status) {
                    return None;
                }
                let target = Some(Target { workspace_id: Some(plan.workspace_id.clone()), what: What::Plan { id: plan.id.clone() } });
                let said = truncate(&plan.summary, 120);
                match plan.status {
                    PlanStatus::AwaitingApproval => Some(Notice::new(Kind::Approval, "A plan is ready for you", said, target)),
                    // Not a plan you merged or kept before this app start: that one is old news.
                    PlanStatus::Done if plan.merged_sha.is_none() && !plan.kept => {
                        Some(Notice::new(Kind::PlanDone, "Plan finished", format!("{said} — ready to merge"), target))
                    }
                    PlanStatus::Failed => Some(Notice::new(Kind::PlanFailed, "Plan failed", said, target)),
                    _ => None,
                }
            }
            EventKind::TaskChanged { task } => {
                let reason = task.status_reason.as_deref().unwrap_or_default();
                if !stopped(task) {
                    self.stopped.remove(&task.id);
                    return None;
                }
                if self.stopped.insert(task.id.clone(), task.status) == Some(task.status) {
                    return None;
                }
                let target = Some(at(What::Task { id: task.id.clone() }));
                Some(if task.status == TaskStatus::Failed {
                    Notice::new(Kind::TaskBlocked, "Task failed", format!("{} — {}", task.key, truncate(&task.title, 80)), target)
                } else {
                    let why = truncate(reason.strip_prefix("blocked: ").unwrap_or(reason), 100);
                    Notice::new(Kind::TaskBlocked, "Task blocked", format!("{} — {}: {why}", task.key, truncate(&task.title, 60)), target)
                })
            }
            // Codly commits a solo agent's work when its turn ends: its branch has new work to
            // merge. A task's commits merge with its plan, which has a notice of its own.
            EventKind::Committed { files: changed, message, .. } if ev.task_id.is_none() && solo_turn(message) => {
                let (office, agent) = names(ws, ev.agent_id.as_deref());
                let agent = agent.unwrap_or_else(|| "An agent".to_string());
                Some(Notice {
                    kind: Kind::MergeReady,
                    title: in_office(office, format!("{agent}’s work is ready to merge")),
                    body: format!("Its last turn changed {}.", files(changed.len())),
                    line: format!("{agent} changed {}", files(changed.len())),
                    target: ev.agent_id.clone().map(|id| at(What::Agent { id })),
                })
            }
            _ => None,
        }
    }

    /// False when what the notice asks of you was settled while it waited to be shown: an
    /// approval that got its answer, a plan that started (an Auto request runs its plan at once).
    pub fn stands(&self, notice: &Notice) -> bool {
        match (notice.kind, notice.target.as_ref().map(|t| &t.what)) {
            (_, Some(What::Approval { id, .. })) => self.asked.contains_key(id),
            (Kind::Approval, Some(What::Plan { id })) => self.plans.get(id) == Some(&PlanStatus::AwaitingApproval),
            _ => true,
        }
    }
}

/// Did Codly stop this task (blocked), or did it fail? Not a task you stopped yourself, and
/// not one that asks you a question (its card has a notification of its own).
fn stopped(task: &Task) -> bool {
    let reason = task.status_reason.as_deref().unwrap_or_default();
    let yours = ["stopped by", "review stopped by", "waiting for you:"].iter().any(|p| reason.starts_with(p));
    task.status == TaskStatus::Failed || (task.status == TaskStatus::Blocked && !yours)
}

/// Is this the commit of a solo agent's turn (`ingest::solo_snapshot`)? The commits Codly makes
/// of its leftovers when you merge its branch or open a pull request are not: you are there.
fn solo_turn(message: &str) -> bool {
    message.starts_with("codly(solo/") && !message.ends_with(": before merge") && !message.ends_with(": before pull request")
}

fn in_office(office: Option<String>, title: String) -> String {
    office.map_or_else(|| title.clone(), |o| format!("{o}: {title}"))
}

/// "1 file", "3 files".
fn files(n: usize) -> String {
    if n == 1 { "1 file".to_string() } else { format!("{n} files") }
}

fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// Whether a notice of this kind is shown: its switch is on, and Codly is not in front when
/// notifications are for the background only.
pub fn shows(settings: &NotificationSettings, kind: Kind, in_front: bool) -> bool {
    let on = match kind {
        Kind::Approval => settings.approval,
        Kind::Question => settings.question,
        Kind::PlanDone => settings.plan_done,
        Kind::PlanFailed => settings.plan_failed,
        Kind::TaskBlocked => settings.task_blocked,
        Kind::AgentStalled => settings.agent_stalled,
        Kind::Budget => settings.budget,
        Kind::MergeReady => settings.merge_ready,
    };
    on && !(in_front && settings.only_in_background)
}

/// Notices that wait a moment for others of their kind, so five tasks that block together
/// are one notification and not five.
#[derive(Default)]
pub struct Groups {
    open: Vec<Group>,
}

struct Group {
    kind: Kind,
    /// When it is shown: `GROUP_WINDOW` after its first notice.
    due: Instant,
    notices: Vec<Notice>,
}

impl Groups {
    pub fn add(&mut self, notice: Notice, now: Instant) {
        match self.open.iter_mut().find(|g| g.kind == notice.kind) {
            Some(group) => group.notices.push(notice),
            None => self.open.push(Group { kind: notice.kind, due: now + GROUP_WINDOW, notices: vec![notice] }),
        }
    }

    /// When the next group is due (None: nothing waits).
    pub fn next_due(&self) -> Option<Instant> {
        self.open.iter().map(|g| g.due).min()
    }

    /// The groups that are due, each with its notices in the order they came.
    pub fn take_due(&mut self, now: Instant) -> Vec<Vec<Notice>> {
        let (due, open): (Vec<Group>, Vec<Group>) = std::mem::take(&mut self.open).into_iter().partition(|g| g.due <= now);
        self.open = open;
        due.into_iter().map(|g| g.notices).collect()
    }
}

/// One notification for the notices of one group: the notice itself, or a count with a line
/// for each of the first few. A click opens the first.
pub fn merged(mut notices: Vec<Notice>) -> Option<Notice> {
    if notices.len() < 2 {
        return notices.pop();
    }
    let mut body = notices.iter().take(GROUP_LINES).map(|n| n.line.as_str()).collect::<Vec<_>>().join("\n");
    if notices.len() > GROUP_LINES {
        body.push_str(&format!("\nand {} more", notices.len() - GROUP_LINES));
    }
    let first = notices.swap_remove(0);
    Some(Notice { kind: first.kind, title: first.kind.many(notices.len() + 1), line: body.clone(), body, target: first.target })
}

/// What the last notification shown behind Codly's window names, and when it was shown.
#[derive(Default)]
pub struct Shown(Option<(Target, Instant)>);

impl Shown {
    pub fn set(&mut self, target: Target, now: Instant) {
        self.0 = Some((target, now));
    }

    /// What to open now that Codly came to the front: what the last notification names, if it
    /// may still be on screen. Either way it is forgotten.
    pub fn take(&mut self, now: Instant) -> Option<Target> {
        let (target, at) = self.0.take()?;
        (now.duration_since(at) <= OPEN_WITHIN).then_some(target)
    }
}

/// Runs for the app's lifetime: shows the notifications.
pub async fn run<R: Runtime>(app: AppHandle<R>, core: Arc<Core>) {
    let mut rx = core.subscribe();
    // Listening first, then the read: every change after the read is seen here.
    let mut notifier = match core.standing() {
        Ok(standing) => Notifier::knowing(&standing),
        Err(e) => {
            tracing::warn!("what was told before this start cannot be read, so it may be told again: {e}");
            Notifier::default()
        }
    };
    let mut groups = Groups::default();
    let shown = Arc::new(Mutex::new(Shown::default()));
    open_on_return(&app, shown.clone());
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let ev = match ev {
                    Ok(ev) => ev,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let names = |ws: Option<&str>, agent: Option<&str>| core.names_for(ws, agent);
                if let Some(notice) = notifier.notice(&ev, &names) {
                    groups.add(notice, Instant::now());
                }
            }
            _ = due(groups.next_due()) => {
                let settings = core.settings().notifications;
                let in_front = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
                for notices in groups.take_due(Instant::now()) {
                    let Some(notice) = merged(notices.into_iter().filter(|n| notifier.stands(n)).collect()) else { continue };
                    if !shows(&settings, notice.kind, in_front) {
                        continue;
                    }
                    if let Err(e) = app.notification().builder().title(&notice.title).body(&notice.body).show() {
                        tracing::warn!("notification not shown: {e}");
                    }
                    if let Some(target) = notice.target.filter(|_| !in_front) {
                        shown.lock().unwrap().set(target, Instant::now());
                    }
                }
            }
        }
    }
}

/// Sleeps until the next group is due; for ever when nothing waits.
async fn due(at: Option<Instant>) {
    match at {
        Some(at) => sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// When the window comes to the front, tell the UI what the last notification names.
fn open_on_return<R: Runtime>(app: &AppHandle<R>, shown: Arc<Mutex<Shown>>) {
    let Some(window) = app.get_webview_window("main") else { return };
    let app = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(true) = event {
            returned(&app, &shown, Instant::now());
        }
    });
}

fn returned<R: Runtime>(app: &AppHandle<R>, shown: &Mutex<Shown>, now: Instant) {
    let target = shown.lock().unwrap().take(now);
    if let Some(target) = target
        && let Err(e) = app.emit(OPEN_EVENT, target)
    {
        tracing::warn!("the UI was not told what the notification names: {e}");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// An event of office "ws", as the core sends it (`kind` in its JSON form).
    fn ev(agent: Option<&str>, kind: Value) -> EventRow {
        serde_json::from_value(json!({ "id": 1, "ts": 0, "agent_id": agent, "task_id": null, "run_id": null, "workspace_id": "ws", "kind": kind })).unwrap()
    }

    /// The office is "Shop"; agent "a1" is Ada, nobody else is known.
    fn names(_ws: Option<&str>, agent: Option<&str>) -> (Option<String>, Option<String>) {
        (Some("Shop".to_string()), agent.filter(|a| *a == "a1").map(|_| "Ada".to_string()))
    }

    fn tell(n: &mut Notifier, ev: &EventRow) -> Option<Notice> {
        n.notice(ev, &names)
    }

    fn in_ws(what: What) -> Option<Target> {
        Some(Target { workspace_id: Some("ws".to_string()), what })
    }

    fn approval(id: &str, subject: Value) -> EventRow {
        ev(
            Some("a1"),
            json!({ "type": "approval_requested", "approval": {
                "id": id, "agent_id": "a1", "run_id": "r1", "task_id": null, "call_id": null, "subject": subject, "reason": null,
                "choices": [], "risks": [], "status": "pending", "decided_by": null, "created_at": 0,
            } }),
        )
    }

    fn decision(agent: Option<&str>, decision_type: &str, summary: &str, detail: Value) -> EventRow {
        ev(agent, json!({ "type": "decision", "id": "d1", "decision_type": decision_type, "method": "rule", "summary": summary, "detail": detail }))
    }

    fn asking(approval_id: &str) -> EventRow {
        decision(Some("a1"), "approval_policy", "asking you: deletes files", json!({ "approval_id": approval_id }))
    }

    fn answered(approval_id: &str) -> EventRow {
        ev(Some("a1"), json!({ "type": "approval_resolved", "approval_id": approval_id, "status": "approved", "by": "user" }))
    }

    fn task(status: &str, reason: &str) -> EventRow {
        ev(
            None,
            json!({ "type": "task_changed", "task": {
                "id": "t", "plan_id": null, "key": "t1", "title": "Add sign-in", "goal": "", "acceptance": [], "write_scope": [],
                "verify": [], "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
                "attempt": 1, "status_reason": reason, "summary": null, "created_at": 0, "updated_at": 0,
            } }),
        )
    }

    fn plan(status: &str, patch: Value) -> EventRow {
        let mut plan = json!({
            "id": "p", "workspace_id": "ws", "request": "", "summary": "Sign-in page", "assumptions": [], "open_questions": [],
            "status": status, "base_commit": null, "integration_branch": null, "integration_worktree": null, "created_at": 0, "updated_at": 0,
        });
        plan.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        ev(None, json!({ "type": "plan_changed", "plan": plan }))
    }

    fn blocker(patch: Value) -> EventRow {
        let mut blocker = json!({
            "id": "b1", "workspace_id": "ws", "agent_id": "a1", "task_id": null, "thread": "lead", "title": "Which sign-in provider?", "body": "",
            "severity": "blocking", "status": "open", "answer": null, "created_at": 10, "updated_at": 10, "answered_at": null, "reminded_at": null,
        });
        blocker.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        ev(Some("a1"), json!({ "type": "blocker_changed", "blocker": blocker }))
    }

    fn commit(message: &str) -> EventRow {
        ev(Some("a1"), json!({ "type": "committed", "sha": "abc", "files": ["src/a.rs", "src/b.rs"], "message": message }))
    }

    /// A notice of `kind` that says `line`.
    fn said(kind: Kind, line: &str) -> Notice {
        Notice::new(kind, "One", line.to_string(), in_ws(What::Task { id: line.to_string() }))
    }

    /// What the core reads for a notifier that starts: the rows of these events.
    fn standing(events: &[EventRow]) -> Standing {
        let mut standing = Standing { tasks: vec![], plans: vec![] };
        for ev in events {
            match &ev.kind {
                EventKind::TaskChanged { task } => standing.tasks.push(task.clone()),
                EventKind::PlanChanged { plan } => standing.plans.push(plan.clone()),
                other => panic!("not a task or a plan: {other:?}"),
            }
        }
        standing
    }

    #[test]
    fn what_stood_before_a_restart_is_not_told_again() {
        let before = [task("blocked", "blocked: the gate failed"), plan("done", json!({}))];
        // A notifier that knows nothing tells both at their next change: old news after a restart.
        let mut new = Notifier::default();
        assert!(before.iter().all(|ev| tell(&mut new, ev).is_some()));

        let mut n = Notifier::knowing(&standing(&before));
        assert_eq!(tell(&mut n, &task("blocked", "blocked: the gate failed")), None);
        assert_eq!(tell(&mut n, &plan("done", json!({ "summary": "Sign-in page, with tests" }))), None);
        // The task went on and stopped again: that is news.
        assert_eq!(tell(&mut n, &task("running", "")), None);
        assert_eq!(tell(&mut n, &task("blocked", "blocked: the gate failed")).unwrap().title, "Task blocked");
        // So is another end of the plan.
        assert_eq!(tell(&mut n, &plan("failed", json!({}))).unwrap().title, "Plan failed");
    }

    /// A task you stopped yourself is no stop Codly tells: it is not remembered as told
    /// either, so Codly blocking it later is news.
    #[test]
    fn a_task_you_stopped_before_a_restart_is_told_when_codly_blocks_it() {
        let mut n = Notifier::knowing(&standing(&[task("blocked", "stopped by you")]));
        assert_eq!(tell(&mut n, &task("blocked", "stopped by you")), None);
        assert!(tell(&mut n, &task("blocked", "blocked: the gate failed")).is_some());
    }

    #[test]
    fn an_approval_codly_leaves_to_you_is_told_once_with_who_and_where() {
        let mut n = Notifier::default();
        assert_eq!(tell(&mut n, &approval("ap1", json!({ "kind": "command", "command": "rm -rf dist", "cwd": null }))), None, "the broker may still allow it");
        let notice = tell(&mut n, &asking("ap1")).expect("left to you: worth a notification");
        assert_eq!(notice.kind, Kind::Approval);
        assert_eq!((notice.title.as_str(), notice.body.as_str()), ("Shop: Ada needs you", "It wants to run `rm -rf dist`."));
        assert_eq!(notice.line, "Ada wants to run `rm -rf dist`");
        assert_eq!(notice.target, in_ws(What::Approval { id: "ap1".to_string(), agent_id: "a1".to_string() }));
        assert_eq!(tell(&mut n, &asking("ap1")), None, "told once");
        assert_eq!(tell(&mut n, &asking("ap9")), None, "an approval nobody asked for");
    }

    #[test]
    fn an_approval_the_rules_settle_says_nothing() {
        let mut n = Notifier::default();
        tell(&mut n, &approval("ap1", json!({ "kind": "command", "command": "ls", "cwd": null })));
        let allowed = decision(Some("a1"), "approval_policy", "auto-allowed read-only command", json!({ "approval_id": "ap1" }));
        assert_eq!(tell(&mut n, &allowed), None);
    }

    #[test]
    fn each_approval_subject_says_what_the_agent_wants() {
        let cases = [
            (json!({ "kind": "file_change", "paths": ["a", "b"], "diff": null }), Kind::Approval, "It wants to change 2 files."),
            (json!({ "kind": "file_change", "paths": ["a"], "diff": null }), Kind::Approval, "It wants to change 1 file."),
            (json!({ "kind": "tool", "name": "WebFetch", "input": {} }), Kind::Approval, "It wants to use WebFetch."),
            // The request of Claude Code's sandbox for a host: the host, not the tool's name.
            (json!({ "kind": "tool", "name": "SandboxNetworkAccess", "input": { "host": "example.com" } }), Kind::Approval, "It wants to reach example.com."),
            (json!({ "kind": "tool", "name": "SandboxNetworkAccess", "input": {} }), Kind::Approval, "It wants to use the network."),
            (json!({ "kind": "permissions", "detail": {} }), Kind::Approval, "It wants to get more permissions."),
            (json!({ "kind": "question", "questions": [] }), Kind::Question, "It wants to ask you a question."),
        ];
        for (i, (subject, kind, body)) in cases.into_iter().enumerate() {
            let mut n = Notifier::default();
            let id = format!("ap{i}");
            tell(&mut n, &approval(&id, subject));
            let notice = tell(&mut n, &asking(&id)).unwrap();
            assert_eq!((notice.kind, notice.body.as_str()), (kind, body));
        }
    }

    #[test]
    fn an_approval_answered_while_its_notice_waited_is_not_shown() {
        let mut n = Notifier::default();
        tell(&mut n, &approval("ap1", json!({ "kind": "command", "command": "rm -rf dist", "cwd": null })));
        let notice = tell(&mut n, &asking("ap1")).unwrap();
        assert!(n.stands(&notice));
        tell(&mut n, &answered("ap1"));
        assert!(!n.stands(&notice));
    }

    #[test]
    fn a_blocking_question_is_told_when_new_and_when_it_waited_for_days() {
        let mut n = Notifier::default();
        let notice = tell(&mut n, &blocker(json!({}))).expect("a new blocking question");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::Question, "An agent needs a decision", "Which sign-in provider?"));
        assert_eq!(notice.target, in_ws(What::Questions));
        let reminder = tell(&mut n, &blocker(json!({ "reminded_at": 99, "updated_at": 99 }))).expect("a reminder");
        assert_eq!(reminder.title, "Still waiting for your answer");
        assert_eq!(tell(&mut n, &blocker(json!({ "updated_at": 11 }))), None, "an edit of an open question");
        assert_eq!(tell(&mut n, &blocker(json!({ "severity": "heads_up" }))), None, "no action needed");
        assert_eq!(tell(&mut n, &blocker(json!({ "status": "answered" }))), None);
    }

    #[test]
    fn a_plan_that_waits_for_you_is_told_once_and_not_when_it_started_meanwhile() {
        let mut n = Notifier::default();
        let notice = tell(&mut n, &plan("awaiting_approval", json!({}))).expect("a plan to review");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::Approval, "A plan is ready for you", "Sign-in page"));
        assert_eq!(notice.target, in_ws(What::Plan { id: "p".to_string() }));
        assert_eq!(tell(&mut n, &plan("awaiting_approval", json!({ "summary": "Sign-in page, with tests" }))), None, "an edit of the same plan");
        assert!(n.stands(&notice));
        // An Auto request runs its plan at once: nothing waits for you.
        assert_eq!(tell(&mut n, &plan("running", json!({}))), None);
        assert!(!n.stands(&notice));
    }

    #[test]
    fn a_finished_plan_is_told_once_and_not_again_when_you_merge_it() {
        let mut n = Notifier::default();
        assert_eq!(tell(&mut n, &plan("running", json!({}))), None);
        let notice = tell(&mut n, &plan("done", json!({}))).expect("a finished plan");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::PlanDone, "Plan finished", "Sign-in page — ready to merge"));
        assert!(n.stands(&notice));
        assert_eq!(tell(&mut n, &plan("done", json!({ "merged_sha": "abc" }))), None);
        // The first event this app start sees of a plan you merged or kept before: old news.
        assert_eq!(tell(&mut Notifier::default(), &plan("done", json!({ "merged_sha": "abc" }))), None);
        assert_eq!(tell(&mut Notifier::default(), &plan("done", json!({ "kept": true }))), None);
    }

    #[test]
    fn a_failed_plan_is_told_once() {
        let mut n = Notifier::default();
        assert_eq!(tell(&mut n, &plan("running", json!({}))), None);
        let notice = tell(&mut n, &plan("failed", json!({}))).expect("a failed plan");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::PlanFailed, "Plan failed", "Sign-in page"));
        assert_eq!(tell(&mut n, &plan("failed", json!({}))), None);
    }

    #[test]
    fn a_plan_that_cannot_go_on_is_told_with_the_plan_to_open() {
        let mut n = Notifier::default();
        let stuck = decision(
            None,
            "schedule",
            "plan \"Sign-in page\" cannot go on by itself; told you the ways out",
            json!({ "plan_id": "p", "merged": 1, "stopped": ["t1"], "waiting": ["t2"] }),
        );
        let notice = tell(&mut n, &stuck).expect("a stuck plan");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::PlanFailed, "Plan stuck", "\"Sign-in page\" cannot go on by itself"));
        assert_eq!(notice.target, in_ws(What::Plan { id: "p".to_string() }));
        // Other scheduling decisions are not for you.
        assert_eq!(tell(&mut n, &decision(None, "schedule", "started t2 on Ada", json!({ "plan_id": "p" }))), None);
    }

    #[test]
    fn a_task_codly_blocked_is_told_once_and_again_after_a_retry() {
        let mut n = Notifier::default();
        let blocked = task("blocked", "blocked: could not start the worker: Claude Code is not signed in");
        let notice = tell(&mut n, &blocked).expect("a blocked task is worth a notification");
        assert_eq!(notice.kind, Kind::TaskBlocked);
        assert_eq!((notice.title.as_str(), notice.body.as_str()), ("Task blocked", "t1 — Add sign-in: could not start the worker: Claude Code is not signed in"));
        assert_eq!(notice.target, in_ws(What::Task { id: "t".to_string() }));
        assert_eq!(tell(&mut n, &blocked), None, "another change of the same blocked task says nothing");
        assert_eq!(tell(&mut n, &task("ready", "")), None);
        assert!(tell(&mut n, &blocked).is_some(), "blocked again after your Retry: told again");
    }

    #[test]
    fn a_task_you_stopped_or_that_asks_you_has_no_blocked_notice() {
        let mut n = Notifier::default();
        for reason in ["stopped by you; Retry to resume", "review stopped by you; Retry to re-check", "waiting for you: Which provider?"] {
            assert_eq!(tell(&mut n, &task("blocked", reason)), None, "{reason}");
        }
    }

    #[test]
    fn a_failed_task_is_told_once() {
        let mut n = Notifier::default();
        assert_eq!(tell(&mut n, &task("running", "")), None);
        let notice = tell(&mut n, &task("failed", "failed: checks")).expect("a failed task");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::TaskBlocked, "Task failed", "t1 — Add sign-in"));
        assert_eq!(tell(&mut n, &task("failed", "failed: checks")), None, "bookkeeping on a task that already failed");
    }

    #[test]
    fn the_watchdog_and_the_circuit_breaker_tell_what_needs_a_look() {
        let mut n = Notifier::default();
        for action in ["flagged", "idle", "interrupted", "stopped"] {
            let notice = tell(&mut n, &decision(Some("a1"), "watchdog", "Ada gave no output for 10 min.", json!({ "agent_id": "a1", "action": action }))).expect(action);
            assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::AgentStalled, "An agent needs a look", "Ada gave no output for 10 min."));
            assert_eq!(notice.target, in_ws(What::Agent { id: "a1".to_string() }));
        }
        assert_eq!(tell(&mut n, &decision(Some("a1"), "watchdog", "Codly asks Ada to continue.", json!({ "action": "continued" }))), None);
        for action in ["constrained", "stopped"] {
            let notice = tell(&mut n, &decision(Some("a1"), "breaker", "Circuit breaker constrained Ada: 3 failed checks", json!({ "action": action }))).expect(action);
            assert_eq!(notice.kind, Kind::AgentStalled);
        }
        assert_eq!(tell(&mut n, &decision(Some("a1"), "breaker", "Circuit breaker: Ada is healthy again", json!({ "action": "resumed" }))), None);
    }

    #[test]
    fn a_budget_that_is_used_up_is_told_and_opens_the_budgets() {
        let mut n = Notifier::default();
        let notice = tell(&mut n, &decision(None, "breaker", "Budget reached: 5.00 USD today. No new task starts.", json!({ "action": "budget" }))).expect("a budget stop");
        assert_eq!((notice.kind, notice.title.as_str(), notice.body.as_str()), (Kind::Budget, "Budget reached", "5.00 USD today. No new task starts."));
        assert_eq!(notice.target, Some(Target { workspace_id: None, what: What::Budgets }));
        assert_eq!(tell(&mut n, &decision(None, "breaker", "The budget has room again: waiting tasks start.", json!({ "action": "budget_cleared" }))), None);
    }

    #[test]
    fn a_solo_agents_turn_that_left_changes_is_ready_to_merge() {
        let mut n = Notifier::default();
        let notice = tell(&mut n, &commit("codly(solo/Ada): add sign-in")).expect("new work on its branch");
        assert_eq!(notice.kind, Kind::MergeReady);
        assert_eq!((notice.title.as_str(), notice.body.as_str()), ("Shop: Ada’s work is ready to merge", "Its last turn changed 2 files."));
        assert_eq!(notice.line, "Ada changed 2 files");
        assert_eq!(notice.target, in_ws(What::Agent { id: "a1".to_string() }));
        // The commits Codly makes when you merge the branch or open a pull request: you are there.
        assert_eq!(tell(&mut n, &commit("codly(solo/Ada): before merge")), None);
        assert_eq!(tell(&mut n, &commit("codly(solo/Ada): before pull request")), None);
        // A task's commit merges with its plan.
        let mut of_task = commit("codly(t1): add sign-in");
        assert_eq!(tell(&mut n, &of_task), None);
        of_task = commit("codly(solo/Ada): add sign-in");
        of_task.task_id = Some("t".to_string());
        assert_eq!(tell(&mut n, &of_task), None);
    }

    #[test]
    fn events_that_need_nothing_from_you_say_nothing() {
        let mut n = Notifier::default();
        assert_eq!(tell(&mut n, &ev(Some("a1"), json!({ "type": "turn_started", "input": "hello" }))), None);
        assert_eq!(tell(&mut n, &ev(Some("a1"), json!({ "type": "error", "message": "boom", "fatal": true }))), None);
    }

    const KINDS: [Kind; 8] =
        [Kind::Approval, Kind::Question, Kind::PlanDone, Kind::PlanFailed, Kind::TaskBlocked, Kind::AgentStalled, Kind::Budget, Kind::MergeReady];

    /// The settings with the switch of `kind` set to `on`.
    fn with(mut s: NotificationSettings, kind: Kind, on: bool) -> NotificationSettings {
        let switch = match kind {
            Kind::Approval => &mut s.approval,
            Kind::Question => &mut s.question,
            Kind::PlanDone => &mut s.plan_done,
            Kind::PlanFailed => &mut s.plan_failed,
            Kind::TaskBlocked => &mut s.task_blocked,
            Kind::AgentStalled => &mut s.agent_stalled,
            Kind::Budget => &mut s.budget,
            Kind::MergeReady => &mut s.merge_ready,
        };
        *switch = on;
        s
    }

    #[test]
    fn each_kind_has_its_own_switch() {
        let all_on = NotificationSettings::default();
        let all_off = KINDS.iter().fold(all_on.clone(), |s, k| with(s, *k, false));
        for kind in KINDS {
            assert!(shows(&all_on, kind, false), "{kind:?} is on by default");
            assert!(!shows(&all_off, kind, false), "{kind:?} off");
            let only = with(all_off.clone(), kind, true);
            let but = with(all_on.clone(), kind, false);
            for other in KINDS {
                assert_eq!(shows(&only, other, false), other == kind, "only {kind:?} on: {other:?}");
                assert_eq!(shows(&but, other, false), other != kind, "only {kind:?} off: {other:?}");
            }
        }
    }

    #[test]
    fn in_front_of_you_codly_stays_quiet_unless_you_turn_that_off() {
        let quiet = NotificationSettings::default();
        let always = NotificationSettings { only_in_background: false, ..Default::default() };
        for kind in KINDS {
            assert!(!shows(&quiet, kind, true), "{kind:?}");
            assert!(shows(&quiet, kind, false), "{kind:?}");
            assert!(shows(&always, kind, true), "{kind:?}");
            assert!(!shows(&with(always.clone(), kind, false), kind, true), "{kind:?} is off: not in front either");
        }
    }

    #[test]
    fn one_notice_shows_alone_after_the_window() {
        let t0 = Instant::now();
        let mut groups = Groups::default();
        assert_eq!(groups.next_due(), None);
        let notice = said(Kind::TaskBlocked, "t1 — Add sign-in: no worker");
        groups.add(notice.clone(), t0);
        assert_eq!(groups.next_due(), Some(t0 + GROUP_WINDOW));
        assert!(groups.take_due(t0 + GROUP_WINDOW - Duration::from_millis(1)).is_empty(), "it waits for others of its kind");
        let due = groups.take_due(t0 + GROUP_WINDOW);
        assert_eq!(due, vec![vec![notice.clone()]]);
        assert_eq!(merged(due.into_iter().next().unwrap()), Some(notice), "one notice is shown as it is");
        assert_eq!(groups.next_due(), None);
    }

    #[test]
    fn several_notices_of_one_kind_within_the_window_are_one_notification() {
        let t0 = Instant::now();
        let mut groups = Groups::default();
        for (i, line) in ["t1 — A: no worker", "t2 — B: no worker", "t3 — C: checks failed"].into_iter().enumerate() {
            groups.add(said(Kind::TaskBlocked, line), t0 + Duration::from_secs(i as u64));
        }
        assert_eq!(groups.next_due(), Some(t0 + GROUP_WINDOW), "the first notice sets the time: a stream of them cannot put it off");
        let due = groups.take_due(t0 + GROUP_WINDOW);
        assert_eq!(due.len(), 1);
        let one = merged(due.into_iter().next().unwrap()).unwrap();
        assert_eq!(one.kind, Kind::TaskBlocked);
        assert_eq!((one.title.as_str(), one.body.as_str()), ("3 tasks stopped", "t1 — A: no worker\nt2 — B: no worker\nt3 — C: checks failed"));
        assert_eq!(one.target, in_ws(What::Task { id: "t1 — A: no worker".to_string() }), "a click opens the first");
    }

    #[test]
    fn a_long_group_counts_what_it_does_not_list() {
        let notices: Vec<Notice> = (1..=5).map(|i| said(Kind::Approval, &format!("Agent {i} wants to run `x`"))).collect();
        let one = merged(notices).unwrap();
        assert_eq!(one.title, "5 approvals wait for you");
        assert_eq!(one.body, "Agent 1 wants to run `x`\nAgent 2 wants to run `x`\nAgent 3 wants to run `x`\nand 2 more");
        assert_eq!(merged(vec![]), None);
    }

    #[test]
    fn kinds_do_not_mix_and_a_later_notice_starts_a_new_group() {
        let t0 = Instant::now();
        let mut groups = Groups::default();
        groups.add(said(Kind::TaskBlocked, "t1"), t0);
        groups.add(said(Kind::PlanDone, "Sign-in page"), t0 + Duration::from_secs(1));
        groups.add(said(Kind::TaskBlocked, "t2"), t0 + Duration::from_secs(2));
        let first = groups.take_due(t0 + GROUP_WINDOW);
        assert_eq!(first, vec![vec![said(Kind::TaskBlocked, "t1"), said(Kind::TaskBlocked, "t2")]]);
        assert_eq!(groups.next_due(), Some(t0 + Duration::from_secs(1) + GROUP_WINDOW), "the plan's notice has its own time");
        // After its group was shown, a task that blocks starts a new one.
        groups.add(said(Kind::TaskBlocked, "t3"), t0 + Duration::from_secs(4));
        let rest = groups.take_due(t0 + Duration::from_secs(4) + GROUP_WINDOW);
        assert_eq!(rest, vec![vec![said(Kind::PlanDone, "Sign-in page")], vec![said(Kind::TaskBlocked, "t3")]]);
    }

    #[test]
    fn each_kind_has_a_title_for_several() {
        let titles: Vec<String> = KINDS.iter().map(|k| k.many(2)).collect();
        assert_eq!(
            titles,
            [
                "2 approvals wait for you",
                "2 questions wait for you",
                "2 plans finished",
                "2 plans stopped",
                "2 tasks stopped",
                "2 agents need a look",
                "2 budgets reached",
                "2 agents have work ready to merge",
            ]
        );
    }

    #[test]
    fn coming_back_while_a_notification_shows_opens_what_it_names_once() {
        let t0 = Instant::now();
        let target = in_ws(What::Task { id: "t".to_string() }).unwrap();
        let mut shown = Shown::default();
        assert_eq!(shown.take(t0), None, "no notification: nothing to open");
        shown.set(target.clone(), t0);
        assert_eq!(shown.take(t0 + Duration::from_secs(5)), Some(target.clone()), "a click on the banner");
        assert_eq!(shown.take(t0 + Duration::from_secs(5)), None, "opened once");
        shown.set(target.clone(), t0);
        assert_eq!(shown.take(t0 + OPEN_WITHIN), Some(target.clone()), "a click as the banner leaves");
        shown.set(target, t0);
        assert_eq!(shown.take(t0 + Duration::from_secs(20)), None, "the banner is long gone: you are back for your own reasons, and the screen stays");
        assert_eq!(shown.take(t0), None, "and it is forgotten");
    }

    /// The payload of `open-notice`, as src/lib/state/openNotice.ts reads it.
    #[test]
    fn a_target_reads_as_the_ui_expects() {
        let json = |what: What| serde_json::to_value(Target { workspace_id: Some("ws".to_string()), what }).unwrap();
        assert_eq!(json(What::Approval { id: "ap1".to_string(), agent_id: "a1".to_string() }), json!({ "workspace_id": "ws", "kind": "approval", "id": "ap1", "agent_id": "a1" }));
        assert_eq!(json(What::Questions), json!({ "workspace_id": "ws", "kind": "questions" }));
        assert_eq!(json(What::Plan { id: "p".to_string() }), json!({ "workspace_id": "ws", "kind": "plan", "id": "p" }));
        assert_eq!(json(What::Task { id: "t".to_string() }), json!({ "workspace_id": "ws", "kind": "task", "id": "t" }));
        assert_eq!(json(What::Agent { id: "a1".to_string() }), json!({ "workspace_id": "ws", "kind": "agent", "id": "a1" }));
        assert_eq!(serde_json::to_value(Target { workspace_id: None, what: What::Budgets }).unwrap(), json!({ "workspace_id": null, "kind": "budgets" }));
    }

    #[test]
    fn the_ui_is_told_what_to_open_when_codly_comes_to_the_front() {
        use tauri::Listener;

        let app = tauri::test::mock_builder().build(tauri::generate_context!(test = true)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        app.listen(OPEN_EVENT, move |event| tx.send(event.payload().to_string()).unwrap());
        let shown = Mutex::new(Shown::default());
        let t0 = Instant::now();
        returned(app.handle(), &shown, t0);
        assert!(rx.try_recv().is_err(), "no notification was shown: the screen stays");
        shown.lock().unwrap().set(in_ws(What::Plan { id: "p".to_string() }).unwrap(), t0);
        returned(app.handle(), &shown, t0 + Duration::from_secs(5));
        let payload: Value = serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).expect("the UI is told")).unwrap();
        assert_eq!(payload, json!({ "workspace_id": "ws", "kind": "plan", "id": "p" }));
        returned(app.handle(), &shown, t0 + Duration::from_secs(6));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "told once");
    }
}
