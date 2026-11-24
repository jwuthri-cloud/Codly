//! What you do from the agent sheet to the things that run beside a live Claude agent's own
//! calls: its MCP servers and what it left running in the background. All go to the live
//! session as the control requests its CLI takes (`SessionRequest`, sent as the other live
//! controls are: controls.rs), and all are on record as your act.
//!
//! - **Reconnect** an MCP server (`mcp_reconnect`). The CLI's answer is yours to read: when
//!   the server still does not connect, its own reason is the error. The adapter then asks the
//!   CLI where its servers stand, so the agent's `McpServer` events follow.
//! - **Turn off** and **Turn on** an MCP server (`mcp_toggle`). The server's state follows as
//!   after a reconnect, and the act is on record once the CLI answered: a switch the CLI
//!   refused is on record as what you asked, with its reason. The CLI saves the switch in
//!   `~/.claude.json`, under the folder the run works in: off holds for every later Claude Code
//!   process in that folder, new or resumed, until "Turn on" (part 5 §1,
//!   `r7-mcp-toggle-saved-*`, n=2, in a throw-away HOME, in a plain folder), so the record
//!   says it is for the folder the agent works in. Not recorded for a git worktree (each of
//!   Codly's worktrees is one): which folder's entry gets the switch there, so the record names
//!   no more than that folder. Codly's own server stays on: the agent reports through it.
//! - **Stop** a background command or helper (`stop_task`), or a helper whose agent still
//!   waits for it, by the id of its call: the agent's turn goes on (recorded, part 4 §2). The
//!   CLI answers the same whether or not something ended; its next list of background tasks
//!   (`BackgroundTasks`), or the helper's own row, says that it was stopped.
//!
//! The sheet knows the servers and the background work from the run's events: a server's
//! `McpServer` event when its state changes, and the CLI's whole list of background tasks at
//! every change (`BackgroundTasks`, the one record of that work: the gate waits on the same
//! events, background.rs). A snapshot has only the newest events of the workspace, so the run
//! keeps the rows a sheet needs (`SessionRows`) and a snapshot carries the ones its events do
//! not hold: a run that lives for hours (a Lead's) still shows its servers and what it runs in
//! the background after you come back to its office.
//!
//! Every run reports each of its servers once when it starts. A server that stands where the
//! agent's last row of it says gets no new row (`mcp_server_repeats`): the event still goes to
//! the open window, and the run keeps it for a snapshot, but ten runs with fifteen servers do
//! not fill the History with rows that say nothing new.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};

use super::{Inner, lock, prompts};
use crate::adapters::SessionRequest;
use crate::model::{EventKind, EventRow, Id, McpServerStatus};

/// How long Claude Code has to answer a reconnect, or a server turned on (which starts the
/// server again). It answers when its try to connect is over (recorded: the answer carries
/// the connect error), and it gives a server `MCP_TIMEOUT` to start: 30 s unless you set
/// another (docs/research/cli-claude-codex.md).
const RECONNECT_ANSWER_LIMIT: Duration = Duration::from_secs(60);

/// Codly's own MCP server in every run (`claude::mcp_config`): the agent reports through it.
const CODLY_SERVER: &str = "codly";

/// Per agent: where each of its MCP servers stood in the last `McpServer` row on record, by
/// the server's name (its state and the CLI's reason). Kept while Codly runs: after a
/// restart the first run of an agent records its servers once more.
pub(super) type McpRecorded = HashMap<Id, HashMap<String, (McpServerStatus, Option<String>)>>;

/// What runs beside a live run's own calls, as its events last said it: the last `McpServer`
/// row of each server, and the run's last `BackgroundTasks` row (the CLI's whole list).
#[derive(Default)]
pub(super) struct SessionRows(Vec<EventRow>);

impl SessionRows {
    /// One more `McpServer` or `BackgroundTasks` row of the run: it takes the place of the
    /// row of the same server, or of the list before it.
    fn seen(&mut self, row: &EventRow) {
        match &row.kind {
            EventKind::McpServer { name, .. } => self.0.retain(|r| !matches!(&r.kind, EventKind::McpServer { name: known, .. } if known == name)),
            EventKind::BackgroundTasks { .. } => self.0.retain(|r| !matches!(r.kind, EventKind::BackgroundTasks { .. })),
            _ => return,
        }
        self.0.push(row.clone());
    }

