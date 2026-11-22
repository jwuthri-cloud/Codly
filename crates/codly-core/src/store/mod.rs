//! SQLite store (rusqlite, bundled, WAL). Schema in schema.sql (migration 1); later
//! migrations are appended to `migrations()` as new files, never edits of an applied one.
//!
//! The `event` table is the source of truth. `insert_event` appends a row and updates the
//! projection it implies (agent, plan, task, chat, approval, decision) in the same
//! transaction, so a crash can never leave a projection ahead of or behind its log.
//! One exception, `save_live_agent`: an agent's activity and usage change at every tool call
//! and are saved on its row without an `agent_changed` event.
//! Memories and triples are written by `crate::memory` inside the caller's transaction.
//!
//! ponytail: one `Mutex<Connection>` shared by readers and the writer instead of a batching
//! writer actor plus a read pool (CRITIQUE D3). Every call is a short local transaction;
//! move to an actor when profiles show lock contention.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use rusqlite_migration::{M, Migrations};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::model::*;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("schema.sql")), M::up(include_str!("m2_run_account.sql")), M::up(include_str!("m3_memory_data.sql")), M::up(include_str!("m4_workspace_closed.sql")), M::up(include_str!("m5_lookup_indexes.sql"))])
}

/// One agent process lifetime (`run` table). Not a UI type: the UI sees runs through events.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: Id,
    pub agent_id: Id,
    pub task_id: Option<Id>,
    pub role: Role,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: String,
    pub session_id: Option<String>,
    /// `AccountInfo::id` of the sign-in it ran on (None = the CLI's primary, older runs).
    pub account: Option<String>,
    /// starting | running | completed | failed | cancelled | lost
    pub status: String,
    pub raw_log: String,
    /// `codly_submit_report` / `codly_submit_review` payload.
    pub report: Option<Value>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the database, set the pragmas and run pending migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Store> {
        // tauri-stack §5.4. journal_mode must be set outside a transaction.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;",
        )?;
        migrations().to_latest(&mut conn).context("migrate codly.sqlite")?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    /// The connection, recovering from poisoning: a panic in one caller (e.g. a module
    /// that is still `todo!()`) must not take the whole store down with it.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn with<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        f(&self.conn())
    }

    /// Run `f` in one IMMEDIATE transaction (commit on Ok, rollback on Err or panic).
    pub fn tx<R>(&self, f: impl FnOnce(&Transaction) -> Result<R>) -> Result<R> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let r = f(&tx)?;
        tx.commit()?;
        Ok(r)
    }

    /// Append one event (and its projection) in its own transaction. Returns the new id.
    pub fn append_event(&self, workspace_id: Option<&str>, row: &EventRow) -> Result<i64> {
        self.tx(|tx| insert_event(tx, workspace_id, row))
    }

    /// Checkpoint the WAL into the main file (on shutdown / idle).
    pub fn checkpoint(&self) {
        let _ = self.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    // ───────────── events ─────────────

    pub fn last_event_id(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COALESCE(MAX(id), 0) FROM event", [], |r| r.get(0))?)
    }

    /// A workspace's career events (commits, checks, reviews, merges) since `ts`, oldest first.
    pub fn work_since(&self, workspace_id: &str, ts: i64) -> Result<Vec<EventRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
             WHERE workspace_id = ?1 AND ts >= ?2 AND type IN ('committed', 'verified', 'reviewed', 'merged')
             ORDER BY id",
        )?;
        let rows = st.query_map(params![workspace_id, ts], event_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Newest first, for paging the timeline. Events with no workspace (CLI-level) are included.
    pub fn events(&self, workspace_id: Option<&str>, agent_id: Option<&str>, before: Option<i64>, limit: i64) -> Result<Vec<EventRow>> {
        let conn = self.conn();
        // The cursor is a plain range (no "?3 IS NULL OR"), so a page starts at `before` instead
        // of reading every newer row first; one agent's page walks `event_agent`.
        let (before, limit) = (before.unwrap_or(i64::MAX), limit.clamp(1, 5000));
        let rows = match agent_id {
            Some(agent) => conn
                .prepare_cached(
                    "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
                     WHERE agent_id = ?2 AND id < ?3 AND (?1 IS NULL OR workspace_id = ?1 OR workspace_id IS NULL)
                     ORDER BY id DESC LIMIT ?4",
                )?
                .query_map(params![workspace_id, agent, before, limit], event_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => conn
                .prepare_cached(
                    "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
                     WHERE id < ?2 AND (?1 IS NULL OR workspace_id = ?1 OR workspace_id IS NULL)
                     ORDER BY id DESC LIMIT ?3",
                )?
                .query_map(params![workspace_id, before, limit], event_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    /// One page of a conversation's history, newest first (the "Load earlier" control). A thread
    /// shows more than one agent's events, so its page is the newest `limit` rows before `before` of:
    /// - every event of `owner`, the agent the thread belongs to (a request's Lead, a 1:1's agent);
    /// - approvals (asked and answered) and results (commits, checks, reviews, merges) of the
    ///   thread's tasks, by whoever did them: a request's tasks are its plans' (tasks with no
    ///   plan belong to "lead"), a 1:1's are the ones assigned to its agent;
    /// - a request's messages. A closed request's messages moved to its archive thread, so the
    ///   `chat` table says which ones are this thread's (a 1:1's are all its agent's own events).
    ///
    /// An `approval_requested` row carries the approval as it is now (its answer may be in a
    /// newer page, which the caller already has).
    pub fn thread_events(&self, workspace_id: &str, thread: &str, owner: Option<&str>, before: Option<i64>, limit: i64) -> Result<Vec<EventRow>> {
        /// A message's time is taken just before its event's. The walk for a request's messages
        /// ends at the first event this much older than the thread's first message: a minute
        /// between the two clocks before it would miss a message.
        const CLOCK_SLACK_MS: i64 = 60_000;
        let (before, limit) = (before.unwrap_or(i64::MAX), limit.clamp(1, 5000));
        let request = is_lead_thread(thread);
        let mut rows = match owner {
            Some(owner) => self.events(Some(workspace_id), Some(owner), Some(before), limit)?,
            None => vec![],
        };
        let newest_first = |rows: &mut Vec<EventRow>| {
            rows.sort_unstable_by_key(|r| std::cmp::Reverse(r.id));
            rows.dedup_by_key(|r| r.id);
            rows.truncate(limit as usize);
        };
        {
            let conn = self.conn();
            let tasks = if request {
                Some((
                    "SELECT t.id FROM task t LEFT JOIN plan p ON p.id = t.plan_id
                     WHERE t.workspace_id = ?1 AND COALESCE(NULLIF(json_extract(p.json, '$.thread'), ''), 'lead') = ?2",
                    thread,
                ))
            } else {
                owner.map(|o| ("SELECT id FROM task WHERE workspace_id = ?1 AND json_extract(json, '$.assignee') = ?2", o))
            };
            if let Some((tasks, of)) = tasks {
                let mut st = conn.prepare_cached(&format!(
                    "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
                     WHERE task_id IN ({tasks}) AND id < ?3
                       AND type IN ('approval_requested', 'approval_resolved', 'committed', 'verified', 'reviewed', 'merged')
                     ORDER BY id DESC LIMIT ?4"
                ))?;
                let found = st.query_map(params![workspace_id, of, before, limit], event_from_row)?;
                rows.extend(found.collect::<rusqlite::Result<Vec<_>>>()?);
                newest_first(&mut rows);
            }
            if request {
                // No index finds a thread's `chat` events, so this walks the workspace's events
                // backwards. It stops where the page ends (a full page), after `limit` messages,
                // or at the events from before the thread's first message, whichever is first.
                let page_end = if rows.len() as i64 >= limit { rows.last().map_or(0, |r| r.id) } else { 0 };
                let first_message: Option<i64> =
                    conn.query_row("SELECT MIN(ts) FROM chat WHERE workspace_id = ?1 AND thread = ?2", params![workspace_id, thread], |r| r.get(0))?;
                if let Some(first_message) = first_message {
                    // On the way only a `chat` row has its payload read; the rows found are read whole after.
                    let mut walk = conn.prepare_cached(
                        "SELECT id, ts,
                                CASE WHEN type = 'chat' AND json_extract(payload, '$.message.id') IN (SELECT id FROM chat WHERE workspace_id = ?1 AND thread = ?2)
                                     THEN 1 ELSE 0 END
                         FROM event WHERE workspace_id = ?1 AND id < ?3 AND id > ?4
                         ORDER BY id DESC",
                    )?;
                    let mut walked = walk.query(params![workspace_id, thread, before, page_end])?;
                    let mut messages: Vec<i64> = vec![];
                    while (messages.len() as i64) < limit
                        && let Some(r) = walked.next()?
                        && r.get::<_, i64>(1)? >= first_message.saturating_sub(CLOCK_SLACK_MS)
                    {
                        if r.get(2)? {
                            messages.push(r.get(0)?);
                        }
                    }
                    let mut whole = conn.prepare_cached("SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event WHERE id = ?1")?;
                    for id in messages {
                        rows.push(whole.query_row([id], event_from_row)?);
                    }
                    newest_first(&mut rows);
                }
            }
        }
        for row in &mut rows {
            if let EventKind::ApprovalRequested { approval } = &mut row.kind
                && let Some(now) = self.approval(&approval.id)?
            {
                *approval = now;
            }
        }
        Ok(rows)
    }

    /// The latest `limit` events in chronological order (snapshot / world replay).
    pub fn latest_events(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<EventRow>> {
        let mut v = self.events(workspace_id, None, None, limit)?;
        v.reverse();
        Ok(v)
    }

    /// Artifacts announced by `artifact_added` events, newest first.
    pub fn artifacts(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<Artifact>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(EVENTS_OF_TYPE_SQL)?;
        let rows = st.query_map(params!["artifact_added", workspace_id, limit], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for p in rows {
            if let Ok(EventKind::ArtifactAdded { artifact }) = serde_json::from_str::<EventKind>(&p?) {
                out.push(artifact);
            }
        }
        Ok(out)
    }

    /// One recorded artifact by id (the only files `read_artifact` may serve).
    pub fn artifact(&self, id: &str) -> Result<Option<Artifact>> {
        let conn = self.conn();
        let payload: Option<String> = conn.query_row(ARTIFACT_SQL, [id], |r| r.get(0)).optional()?;
        Ok(payload.and_then(|p| match serde_json::from_str::<EventKind>(&p) {
            Ok(EventKind::ArtifactAdded { artifact }) => Some(artifact),
            _ => None,
        }))
    }

    /// Memories announced by `memory_added` events, newest first. A read path for the
    /// blackboard that does not depend on the memory module's FTS tables.
    pub fn memory_events(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<Memory>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(EVENTS_OF_TYPE_SQL)?;
        let rows = st.query_map(params!["memory_added", workspace_id, limit], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for p in rows {
            if let Ok(EventKind::MemoryAdded { memory }) = serde_json::from_str::<EventKind>(&p?) {
                out.push(memory);
            }
        }
        Ok(out)
    }

    // ───────────── workspaces & settings ─────────────

    /// Insert or refresh a workspace row and mark it as the most recently opened.
    pub fn upsert_workspace(&self, ws: &Workspace) -> Result<()> {
        self.conn().execute(
            "INSERT INTO workspace (id, repo_path, name, default_branch, created_at, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, default_branch = excluded.default_branch,
               opened_at = excluded.opened_at, closed_at = NULL",
            params![ws.id, ws.repo_path, ws.name, ws.default_branch, ws.created_at, now_ms()],
        )?;
        Ok(())
    }

    pub fn workspace(&self, id: &str) -> Result<Option<Workspace>> {
        self.workspace_where("id = ?1", id)
    }

    pub fn workspace_by_path(&self, repo_path: &str) -> Result<Option<Workspace>> {
        self.workspace_where("repo_path = ?1", repo_path)
    }

    fn workspace_where(&self, cond: &str, arg: &str) -> Result<Option<Workspace>> {
        let sql = format!("SELECT id, repo_path, name, default_branch, created_at FROM workspace WHERE {cond}");
        Ok(self
            .conn()
            .query_row(&sql, [arg], |r| {
                Ok(Workspace {
                    id: r.get(0)?,
                    repo_path: r.get(1)?,
                    name: r.get(2)?,
                    default_branch: r.get(3)?,
                    current_branch: None,
                    head: None,
                    dirty: false,
                    created_at: r.get(4)?,
                })
            })
            .optional()?)
    }

    pub fn last_workspace(&self) -> Result<Option<Workspace>> {
        let id: Option<String> =
            self.conn().query_row("SELECT id FROM workspace ORDER BY opened_at DESC LIMIT 1", [], |r| r.get(0)).optional()?;
        match id {
            Some(id) => self.workspace(&id),
            None => Ok(None),
        }
    }

    /// Close an office (hide the repo); `upsert_workspace` (opening it again) reopens it.
    pub fn close_workspace(&self, id: &str) -> Result<()> {
        self.conn().execute("UPDATE workspace SET closed_at = ?2 WHERE id = ?1", params![id, now_ms()])?;
        Ok(())
    }

    pub fn recent_workspaces(&self, limit: i64) -> Result<Vec<RecentWorkspace>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT repo_path, name, opened_at FROM workspace WHERE closed_at IS NULL ORDER BY opened_at DESC LIMIT ?1")?;
        let rows = st.query_map([limit], |r| Ok(RecentWorkspace { repo_path: r.get(0)?, name: r.get(1)?, opened_at: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A request thread's choices (defaults when unset or unreadable).
    pub fn request_prefs(&self, workspace_id: &str, thread: &str) -> Result<RequestPrefs> {
        Ok(self.kv_get(&format!("request:{workspace_id}:{thread}"))?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    /// None removes them (the thread goes back to the Settings defaults).
    pub fn save_request_prefs(&self, workspace_id: &str, thread: &str, p: Option<&RequestPrefs>) -> Result<()> {
        let key = format!("request:{workspace_id}:{thread}");
        match p {
            Some(p) => self.kv_set(&key, &serde_json::to_string(p)?),
            None => {
                self.conn().execute("DELETE FROM kv WHERE key = ?1", [key])?;
                Ok(())
            }
        }
    }

    /// Every request thread's choices in a workspace, by thread.
    pub fn all_request_prefs(&self, workspace_id: &str) -> Result<std::collections::HashMap<String, RequestPrefs>> {
        let prefix = format!("request:{workspace_id}:");
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT key, value FROM kv WHERE substr(key, 1, length(?1)) = ?1")?;
        let rows = st.query_map([&prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            if let Ok(p) = serde_json::from_str(&v) {
                out.insert(k[prefix.len()..].to_string(), p);
            }
        }
        Ok(out)
    }

    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    /// Every (key, value) whose key starts with `prefix` ("lot:slot:" for every plot's lot).
    pub fn kv_prefixed(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT key, value FROM kv WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")?;
        let rows = st.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Several kv writes in one transaction (None deletes the key): a purchase and what it
    /// bought land together or not at all.
    pub fn kv_write(&self, writes: &[(String, Option<String>)]) -> Result<()> {
        self.tx(|tx| {
            for (key, value) in writes {
                match value {
                    Some(v) => tx.execute("INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, v])?,
                    None => tx.execute("DELETE FROM kv WHERE key = ?1", [key])?,
                };
            }
            Ok(())
        })
    }

    /// Settings (defaults when unset). A corrupt value falls back to defaults rather than
    /// failing startup: defaults are the safe configuration.
    pub fn settings(&self) -> Result<Settings> {
        Ok(match self.kv_get("settings")? {
            Some(s) => serde_json::from_str::<serde_json::Value>(&s).map(migrate_settings).and_then(serde_json::from_value).unwrap_or_else(|e| {
                tracing::warn!("settings unreadable, using defaults: {e}");
                Settings::default()
            }),
            None => Settings::default(),
        })
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.kv_set("settings", &serde_json::to_string(s)?)
    }

    // ───────────── agents ─────────────

    pub fn agent(&self, id: &str) -> Result<Option<Agent>> {
        self.json_row("SELECT json FROM agent WHERE id = ?1", id)
    }

    /// Non-archived agents of a workspace, oldest first (stable character order).
    pub fn agents(&self, workspace_id: &str) -> Result<Vec<Agent>> {
        self.json_rows("SELECT json FROM agent WHERE workspace_id = ?1 AND archived = 0 ORDER BY created_at, id", workspace_id)
    }

    /// A workspace's archived agents (let go), newest first: the office's ghosts.
    pub fn archived_agents(&self, workspace_id: &str) -> Result<Vec<Agent>> {
        self.json_rows("SELECT json FROM agent WHERE workspace_id = ?1 AND archived = 1 ORDER BY updated_at DESC, id", workspace_id)
    }

    pub fn agent_workspace(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT workspace_id FROM agent WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten())
    }

    /// Move an agent to another workspace (a transfer between offices): its row, same id, and
    /// the private memories it keeps (scope `agent`). Its events stay in the repo they happened in.
    pub fn move_agent(&self, id: &str, workspace_id: &str) -> Result<()> {
        self.tx(|tx| {
            if tx.execute("UPDATE agent SET workspace_id = ?2 WHERE id = ?1", params![id, workspace_id])? == 0 {
                return Err(anyhow!("unknown agent {id}"));
            }
            tx.execute("UPDATE memory SET workspace_id = ?2 WHERE scope = 'agent' AND scope_id = ?1", params![id, workspace_id])?;
            Ok(())
        })
    }

    /// Every workspace with its crew (agents on the team, the Lead aside), its tasks in flight
    /// and what waits for you there, most recently opened first. `needs_you` counts approvals
    /// and plans here; the open blocking questions are added by the one caller that serves the
    /// field (`Inner::offices`): they cost a read of every blocker event, too much for every caller.
    pub fn offices(&self) -> Result<Vec<Office>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT w.id, w.name, w.repo_path,
               (SELECT COUNT(*) FROM agent a WHERE a.workspace_id = w.id AND a.archived = 0 AND a.role != 'lead'),
               (SELECT COUNT(*) FROM task t WHERE t.workspace_id = w.id AND t.status IN ({ACTIVE})),
               (SELECT COUNT(*) FROM approval p WHERE p.workspace_id = w.id AND p.status = 'pending')
                 + (SELECT COUNT(*) FROM plan p WHERE p.workspace_id = w.id AND p.status = 'awaiting_approval')
             FROM workspace w WHERE w.closed_at IS NULL ORDER BY w.opened_at DESC"
        );
        let mut st = conn.prepare_cached(&sql)?;
        let rows = st.query_map([], |r| {
            Ok(Office { id: r.get(0)?, name: r.get(1)?, repo_path: r.get(2)?, crew: r.get(3)?, working: r.get(4)?, needs_you: r.get(5)?, home: vec![] })
        })?;
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        // Who is at home now, in each (the map walks them down the street).
        let mut st = conn.prepare_cached(
            "SELECT workspace_id, json FROM agent WHERE archived = 0 AND role != 'lead' AND json_extract(json, '$.away_until') > ?1 ORDER BY created_at, id",
        )?;
        let away = st.query_map([now_ms()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in away {
            let (ws, json) = row?;
            let Ok(a) = serde_json::from_str::<Agent>(&json) else { continue };
            if let (Some(o), Some(until)) = (out.iter_mut().find(|o| o.id == ws), a.away_until) {
                o.home.push(AwayAgent { id: a.id, name: a.name, color: a.color, since: a.away_since, until });
            }
        }
        Ok(out)
    }

    /// Workspaces with work in flight: a running plan, or a task that is ready or active.
    pub fn working_workspaces(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT workspace_id FROM plan WHERE status = 'running'
             UNION SELECT workspace_id FROM task WHERE status IN ('ready', {ACTIVE})"
        );
        let mut st = conn.prepare_cached(&sql)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A workspace's team-member agents, archived ones included (a member switched back on
    /// gets its old agent, session and memory back).
    pub fn member_agents(&self, workspace_id: &str) -> Result<Vec<Agent>> {
        self.json_rows(
            "SELECT json FROM agent WHERE workspace_id = ?1 AND json_extract(json, '$.member') IS NOT NULL ORDER BY created_at, id",
            workspace_id,
        )
    }

    /// A repo's changes to the global team (none when unset or unreadable).
    pub fn workspace_team(&self, workspace_id: &str) -> Result<WorkspaceTeam> {
        Ok(match self.kv_get(&format!("team:{workspace_id}"))? {
            Some(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("team of {workspace_id} unreadable, using the global team: {e}");
                WorkspaceTeam::default()
            }),
            None => WorkspaceTeam::default(),
        })
    }

    pub fn save_workspace_team(&self, workspace_id: &str, t: &WorkspaceTeam) -> Result<()> {
        self.kv_set(&format!("team:{workspace_id}"), &serde_json::to_string(t)?)
    }

    // ───────────── plans & tasks ─────────────

    pub fn plan(&self, id: &str) -> Result<Option<Plan>> {
        self.json_row("SELECT json FROM plan WHERE id = ?1", id)
    }

    /// Newest first.
    pub fn plans(&self, workspace_id: &str, limit: i64) -> Result<Vec<Plan>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT json FROM plan WHERE workspace_id = ?1 ORDER BY created_at DESC LIMIT ?2")?;
        let rows = st.query_map(params![workspace_id, limit], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }

    /// The validated planner output (integration checks, sizes, risks…) kept beside the Plan.
    pub fn plan_json(&self, id: &str) -> Result<Value> {
        let s: Option<String> = self.conn().query_row("SELECT plan_json FROM plan WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null))
    }

    pub fn task(&self, id: &str) -> Result<Option<Task>> {
        self.json_row("SELECT json FROM task WHERE id = ?1", id)
    }

    pub fn tasks(&self, workspace_id: &str) -> Result<Vec<Task>> {
        self.json_rows("SELECT json FROM task WHERE workspace_id = ?1 ORDER BY created_at, id", workspace_id)
    }

    pub fn plan_tasks(&self, plan_id: &str) -> Result<Vec<Task>> {
        self.json_rows("SELECT json FROM task WHERE plan_id = ?1 ORDER BY created_at, id", plan_id)
    }

    /// Tasks in any of `statuses`, across workspaces (crash recovery).
    pub fn tasks_with_status(&self, statuses: &[TaskStatus]) -> Result<Vec<Task>> {
        let mut out = vec![];
        for s in statuses {
            out.extend(self.json_rows::<Task>("SELECT json FROM task WHERE status = ?1 ORDER BY created_at", &enum_str(s))?);
        }
        Ok(out)
    }

    pub fn plans_with_status(&self, statuses: &[PlanStatus]) -> Result<Vec<Plan>> {
        let mut out = vec![];
        for s in statuses {
            out.extend(self.json_rows::<Plan>("SELECT json FROM plan WHERE status = ?1 ORDER BY created_at", &enum_str(s))?);
        }
        Ok(out)
    }

    /// Every agent ever, in every workspace, let go ones too (the wallet's earnings).
    pub fn every_agent(&self) -> Result<Vec<Agent>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT json FROM agent ORDER BY created_at, id")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }

    /// Every non-archived agent of every workspace (crash recovery).
    pub fn all_agents(&self) -> Result<Vec<Agent>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT json FROM agent WHERE archived = 0 ORDER BY created_at")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }

    /// A task's events of one type, oldest first (e.g. its `verified` results).
    pub fn task_events(&self, task_id: &str, ty: &str) -> Result<Vec<EventRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(TASK_EVENTS_SQL)?;
        let rows = st.query_map(params![task_id, ty], event_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The text of the latest message `author` wrote in `thread` (the user's request for a plan).
    pub fn last_chat_by(&self, workspace_id: Option<&str>, thread: &str, author: &str) -> Result<Option<String>> {
        let s: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM chat WHERE workspace_id IS ?1 AND thread = ?2 AND json_extract(json, '$.author') = ?3
                 ORDER BY ts DESC LIMIT 1",
                params![workspace_id, thread, author],
                |r| r.get(0),
            )
            .optional()?;
        Ok(s.and_then(|s| serde_json::from_str::<ChatMessage>(&s).ok()).map(|m| m.text))
    }

    /// The messages of `thread` that came from outside Codly (a webhook, Slack: an `inbound`
    /// attachment) after the user's last own words there, newest first. The user's own words
    /// are a message they wrote, or a schedule of theirs that fired (`triggers::is_schedule`).
    /// The read stops at them, so its cost follows what came in since, not the length of the
    /// thread.
    pub fn inbound_since_user(&self, workspace_id: Option<&str>, thread: &str) -> Result<Vec<ChatMessage>> {
        let conn = self.conn();
        let mut newest_first = conn.prepare_cached("SELECT json FROM chat WHERE workspace_id IS ?1 AND thread = ?2 ORDER BY ts DESC, rowid DESC")?;
        let mut rows = newest_first.query(params![workspace_id, thread])?;
        let mut out = vec![];
        while let Some(row) = rows.next()? {
            let Ok(m) = serde_json::from_str::<ChatMessage>(&row.get::<_, String>(0)?) else { continue };
            if m.author == "user" || crate::triggers::is_schedule(m.attachment.as_ref()) {
                break;
            }
            if crate::inbound::is_inbound(m.attachment.as_ref()) {
                out.push(m);
            }
        }
        Ok(out)
    }

    pub fn task_workspace(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT workspace_id FROM task WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    /// (base_commit, head_commit) of a task branch.
    pub fn task_commits(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        Ok(self
            .conn()
            .query_row("SELECT base_commit, head_commit FROM task WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or((None, None)))
    }

    /// Set the commits that are known (None leaves a column unchanged).
    pub fn set_task_commits(&self, id: &str, base: Option<&str>, head: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE task SET base_commit = COALESCE(?2, base_commit), head_commit = COALESCE(?3, head_commit) WHERE id = ?1",
            params![id, base, head],
        )?;
        Ok(())
    }

    // ───────────── runs ─────────────

    pub fn insert_run(&self, run: &RunRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO run (id, agent_id, task_id, role, provider, model, cwd, session_id, status, raw_log, report_json, started_at, ended_at, account)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                run.id,
                run.agent_id,
                run.task_id,
                enum_str(&run.role),
                run.provider.as_str(),
                run.model,
                run.cwd,
                run.session_id,
                run.status,
                run.raw_log,
                run.report.as_ref().map(|v| v.to_string()),
                run.started_at,
                run.ended_at,
                run.account,
            ],
        )?;
        Ok(())
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRow>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("{RUN_COLS} WHERE id = ?1"), [id], run_from_row).optional()?)
    }

    pub fn runs_with_status(&self, statuses: &[&str]) -> Result<Vec<RunRow>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!("{RUN_COLS} WHERE status = ?1 ORDER BY started_at"))?;
        let mut out = vec![];
        for s in statuses {
            for r in st.query_map([s], run_from_row)? {
                out.push(r?);
            }
        }
        Ok(out)
    }

    /// The newest run of a task, optionally restricted to one role (worker / reviewer).
    pub fn latest_task_run(&self, task_id: &str, role: Option<Role>) -> Result<Option<RunRow>> {
        let conn = self.conn();
        let role = role.map(|r| enum_str(&r));
        Ok(conn.query_row(&latest_task_run_sql(), params![task_id, role], run_from_row).optional()?)
    }

    /// Mark a run's status; terminal statuses also stamp `ended_at`.
    pub fn set_run_status(&self, id: &str, status: &str) -> Result<()> {
        let ended = matches!(status, "completed" | "failed" | "cancelled" | "lost").then(now_ms);
        self.conn().execute("UPDATE run SET status = ?2, ended_at = COALESCE(?3, ended_at) WHERE id = ?1", params![id, status, ended])?;
        Ok(())
    }

    pub fn set_run_session(&self, id: &str, session_id: &str) -> Result<()> {
        self.conn().execute("UPDATE run SET session_id = ?2 WHERE id = ?1", params![id, session_id])?;
        Ok(())
    }

    pub fn set_run_report(&self, id: &str, report: &Value) -> Result<()> {
        let n = self.conn().execute("UPDATE run SET report_json = ?2 WHERE id = ?1", params![id, report.to_string()])?;
        if n == 0 {
            return Err(anyhow!("unknown run {id}"));
        }
        Ok(())
    }

    // ───────────── approvals, chats, decisions ─────────────

    pub fn approval(&self, id: &str) -> Result<Option<Approval>> {
        self.json_row("SELECT json FROM approval WHERE id = ?1", id)
    }

    pub fn pending_approvals(&self, workspace_id: Option<&str>) -> Result<Vec<Approval>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT json FROM approval WHERE status = 'pending' AND (?1 IS NULL OR workspace_id = ?1) ORDER BY created_at",
        )?;
        let rows = st.query_map(params![workspace_id], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }

    pub fn pending_approvals_for_run(&self, run_id: &str) -> Result<Vec<Approval>> {
        self.json_rows("SELECT json FROM approval WHERE status = 'pending' AND run_id = ?1 ORDER BY created_at", run_id)
    }

    /// Move a thread's messages to another one (a closed request's go to its archive thread).
    pub fn move_chats(&self, workspace_id: &str, from: &str, to: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE chat SET thread = ?3, json = json_set(json, '$.thread', ?3) WHERE workspace_id = ?1 AND thread = ?2",
            params![workspace_id, from, to],
        )?)
    }

    /// The last `per_thread` messages of every open thread (closed requests' are left out), in
    /// chronological order. Reads the tail of each thread from the index, never its whole
    /// history: the cost follows what is returned, not how long the office has been open.
    pub fn recent_chats(&self, workspace_id: Option<&str>, per_thread: i64) -> Result<Vec<ChatMessage>> {
        let conn = self.conn();
        let mut threads = conn.prepare_cached(CHAT_THREADS_SQL)?;
        let mut tail = conn.prepare_cached(CHAT_TAIL_SQL)?;
        let mut rows: Vec<(i64, i64, String)> = vec![];
        for thread in threads.query_map(params![workspace_id], |r| r.get::<_, String>(0))? {
            let thread = thread?;
            for row in tail.query_map(params![workspace_id, thread, per_thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
                rows.push(row?);
            }
        }
        // Messages of the same millisecond keep the order they were written in.
        rows.sort_unstable_by_key(|(ts, rowid, _)| (*ts, *rowid));
        decode_all(rows.into_iter().map(|(_, _, json)| Ok(json)))
    }

    /// Record how a decision turned out (calibration log, agents-backend §5.6).
    pub fn set_decision_outcome(&self, id: &str, outcome: &str) -> Result<()> {
        self.conn().execute("UPDATE decision SET outcome = ?2 WHERE id = ?1", params![id, outcome])?;
        Ok(())
    }

    // ───────────── helpers ─────────────

    fn json_row<T: DeserializeOwned>(&self, sql: &str, arg: &str) -> Result<Option<T>> {
        let s: Option<String> = self.conn().query_row(sql, [arg], |r| r.get(0)).optional()?;
        s.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
    }

    fn json_rows<T: DeserializeOwned>(&self, sql: &str, arg: &str) -> Result<Vec<T>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(sql)?;
        let rows = st.query_map([arg], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }
}

/// Store the validated planner output beside its Plan (inside the caller's transaction).
pub fn set_plan_json(db: &Connection, plan_id: &str, v: &Value) -> Result<()> {
    db.execute("UPDATE plan SET plan_json = ?2 WHERE id = ?1", params![plan_id, v.to_string()])?;
    Ok(())
}

/// Insert one event row and apply its projection, inside the caller's transaction.
/// `workspace_id` attributes the row (and new agent/task rows) to a workspace.
pub fn insert_event(db: &Connection, workspace_id: Option<&str>, row: &EventRow) -> Result<i64> {
    if row.kind.is_ephemeral() {
        return Err(anyhow!("ephemeral {} events are never persisted", row.kind.type_name()));
    }
    db.prepare_cached(
        "INSERT INTO event (ts, workspace_id, agent_id, task_id, run_id, type, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?
    .execute(params![
        row.ts,
        workspace_id,
        row.agent_id,
        row.task_id,
        row.run_id,
        row.kind.type_name(),
        serde_json::to_string(&row.kind)?,
    ])?;
    let id = db.last_insert_rowid();
    project(db, workspace_id, row)?;
    Ok(id)
}

/// Save an agent whose change the event log does not keep: what it is doing right now and
/// its usage follow every tool call, and the tool and usage events they come from are already
/// in the log. The one projection write that has no event row of its own.
pub fn save_live_agent(db: &Connection, agent: &Agent) -> Result<()> {
    let n = db
        .prepare_cached("UPDATE agent SET json = ?2, updated_at = ?3 WHERE id = ?1")?
        .execute(params![agent.id, serde_json::to_string(agent)?, agent.updated_at])?;
    if n == 0 {
        return Err(anyhow!("unknown agent {}", agent.id));
    }
    Ok(())
}

fn project(db: &Connection, ws: Option<&str>, row: &EventRow) -> Result<()> {
    match &row.kind {
        EventKind::AgentChanged { agent } => {
            db.prepare_cached(
                "INSERT INTO agent (id, workspace_id, name, provider, role, json, archived, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, json = excluded.json, updated_at = excluded.updated_at,
                   archived = excluded.archived, workspace_id = COALESCE(agent.workspace_id, excluded.workspace_id)",
            )?
            .execute(params![
                agent.id,
                ws,
                agent.name,
                agent.provider.as_str(),
                enum_str(&agent.role),
                serde_json::to_string(agent)?,
                agent.archived,
                agent.created_at,
                agent.updated_at,
            ])?;
        }
        EventKind::PlanChanged { plan } => {
            db.prepare_cached(
                "INSERT INTO plan (id, workspace_id, status, json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET status = excluded.status, json = excluded.json, updated_at = excluded.updated_at",
            )?
            .execute(params![
                plan.id,
                plan.workspace_id,
                enum_str(&plan.status),
                serde_json::to_string(plan)?,
                plan.created_at,
                plan.updated_at,
            ])?;
        }
        EventKind::TaskChanged { task } => {
            let ws = match ws {
                Some(ws) => ws.to_string(),
                None => db
                    .query_row("SELECT workspace_id FROM task WHERE id = ?1", [&task.id], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| anyhow!("task {} has no workspace", task.key))?,
            };
            db.prepare_cached(
                "INSERT INTO task (id, workspace_id, plan_id, status, json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET status = excluded.status, json = excluded.json, updated_at = excluded.updated_at",
            )?
            .execute(params![
                task.id,
                ws,
                task.plan_id,
                enum_str(&task.status),
                serde_json::to_string(task)?,
                task.created_at,
                task.updated_at,
            ])?;
        }
        EventKind::Chat { message } => {
            db.prepare_cached("INSERT OR REPLACE INTO chat (id, workspace_id, thread, json, ts) VALUES (?1, ?2, ?3, ?4, ?5)")?
                .execute(params![message.id, ws, message.thread, serde_json::to_string(message)?, message.ts])?;
        }
        EventKind::ApprovalRequested { approval } => {
            db.prepare_cached(
                "INSERT OR REPLACE INTO approval (id, workspace_id, run_id, status, json, created_at, decided_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            )?
            .execute(params![
                approval.id,
                ws,
                approval.run_id,
                enum_str(&approval.status),
                serde_json::to_string(approval)?,
                approval.created_at,
            ])?;
        }
        EventKind::ApprovalResolved { approval_id, status, by } => {
            // The first answer stands, except against the CLI's own word: an adapter says
            // "provider" only for a request whose answer never reached the CLI.
            let status = enum_str(status);
            let n = db
                .prepare_cached(
                    "UPDATE approval SET status = ?2, decided_at = ?3, json = json_set(json, '$.status', ?2, '$.decided_by', ?4)
                     WHERE id = ?1 AND (status = 'pending' OR ?4 = 'provider')",
                )?
                .execute(params![approval_id, status, row.ts, by])?;
            if n == 0 {
                // or was never recorded; the caller's transaction drops the event with it
                return Err(anyhow!("approval {approval_id} was already resolved"));
            }
        }
        EventKind::Decision { id, decision_type, method, summary, detail } => {
            db.prepare_cached(
                "INSERT OR REPLACE INTO decision (id, workspace_id, plan_id, task_id, type, method, summary, detail, outcome, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                id,
                ws,
                detail.get("plan_id").and_then(Value::as_str),
                row.task_id,
                decision_type,
                method,
                summary,
                detail.to_string(),
                detail.get("outcome").and_then(Value::as_str),
                row.ts,
            ])?;
        }
        _ => {}
    }
    Ok(())
}

const RUN_COLS: &str =
    "SELECT id, agent_id, task_id, role, provider, model, cwd, session_id, status, raw_log, report_json, started_at, ended_at, account FROM run";

// The reads below must stay on an index: the tables they read grow for as long as Codly is
// used. `reads_that_grow_with_history_use_an_index` checks the plan of each.

/// Payloads of one kind of event (?1), newest first, of a workspace (?2, NULL = all), at most ?3.
const EVENTS_OF_TYPE_SQL: &str = "SELECT payload FROM event WHERE type = ?1 AND (?2 IS NULL OR workspace_id = ?2) ORDER BY id DESC LIMIT ?3";

/// One artifact by id. Spelled as the `event_artifact` index is (m5), or SQLite will not use it.
const ARTIFACT_SQL: &str = "SELECT payload FROM event WHERE type = 'artifact_added' AND json_extract(payload, '$.artifact.id') = ?1 LIMIT 1";

/// A task's events of one type. `+type` keeps SQLite on the task's own events (`event_task`)
/// instead of every event of that type ever written (`event_type`).
const TASK_EVENTS_SQL: &str = "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event WHERE task_id = ?1 AND +type = ?2 ORDER BY id";

/// Open threads of a workspace: answered from the `chat_thread` index alone.
const CHAT_THREADS_SQL: &str = "SELECT DISTINCT thread FROM chat WHERE workspace_id IS ?1 AND thread NOT LIKE 'closed:%'";

/// The newest ?3 messages of one thread, as (ts, rowid, json).
const CHAT_TAIL_SQL: &str = "SELECT ts, rowid, json FROM chat WHERE workspace_id IS ?1 AND thread = ?2 ORDER BY ts DESC, rowid DESC LIMIT ?3";

fn latest_task_run_sql() -> String {
    format!("{RUN_COLS} WHERE task_id = ?1 AND (?2 IS NULL OR role = ?2) ORDER BY started_at DESC, rowid DESC LIMIT 1")
}

fn run_from_row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    let role: String = r.get(3)?;
    let provider: String = r.get(4)?;
    let report: Option<String> = r.get(10)?;
    Ok(RunRow {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        task_id: r.get(2)?,
        role: parse_enum(&role).unwrap_or(Role::Worker),
        provider: parse_enum(&provider).unwrap_or(Provider::Claude),
        model: r.get(5)?,
        cwd: r.get(6)?,
        session_id: r.get(7)?,
        status: r.get(8)?,
        raw_log: r.get(9)?,
        report: report.and_then(|s| serde_json::from_str(&s).ok()),
        started_at: r.get(11)?,
        ended_at: r.get(12)?,
        account: r.get(13)?,
    })
}

/// Task statuses that hold a worker (`scheduler::is_active`), as SQL.
const ACTIVE: &str = "'running', 'verifying', 'reviewing', 'changes_requested', 'merging'";

fn event_from_row(r: &rusqlite::Row) -> rusqlite::Result<EventRow> {
    let ty: String = r.get(5)?;
    let payload: String = r.get(6)?;
    // A row written by an older/newer schema must not break the timeline (G12).
    let kind = serde_json::from_str(&payload).unwrap_or(EventKind::Unknown { raw_type: ty });
    Ok(EventRow { id: r.get(0)?, ts: r.get(1)?, agent_id: r.get(2)?, task_id: r.get(3)?, run_id: r.get(4)?, workspace_id: None, kind })
}

fn decode_all<T: DeserializeOwned>(rows: impl Iterator<Item = rusqlite::Result<String>>) -> Result<Vec<T>> {
    let mut out = vec![];
    for s in rows {
        let s = s?;
        match serde_json::from_str(&s) {
            Ok(v) => out.push(v),
            // Skip (and log) a corrupt row instead of hiding every other row behind it.
            Err(e) => tracing::warn!("skipping unreadable row: {e}"),
        }
    }
    Ok(out)
}

/// The serde string of a snake_case enum ("waiting_approval").
pub fn enum_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

pub fn parse_enum<T: DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_string())).ok()
}

/// Settings saved by older versions: `byok_openrouter: true` (the one BYOK switch there was)
/// becomes `byok.openrouter = key`.
fn migrate_settings(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(o) = v.as_object_mut()
        && let Some(old) = o.remove("byok_openrouter")
        && !o.contains_key("byok")
        && old == serde_json::Value::Bool(true)
    {
        o.insert("byok".into(), serde_json::json!({ "openrouter": { "kind": "key" } }));
    }
    v
}

/// The plan SQLite chose for `sql`, one step per line (tests: does a read stay on an index?).
#[cfg(test)]
pub(crate) fn query_plan(conn: &Connection, sql: &str) -> String {
    let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let nulls = vec![rusqlite::types::Null; st.parameter_count()];
    let steps: Vec<String> = st.query_map(rusqlite::params_from_iter(nulls), |r| r.get(3)).unwrap().map(Result::unwrap).collect();
    steps.join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn old_openrouter_switch_becomes_a_byok_source() {
        use crate::byok::ByokSource;
        let read = |json: serde_json::Value| -> Settings { serde_json::from_value(super::migrate_settings(json)).unwrap() };
        let base = serde_json::to_value(Settings::default()).unwrap();
        let mut on = base.clone();
        on.as_object_mut().unwrap().remove("byok");
        on["byok_openrouter"] = true.into();
        assert_eq!(read(on.clone()).byok.openrouter, ByokSource::Key);
        on["byok_openrouter"] = false.into();
        assert_eq!(read(on).byok.openrouter, ByokSource::Off);
        assert_eq!(read(base).byok, Default::default());
    }

    use super::*;

    fn ws(store: &Store) -> Workspace {
        let w = Workspace {
            id: new_id(),
            repo_path: "/tmp/repo".into(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&w).unwrap();
        w
    }

    fn row(kind: EventKind) -> EventRow {
        EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind }
    }

    #[test]
    fn migrates_and_projects() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let plan = Plan {
            id: new_id(),
            workspace_id: w.id.clone(),
            thread: default_lead_thread(),
            request: "r".into(),
            summary: "s".into(),
            assumptions: vec![],
            open_questions: vec![],
            status: PlanStatus::AwaitingApproval,
            base_commit: None,
            base_branch: None,
            integration_branch: None,
            integration_worktree: None,
            merged_sha: None,
            kept: false,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        let id1 = store.append_event(Some(&w.id), &row(EventKind::PlanChanged { plan: plan.clone() })).unwrap();
        assert!(id1 > 0);
        assert_eq!(store.plan(&plan.id).unwrap().unwrap().status, PlanStatus::AwaitingApproval);
        let mut p2 = plan.clone();
        p2.status = PlanStatus::Running;
        let id2 = store.append_event(Some(&w.id), &row(EventKind::PlanChanged { plan: p2 })).unwrap();
        assert!(id2 > id1);
        assert_eq!(store.plans(&w.id, 10).unwrap()[0].status, PlanStatus::Running);
        assert_eq!(store.last_event_id().unwrap(), id2);
        let evs = store.events(Some(&w.id), None, None, 10).unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].id, id2);
        assert!(store.append_event(None, &row(EventKind::TextDelta { item_id: "i".into(), text: "x".into() })).is_err());
    }

    #[test]
    fn an_approval_resolves_once_and_a_provider_cancel_wins() {
        let store = Store::open_in_memory().unwrap();
        let a = Approval {
            id: new_id(),
            agent_id: "a".into(),
            run_id: "r".into(),
            task_id: None,
            call_id: None,
            subject: ApprovalSubject::Command { command: "ls".into(), cwd: None },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        store.append_event(None, &row(EventKind::ApprovalRequested { approval: a.clone() })).unwrap();
        assert_eq!(store.pending_approvals(None).unwrap().len(), 1);
        store
            .append_event(
                None,
                &row(EventKind::ApprovalResolved { approval_id: a.id.clone(), status: ApprovalStatus::Approved, by: "user".into() }),
            )
            .unwrap();
        let got = store.approval(&a.id).unwrap().unwrap();
        assert_eq!(got.status, ApprovalStatus::Approved);
        assert_eq!(got.decided_by.as_deref(), Some("user"));
        assert!(store.pending_approvals(None).unwrap().is_empty());

        // The first answer stands: a second one is refused, and its event is not kept.
        let resolved = |status, by: &str| row(EventKind::ApprovalResolved { approval_id: a.id.clone(), status, by: by.into() });
        let last = store.last_event_id().unwrap();
        assert!(store.append_event(None, &resolved(ApprovalStatus::Denied, "user")).is_err());
        assert_eq!(store.approval(&a.id).unwrap().unwrap().status, ApprovalStatus::Approved);
        assert_eq!(store.last_event_id().unwrap(), last);
        // Except by the CLI's own word: the answer never reached it.
        store.append_event(None, &resolved(ApprovalStatus::Cancelled, "provider")).unwrap();
        let got = store.approval(&a.id).unwrap().unwrap();
        assert_eq!((got.status, got.decided_by.as_deref()), (ApprovalStatus::Cancelled, Some("provider")));
    }

    /// Outside messages count from the user's last own words: a message they wrote, or a
    /// schedule of theirs that fired. Another thread's lines and Codly's other lines do not count.
    #[test]
    fn inbound_messages_are_read_back_to_your_last_own_words() {
        let store = Store::open_in_memory().unwrap();
        let inbound = |source: &str| Some(serde_json::json!({ "inbound": { "source": source, "auto_run": false } }));
        let schedule = Some(serde_json::json!({ "schedule": "s1" }));
        let mut ts = 0;
        let mut say = |thread: &str, author: &str, attachment: Option<Value>| {
            ts += 1;
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: format!("m{ts}"), attachment, ts };
            store.append_event(None, &row(EventKind::Chat { message })).unwrap();
        };
        let since = |thread: &str| -> Vec<String> {
            let found = store.inbound_since_user(None, thread).unwrap();
            found.iter().map(|m| m.attachment.as_ref().unwrap()["inbound"]["source"].as_str().unwrap().to_string()).collect()
        };

        say("lead", "codly", inbound("old"));
        say("lead", "user", None);
        assert!(since("lead").is_empty(), "your message came after it");
        say("lead", "codly", inbound("a"));
        say("lead", "codly", Some(serde_json::json!({ "plan_id": "p1" })));
        say("lead:2", "user", None);
        say("lead", "codly", inbound("b"));
        assert_eq!(since("lead"), vec!["b", "a"], "newest first, back to your message");
        assert!(since("lead:2").is_empty());

        // Your schedule fires: what came in before it no longer counts, what comes in after does.
        say("lead", "codly", schedule);
        assert!(since("lead").is_empty());
        say("lead", "codly", inbound("c"));
        assert_eq!(since("lead"), vec!["c"]);
    }

    #[test]
    fn recent_chats_keep_the_tail_of_each_thread() {
        let store = Store::open_in_memory().unwrap();
        for (i, thread) in ["lead", "lead", "lead", "agent:x"].iter().enumerate() {
            let message = ChatMessage {
                id: new_id(),
                thread: thread.to_string(),
                author: "user".into(),
                text: format!("m{i}"),
                attachment: None,
                ts: i as i64,
            };
            store.append_event(None, &row(EventKind::Chat { message })).unwrap();
        }
        let texts: Vec<_> = store.recent_chats(None, 2).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["m1", "m2", "m3"]);
    }

    #[test]
    fn recent_chats_leave_closed_threads_and_other_offices_out() {
        let store = Store::open_in_memory().unwrap();
        let here = ws(&store);
        let there = Workspace { id: new_id(), repo_path: "/tmp/other".into(), ..here.clone() };
        store.upsert_workspace(&there).unwrap();
        // (office, thread, text, ts); two messages share a millisecond.
        for (w, thread, text, ts) in [
            (&here, "lead", "first", 5),
            (&here, "agent:x", "second", 5),
            (&here, "closed:lead:1", "archived", 6),
            (&there, "lead", "elsewhere", 7),
            (&here, "lead", "third", 8),
        ] {
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: "user".into(), text: text.into(), attachment: None, ts };
            store.append_event(Some(&w.id), &row(EventKind::Chat { message })).unwrap();
        }
        let texts: Vec<_> = store.recent_chats(Some(&here.id), 10).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn events_of_one_type_are_read_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let artifact = |title: &str| Artifact {
            id: new_id(),
            kind: ArtifactKind::Image,
            mime: "image/png".into(),
            path: format!("{title}.png"),
            abs_path: format!("/tmp/repo/{title}.png"),
            title: title.into(),
            bytes: 1,
            agent_id: None,
            task_id: None,
            caption: None,
            created_at: now_ms(),
        };
        let (a, b) = (artifact("a"), artifact("b"));
        for art in [&a, &b] {
            store.append_event(Some(&w.id), &row(EventKind::ArtifactAdded { artifact: art.clone() })).unwrap();
            store.append_event(Some(&w.id), &row(EventKind::TurnStarted { input: None })).unwrap();
        }
        let titles = |ws: Option<&str>, limit| store.artifacts(ws, limit).unwrap().into_iter().map(|x| x.title).collect::<Vec<_>>();
        assert_eq!(titles(Some(&w.id), 10), ["b", "a"]);
        assert_eq!(titles(None, 1), ["b"]);
        assert!(titles(Some("another-office"), 10).is_empty());
        assert_eq!(store.artifact(&a.id).unwrap().unwrap().title, "a");
        assert!(store.artifact("unknown").unwrap().is_none());
        assert!(store.memory_events(Some(&w.id), 10).unwrap().is_empty(), "artifacts are not memories");
    }

    #[test]
    fn reads_that_grow_with_history_use_an_index() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        // The read goes through `index`, with no table scan and (`sorted`) no sort afterwards.
        let check = |sql: &str, index: &str, sorted: bool| {
            let plan = query_plan(&conn, sql);
            let ok = plan.contains(index) && !plan.lines().any(|l| l.starts_with("SCAN")) && !(sorted && plan.contains("TEMP B-TREE"));
            assert!(ok, "{sql}\n{plan}");
        };
        check(EVENTS_OF_TYPE_SQL, "event_type", true);
        check(ARTIFACT_SQL, "event_artifact", false);
        check(TASK_EVENTS_SQL, "event_task", true);
        check(&latest_task_run_sql(), "run_task", true);
        check(CHAT_THREADS_SQL, "chat_thread", false);
        check(CHAT_TAIL_SQL, "chat_thread", true);
        // The open questions the office tabs poll (`blockers::list`).
        check("SELECT payload FROM event WHERE type = 'blocker_changed' AND (?1 IS NULL OR workspace_id = ?1) ORDER BY id", "event_type", true);
    }

    #[test]
    fn an_existing_database_gets_the_lookup_indexes() {
        // A database as the version before left it, with history in it.
        let mut conn = Connection::open_in_memory().unwrap();
        migrations().to_version(&mut conn, 4).unwrap();
        conn.execute_batch(
            r#"INSERT INTO event (ts, type, payload) VALUES
                 (1, 'artifact_added', '{"type":"artifact_added","artifact":{"id":"a1"}}'),
                 (2, 'turn_started', '{"type":"turn_started","input":null}');"#,
        )
        .unwrap();
        let store = Store::init(conn).unwrap();
        let conn = store.conn();
        let indexes: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('event_type', 'event_artifact', 'run_task')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(indexes, 3);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM event", [], |r| r.get::<_, i64>(0)).unwrap(), 2, "history is untouched");
        // A row written before the index existed is found through it.
        let found: String = conn.query_row(ARTIFACT_SQL, ["a1"], |r| r.get(0)).unwrap();
        assert!(found.contains("a1"));
    }

    /// A repo with two requests: "lead:x" (plan p1, task t1 assigned to `worker`) and "lead:y"
    /// (plan p2, task t2 assigned to `other`).
    struct Threads {
        store: Store,
        ws: Id,
    }

    impl Threads {
        fn new() -> Threads {
            let store = Store::open_in_memory().unwrap();
            let ws = ws(&store).id;
            for (plan, thread, task, assignee) in [("p1", "lead:x", "t1", "worker"), ("p2", "lead:y", "t2", "other")] {
                let conn = store.conn();
                conn.execute(
                    "INSERT INTO plan (id, workspace_id, status, json, created_at, updated_at) VALUES (?1, ?2, 'running', json_object('thread', ?3), 0, 0)",
                    params![plan, ws, thread],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO task (id, workspace_id, plan_id, status, json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'running', json_object('assignee', ?4), 0, 0)",
                    params![task, ws, plan, assignee],
                )
                .unwrap();
            }
            Threads { store, ws }
        }

        fn add(&self, agent: Option<&str>, task: Option<&str>, kind: EventKind) -> i64 {
            let row = EventRow { id: 0, ts: now_ms(), agent_id: agent.map(Into::into), task_id: task.map(Into::into), run_id: None, workspace_id: None, kind };
            self.store.append_event(Some(&self.ws), &row).unwrap()
        }

        fn work(&self, agent: &str) -> i64 {
            self.add(Some(agent), None, EventKind::StatusChanged { status: AgentStatus::Working })
        }

        /// A message, attributed as the core does it: to the agent who wrote it.
        fn say(&self, thread: &str, author: &str) -> i64 {
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: "hi".into(), attachment: None, ts: now_ms() };
            self.add((author != "user").then_some(author), None, EventKind::Chat { message })
        }

        fn page(&self, thread: &str, owner: Option<&str>, before: Option<i64>, limit: i64) -> Vec<i64> {
            self.store.thread_events(&self.ws, thread, owner, before, limit).unwrap().into_iter().map(|e| e.id).collect()
        }
    }

    #[test]
    fn a_thread_page_is_its_agent_its_messages_and_its_tasks() {
        let t = Threads::new();
        let approval = Approval {
            id: new_id(),
            agent_id: "worker".into(),
            run_id: "r".into(),
            task_id: Some("t1".into()),
            call_id: None,
            subject: ApprovalSubject::Command { command: "ls".into(), cwd: None },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        let verified = || EventKind::Verified { command: "cargo test".into(), ok: true, exit_code: Some(0), tail: String::new() };

        let lead_work = t.work("lead");
        let user_said = t.say("lead:x", "user");
        t.say("lead:y", "user"); // the other request's message
        let worker_asked = t.say("lead:x", "worker");
        t.work("worker"); // a worker's own work shows in its 1:1, not in the request
        let asked = t.add(Some("worker"), Some("t1"), EventKind::ApprovalRequested { approval: approval.clone() });
        let answer = EventKind::ApprovalResolved { approval_id: approval.id.clone(), status: ApprovalStatus::Approved, by: "user".into() };
        let answered = t.add(Some("worker"), Some("t1"), answer);
        let checked = t.add(None, Some("t1"), verified());
        t.add(None, Some("t2"), verified()); // the other request's task
        t.work("other-lead");
        let lead_said = t.say("lead:x", "lead");

        // Newest first: the Lead's own events, the thread's messages (the user's and a worker's
        // too), and its task's approval and check. Nothing of the other request.
        assert_eq!(t.page("lead:x", Some("lead"), None, 50), vec![lead_said, checked, answered, asked, worker_asked, user_said, lead_work]);
        // The request row says how the approval ended, also when its answer is on a newer page.
        let page = t.store.thread_events(&t.ws, "lead:x", Some("lead"), Some(answered), 50).unwrap();
        assert!(page.iter().any(|e| matches!(&e.kind, EventKind::ApprovalRequested { approval } if approval.status == ApprovalStatus::Approved)));

        // Pages follow the cursor: no row twice, none skipped.
        let first = t.page("lead:x", Some("lead"), None, 3);
        assert_eq!(first, vec![lead_said, checked, answered]);
        let second = t.page("lead:x", Some("lead"), first.last().copied(), 3);
        assert_eq!(second, vec![asked, worker_asked, user_said]);
        assert_eq!(t.page("lead:x", Some("lead"), second.last().copied(), 3), vec![lead_work]);
        assert!(t.page("lead:x", Some("lead"), Some(lead_work), 3).is_empty());

        // A request whose Lead has not started yet still has its messages and its tasks.
        assert_eq!(t.page("lead:x", None, None, 50), vec![lead_said, checked, answered, asked, worker_asked, user_said]);
        // A 1:1 thread: everything its agent did, and the results of the tasks assigned to it.
        assert_eq!(t.page("agent:worker", Some("worker"), None, 50).len(), 5, "its message, its work, the approval (asked, answered) and the check");
    }

    #[test]
    fn a_closed_requests_messages_stay_out_of_the_reused_thread() {
        let t = Threads::new();
        t.say("lead", "user");
        t.store.move_chats(&t.ws, "lead", "closed:lead:1").unwrap();
        let new = t.say("lead", "user");
        assert_eq!(t.page("lead", None, None, 50), vec![new]);
    }

    #[test]
    fn the_walk_for_a_requests_messages_ends_at_its_first_message() {
        let t = Threads::new();
        // Seen through a row that real data never has: the event of one of the thread's messages,
        // ten minutes older than every message of the thread. The walk ends before that row
        // (and so before every event of the repo's life from before this request).
        let message = ChatMessage { id: new_id(), thread: "lead:x".into(), author: "user".into(), text: "hi".into(), attachment: None, ts: now_ms() };
        let stray = EventRow { id: 0, ts: message.ts - 600_000, agent_id: None, task_id: None, run_id: None, workspace_id: None, kind: EventKind::Chat { message } };
        t.store.append_event(Some(&t.ws), &stray).unwrap();
        let said = t.say("lead:x", "user");
        assert_eq!(t.page("lead:x", None, None, 50), vec![said]);
        // The peek of a thread with nothing older (one row asked, no owner): nothing, and no further.
        assert!(t.page("lead:x", None, Some(said), 1).is_empty());
    }

    #[test]
    fn an_agents_page_walks_its_index() {
        let t = Threads::new();
        let ids: Vec<i64> = (0..5).map(|_| t.work("lead")).collect();
        t.work("worker");
        let page: Vec<i64> = t.store.events(Some(&t.ws), Some("lead"), Some(ids[4]), 2).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(page, vec![ids[3], ids[2]]);
        let conn = t.store.conn();
        let plan: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
                 WHERE agent_id = ?2 AND id < ?3 AND (?1 IS NULL OR workspace_id = ?1 OR workspace_id IS NULL)
                 ORDER BY id DESC LIMIT ?4",
            )
            .unwrap()
            .query_map(params![t.ws, "lead", i64::MAX, 2], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(plan.iter().any(|step| step.contains("event_agent")), "{plan:?}");
    }
}
