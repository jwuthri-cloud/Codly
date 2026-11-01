//! Core glue for housekeeping (policy and catalog logic: `crate::housekeeping`):
//! - board archiving: the policy lives in `kv`, `board_sweep` judges the open workspace's
//!   tasks and pending questions and copies archived tasks to the archive file, and a slow
//!   background loop runs it;
//! - default MCP servers: the switches live in `kv`, keys in the Keychain, and
//!   `prepare_default_mcp` writes the enabled servers into a run's directory at spawn.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::bail;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{Core, Inner};
use crate::housekeeping::archive::{self, ArchiveFile, ArchivePolicy, ArchivedItem, BoardFlag, BoardItem, ItemKind, Verdict};
use crate::housekeeping::connections::{self, Connection, KeychainStore, McpPrefs, SecretStore};
use crate::model::*;

const POLICY_KEY: &str = "board_archive_policy";
const MCP_KEY: &str = "mcp_defaults";

/// How often the background sweep runs.
const SWEEP_EVERY: Duration = Duration::from_secs(30 * 60);

/// What a sweep found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SweepReport {
    /// Items that want your attention: untouched tasks, questions that waited too long.
    pub flagged: Vec<BoardFlag>,
    /// Every task id on the archive (hide these from the board).
    pub archived_ids: Vec<String>,
    /// Archived by this sweep.
    pub newly_archived: i64,
}

/// A key for one of a connection's slots.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct ConnectionSecret {
    pub env: String,
    /// None = remove the stored key.
    pub value: Option<String>,
}

impl Inner {
    fn board_policy(&self) -> ArchivePolicy {
        let stored = self.store.kv_get(POLICY_KEY).ok().flatten().and_then(|s| serde_json::from_str::<ArchivePolicy>(&s).ok());
        stored.unwrap_or_default().normalized()
    }

    fn archive_file(&self, ws: &Workspace) -> ArchiveFile {
        ArchiveFile::new(self.data("archive").join(format!("{}.jsonl", ws.id)))
    }

    /// Judge the board now (`now` is a parameter so tests control the clock).
    pub(super) fn board_sweep_at(&self, now: i64) -> anyhow::Result<SweepReport> {
        self.sweep_office(&self.require_ws()?, now)
    }

    fn sweep_office(&self, ws: &Workspace, now: i64) -> anyhow::Result<SweepReport> {
        let policy = self.board_policy();
        let file = self.archive_file(ws);
        let mut report = SweepReport::default();
        let mut to_archive = vec![];
        for t in self.store.tasks(&ws.id)? {
            match archive::judge(&archive::task_item(&t), now, &policy) {
                Verdict::Keep => {}
                Verdict::Flag(reason) => report.flagged.push(BoardFlag {
                    id: t.id.clone(),
                    kind: ItemKind::Task,
                    reason,
                    days: archive::idle_days(t.updated_at, now),
                    title: t.title.clone(),
                }),
                Verdict::Archive(reason) => to_archive.push(archive::archive_task(&t, reason, now)),
            }
        }
        for a in self.store.pending_approvals(Some(&ws.id))? {
            let ApprovalSubject::Question { .. } = a.subject else { continue };
            let item = BoardItem { id: a.id.clone(), kind: ItemKind::Question, finished: false, live: false, touched_at: a.created_at };
            if let Verdict::Flag(reason) = archive::judge(&item, now, &policy) {
                let title = a.reason.clone().unwrap_or_else(|| "An agent is waiting for your answer".into());
                report.flagged.push(BoardFlag { id: a.id.clone(), kind: ItemKind::Question, reason, days: archive::idle_days(a.created_at, now), title });
            }
        }
        report.newly_archived = file.append(&to_archive)? as i64;
        report.archived_ids = file.load().into_iter().filter(|i| i.kind == ItemKind::Task).map(|i| i.id).collect();
        Ok(report)
    }

    fn mcp_prefs(&self) -> McpPrefs {
        self.store.kv_get(MCP_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Called as a run is prepared: write the enabled default servers beside its raw log. The
    /// adapters merge them next to the `codly` server. Never fails the spawn.
    pub(super) fn prepare_default_mcp(&self, raw_log: &Path, cwd: &Path) {
        let extra = connections::resolve(&self.mcp_prefs(), &KeychainStore, cwd);
        if let Err(e) = connections::write_sidecar(raw_log, &extra) {
            tracing::warn!("could not write the default MCP servers: {e}");
        }
    }
}

/// Sweep the board once soon after start, then every half hour.
pub(super) async fn sweeper(inner: Arc<Inner>) {
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        if inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        // Every office, open or not: one you left keeps its board tidy too.
        for ws in inner.store.offices().unwrap_or_default() {
            let Ok(Some(ws)) = inner.store.workspace(&ws.id) else { continue };
            if Path::new(&ws.repo_path).is_dir()
                && let Err(e) = inner.sweep_office(&ws, now_ms())
            {
                tracing::warn!("board sweep of {} failed: {e}", ws.name);
            }
        }
        tokio::time::sleep(SWEEP_EVERY).await;
    }
}

impl Core {
    /// The day thresholds of the board's archiving (defaults until you change them).
    pub async fn board_policy(&self, set: Option<ArchivePolicy>) -> anyhow::Result<ArchivePolicy> {
        if let Some(p) = set {
            let p = p.normalized();
            self.inner.store.kv_set(POLICY_KEY, &serde_json::to_string(&p)?)?;
        }
        Ok(self.inner.board_policy())
    }

    /// Archive what is due, report what is flagged.
    pub async fn board_sweep(&self) -> anyhow::Result<SweepReport> {
        self.inner.board_sweep_at(now_ms())
    }

    /// `board_sweep` with the clock given (tests).
    pub async fn board_sweep_at(&self, now: i64) -> anyhow::Result<SweepReport> {
        self.inner.board_sweep_at(now)
    }

    /// Archived tasks whose text matches `query` (all of them when empty), newest first.
    pub async fn board_archive(&self, query: Option<&str>) -> anyhow::Result<Vec<ArchivedItem>> {
        let ws = self.inner.require_ws()?;
        Ok(self.inner.archive_file(&ws).search(query.unwrap_or("")).into_iter().filter(|i| i.kind == ItemKind::Task).collect())
    }

    /// The default MCP servers with their switches and whether each key is stored.
    pub async fn connections(&self) -> anyhow::Result<Vec<Connection>> {
        Ok(connections::connections(&self.inner.mcp_prefs(), &KeychainStore))
    }

    /// Switch a server and/or store its key. Applies to agents that start from now on; running
    /// agents keep the servers they started with.
    pub async fn update_connection(&self, id: &str, enabled: Option<bool>, secret: Option<ConnectionSecret>) -> anyhow::Result<Vec<Connection>> {
        let Some(entry) = connections::entry(id) else { bail!("unknown connection {id}") };
        if let Some(s) = secret {
            let Some(slot) = entry.secrets.iter().find(|x| x.env == s.env) else { bail!("{} has no {}", entry.label, s.env) };
            let acct = connections::account(entry.id, slot.env);
            match s.value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                Some(v) => KeychainStore.set(&acct, v)?,
                None => KeychainStore.delete(&acct)?,
            }
        }
        if let Some(on) = enabled {
            let mut prefs = self.inner.mcp_prefs();
            prefs.enabled.insert(entry.id.to_string(), on);
            self.inner.store.kv_set(MCP_KEY, &serde_json::to_string(&prefs)?)?;
        }
        self.connections().await
    }
}