    /// What the run's last list of background tasks says the task `id` is.
    fn background_task(&self, id: &str) -> Option<String> {
        self.0.iter().find_map(|r| match &r.kind {
            EventKind::BackgroundTasks { tasks, .. } => tasks.iter().find(|t| t.id == id).map(|t| t.description.clone()),
            _ => None,
        })
    }
}

impl Inner {
    /// Does this event only say again where the agent's last row on record has that MCP
    /// server (the same state, the same reason)? Then the ingest pipeline shows it and does
    /// not store it.
    pub(super) fn mcp_server_repeats(&self, agent_id: &str, kind: &EventKind) -> bool {
        let EventKind::McpServer { name, status, error } = kind else { return false };
        lock(&self.mcp_recorded).get(agent_id).and_then(|servers| servers.get(name)).is_some_and(|(s, e)| s == status && e == error)
    }

    /// The ingest pipeline's hook: an `McpServer` or `BackgroundTasks` row of an agent's run,
    /// on record (its id) or, with id 0, a server that stands where the record has it.
    pub(super) fn session_row_seen(&self, agent_id: &str, row: &EventRow) {
        if let (EventKind::McpServer { name, status, error }, true) = (&row.kind, row.id > 0) {
            lock(&self.mcp_recorded).entry(agent_id.to_string()).or_default().insert(name.clone(), (*status, error.clone()));
        }
        if let Some(l) = lock(&self.live).get_mut(agent_id).filter(|l| row.run_id.as_deref() == Some(l.run_id.as_str())) {
            l.session_rows.seen(row);
        }
    }

    /// For a snapshot of the workspace `ws` with these `events` (oldest first): the rows its
    /// live runs keep that the events do not hold, oldest first. Those are the rows older
    /// than the events, and the rows that are in no log (id 0, before the others).
    pub(super) fn session_rows(&self, ws: Option<&str>, events: &[EventRow]) -> Vec<EventRow> {
        let first = events.first().map(|e| e.id);
        let missing = |r: &&EventRow| r.id == 0 || first.is_some_and(|first| r.id < first);
        let live = lock(&self.live);
        let mut rows: Vec<EventRow> =
            live.values().filter(|l| l.workspace_id.as_deref() == ws).flat_map(|l| &l.session_rows.0).filter(missing).cloned().collect();
        rows.sort_by_key(|r| r.id);
        rows
    }

    async fn reconnect_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let server = server.trim();
        if server.is_empty() {
            bail!("which MCP server?");
        }
        let answer = self.ask_session(agent_id, SessionRequest::McpReconnect(server.to_string()))?;
        self.control_decision(agent_id, format!("asked {} to reconnect its MCP server {server}", agent.name));
        Self::session_answer_within(answer, RECONNECT_ANSWER_LIMIT).await?;
        Ok(())
    }

    async fn set_mcp_enabled(&self, agent_id: &str, server: &str, on: bool) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let server = server.trim();
        if server.is_empty() {
            bail!("which MCP server?");
        }
        if server == CODLY_SERVER {
            bail!("{CODLY_SERVER} is Codly's own server: {} reports to Codly through it, so it stays on", agent.name);
        }
        let answer = self.ask_session(agent_id, SessionRequest::McpEnabled(server.to_string(), on))?;
        let switched = if on { "on" } else { "off" };
        // "You turned" is true only once the CLI answered that it did. A request it refused,
        // or did not answer in time (it can still take effect then), is on record as what you
        // asked and what came back.
        match Self::session_answer_within(answer, RECONNECT_ANSWER_LIMIT).await {
            Ok(_) => {
                self.control_decision(agent_id, switch_record(server, on, &agent.name));
                Ok(())
            }
            Err(e) => {
                self.control_decision(agent_id, format!("asked {} to turn {server} {switched}: {}", agent.name, prompts::clip(&e.to_string(), 200)));
                Err(e)
            }
        }
    }

    async fn stop_background_task(&self, agent_id: &str, task_id: &str) -> Result<()> {
        let agent = self.live_agent(agent_id)?;
        let task_id = task_id.trim();
        if task_id.is_empty() {
            bail!("which background command?");
        }
        // What the run's last list calls it: a command, or a helper.
        let named = lock(&self.live).get(agent_id).and_then(|l| l.session_rows.background_task(task_id));
        let answer = self.ask_session(agent_id, SessionRequest::StopTask(task_id.to_string()))?;
        // The record follows the answer: the adapter refuses an id that names nothing running
        // any more (the list on record can be behind its own), and then nothing reached the CLI.
        // In no list on record: the call id of a helper at work, which the adapter knows (and
        // names in its answer), or a task of a list that is not on record yet.
        let said = Self::session_answer(answer).await?;
        let summary = match (named, said["helper"].as_str()) {
            (Some(description), _) => format!("asked {} to stop what it left running in the background: {description}", agent.name),
            (None, Some(helper)) => format!("asked {} to stop its helper: {helper}", agent.name),
            (None, None) => format!("asked {} to stop what it left running in the background", agent.name),
        };
        self.control_decision(agent_id, summary);
        Ok(())
    }
}

