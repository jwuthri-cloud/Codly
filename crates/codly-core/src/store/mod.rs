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
//! ponytail: one `Mutex<Connection>` for the writer and the short reads instead of a batching
//! writer actor plus a read pool (CRITIQUE D3). Every call is a short local transaction;
//! move to an actor when profiles show lock contention. The reads that grow with the history
//! (pages of the event log) use a second, read-only connection: see `Store::read`.
//!
//! Copies of the database and the way back from one that cannot be used: `backup`.

pub mod backup;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use rusqlite_migration::{M, Migrations};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::model::*;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("schema.sql")), M::up(include_str!("m2_run_account.sql")), M::up(include_str!("m3_memory_data.sql")), M::up(include_str!("m4_workspace_closed.sql")), M::up(include_str!("m5_lookup_indexes.sql")), M::up(include_str!("m6_approval_rules.sql"))])
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

/// The database file in Codly's data folder.
pub const FILE: &str = "codly.sqlite";

pub struct Store {
    conn: Mutex<Connection>,
    /// A second, read-only connection to the same file (`read`). None for a database in
    /// memory, which has one connection.
    reader: Option<Mutex<Connection>>,
    /// The database file. None for a database in memory.
    path: Option<PathBuf>,
}

impl Store {
    /// Open (or create) the database, set the pragmas and run pending migrations. A file with
    /// data is copied before a migration changes it. A file that cannot be used fails with a
    /// `backup::StoreError` and is left as it is.
    pub fn open(path: &Path) -> Result<Store> {
        Self::open_with(path, &migrations())
    }

    /// `open` with the migrations given (tests: an update that fails).
    fn open_with(path: &Path, migrations: &Migrations<'_>) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::pragmas(&conn).map_err(backup::read_error)?;
        backup::update(&mut conn, path, migrations)?;
        let reader = Connection::open(path).with_context(|| format!("open {} to read", path.display()))?;
        reader.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA query_only = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;",
        )?;
        Ok(Store { conn: Mutex::new(conn), reader: Some(Mutex::new(reader)), path: Some(path.to_path_buf()) })
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Store> {
        Self::pragmas(&conn)?;
        migrations().to_latest(&mut conn).context("migrate codly.sqlite")?;
        Ok(Store { conn: Mutex::new(conn), reader: None, path: None })
    }

