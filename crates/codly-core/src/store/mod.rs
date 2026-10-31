//! SQLite store (rusqlite, bundled, WAL). Schema in schema.sql (migration 1); later
//! migrations are appended to `migrations()` as new files, never edits of an applied one.
//!
//! The `event` table is the source of truth. `insert_event` appends a row and updates the
//! projection it implies (agent, plan, task, chat, approval, decision) in the same
//! transaction, so a crash can never leave a projection ahead of or behind its log.
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
    Migrations::new(vec![M::up(include_str!("schema.sql")), M::up(include_str!("m2_run_account.sql")), M::up(include_str!("m3_memory_data.sql"))])
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
        let mut st = conn.prepare_cached(
            "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event
             WHERE (?1 IS NULL OR workspace_id = ?1 OR workspace_id IS NULL)
               AND (?2 IS NULL OR agent_id = ?2)
               AND (?3 IS NULL OR id < ?3)
             ORDER BY id DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![workspace_id, agent_id, before, limit.clamp(1, 5000)], event_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
        let mut st = conn.prepare_cached(
            "SELECT payload FROM event WHERE type = 'artifact_added' AND (?1 IS NULL OR workspace_id = ?1)
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![workspace_id, limit], |r| r.get::<_, String>(0))?;
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
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM event WHERE type = 'artifact_added' AND json_extract(payload, '$.artifact.id') = ?1 LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(payload.and_then(|p| match serde_json::from_str::<EventKind>(&p) {
            Ok(EventKind::ArtifactAdded { artifact }) => Some(artifact),
            _ => None,
        }))
    }

    /// Memories announced by `memory_added` events, newest first. A read path for the
    /// blackboard that does not depend on the memory module's FTS tables.
    pub fn memory_events(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<Memory>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT payload FROM event WHERE type = 'memory_added' AND (?1 IS NULL OR workspace_id = ?1)
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![workspace_id, limit], |r| r.get::<_, String>(0))?;
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
               opened_at = excluded.opened_at",
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

    pub fn recent_workspaces(&self, limit: i64) -> Result<Vec<RecentWorkspace>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT repo_path, name, opened_at FROM workspace ORDER BY opened_at DESC LIMIT ?1")?;
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

    /// Every (key, value) whose key starts with `prefix` ("decor:" for every repo's furniture).
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

    /// Settings (defaults when unset). A corrupt value falls back to defaults rather than
    /// failing startup: defaults are the safe configuration.
    pub fn settings(&self) -> Result<Settings> {
        Ok(match self.kv_get("settings")? {
            Some(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
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
        let mut st = conn.prepare_cached(
            "SELECT id, ts, agent_id, task_id, run_id, type, payload FROM event WHERE task_id = ?1 AND type = ?2 ORDER BY id",
        )?;
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
        Ok(conn
            .query_row(
                &format!("{RUN_COLS} WHERE task_id = ?1 AND (?2 IS NULL OR role = ?2) ORDER BY started_at DESC, rowid DESC LIMIT 1"),
                params![task_id, role],
                run_from_row,
            )
            .optional()?)
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
    /// chronological order.
    pub fn recent_chats(&self, workspace_id: Option<&str>, per_thread: i64) -> Result<Vec<ChatMessage>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT json FROM (
               SELECT json, ts, ROW_NUMBER() OVER (PARTITION BY thread ORDER BY ts DESC, rowid DESC) AS rn
               FROM chat WHERE workspace_id IS ?1 AND thread NOT LIKE 'closed:%')
             WHERE rn <= ?2 ORDER BY ts, rn DESC",
        )?;
        let rows = st.query_map(params![workspace_id, per_thread], |r| r.get::<_, String>(0))?;
        decode_all(rows)
    }

    /// Record how a decision turned out (calibration log, agents-backend §5.6).
    pub fn set_decision_outcome(&self, id: &str, outcome: &str) -> Result<()> {
        self.conn().execute("UPDATE decision SET outcome = ?2 WHERE id = ?1", params![id, outcome])?;
        Ok(())
    }

    /// (type, summary, outcome) of a task's decisions, oldest first.
    pub fn task_decisions(&self, task_id: &str) -> Result<Vec<(String, String, Option<String>)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT type, summary, outcome FROM decision WHERE task_id = ?1 ORDER BY created_at, rowid")?;
        let rows = st.query_map([task_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
            let status = enum_str(status);
            db.prepare_cached(
                "UPDATE approval SET status = ?2, decided_at = ?3, json = json_set(json, '$.status', ?2, '$.decided_by', ?4)
                 WHERE id = ?1",
            )?
            .execute(params![approval_id, status, row.ts, by])?;
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

fn event_from_row(r: &rusqlite::Row) -> rusqlite::Result<EventRow> {
    let ty: String = r.get(5)?;
    let payload: String = r.get(6)?;
    // A row written by an older/newer schema must not break the timeline (G12).
    let kind = serde_json::from_str(&payload).unwrap_or(EventKind::Unknown { raw_type: ty });
    Ok(EventRow { id: r.get(0)?, ts: r.get(1)?, agent_id: r.get(2)?, task_id: r.get(3)?, run_id: r.get(4)?, kind })
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

#[cfg(test)]
mod tests {
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
        EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind }
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
    fn approvals_resolve_in_place() {
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
}