/// The record of a switch the CLI took. Claude Code saves it for the folder the run works in,
/// not for the run (`r7-mcp-toggle-saved-*`): off holds for every later process there until
/// "Turn on" (recorded in a plain folder). Not recorded for a git worktree, as each of Codly's
/// worktrees is: which folder's entry gets the switch there (the worktree's, or its main
/// repository's, which would also hold for that repository's other worktrees). So the words
/// name the folder the agent works in and say nothing of other folders.
fn switch_record(server: &str, on: bool, agent: &str) -> String {
    if on {
        format!("you turned {server} on for Claude Code in the folder {agent} works in")
    } else {
        format!("you turned {server} off for Claude Code in the folder {agent} works in (this run and every later one there, until you turn it on)")
    }
}

impl super::Core {
    /// Try again to connect one MCP server of a live Claude agent's session. Err = the server
    /// still does not connect, with Claude Code's own reason; the agent's `McpServer` events
    /// say where the server stands after the try. Claude Code answers when its try is over:
    /// that can take as long as the server takes to start.
    pub async fn reconnect_agent_mcp(&self, agent_id: &str, server: &str) -> Result<()> {
        self.inner.reconnect_mcp(agent_id, server).await
    }

    /// Turn one MCP server of a live Claude agent's session off or on. The agent's
    /// `McpServer` events say where the server stands then. Err = what Claude Code answered
    /// (or that it did not answer), or a refusal: Codly's own server stays on. Claude Code
    /// saves the switch for the folder the run works in: off holds for its later runs there.
    pub async fn set_agent_mcp_enabled(&self, agent_id: &str, server: &str, enabled: bool) -> Result<()> {
        self.inner.set_mcp_enabled(agent_id, server, enabled).await
    }