    fn pragmas(conn: &Connection) -> rusqlite::Result<()> {
        // tauri-stack §5.4. journal_mode must be set outside a transaction.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;",
        )
    }

    /// The connection, recovering from poisoning: a panic in one caller (e.g. a module
    /// that is still `todo!()`) must not take the whole store down with it.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The connection for the reads that grow with the history: pages of the event log. On a
    /// file it is a second connection that cannot write. Under WAL it reads while a write is
    /// in progress, so a long page does not make the event ingest wait for the store, and it
    /// sees every write that returned before the read began.
    fn read(&self) -> MutexGuard<'_, Connection> {
        match &self.reader {
            Some(reader) => reader.lock().unwrap_or_else(|e| e.into_inner()),
            None => self.conn(),
        }
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
        let conn = self.read();
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
        let conn = self.read();
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
            let conn = self.read();
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
        let conn = self.read();
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
        let conn = self.read();
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

    /// Delete `key` while it still has `value`, in one statement: a reader that found this
    /// value out of date does not remove a newer one written since. True = deleted.
    pub fn kv_delete_if(&self, key: &str, value: &str) -> Result<bool> {
        Ok(self.conn().execute("DELETE FROM kv WHERE key = ?1 AND value = ?2", params![key, value])? > 0)
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

    /// The text of the latest message `author` wrote in `thread`.
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

    /// The user's last own words in `thread` (None: they said nothing there), and the messages
    /// that came from outside Codly (a webhook, Slack: an `inbound` attachment) after them,
    /// newest first: what a plan made now answers. The user's own words are a message they
    /// wrote, or a schedule of theirs that fired (`triggers::is_schedule`). The read stops at
    /// them, so its cost follows what came in since, not the length of the thread.
    pub fn since_user(&self, workspace_id: Option<&str>, thread: &str) -> Result<(Option<ChatMessage>, Vec<ChatMessage>)> {
        let conn = self.conn();
        let mut newest_first = conn.prepare_cached("SELECT json FROM chat WHERE workspace_id IS ?1 AND thread = ?2 ORDER BY ts DESC, rowid DESC")?;
        let mut rows = newest_first.query(params![workspace_id, thread])?;
        let mut inbound = vec![];
        while let Some(row) = rows.next()? {
            let Ok(m) = serde_json::from_str::<ChatMessage>(&row.get::<_, String>(0)?) else { continue };
            if m.author == "user" || crate::triggers::is_schedule(m.attachment.as_ref()) {
                return Ok((Some(m), inbound));
            }
            if crate::inbound::is_inbound(m.attachment.as_ref()) {
                inbound.push(m);
            }
        }
        Ok((None, inbound))
    }

    /// The messages of `thread` that came from outside Codly after the user's last own words
    /// there, newest first (`since_user`).
    pub fn inbound_since_user(&self, workspace_id: Option<&str>, thread: &str) -> Result<Vec<ChatMessage>> {
        Ok(self.since_user(workspace_id, thread)?.1)
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

    /// The last usage report of each run that worked on a task (its worker and reviewer runs).
    /// A run's counts are totals, so its last report is what the run used.
    pub fn task_run_usage(&self, task_id: &str) -> Result<Vec<Usage>> {
        let conn = self.conn();
        // With a lone MAX(id), SQLite takes `payload` from the row that holds the maximum. The
        // task's own index is named: the other candidate, `event_type`, walks every usage report.
        let mut st = conn.prepare_cached(
            "SELECT payload, MAX(id) FROM event INDEXED BY event_task
             WHERE task_id = ?1 AND type = 'usage_updated' GROUP BY run_id",
        )?;
        let rows = st.query_map([task_id], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for payload in rows {
            if let Ok(EventKind::UsageUpdated { usage }) = serde_json::from_str::<EventKind>(&payload?) {
                out.push(usage);
            }
        }
        Ok(out)
    }

    /// What an agent's runs on one provider session cost together: the cost in the last usage
    /// report of each run (a run's cost starts from zero), added up.
    pub fn session_cost(&self, agent_id: &str, session_id: &str) -> Result<f64> {
        let conn = self.conn();
        // The agent's own index is named, as in `task_run_usage`: one walk over its events.
        let mut st = conn.prepare_cached(
            "SELECT payload, MAX(id) FROM event INDEXED BY event_agent
             WHERE agent_id = ?1 AND type = 'usage_updated'
               AND run_id IN (SELECT id FROM run WHERE agent_id = ?1 AND session_id = ?2)
             GROUP BY run_id",
        )?;
        let rows = st.query_map(params![agent_id, session_id], |r| r.get::<_, String>(0))?;
        let mut total = 0.0;
        for payload in rows {
            if let Ok(EventKind::UsageUpdated { usage }) = serde_json::from_str::<EventKind>(&payload?) {
                total += usage.cost_usd.unwrap_or(0.0);
            }
        }
        Ok(total)
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

    // ───────────── saved approval rules ("always allow in this repo") ─────────────

    /// A repo's saved rules, oldest first.
    pub fn approval_rules(&self, workspace_id: &str) -> Result<Vec<ApprovalRule>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!("SELECT {RULE_COLUMNS} FROM approval_rule WHERE workspace_id = ?1 ORDER BY rowid"))?;
        let rows = st.query_map([workspace_id], approval_rule_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Save a rule. Saving the same rule again returns the one that is there.
    pub fn add_approval_rule(&self, workspace_id: &str, kind: ApprovalRuleKind, command: &str) -> Result<ApprovalRule> {
        self.tx(|tx| {
            tx.execute(
                "INSERT INTO approval_rule (id, workspace_id, kind, command, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(workspace_id, kind, command) DO NOTHING",
                params![new_id(), workspace_id, enum_str(&kind), command, now_ms()],
            )?;
            Ok(tx.query_row(
                &format!("SELECT {RULE_COLUMNS} FROM approval_rule WHERE workspace_id = ?1 AND kind = ?2 AND command = ?3"),
                params![workspace_id, enum_str(&kind), command],
                approval_rule_from_row,
            )?)
        })
    }

    /// Remove a rule; returns it, or None when there is no such rule.
    pub fn remove_approval_rule(&self, id: &str) -> Result<Option<ApprovalRule>> {
        self.tx(|tx| {
            let rule = tx.query_row(&format!("SELECT {RULE_COLUMNS} FROM approval_rule WHERE id = ?1"), [id], approval_rule_from_row).optional()?;
            tx.execute("DELETE FROM approval_rule WHERE id = ?1", [id])?;
            Ok(rule)
        })
    }

    /// Record the rules a request can be saved as (the broker does, when it asks you).
    pub fn set_rule_offer(&self, approval_id: &str, offer: &ApprovalRuleOffer) -> Result<()> {
        self.conn().execute("UPDATE approval SET rule_offer = ?2 WHERE id = ?1", params![approval_id, serde_json::to_string(offer)?])?;
        Ok(())
    }

    /// The rules a request can be saved as: none once it is answered, or when Codly always
    /// asks about it.
    pub fn rule_offer(&self, approval_id: &str) -> Result<ApprovalRuleOffer> {
        let saved: Option<Option<String>> = self
            .conn()
            .query_row("SELECT rule_offer FROM approval WHERE id = ?1 AND status = 'pending'", [approval_id], |r| r.get(0))
            .optional()?;
        Ok(saved.flatten().map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default())
    }

    /// The requests of a repo that wait for you and can be saved as a rule, oldest first.
    pub fn pending_rule_offers(&self, workspace_id: &str) -> Result<Vec<(Id, ApprovalRuleOffer)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, rule_offer FROM approval WHERE status = 'pending' AND workspace_id = ?1 AND rule_offer IS NOT NULL ORDER BY created_at",
        )?;
        let rows = st.query_map([workspace_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = vec![];
        for row in rows {
            let (id, offer) = row?;
            out.push((id, serde_json::from_str(&offer)?));
        }
        Ok(out)
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

    /// The requests closed in a workspace, newest first, at most `limit`: each archive thread
    /// with the start of the user's first message and how many messages it holds.
    pub fn closed_requests(&self, workspace_id: &str, limit: usize) -> Result<Vec<ClosedRequest>> {
        let conn = self.conn();
        let mut threads = conn.prepare_cached(CLOSED_THREADS_SQL)?;
        // An archive's name ends with the time it was closed ("closed:<thread>:<ms>").
        let mut found: Vec<(i64, String)> = vec![];
        for thread in threads.query_map(params![workspace_id], |r| r.get::<_, String>(0))? {
            let thread = thread?;
            let closed_at = thread.rsplit(':').next().and_then(|ms| ms.parse::<i64>().ok());
            if let Some(closed_at) = closed_at {
                found.push((closed_at, thread));
            }
        }
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.truncate(limit);
        let mut first = conn.prepare_cached(CLOSED_TITLE_SQL)?;
        let mut count = conn.prepare_cached(CHAT_COUNT_SQL)?;
        let mut closed = Vec::with_capacity(found.len());
        for (closed_at, thread) in found {
            let title: Option<String> = first.query_row(params![workspace_id, thread], |r| r.get(0)).optional()?;
            let messages = count.query_row(params![workspace_id, thread], |r| r.get(0))?;
            closed.push(ClosedRequest { thread, title: title.unwrap_or_default(), closed_at, messages });
        }
        Ok(closed)
    }

    /// One closed request to read again: its newest `limit` messages in the order they were
    /// written, how many it has in all, and the names of the agents who wrote them.
    pub fn closed_transcript(&self, workspace_id: &str, thread: &str, limit: i64) -> Result<ClosedTranscript> {
        let (messages, total) = {
            let conn = self.conn();
            let mut tail = conn.prepare_cached(CHAT_TAIL_SQL)?;
            let mut rows = tail.query_map(params![workspace_id, thread, limit], |r| r.get::<_, String>(2))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse();
            let messages: Vec<ChatMessage> = decode_all(rows.into_iter().map(Ok))?;
            let total: i64 = conn.prepare_cached(CHAT_COUNT_SQL)?.query_row(params![workspace_id, thread], |r| r.get(0))?;
            (messages, total)
        };
        let mut authors = std::collections::BTreeMap::new();
        for m in &messages {
            if m.author == "user" || m.author == "codly" || authors.contains_key(&m.author) {
                continue;
            }
            if let Some(agent) = self.agent(&m.author)? {
                authors.insert(m.author.clone(), agent.name);
            }
        }
        Ok(ClosedTranscript { messages, total, authors })
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

/// How many closed requests are listed, and how many of one's messages are read back.
pub const CLOSED_LISTED: usize = 50;
pub const CLOSED_MESSAGES: i64 = 500;

/// Archive threads of a workspace's closed requests: one range of the `chat_thread` index
/// (';' is the character after ':').
const CLOSED_THREADS_SQL: &str = "SELECT DISTINCT thread FROM chat WHERE workspace_id IS ?1 AND thread >= 'closed:' AND thread < 'closed;'";

/// The start of the user's first message in one thread.
const CLOSED_TITLE_SQL: &str = "SELECT substr(json_extract(json, '$.text'), 1, 300) FROM chat
     WHERE workspace_id IS ?1 AND thread = ?2 AND json_extract(json, '$.author') = 'user' ORDER BY ts, rowid LIMIT 1";

/// How many messages one thread holds.
const CHAT_COUNT_SQL: &str = "SELECT COUNT(*) FROM chat WHERE workspace_id IS ?1 AND thread = ?2";

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

const RULE_COLUMNS: &str = "id, workspace_id, kind, command, created_at";

/// An `approval_rule` row, read as `RULE_COLUMNS`.
fn approval_rule_from_row(r: &rusqlite::Row) -> rusqlite::Result<ApprovalRule> {
    let kind: String = r.get(2)?;
    let kind = parse_enum(&kind).ok_or_else(|| rusqlite::Error::InvalidColumnType(2, format!("approval rule kind {kind}"), rusqlite::types::Type::Text))?;
    Ok(ApprovalRule { id: r.get(0)?, workspace_id: r.get(1)?, kind, command: r.get(3)?, created_at: r.get(4)? })
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
            weak_checks: vec![],
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
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: format!("m{ts}"), attachment, ts, item_id: None };
            store.append_event(None, &row(EventKind::Chat { message })).unwrap();
        };
        let since = |thread: &str| -> Vec<String> {
            let found = store.inbound_since_user(None, thread).unwrap();
            found.iter().map(|m| m.attachment.as_ref().unwrap()["inbound"]["source"].as_str().unwrap().to_string()).collect()
        };
        // The text of your last own words in a thread.
        let own = |thread: &str| store.since_user(None, thread).unwrap().0.map(|m| m.text);

        say("lead", "codly", inbound("old"));
        assert_eq!((since("lead"), own("lead")), (vec!["old".to_string()], None), "you said nothing there yet");
        say("lead", "user", None);
        assert!(since("lead").is_empty(), "your message came after it");
        assert_eq!(own("lead").as_deref(), Some("m2"));
        say("lead", "codly", inbound("a"));
        say("lead", "codly", Some(serde_json::json!({ "plan_id": "p1" })));
        say("lead:2", "user", None);
        say("lead", "codly", inbound("b"));
        assert_eq!(since("lead"), vec!["b", "a"], "newest first, back to your message");
        assert_eq!(own("lead").as_deref(), Some("m2"), "what came in since does not replace your words");
        assert!(since("lead:2").is_empty());

        // Your schedule fires: what came in before it no longer counts, what comes in after does.
        // Its line is your last own words now, not the message you typed before it.
        say("lead", "codly", schedule);
        assert!(since("lead").is_empty());
        assert_eq!(own("lead").as_deref(), Some("m7"));
        say("lead", "codly", inbound("c"));
        assert_eq!(since("lead"), vec!["c"]);
        assert_eq!(own("lead").as_deref(), Some("m7"));
    }

    #[test]
    fn approval_rules_are_kept_per_repo_and_an_offer_lasts_while_the_request_waits() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let rule = store.add_approval_rule(&w.id, ApprovalRuleKind::Prefix, "cargo test").unwrap();
        assert_eq!((rule.kind, rule.command.as_str(), rule.workspace_id.as_str()), (ApprovalRuleKind::Prefix, "cargo test", w.id.as_str()));
        // The same rule again is the one that is there; the exact command is another rule.
        assert_eq!(store.add_approval_rule(&w.id, ApprovalRuleKind::Prefix, "cargo test").unwrap().id, rule.id);
        let exact = store.add_approval_rule(&w.id, ApprovalRuleKind::Exact, "cargo test").unwrap();
        assert_eq!(store.approval_rules(&w.id).unwrap(), vec![rule.clone(), exact.clone()]);
        assert!(store.approval_rules("another-repo").unwrap().is_empty());
        assert_eq!(store.remove_approval_rule(&rule.id).unwrap(), Some(rule.clone()));
        assert_eq!(store.remove_approval_rule(&rule.id).unwrap(), None);
        assert_eq!(store.approval_rules(&w.id).unwrap(), vec![exact]);

        let a = Approval {
            id: new_id(),
            agent_id: "a".into(),
            run_id: "r".into(),
            task_id: None,
            call_id: None,
            subject: ApprovalSubject::Command { command: "pnpm install".into(), cwd: None },
            reason: None,
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            risks: vec![],
            status: ApprovalStatus::Pending,
            decided_by: None,
            created_at: now_ms(),
        };
        store.append_event(Some(&w.id), &row(EventKind::ApprovalRequested { approval: a.clone() })).unwrap();
        assert!(store.rule_offer(&a.id).unwrap().is_empty(), "nothing recorded yet");
        assert!(store.pending_rule_offers(&w.id).unwrap().is_empty());
        let offer = ApprovalRuleOffer { exact: Some("pnpm install".into()), prefix: Some("pnpm install".into()) };
        store.set_rule_offer(&a.id, &offer).unwrap();
        assert_eq!(store.rule_offer(&a.id).unwrap(), offer);
        assert_eq!(store.pending_rule_offers(&w.id).unwrap(), vec![(a.id.clone(), offer)]);
        let answered = EventKind::ApprovalResolved { approval_id: a.id.clone(), status: ApprovalStatus::Approved, by: "user".into() };
        store.append_event(Some(&w.id), &row(answered)).unwrap();
        assert!(store.rule_offer(&a.id).unwrap().is_empty(), "an answered request offers nothing");
        assert!(store.pending_rule_offers(&w.id).unwrap().is_empty());
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
                item_id: None,
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
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: "user".into(), text: text.into(), attachment: None, ts, item_id: None };
            store.append_event(Some(&w.id), &row(EventKind::Chat { message })).unwrap();
        }
        let texts: Vec<_> = store.recent_chats(Some(&here.id), 10).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn closed_requests_are_listed_newest_first_and_read_back_in_order() {
        let store = Store::open_in_memory().unwrap();
        let here = ws(&store);
        let there = Workspace { id: new_id(), repo_path: "/tmp/other".into(), ..here.clone() };
        store.upsert_workspace(&there).unwrap();
        // Its Lead, archived when the request closed: the store still knows its name.
        let lead: Agent = serde_json::from_value(serde_json::json!({
            "id": "l1", "name": "Mira", "provider": "claude", "model": null, "role": "lead", "status": "offline",
            "activity": null, "activity_label": null, "task_id": null, "run_id": null, "session_id": null,
            "worktree": null, "billing": "subscription", "color": 1, "look_seed": "l1", "thread": "closed:lead:100",
            "archived": true, "usage": Usage::default(),
            "created_at": 1, "updated_at": 1
        }))
        .unwrap();
        store.append_event(Some(&here.id), &row(EventKind::AgentChanged { agent: lead.clone() })).unwrap();
        // (office, thread, author, text, ts)
        for (w, thread, author, text, ts) in [
            (&here, "lead", "user", "still open", 1),
            (&here, "closed:lead:100", lead.id.as_str(), "On it.", 2),
            (&here, "closed:lead:100", "user", "Fix the login", 3),
            (&here, "closed:lead:100", "codly", "Plan cancelled.", 4),
            (&here, "closed:lead:b:900", "user", "Add a bye file", 5),
            (&here, "closed:lead:c:500", "gone", "Nobody asked.", 6),
            (&there, "closed:lead:999", "user", "elsewhere", 7),
        ] {
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: text.into(), attachment: None, ts, item_id: None };
            store.append_event(Some(&w.id), &row(EventKind::Chat { message })).unwrap();
        }

        // Newest closed first; the title is the user's first message, not the first message.
        let closed = store.closed_requests(&here.id, 10).unwrap();
        let listed: Vec<_> = closed.iter().map(|c| (c.thread.as_str(), c.title.as_str(), c.closed_at, c.messages)).collect();
        assert_eq!(listed, vec![("closed:lead:b:900", "Add a bye file", 900, 1), ("closed:lead:c:500", "", 500, 1), ("closed:lead:100", "Fix the login", 100, 3)]);
        assert_eq!(store.closed_requests(&here.id, 1).unwrap().len(), 1, "at most `limit`");
        assert_eq!(store.closed_requests(&there.id, 10).unwrap().len(), 1, "each office has its own");

        // Read back in the order written, with the names of the agents Codly still knows.
        let t = store.closed_transcript(&here.id, "closed:lead:100", 10).unwrap();
        assert_eq!(t.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["On it.", "Fix the login", "Plan cancelled."]);
        assert_eq!(t.total, 3);
        assert_eq!(t.authors.get(&lead.id).map(String::as_str), Some("Mira"));
        assert_eq!(t.authors.len(), 1, "you and Codly are not agents");
        // A long one keeps its newest messages and says how many there are.
        let t = store.closed_transcript(&here.id, "closed:lead:100", 2).unwrap();
        assert_eq!((t.messages[0].text.as_str(), t.messages.len(), t.total), ("Fix the login", 2, 3));
        assert!(store.closed_transcript(&here.id, "closed:lead:c:500", 10).unwrap().authors.is_empty(), "an agent Codly no longer has gets no name");
        assert_eq!(store.closed_transcript(&there.id, "closed:lead:100", 10).unwrap().total, 0, "another office's is not read");
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
        check(CLOSED_THREADS_SQL, "chat_thread", false);
        check(CLOSED_TITLE_SQL, "chat_thread", true);
        check(CHAT_COUNT_SQL, "chat_thread", false);
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
            let message = ChatMessage { id: new_id(), thread: thread.into(), author: author.into(), text: "hi".into(), attachment: None, ts: now_ms(), item_id: None };
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
        let message = ChatMessage { id: new_id(), thread: "lead:x".into(), author: "user".into(), text: "hi".into(), attachment: None, ts: now_ms(), item_id: None };
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

    #[test]
    fn a_file_database_reads_its_history_while_a_write_is_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join(FILE)).unwrap();
        let w = ws(&store);
        let event = EventRow { agent_id: Some("worker".into()), ..row(EventKind::StatusChanged { status: AgentStatus::Working }) };
        let id = store.append_event(Some(&w.id), &event).unwrap();
        // A write that is not committed yet, and that holds the store's own connection.
        let writer = store.conn();
        writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO kv (key, value) VALUES ('k', 'v');").unwrap();
        // The log is read all the same, with every write that returned before.
        let page: Vec<i64> = store.events(Some(&w.id), None, None, 10).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(page, [id]);
        assert_eq!(store.work_since(&w.id, 0).unwrap().len(), 0);
        writer.execute_batch("COMMIT;").unwrap();
        drop(writer);
        assert!(store.read().execute("DELETE FROM event", []).is_err(), "the reading connection cannot write");
        assert_eq!(store.last_event_id().unwrap(), id);
        // A database in memory has one connection for both.
        assert!(Store::open_in_memory().unwrap().reader.is_none());
    }

    #[test]
    fn a_tasks_usage_is_the_last_report_of_each_of_its_runs() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let report = |task: &str, run: &str, input: i64| {
            let usage = Usage { input_tokens: input, cost_usd: Some(input as f64 / 1000.0), ..Default::default() };
            let ev = EventRow { task_id: Some(task.into()), run_id: Some(run.into()), ..row(EventKind::UsageUpdated { usage }) };
            store.append_event(Some(&w.id), &ev).unwrap();
        };
        report("t1", "worker-1", 100);
        report("t1", "worker-1", 400); // the same run, later: its counts are totals
        report("t2", "other", 9_000);
        report("t1", "reviewer", 50);
        report("t1", "worker-2", 30); // a retry: a new run starts from zero
        let mut used: Vec<(i64, Option<f64>)> = store.task_run_usage("t1").unwrap().into_iter().map(|u| (u.input_tokens, u.cost_usd)).collect();
        used.sort_by_key(|u| u.0);
        assert_eq!(used, [(30, Some(0.03)), (50, Some(0.05)), (400, Some(0.4))]);
        assert!(store.task_run_usage("no-such-task").unwrap().is_empty());
    }

    #[test]
    fn a_sessions_cost_is_the_total_of_its_runs() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        // Runs only: their agents are not rows here.
        store.conn().execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        let run = |id: &str, agent: &str, session: &str| {
            let run = RunRow {
                id: id.into(),
                agent_id: agent.into(),
                task_id: None,
                role: Role::Lead,
                provider: Provider::Opencode,
                model: None,
                cwd: "/w".into(),
                session_id: Some(session.into()),
                account: None,
                status: "completed".into(),
                raw_log: String::new(),
                report: None,
                started_at: now_ms(),
                ended_at: None,
            };
            store.insert_run(&run).unwrap();
        };
        let report = |agent: &str, run: &str, cost_usd: Option<f64>| {
            let usage = Usage { cost_usd, ..Default::default() };
            let ev = EventRow { agent_id: Some(agent.into()), run_id: Some(run.into()), ..row(EventKind::UsageUpdated { usage }) };
            store.append_event(Some(&w.id), &ev).unwrap();
        };
        run("r1", "ana", "ses-1");
        run("r2", "ana", "ses-1");
        run("r3", "ana", "ses-2");
        run("r4", "ana", "ses-1");
        run("r5", "ben", "ses-9");
        report("ana", "r1", Some(0.125));
        report("ana", "r1", Some(0.25)); // the same run, later: its cost is a total
        report("ana", "r2", Some(0.5)); // the session resumed: a new run starts from zero
        report("ana", "r3", Some(8.0)); // another session
        report("ana", "r4", None); // a run with no cost reported adds none
        report("ben", "r5", Some(16.0)); // another agent
        assert_eq!(store.session_cost("ana", "ses-1").unwrap(), 0.75);
        assert_eq!(store.session_cost("ana", "ses-2").unwrap(), 8.0);
        assert_eq!(store.session_cost("ana", "no-such-session").unwrap(), 0.0);
    }

    #[test]
    fn a_value_is_deleted_only_while_it_is_the_one_that_was_read() {
        let store = Store::open_in_memory().unwrap();
        store.kv_set("k", "old").unwrap();
        let read = store.kv_get("k").unwrap().unwrap();
        // Another writer stores a newer value before the reader deletes what it read.
        store.kv_set("k", "new").unwrap();
        assert!(!store.kv_delete_if("k", &read).unwrap());
        assert_eq!(store.kv_get("k").unwrap().as_deref(), Some("new"));
        assert!(store.kv_delete_if("k", "new").unwrap());
        assert_eq!(store.kv_get("k").unwrap(), None);
        assert!(!store.kv_delete_if("k", "new").unwrap(), "nothing is left to delete");
    }
}