    /// Stop what a live Claude agent left running in the background: a command or a helper
    /// (`task_id`: its id in the agent's `BackgroundTasks` event), or any helper at work (the
    /// `call_id` of its `SubAgent` event). The agent's next event of that kind says that it
    /// was stopped. Err for an id that names neither.
    pub async fn stop_agent_task(&self, agent_id: &str, task_id: &str) -> Result<()> {
        self.inner.stop_background_task(agent_id, task_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BackgroundTask;

    fn row(id: i64, kind: EventKind) -> EventRow {
        EventRow { id, ts: 1000 + id, agent_id: Some("a1".into()), task_id: None, run_id: Some("r1".into()), workspace_id: None, kind }
    }

    fn server(name: &str, status: McpServerStatus) -> EventKind {
        EventKind::McpServer { name: name.into(), status, error: None }
    }

    fn list(ids: &[&str]) -> EventKind {
        let tasks = ids.iter().map(|id| BackgroundTask { id: (*id).into(), kind: "command".into(), description: "npm run dev".into(), started_at: None }).collect();
        EventKind::BackgroundTasks { tasks, ended: vec![] }
    }

    /// The ids of the rows a run keeps, in the order it got them.
    fn kept(rows: &SessionRows) -> Vec<i64> {
        rows.0.iter().map(|r| r.id).collect()
    }

    #[test]
    fn a_run_keeps_its_last_list_of_background_tasks_and_the_last_row_of_each_server() {
        let mut rows = SessionRows::default();
        rows.seen(&row(1, server("docs", McpServerStatus::Connecting)));
        rows.seen(&row(2, list(&["tid-1"])));
        rows.seen(&row(3, server("linear", McpServerStatus::NeedsSignIn)));
        rows.seen(&row(4, server("docs", McpServerStatus::Connected)));
        assert_eq!(kept(&rows), [2, 3, 4]);
        // The list is the CLI's whole list each time: one row per run, the newest.
        rows.seen(&row(5, list(&["tid-1", "tid-2"])));
        assert_eq!(kept(&rows), [3, 4, 5]);
        // "Stop" names what it stops by that list.
        assert_eq!(rows.background_task("tid-2").as_deref(), Some("npm run dev"));
        assert_eq!(rows.background_task("tid-9"), None, "not in the run's last list");
        // An empty list is the newest list too: the sheet reads from it what ended last.
        rows.seen(&row(6, list(&[])));
        assert_eq!(kept(&rows), [3, 4, 6]);
        assert!(matches!(&rows.0[2].kind, EventKind::BackgroundTasks { tasks, .. } if tasks.is_empty()));
        // A server that stands where an earlier run recorded it has no row in the log (id 0):
        // the run keeps it all the same, and a row of its change takes its place.
        rows.seen(&row(0, server("search", McpServerStatus::Connected)));
        assert_eq!(kept(&rows), [3, 4, 6, 0]);
        rows.seen(&row(7, server("search", McpServerStatus::Failed)));
        assert_eq!(kept(&rows), [3, 4, 6, 7]);
        // Any other row is not kept.
        rows.seen(&row(8, EventKind::Unknown { raw_type: "step".into() }));
        assert_eq!(kept(&rows), [3, 4, 6, 7]);
    }

    /// An earlier build wrote one row per background command, of a kind that is gone
    /// (`background_command`), and list rows with no `ended`. Such rows are still in stores:
    /// the first loads as a row of an unknown kind, the second with nothing ended, and the
    /// rows around them load as ever.
    #[test]
    fn rows_that_an_earlier_build_wrote_about_background_work_still_load() {
        let store = crate::store::Store::open_in_memory().unwrap();
        let insert = "INSERT INTO event (ts, agent_id, run_id, type, payload) VALUES (1, 'a1', 'r1', ?1, ?2)";
        let old_command = r#"{"type":"background_command","task_id":"tid-1","call_id":"toolu-1","description":"npm run dev","status":"running"}"#;
        let old_list = r#"{"type":"background_tasks","tasks":[{"id":"tid-1","kind":"command","description":"npm run dev"}]}"#;
        store.conn().execute(insert, ["background_command", old_command]).unwrap();
        store.conn().execute(insert, ["background_tasks", old_list]).unwrap();
        store.append_event(None, &row(0, server("docs", McpServerStatus::Connected))).unwrap();

        let rows = store.latest_events(None, 10).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(matches!(&rows[0].kind, EventKind::Unknown { raw_type } if raw_type == "background_command"), "{:?}", rows[0].kind);
        assert!(matches!(&rows[1].kind, EventKind::BackgroundTasks { tasks, ended } if tasks.len() == 1 && ended.is_empty()), "{:?}", rows[1].kind);
        assert!(matches!(&rows[2].kind, EventKind::McpServer { name, .. } if name == "docs"), "{:?}", rows[2].kind);
    }

    /// Recorded (part 5 §1, `r7-mcp-toggle-saved-*`, n=2): Claude Code saves "off" for the
    /// folder, and every later process there starts that server off. Before, the record said
    /// the switch was "for Ada's run". Not recorded in a git worktree, so the words say nothing
    /// of other folders.
    #[test]
    fn a_switch_is_on_record_for_the_folder_the_run_works_in() {
        assert_eq!(
            switch_record("linear", false, "Ada"),
            "you turned linear off for Claude Code in the folder Ada works in (this run and every later one there, until you turn it on)"
        );
        assert_eq!(switch_record("linear", true, "Ada"), "you turned linear on for Claude Code in the folder Ada works in");
    }
}
