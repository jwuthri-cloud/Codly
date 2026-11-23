//! Workspace-level operations: opening a repo (and its code index), the UI snapshot,
//! solo agents, stopping agents, resolving approvals, diffs, and merging a finished plan
//! into the user's branch.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Inner, guard, lock, merge_conflict, names, prompts, row, scheduler, write};
use crate::adapters::AgentCmd;
use crate::git::{MergeOutcome, name_list};
use crate::index::{Index, WorktreeView};
use crate::memory::{self, MemoryView};
use crate::model::*;
use crate::store;

/// Answering an approval is read-check-write over the event log: one at a time.
static RESOLVING: Mutex<()> = Mutex::new(());

/// A search compares the index with the files on disk first, at most once in this time: the
/// comparison reads no file content, but it visits every file.
const INDEX_CHECK_EVERY: Duration = Duration::from_secs(3);
/// How long a search waits for that, and then for git to compare the asker's worktree with
/// the checkout. After it the search answers from what it has, and the result says so.
const INDEX_WAIT: Duration = Duration::from_secs(2);

/// The last merge Codly made into the user's branch in one office (kv `last_merge:<office>`):
/// what Undo merge takes back. The next merge replaces it; an undo removes it.
#[derive(Serialize, Deserialize)]
struct MergeRecord {
    /// The merge commit.
    sha: String,
    /// Where the branch was before the merge: Undo goes back here.
    before: String,
    branch: String,
    /// The branch that was merged: Undo puts it back if it was deleted since.
    source: String,
    plan_id: Option<Id>,
    agent_id: Option<Id>,
}

fn last_merge_key(ws_id: &str) -> String {
    format!("last_merge:{ws_id}")
}

/// Where the user's branch was before the last merge Codly made into it: a ref in their own
/// repository, set before the merge writes anything, and kept until the next merge or an undo.
/// It is on record without Codly too: `git log refs/codly/before-merge` shows the branch as it
/// was.
const BEFORE_MERGE: &str = "refs/codly/before-merge";

impl Inner {
    // ───────────── workspace ─────────────

    pub(super) async fn open_workspace(self: &Arc<Self>, repo_path: &str) -> anyhow::Result<Workspace> {
        let path = expand_home(repo_path);
        // Without a usable git every folder reads as "not a git repository": say the real reason.
        if let Some(problem) = crate::env::git_problem(self.git.bin(), &self.base_env).await {
            bail!(problem);
        }
        let top = self.git.toplevel(&path).await.map_err(|_| anyhow!("{repo_path} is not a git repository"))?;
        let top = std::fs::canonicalize(&top).unwrap_or(top);
        let top_s = top.to_string_lossy().into_owned();
        let default_branch = self.git.default_branch(&top).await.unwrap_or_else(|_| "main".into());
        let mut ws = match self.store.workspace_by_path(&top_s)? {
            Some(w) => w,
            None => Workspace {
                id: new_id(),
                repo_path: top_s.clone(),
                name: top.file_name().map_or_else(|| top_s.clone(), |n| n.to_string_lossy().into_owned()),
                default_branch: default_branch.clone(),
                current_branch: None,
                head: None,
                dirty: false,
                created_at: now_ms(),
            },
        };
        ws.default_branch = default_branch;
        self.store.upsert_workspace(&ws)?;
        super::worktree_setup::show_setup_file_to_git(&top);
        self.refresh_git(&mut ws).await;
        let changed = self.workspace().map(|w| w.id) != Some(ws.id.clone());
        *write(&self.ws) = Some(ws.clone());
        if changed && self.use_index {
            self.open_index(&ws);
        }
        if let Err(e) = self.sync_team(&ws) {
            tracing::warn!("team sync failed: {e}");
        }
        self.warm_models();
        self.kick();
        Ok(ws)
    }

    /// Close an office: the repo leaves Codly (the map, the recent list, the offices) and its plot is
    /// freed, building included, to sell or reuse. Its agents stop; nothing is deleted, so opening
    /// the repo again brings back its agents, history and memory (on a new plot). Refused while
    /// any of its work is unfinished or an agent is mid-turn. The repo on disk is never touched.
    pub(super) async fn close_office(&self, ws_id: &str) -> anyhow::Result<()> {
        let ws = self.store.workspace(ws_id)?.ok_or_else(|| anyhow!("no such office"))?;
        let _g = self.sched_lock.lock().await; // no task starts here while it closes
        let open = |s: TaskStatus| !matches!(s, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled);
        if let Some(t) = self.store.tasks(ws_id)?.into_iter().find(|t| open(t.status)) {
            bail!("{} still has unfinished work ({} \"{}\"): finish or cancel it first", ws.name, t.key, t.title);
        }
        let agents = self.store.agents(ws_id)?;
        if let Some(a) = agents.iter().find(|a| !self.is_quiet(&a.id)) {
            bail!("{} is in the middle of a turn in {}: wait for it, or stop it first", a.name, ws.name);
        }
        for a in &agents {
            if !self.stop_and_wait(&a.id).await {
                bail!("{} did not stop in time; try again", a.name);
            }
        }
        self.store.close_workspace(ws_id)?;
        self.forget_office_tasks(ws_id); // rows the store kept for its tasks (`resume.rs`)
        super::lots::vacate(&self.store, ws_id)?;
        {
            let mut s = write(&self.settings);
            if s.paused_offices.iter().any(|id| id == ws_id) {
                s.paused_offices.retain(|id| id != ws_id);
                self.store.save_settings(&s)?;
            }
        }
        if self.workspace().is_some_and(|w| w.id == ws_id) {
            *write(&self.ws) = None;
        }
        Ok(())
    }

    pub(super) async fn refresh_git(&self, ws: &mut Workspace) {
        let repo = PathBuf::from(&ws.repo_path);
        ws.current_branch = self.git.current_branch(&repo).await.ok().flatten();
        ws.head = self.git.head(&repo).await.ok().flatten();
        ws.dirty = self.git.dirty(&repo, false).await.unwrap_or(false);
    }

    /// Load an office's code index and bring it up to date, in the background. The scheduler
    /// decides which ones stay loaded (`schedule_pass`).
    pub(super) fn open_index(self: &Arc<Self>, ws: &Workspace) {
        let repo = PathBuf::from(&ws.repo_path);
        let db = self.data("index").join(format!("{}.sqlite", ws.id));
        let me = self.clone();
        let ws_id = ws.id.clone();
        tokio::task::spawn_blocking(move || {
            let idx = match guard("code index", || Index::open(&repo, &db)) {
                Ok(i) => Arc::new(i),
                Err(e) => return tracing::warn!("{e}"),
            };
            // One index per office: opened again while it is loaded, the loaded one is refreshed.
            let idx = write(&me.index).entry(ws_id).or_insert(idx).clone();
            match guard("code index", || idx.refresh()) {
                Ok(n) => tracing::info!("indexed {n} files in {}", repo.display()),
                Err(e) => tracing::warn!("index refresh failed: {e}"),
            }
        });
    }

    /// Bring an index up to date with the files on disk before a search or a read of the code
    /// map: edits in your editor, a pull or a branch switch reach it without a restart. Waits
    /// `INDEX_WAIT` at most; a refresh that takes longer (a first build) goes on, and search
    /// results say that the index is behind until it ends.
    pub(super) async fn freshen(&self, idx: &Arc<Index>) {
        let index = idx.clone();
        // Taken here, not on the thread that refreshes: when this search stops waiting, the
        // index knows that the refresh is a long one.
        let until = Instant::now() + INDEX_WAIT;
        let job = tokio::task::spawn_blocking(move || guard("code index", || index.refresh_if_due(INDEX_CHECK_EVERY, until)));
        match tokio::time::timeout_at(until.into(), job).await {
            Ok(Ok(Ok(()))) | Err(_) => {}
            Ok(Ok(Err(e))) => tracing::warn!("index refresh failed: {e}"),
            Ok(Err(e)) => tracing::warn!("index refresh failed: {e}"),
        }
    }

    /// What differs in `dir`, where the asker works, from the checkout the index covers. None
    /// when the asker works in that checkout (the Lead, the app itself).
    async fn worktree_view(&self, idx: &Index, dir: Option<&Path>) -> Option<WorktreeView> {
        let dir = dir.filter(|d| !idx.covers(d))?;
        Some(WorktreeView::read(&self.git, idx.root(), dir, INDEX_WAIT).await)
    }

    /// `ws_id`: the asking agent's office (None = the open one). `dir`: where the asking agent
    /// works (None = in the office's checkout).
    pub(super) async fn search(&self, ws_id: Option<&str>, dir: Option<&Path>, query: &str, k: usize) -> anyhow::Result<SearchResult> {
        let idx = self.index_of(ws_id).ok_or_else(|| anyhow!("the code index is not ready; use grep/rg instead"))?;
        self.freshen(&idx).await;
        let view = self.worktree_view(&idx, dir).await;
        let q = query.to_string();
        tokio::task::spawn_blocking(move || {
            guard("code index", || match &view {
                Some(view) => idx.search_in(view, &q, k),
                None => idx.search(&q, k),
            })
        })
        .await?
    }

    /// The pointer pack for a task prompt, as the task's worktree `dir` sees the code. Bounded
    /// in time (`INDEX_WAIT` twice, then 3 s for the search), so a long index refresh never
    /// delays a worker start.
    pub(super) async fn pointer_pack(&self, ws_id: &str, dir: Option<&Path>, query: &str) -> Option<(String, SearchResult)> {
        let idx = self.index_of(Some(ws_id))?;
        self.freshen(&idx).await;
        let view = self.worktree_view(&idx, dir).await;
        let q = query.to_string();
        let job = tokio::task::spawn_blocking(move || {
            guard("code index", || match &view {
                Some(view) => idx.pointer_pack_in(view, &q),
                None => idx.pointer_pack(&q),
            })
        });
        match tokio::time::timeout(Duration::from_secs(3), job).await {
            Ok(Ok(Ok(p))) => Some(p),
            Ok(Ok(Err(e))) => {
                tracing::debug!("pointer pack: {e}");
                None
            }
            _ => None,
        }
    }

    pub(super) async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        // as_of first: an event landing while we read is then both in the projections and
        // delivered as a delta, never in neither (projections are idempotent upserts).
        let as_of = self.store.last_event_id()?;
        let mut ws = self.workspace();
        if let Some(w) = ws.as_mut() {
            self.refresh_git(w).await;
        }
        let ws_id = ws.as_ref().map(|w| w.id.clone());
        let id = ws_id.as_deref();
        let (agents, plans, tasks) = match id {
            Some(id) => (self.store.agents(id)?, self.store.plans(id, 50)?, self.store.tasks(id)?),
            None => (vec![], vec![], vec![]),
        };
        let view = MemoryView { workspace_id: id, agent_id: None, task_ids: &[] };
        let (memories, triples) = self.store.with(|db| {
            let m = guard("memory", || memory::list_memories(db, &view, None, 200)).unwrap_or_default();
            let t = guard("knowledge graph", || memory::query_triples(db, id, None, None, 300)).unwrap_or_default();
            Ok((m, t))
        })?;
        let events = self.store.latest_events(id, 300)?;
        Ok(Snapshot {
            as_of,
            clis: self.clis(),
            workspace: ws,
            settings: self.settings(),
            workspace_team: id.map(|w| self.store.workspace_team(w)).transpose()?,
            request_prefs: id.map(|w| self.store.all_request_prefs(w)).transpose()?.unwrap_or_default(),
            agents,
            plans,
            tasks,
            approvals: self.store.pending_approvals(id)?,
            chats: self.store.recent_chats(id, 200)?,
            memories,
            triples,
            accounts: self.accounts(),
            artifacts: self.store.artifacts(id, 200)?,
            session_events: self.session_rows(id, &events),
            events,
        })
    }

    // ───────────── agents ─────────────

    pub(super) async fn hire_agent(self: &Arc<Self>, provider: Provider, model: Option<String>, title: Option<String>) -> anyhow::Result<Agent> {
        let ws = self.require_ws()?;
        let title = match title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            None => None,
            Some(t) => Some(role_title(t).ok_or_else(|| anyhow!("'{t}' is not a known title ({})", ROLE_TITLES.join(", ")))?.to_string()),
        };
        let cli = self.clis().into_iter().find(|c| c.provider == provider);
        if !self.provider_usable(provider, false) {
            let why = match cli.filter(|c| c.path.is_some()) {
                None => "not installed".to_string(),
                Some(c) => c.problem.unwrap_or_else(|| "no account is signed in and turned on".into()),
            };
            bail!("{} is not available: {why}", provider.label());
        }
        let agents = self.store.agents(&ws.id)?;
        let name = names::next_name(&agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>());
        let color = names::next_color(&agents.iter().map(|a| a.color).collect::<Vec<_>>());
        let agent = self.create_agent(&ws.id, &name, provider, Role::Solo, color, model)?;
        let (branch, wt) = self.solo_worktree(&ws, &agent);
        let repo = PathBuf::from(&ws.repo_path);
        let start = self.git.head(&repo).await?.ok_or_else(|| anyhow!("the repository has no commits yet"))?;
        {
            let _g = self.repo_lock.lock().await;
            self.git.worktree_add(&repo, &wt, &branch, &start).await?;
        }
        self.mark_new_worktree(&wt)?; // prepared at its first message
        self.update_agent(&agent.id, |a| {
            a.worktree = Some(wt.to_string_lossy().into_owned());
            a.title = title;
        })
    }

    pub(super) fn solo_worktree(&self, ws: &Workspace, agent: &Agent) -> (String, PathBuf) {
        // A hired agent that works in the project folder itself (no worktree to create or commit).
        if self.agent_brief(&agent.id).is_some_and(|b| !b.isolate) {
            return ("(the checked-out branch)".into(), PathBuf::from(&ws.repo_path));
        }
        let slug = format!("{}-{}", names::slug(&agent.name, 16), &agent.id[agent.id.len().saturating_sub(4)..]);
        (format!("codly/solo/{slug}"), self.worktree_root(ws).join("solo").join(slug))
    }

    pub(super) async fn stop_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        self.stop_agent_as(agent_id, "you").await
    }

    /// `by` is who stopped it, for the reason on its task: "you", or the circuit breaker and why.
    pub(super) async fn stop_agent_as(&self, agent_id: &str, by: &str) -> anyhow::Result<()> {
        let agent = self.agent(agent_id)?;
        // The task it holds, as its worker or its reviewer, whatever its own role (a 1:1 agent
        // on staff takes tasks too). The row first: it has the task before a run exists.
        let held = agent.task_id.clone().or_else(|| lock(&self.live).get(agent_id).and_then(|l| l.task_id.clone()));
        if let Some(t) = held.and_then(|id| self.task(&id).ok()) {
            let reason = if t.assignee.as_deref() == Some(agent_id) {
                format!("stopped by {by}; Retry to resume with {}", agent.name)
            } else {
                format!("review stopped by {by}; Retry to re-check")
            };
            // Checked in the write: a task that reached its merge meanwhile is left to finish.
            let mut blocked = false;
            self.update_task(&t.id, |t| {
                blocked = scheduler::is_active(t.status) && t.status != TaskStatus::Merging;
                if blocked {
                    t.status = TaskStatus::Blocked;
                    t.status_reason = Some(reason.clone());
                }
                Ok(())
            })?;
            // After the status: a worker that is still starting re-checks it before its brief,
            // and the turn the stop interrupts ends on a task that is no longer in review. Both
            // its agents go: a worker left running would keep a Retry waiting for it.
            if blocked {
                self.release_task_agents(&t);
            }
        }
        let stopped = self.stop_run(agent_id);
        self.update_agent(agent_id, |a| {
            a.status = AgentStatus::Offline;
            a.activity = None;
            a.activity_label = None;
            a.task_id = None;
        })?;
        if !stopped {
            tracing::debug!("stop_agent: {agent_id} had no live run");
        }
        self.kick();
        Ok(())
    }

    /// App exit, in about 6 s at most. Each adapter's shutdown ladder (Claude Code: interrupt
    /// ≤ 5 s → stop the background commands ≤ 2 s → close stdin ≤ 3 s → SIGTERM → SIGKILL
    /// 3 s later) can take up to ~13 s and Codex's `close()` ~6 s, longer than
    /// the app waits for us; what is still running at the deadline is SIGKILLed, so no agent
    /// (or what stayed in its process group) outlives the app. The commands of a Claude Code
    /// agent are in groups of their own (`proc::command`): one that its CLI did not end before
    /// the SIGKILL runs on.
    pub(super) async fn shutdown(&self) {
        const RUNS_GRACE: Duration = Duration::from_secs(4);
        const CODEX_GRACE: Duration = Duration::from_millis(1500);
        self.shutting_down.store(true, Ordering::SeqCst);
        let ids: Vec<Id> = lock(&self.live).keys().cloned().collect();
        for id in &ids {
            self.stop_run(id);
        }
        // The commands you ran to try work end with the app, inside the same wait.
        tokio::join!(self.wait_for_runs(tokio::time::Instant::now() + RUNS_GRACE), super::preview::stop_all(&self.cfg.data_dir));
        // Codex agents share one app-server per account; it outlives its threads.
        let codex_stopped = tokio::time::timeout(CODEX_GRACE, crate::adapters::codex::shutdown_server()).await.is_ok();
        if !codex_stopped {
            tracing::warn!("a Codex app-server did not stop in time");
        }
        if !codex_stopped || !lock(&self.live).is_empty() {
            let groups = child_groups().await;
            for g in &groups {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(*g), nix::sys::signal::Signal::SIGKILL);
            }
            tracing::warn!("killed {} agent process group(s) that outlived the shutdown", groups.len());
            self.wait_for_runs(tokio::time::Instant::now() + Duration::from_millis(500)).await;
        }
        // Runs that did not exit in time are resumed on the next start, like a crash.
        let left: Vec<Id> = lock(&self.live).drain().map(|(_, l)| l.run_id).collect();
        for run in left {
            let _ = self.store.set_run_status(&run, "lost");
        }
        self.store.checkpoint();
    }

    async fn wait_for_runs(&self, until: tokio::time::Instant) {
        while !lock(&self.live).is_empty() && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // ───────────── approvals ─────────────

    pub(super) fn resolve_approval(&self, approval_id: &str, decision: ApprovalDecision, by: &str) -> anyhow::Result<()> {
        // From the read to the event: of two answers at once, the agent gets the one on record.
        let _g = lock(&RESOLVING);
        let a = self.store.approval(approval_id)?.ok_or_else(|| anyhow!("unknown approval {approval_id}"))?;
        if a.status != ApprovalStatus::Pending {
            bail!("that approval was already {}", store::enum_str(&a.status));
        }
        if !a.choices.is_empty() && !a.choices.contains(&decision.choice) {
            bail!("the agent did not offer '{}'", store::enum_str(&decision.choice));
        }
        let ws = self.store.agent_workspace(&a.agent_id)?;
        let sent = {
            let live = lock(&self.live);
            // A run that is being stopped (you killed the agent, or Codly ended it) takes no
            // answer while its process ends: what it asked goes with it, as once the process is gone.
            live.get(&a.agent_id)
                .filter(|l| l.run_id == a.run_id && !l.stopping)
                .is_some_and(|l| l.handle.send(AgentCmd::Approval { approval_id: a.id.clone(), decision: decision.clone() }))
        };
        let status = if !sent {
            ApprovalStatus::Expired
        } else {
            match decision.choice {
                ApprovalChoice::AllowOnce | ApprovalChoice::AllowSession | ApprovalChoice::AllowAlways => ApprovalStatus::Approved,
                ApprovalChoice::Deny => ApprovalStatus::Denied,
                ApprovalChoice::Cancel => ApprovalStatus::Cancelled,
            }
        };
        self.emit(
            ws.as_deref(),
            row(
                Some(&a.agent_id),
                a.task_id.as_deref(),
                Some(&a.run_id),
                EventKind::ApprovalResolved { approval_id: a.id.clone(), status, by: by.into() },
            ),
        )?;
        if !sent {
            bail!("the agent that asked is no longer running; the request expired");
        }
        Ok(())
    }

    // ───────────── diffs & merge ─────────────

    pub(super) async fn task_diff(&self, task_id: &str) -> anyhow::Result<String> {
        let t = self.task(task_id)?;
        let Some(branch) = t.branch.as_deref() else { return Ok(String::new()) };
        let ws_id = self.store.task_workspace(task_id)?.ok_or_else(|| anyhow!("task without workspace"))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let (base, head) = self.store.task_commits(task_id)?;
        let base = match base {
            Some(b) => b,
            None => match t.plan_id.as_deref().map(|p| self.plan(p)).transpose()?.and_then(|p| p.base_commit) {
                Some(b) => b,
                None => return Ok(String::new()),
            },
        };
        // ponytail: prefer the recorded head sha; the task branch is deleted once its work is merged.
        self.git.diff(Path::new(&ws.repo_path), &base, head.as_deref().unwrap_or(branch)).await
    }

    pub(super) async fn plan_diff(&self, plan_id: &str) -> anyhow::Result<String> {
        let p = self.plan(plan_id)?;
        let (Some(base), Some(branch)) = (p.base_commit.as_deref(), p.integration_branch.as_deref()) else { return Ok(String::new()) };
        let ws = self.store.workspace(&p.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = Path::new(&ws.repo_path);
        let head = match self.git.rev_parse(repo, branch).await? {
            Some(head) => Some(head),
            // A storage cleanup deleted the branch and kept the commit it was at.
            None => match self.deleted_plan_branch(&p.id)? {
                Some(head) => self.git.rev_parse(repo, &head).await?,
                None => None,
            },
        };
        let Some(head) = head else {
            bail!("the plan's branch {branch} was deleted, so its combined diff is gone; each task's diff still works");
        };
        self.git.diff(repo, base, &head).await
    }

    /// Send back, from the diff: your review comments on a task go to the agent that did the
    /// work, as one message, the way the gate sends a reviewer's feedback back.
    /// - Its worker still runs the task: it reads them after its current turn, the task is
    ///   Changes requested, and the gate checks the work again when that turn ends. A review
    ///   that is under way stops: it reads the work as it was before.
    /// - The task is stopped (blocked, failed) or waits to start: its next run starts with them.
    /// New work you asked for gets new fix rounds. A done task takes none: its work is merged,
    /// so the changes are a new request.
    pub(super) async fn send_back_task(&self, task_id: &str, comments: &str) -> anyhow::Result<Task> {
        use TaskStatus::*;
        let comments = comments.trim();
        if comments.is_empty() {
            bail!("there is no comment to send");
        }
        let t = self.task(task_id)?;
        let ws = self.store.task_workspace(task_id)?;
        let prompt = review_comments(&t, comments);
        // What you asked is in the worker's own conversation, where you can read it again.
        let note = |worker: &str| self.chat(ws.as_deref(), &format!("agent:{worker}"), "user", comments, None);
        let open = |s: TaskStatus| matches!(s, Running | ChangesRequested | Verifying | Reviewing);
        let worker = t.assignee.clone().filter(|a| lock(&self.live).get(a).is_some_and(|l| l.task_id.as_deref() == Some(task_id) && !l.stopping));
        let (sent, how) = match (t.status, worker) {
            (Done, _) => bail!("{} is done and its work is merged; ask for the changes in a new request", t.key),
            (Cancelled, _) => bail!("{} is cancelled; reopen it first", t.key),
            (Pending, _) => bail!("{} has not started yet", t.key),
            (s, Some(worker)) if open(s) => {
                // A reviewer that has no brief yet cannot be stopped below: the gate could not
                // give it the brief any more, and would block the task.
                if self.reviewers_of(task_id).iter().any(|(_, briefed)| !briefed) {
                    bail!("a review of {} is starting right now; send your comments again in a moment", t.key);
                }
                note(&worker)?;
                self.send_prompt(&worker, prompt)?;
                self.forget_rounds(task_id);
                // The worker has a turn to come, and its end runs the gate whatever the task
                // says by now: a pass that is under way sees the new message and stops short.
                let mut in_review = false;
                let t = self.update_task(task_id, |t| {
                    if open(t.status) {
                        in_review = t.status == Reviewing;
                        t.status = ChangesRequested;
                        t.status_reason = Some("you asked for changes; Codly checks the work again when its worker is done".into());
                    }
                    Ok(())
                })?;
                // The review under way reads the work as it was before your comments, and its
                // verdict could land on the review of the new work. It stops, now that the task
                // has left Reviewing (its end is no verdict): the gate starts a new one.
                let mut stopped = false;
                if in_review {
                    for (reviewer, briefed) in self.reviewers_of(task_id) {
                        if briefed {
                            self.release_worker(&reviewer, task_id);
                            stopped = true;
                        }
                    }
                }
                (t, if stopped { "its worker reads them now, and the review that was under way stopped" } else { "its worker reads them now" })
            }
            // No run of the task to hand them to, and what Codly does with it right now (a
            // worker that starts, your own re-check, a merge) would not see them.
            (Running | ChangesRequested | Verifying | Reviewing | Merging, _) => {
                bail!("Codly is working on {} right now; send your comments again in a moment", t.key)
            }
            (Blocked | Failed, _) => {
                // Kept before the task can start again: its next run takes them as its prompt.
                let before = self.keep_feedback(task_id, &prompt)?;
                if let Err(e) = self.move_task(task_id, Ready).await {
                    self.clear_feedback(task_id)?;
                    if let Some(b) = before {
                        self.add_feedback(task_id, &b)?;
                    }
                    return Err(e);
                }
                if let Some(worker) = &t.assignee {
                    note(worker)?;
                }
                let t = self.update_task(task_id, |t| {
                    if t.status == Ready {
                        t.status_reason = Some("you asked for changes; its worker starts again with your comments".into());
                    }
                    Ok(())
                })?;
                (t, "its next run starts with them")
            }
            (Ready, _) => {
                // A task starts under the scheduler's lock: still Ready here, its start reads them.
                let _g = self.sched_lock.lock().await;
                let now = self.task(task_id)?;
                if now.status != Ready {
                    bail!("{} has just started; send your comments again in a moment", t.key);
                }
                self.add_feedback(task_id, &prompt)?;
                if let Some(worker) = &t.assignee {
                    note(worker)?;
                }
                (now, "its next run starts with them")
            }
        };
        self.decide_by(
            ws.as_deref(),
            Some(task_id),
            "review",
            "human",
            format!("you sent {} back with review comments; {how}", t.key),
            json!({ "plan_id": t.plan_id, "comments": comments }),
        )?;
        Ok(sent)
    }

    /// The agents with a live reviewer run of a task, and whether each has its brief (false: the
    /// run is still starting).
    fn reviewers_of(&self, task_id: &str) -> Vec<(Id, bool)> {
        lock(&self.live)
            .iter()
            .filter(|(_, l)| l.role == Role::Reviewer && l.task_id.as_deref() == Some(task_id) && !l.stopping)
            .map(|(agent, l)| (agent.clone(), l.starting.is_none() && (l.turn_active || l.queued > 0 || l.turns > 0)))
            .collect()
    }

    /// Keep feedback for a task's next worker run, after what already waits there (the gate's
    /// own feedback, a hand-off note). Returns what waited before.
    fn keep_feedback(&self, task_id: &str, text: &str) -> anyhow::Result<Option<String>> {
        let before = self.feedback(task_id)?;
        self.add_feedback(task_id, text)?;
        Ok(before)
    }

    /// Plans you merged without Codly (git on the command line, another tool): a done plan whose
    /// integration branch is already in your branch counts as merged, and its integration
    /// worktree goes, as after a merge from the app.
    pub(super) async fn notice_outside_merges(&self, ws: &Workspace) -> anyhow::Result<()> {
        let repo = PathBuf::from(&ws.repo_path);
        let open = self.store.plans(&ws.id, 100)?.into_iter().filter(|p| p.status == PlanStatus::Done && p.merged_sha.is_none() && !p.kept);
        for p in open {
            let Some(branch) = p.integration_branch.as_deref() else { continue };
            let merged = if self.git.rev_parse(&repo, branch).await?.is_some() {
                self.git.is_ancestor(&repo, branch, "HEAD").await.unwrap_or(false)
            } else {
                // The branch was deleted: merged if every finished task's commit is in your branch.
                let heads: Vec<Option<String>> = self
                    .store
                    .tasks(&ws.id)?
                    .iter()
                    .filter(|t| t.plan_id.as_deref() == Some(&p.id) && t.status == TaskStatus::Done)
                    .map(|t| self.store.task_commits(&t.id).map(|c| c.1))
                    .collect::<anyhow::Result<_>>()?;
                let mut all = !heads.is_empty();
                for h in &heads {
                    all = all && match h {
                        Some(h) => self.git.is_ancestor(&repo, h, "HEAD").await.unwrap_or(false),
                        None => false,
                    };
                }
                all
            };
            if !merged {
                continue;
            }
            let Some(head) = self.git.head(&repo).await? else { continue };
            self.update_plan(&p.id, |plan| {
                plan.merged_sha = Some(head.clone());
                Ok(())
            })?;
            self.decide(Some(&ws.id), None, "merge", format!("{branch} is already in your branch: merged outside Codly"), json!({ "plan_id": p.id, "sha": head }))?;
            if let Some(wt) = p.integration_worktree.as_deref() {
                let _g = self.repo_lock.lock().await;
                if let Err(e) = self.git.worktree_remove(&repo, Path::new(wt), Some("HEAD"), false).await {
                    tracing::info!("keeping {wt}: {e}");
                }
            }
        }
        Ok(())
    }

    /// Uncommitted work a branch doesn't touch is fine: git itself refuses a merge that would
    /// overwrite a dirty file, and `merge --abort` on conflict keeps unrelated local changes.
    /// Overlapping files are refused up front, with a message the UI can act on.
    async fn check_dirty_overlap(&self, repo: &Path, branch: &str, base: Option<&str>, what: &str) -> anyhow::Result<()> {
        if !self.git.dirty(repo, false).await? {
            return Ok(());
        }
        let dirty = self.git.changed_paths(repo, &["HEAD"]).await.unwrap_or_default();
        // What the branch changed since it left `base` (not what `base` got since).
        let ours = match base {
            Some(base) => self.git.changed_paths(repo, &[format!("{base}...{branch}").as_str()]).await.unwrap_or_default(),
            None => vec![],
        };
        let overlap: Vec<&String> = dirty.iter().filter(|f| ours.contains(f)).collect();
        if !overlap.is_empty() {
            bail!("your checkout has uncommitted changes in {}, which {what} also changes; commit them, then merge again", name_list(&overlap));
        }
        Ok(())
    }

    /// Merge a solo agent's branch (`codly/solo/<slug>`) into the user's checked-out branch, like
    /// a plan. The agent keeps its worktree and goes on working from its own branch. Everything
    /// in its worktree is merged: what its last turn left uncommitted (a turn you stopped, a
    /// run that crashed) is committed first.
    pub(super) async fn merge_agent(&self, agent_id: &str) -> anyhow::Result<String> {
        let agent = self.agent(agent_id)?;
        if agent.role != Role::Solo {
            bail!("only a solo agent's branch can be merged this way; a task's work merges with its plan");
        }
        // A stopped agent counts as working until its process is gone: it may still write files.
        let resting = matches!(agent.status, AgentStatus::Idle | AgentStatus::Done | AgentStatus::Failed | AgentStatus::Offline);
        if (agent.run_id.is_some() && !resting) || !self.is_quiet(agent_id) {
            bail!("{} is still working; merge when its turn ends (Codly commits its work then)", agent.name);
        }
        let wt = PathBuf::from(agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to merge", agent.name))?);
        let ws = self.require_ws()?;
        let repo = PathBuf::from(&ws.repo_path);
        if wt == repo {
            bail!("{} works in your project folder itself, so it has no branch of its own to merge", agent.name);
        }
        let branch = self.git.current_branch(&wt).await?.ok_or_else(|| anyhow!("{}'s worktree is on a detached HEAD", agent.name))?;
        let ws_id = Some(ws.id.as_str());
        // A command you ran to try this work ends first: nothing writes into the worktree
        // while Codly commits and merges it.
        super::preview::stop_in(&wt).await;
        let _g = self.repo_lock.lock().await;
        self.git.check_worktree(&repo, &wt).await?;
        self.check_resolved(&agent, &wt).await?;
        let message = format!("codly(solo/{}): before merge", agent.name);
        if let Some(c) = self.commit_noted(ws_id, Some(agent_id), &agent.name, &wt, &message).await? {
            self.emit(ws_id, row(Some(agent_id), None, None, EventKind::Committed { sha: c.sha, files: c.files, message }))?;
        }
        let target = self
            .git
            .current_branch(&repo)
            .await?
            .ok_or_else(|| anyhow!("your checkout is on a detached HEAD; switch to a branch first"))?;
        if self.git.is_ancestor(&repo, &branch, "HEAD").await? {
            bail!("{target} already has everything on {branch}");
        }
        let base = self.git.merge_base(&repo, "HEAD", &branch).await?;
        self.check_dirty_overlap(&repo, &branch, base.as_deref(), &format!("{}'s branch", agent.name)).await?;
        let before = self.git.head(&repo).await?;
        let msg = format!("Merge {}'s work ({branch})", agent.name);
        let sha = match self.merge_with_way_back(&repo, before.as_deref(), &branch, &msg).await? {
            MergeOutcome::Merged { sha } => {
                self.record_merge(&ws.id, &sha, before.as_deref(), &target, &branch, None, Some(agent_id))?;
                sha
            }
            MergeOutcome::AlreadyMerged { sha } => sha,
            MergeOutcome::Conflict { files, .. } => {
                bail!("{}", merge_conflict::conflict_error(&target, &branch, &files, &format!("{} (from its Git tab)", agent.name)))
            }
        };
        self.emit(ws_id, row(Some(agent_id), None, None, EventKind::Merged { sha: sha.clone(), branch: target.clone() }))?;
        self.decide_by(ws_id, Some(agent_id), "merge", "human", format!("you merged {branch} into {target}"), json!({ "agent_id": agent_id, "sha": sha }))?;
        self.chat(ws_id, &format!("agent:{agent_id}"), "codly", &format!("Merged {}'s work into {target} ({}).", agent.name, &sha[..sha.len().min(10)]), None)?;
        let touched = match before.as_deref() {
            Some(b) => self.git.changed_files(&repo, b).await.unwrap_or_default(),
            None => vec![],
        };
        drop(_g);
        self.reindex_after_merge(&ws.id, touched);
        if let Some(w) = write(&self.ws).as_mut().filter(|w| w.id == ws.id) {
            w.head = Some(sha.clone());
        }
        Ok(sha)
    }

    /// Where the checkout stands against its upstream, its uncommitted tracked files, and
    /// whether the last merge Codly made here can still be undone.
    pub(super) async fn repo_sync(&self) -> anyhow::Result<RepoSync> {
        let ws = self.require_ws()?;
        let repo = PathBuf::from(&ws.repo_path);
        let branch = self.git.current_branch(&repo).await?;
        let (upstream, ahead, behind) = match self.git.ahead_behind(&repo).await? {
            Some((u, a, b)) => (Some(u), a, b),
            None => (None, 0, 0),
        };
        let dirty = self.git.changed_paths(&repo, &["HEAD"]).await.unwrap_or_default();
        let last_merge = match self.last_merge(&ws.id)? {
            Some(m) => {
                let blocked = self.git.undo_merge_blocker(&repo, &m.branch, &m.sha, &m.before, &m.source).await?;
                // Named only while it is of this merge (not of one made before Codly kept a ref).
                let kept = self.git.rev_parse(&repo, BEFORE_MERGE).await?.filter(|at| *at == m.before).map(|_| BEFORE_MERGE.to_string());
                Some(LastMerge { sha: m.sha, branch: m.branch, plan_id: m.plan_id, agent_id: m.agent_id, blocked, kept })
            }
            None => None,
        };
        Ok(RepoSync { branch, upstream, ahead, behind, dirty, last_merge })
    }

    /// Merge `source` into the user's checked-out branch, which is at `before`, with a way
    /// back that needs no Codly: `BEFORE_MERGE` keeps that commit before the merge writes
    /// anything. A merge that is not made (a conflict, nothing to merge, a hook that stopped
    /// it) puts the ref back on the merge before this one, or removes it.
    async fn merge_with_way_back(&self, repo: &Path, before: Option<&str>, source: &str, message: &str) -> anyhow::Result<MergeOutcome> {
        // A branch without a commit has nothing to go back to.
        let Some(before) = before else { return self.git.merge(repo, source, message, false).await };
        let earlier = self.git.rev_parse(repo, BEFORE_MERGE).await?;
        self.git.set_ref(repo, BEFORE_MERGE, before).await?;
        let merged = self.git.merge(repo, source, message, false).await;
        if !matches!(merged, Ok(MergeOutcome::Merged { .. })) {
            let put_back = match &earlier {
                Some(sha) => self.git.set_ref(repo, BEFORE_MERGE, sha).await,
                None => self.git.delete_ref(repo, BEFORE_MERGE).await,
            };
            // The merge's own outcome is what the user must read. A ref left on the branch as
            // it is says nothing wrong, and `repo_sync` names it only for the merge it is of.
            if let Err(e) = put_back {
                tracing::warn!("{BEFORE_MERGE} was not put back after a merge that was not made: {e:#}");
            }
        }
        merged
    }

    fn last_merge(&self, ws_id: &str) -> anyhow::Result<Option<MergeRecord>> {
        Ok(self.store.kv_get(&last_merge_key(ws_id))?.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    /// Remember a merge commit Codly just made on the user's branch, for Undo merge. A branch
    /// that had no commit before the merge (`before` = None) has nothing to go back to.
    #[allow(clippy::too_many_arguments)]
    fn record_merge(&self, ws_id: &str, sha: &str, before: Option<&str>, branch: &str, source: &str, plan_id: Option<&str>, agent_id: Option<&str>) -> anyhow::Result<()> {
        let Some(before) = before else { return Ok(()) };
        let record = MergeRecord {
            sha: sha.into(),
            before: before.into(),
            branch: branch.into(),
            source: source.into(),
            plan_id: plan_id.map(Into::into),
            agent_id: agent_id.map(Into::into),
        };
        self.store.kv_set(&last_merge_key(ws_id), &serde_json::to_string(&record)?)
    }

    /// Undo the last merge Codly made into the user's branch (`sha` = the merge the user saw):
    /// the branch goes back to the commit before it, and the merged work stays on its own
    /// branch (put back if it was deleted since), ready to merge again. Refused with the reason
    /// when it is not safe any more (see `Git::undo_merge_blocker`).
    pub(super) async fn undo_merge(&self, sha: &str) -> anyhow::Result<RepoSync> {
        let ws = self.require_ws()?;
        let repo = PathBuf::from(&ws.repo_path);
        let ws_id = Some(ws.id.as_str());
        let _g = self.repo_lock.lock().await;
        let m = self
            .last_merge(&ws.id)?
            .filter(|m| m.sha == sha)
            .ok_or_else(|| anyhow!("that is not the last merge Codly made into your branch, so it cannot be undone here"))?;
        let restored = self.git.undo_merge(&repo, &m.branch, &m.sha, &m.before, &m.source).await?;
        self.store.kv_write(&[(last_merge_key(&ws.id), None)])?;
        // The branch is where the ref says it was: nothing is left to keep. (The undo is made:
        // a ref that could not be removed is no reason to say it failed.)
        if matches!(self.git.rev_parse(&repo, BEFORE_MERGE).await, Ok(Some(_)))
            && let Err(e) = self.git.delete_ref(&repo, BEFORE_MERGE).await
        {
            tracing::warn!("{BEFORE_MERGE} was not removed after the undo: {e:#}");
        }
        let back = format!("{} is back at {}", m.branch, &m.before[..m.before.len().min(10)]);
        let kept = if restored {
            format!("Its branch {} had been deleted, so Codly put it back; you can merge it again.", m.source)
        } else {
            "The work is still on its branch; you can merge it again.".to_string()
        };
        if let Some(plan_id) = m.plan_id.as_deref() {
            let p = self.update_plan(plan_id, |p| {
                if p.merged_sha.as_deref() == Some(sha) {
                    p.merged_sha = None;
                }
                Ok(())
            })?;
            // Done and not merged again: its integration worktree comes back (the merge removed it).
            if let (Some(wt), Some(branch)) = (p.integration_worktree.as_deref(), p.integration_branch.as_deref())
                && let Err(e) = self.git.worktree_add(&repo, Path::new(wt), branch, branch).await
            {
                tracing::info!("not restoring {wt}: {e}");
            }
            let text = format!("Undid the merge of \"{}\": {back}. {kept}", p.summary);
            self.chat(ws_id, &p.thread, "codly", &text, Some(json!({ "plan_id": p.id })))?;
        }
        if let Some(agent_id) = m.agent_id.as_deref() {
            let name = self.agent(agent_id).map(|a| a.name).unwrap_or_else(|_| "the agent".into());
            let text = format!("Undid the merge of {name}'s work: {back}. {kept}");
            self.chat(ws_id, &format!("agent:{agent_id}"), "codly", &text, None)?;
        }
        self.decide_by(
            ws_id,
            None,
            "undo_merge",
            "human",
            format!("you undid the last merge: {back}"),
            json!({ "plan_id": m.plan_id, "agent_id": m.agent_id, "sha": m.sha, "before": m.before }),
        )?;
        let touched = self.git.changed_paths(&repo, &[m.before.as_str(), m.sha.as_str()]).await.unwrap_or_default();
        drop(_g);
        self.reindex_after_merge(&ws.id, touched);
        if let Some(w) = write(&self.ws).as_mut().filter(|w| w.id == ws.id) {
            w.head = Some(m.before.clone());
        }
        self.repo_sync().await
    }

    /// Commit the user's changes to tracked files in their checkout (their hooks apply).
    pub(super) async fn commit_checkout(&self, message: &str) -> anyhow::Result<String> {
        let message = message.trim();
        if message.is_empty() {
            bail!("write a commit message first");
        }
        let ws = self.require_ws()?;
        let _g = self.repo_lock.lock().await;
        let sha = self.git.commit_tracked(Path::new(&ws.repo_path), message).await?;
        self.decide_by(Some(&ws.id), None, "commit", "human", format!("you committed your changes: {}", prompts::clip(message, 72)), json!({ "sha": sha }))?;
        if let Some(w) = write(&self.ws).as_mut().filter(|w| w.id == ws.id) {
            w.head = Some(sha.clone());
        }
        Ok(sha)
    }

    /// Push the checked-out branch to its remote (`origin` unless it tracks another).
    pub(super) async fn push_checkout(&self) -> anyhow::Result<RepoSync> {
        let ws = self.require_ws()?;
        let repo = PathBuf::from(&ws.repo_path);
        let branch = self
            .git
            .current_branch(&repo)
            .await?
            .ok_or_else(|| anyhow!("your checkout is on a detached HEAD; switch to a branch first"))?;
        let remote = match self.git.ahead_behind(&repo).await? {
            Some((up, ..)) => up.split_once('/').map(|(r, _)| r.to_string()).unwrap_or_else(|| "origin".into()),
            None => "origin".into(),
        };
        {
            let _g = self.repo_lock.lock().await;
            self.git.push(&repo, &remote, &branch).await?;
        }
        self.decide_by(Some(&ws.id), None, "push", "human", format!("you pushed {branch} to {remote}"), json!({ "branch": branch, "remote": remote }))?;
        self.repo_sync().await
    }

    /// The user's checkout changed under the index: re-index what a merge touched.
    fn reindex_after_merge(&self, ws_id: &str, touched: Vec<String>) {
        if let Some(idx) = self.index_of(Some(ws_id)).filter(|_| !touched.is_empty()) {
            let paths: Vec<PathBuf> = touched.into_iter().map(PathBuf::from).collect();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = guard("code index", || idx.update_paths(&paths)) {
                    tracing::warn!("index update after merge: {e}");
                }
            });
        }
    }

    /// What the user should know before the plan is merged into the checked-out branch: it is
    /// not the branch the plan started from, or it no longer has the commit the plan started
    /// at. None when the plan started here (and on a detached HEAD, which the merge refuses).
    pub(super) async fn merge_plan_warning(&self, plan_id: &str) -> anyhow::Result<Option<MergeWarning>> {
        let p = self.plan(plan_id)?;
        let ws = self.store.workspace(&p.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        match self.git.current_branch(&repo).await? {
            Some(target) => self.other_branch_warning(&repo, &p, &target).await,
            None => Ok(None),
        }
    }

    pub(super) async fn other_branch_warning(&self, repo: &Path, p: &Plan, target: &str) -> anyhow::Result<Option<MergeWarning>> {
        let Some(base) = p.base_commit.as_deref() else { return Ok(None) };
        let from = p.base_branch.as_deref().filter(|b| *b != target);
        // Commits the plan was built on that the checked-out branch does not have: the merge
        // would bring them along.
        let extra = self.git.read(repo, &["rev-list", "--count", &format!("HEAD..{base}")]).await?.trim().parse::<u32>()?;
        let commits = if extra == 1 { "1 commit".to_string() } else { format!("{extra} commits") };
        let message = match (from, extra) {
            (None, 0) => return Ok(None),
            (Some(from), 0) => format!("This plan started from {from}, and your checkout is on {target} now."),
            (Some(from), _) => format!("This plan started from {from}. Merging it into {target} also brings {commits} from {from} that {target} does not have."),
            (None, _) => format!("{target} does not have the commit this plan started from. Merging also brings {commits} that {target} does not have."),
        };
        Ok(Some(MergeWarning { target: target.into(), message }))
    }

    /// The checked-out branch a plan would be merged into, when nothing stands in the way of
    /// trying: none of your uncommitted changes is in a file the plan changes, and the branch is
    /// the one the plan started from, or the one you confirmed (`into`).
    pub(super) async fn merge_target(&self, repo: &Path, p: &Plan, branch: &str, into: Option<&str>) -> anyhow::Result<String> {
        self.check_dirty_overlap(repo, branch, p.base_commit.as_deref(), "this plan").await?;
        let target = self
            .git
            .current_branch(repo)
            .await?
            .ok_or_else(|| anyhow!("your checkout is on a detached HEAD; switch to a branch first"))?;
        match into {
            Some(confirmed) if confirmed != target => {
                bail!("your checkout is on {target} now, not {confirmed}; nothing was merged")
            }
            Some(_) => {}
            None => {
                if let Some(w) = self.other_branch_warning(repo, p, &target).await? {
                    bail!("{} Nothing was merged: merge again and confirm {target} to go on.", w.message);
                }
            }
        }
        Ok(target)
    }

    /// Merge the plan's integration branch into the checked-out branch. `into` = the branch the
    /// user confirmed after `merge_plan_warning`; without it, a branch the plan did not start
    /// from is refused. When your branch has commits the plan was never checked with, the
    /// plan's verify commands run on the merge result first, in a scratch worktree, and a
    /// failure stops the merge; `unchecked` = you merge without them.
    pub(super) async fn merge_plan(&self, plan_id: &str, into: Option<&str>, unchecked: bool) -> anyhow::Result<String> {
        let p = self.plan(plan_id)?;
        if matches!(p.status, PlanStatus::Running | PlanStatus::Integrating) {
            bail!("the plan is still running; wait until it is done (or cancel it)");
        }
        let branch = p.integration_branch.clone().ok_or_else(|| anyhow!("the plan never ran, so there is nothing to merge"))?;
        let ws = self.store.workspace(&p.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        // A conflict resolution that did not finish is not merged: it is taken back first, if
        // the plan's last pass could not do it.
        let p = {
            let _g = self.repo_lock.lock().await;
            self.settle_resolution(&p).await?
        };
        // Before the repo lock: the checks can take minutes.
        let checked = match unchecked {
            true => None,
            false => self.check_merge_result(&p, &ws, &branch, into).await?,
        };
        let _g = self.repo_lock.lock().await;
        let target = self.merge_target(&repo, &p, &branch, into).await?;
        let before = self.git.head(&repo).await?;
        if let Some(c) = &checked
            && (before.as_deref() != Some(c.head.as_str()) || self.git.rev_parse(&repo, &branch).await?.as_deref() != Some(c.tip.as_str()))
        {
            bail!("{target} or the plan's branch changed while the checks ran, so they say nothing about this merge; nothing was merged. Merge again.");
        }
        let msg = format!("Merge Codly plan: {}", prompts::clip(&p.summary, 72));
        let sha = match self.merge_with_way_back(&repo, before.as_deref(), &branch, &msg).await? {
            MergeOutcome::Merged { sha } => {
                self.record_merge(&ws.id, &sha, before.as_deref(), &target, &branch, Some(&p.id), None)?;
                sha
            }
            MergeOutcome::AlreadyMerged { sha } => sha,
            MergeOutcome::Conflict { files, .. } => {
                bail!("{}", merge_conflict::conflict_error(&target, &branch, &files, "An agent (on the plan's card)"))
            }
        };
        self.forget_failed_checks(&p.id)?;
        let ws_id = Some(ws.id.as_str());
        self.emit(ws_id, row(None, None, None, EventKind::Merged { sha: sha.clone(), branch: target.clone() }))?;
        self.update_plan(&p.id, |plan| {
            plan.merged_sha = Some(sha.clone());
            Ok(())
        })?;
        // What ran on the merge result before your branch moved, if anything had to.
        let (said, checks) = match (&checked, unchecked) {
            (Some(c), _) => (c.said(&target), c.detail()),
            (None, true) => (" You merged it without the plan's checks on the merge result.".to_string(), json!("skipped by you")),
            (None, false) => (String::new(), json!(null)),
        };
        self.decide_by(
            ws_id,
            None,
            "merge",
            "human",
            format!("you merged {branch} into {target}"),
            json!({ "plan_id": p.id, "sha": sha, "merge_result_checks": checks }),
        )?;
        self.chat(
            ws_id,
            &p.thread,
            "codly",
            &format!("Merged \"{}\" into {target} ({}).{said}", p.summary, &sha[..sha.len().min(10)]),
            Some(json!({ "plan_id": p.id })),
        )?;
        // The user's checkout changed under the index: re-index what the merge touched.
        let touched = match before.as_deref() {
            Some(b) => self.git.changed_files(&repo, b).await.unwrap_or_default(),
            None => vec![],
        };
        // A command you ran to try the plan ends with the merge, before its folder goes.
        if let Some(wt) = p.integration_worktree.as_deref() {
            super::preview::stop_in(Path::new(wt)).await;
        }
        // The integration worktree has served its purpose (the branch stays for the record).
        if let Some(wt) = p.integration_worktree.as_deref()
            && let Err(e) = self.git.worktree_remove(&repo, Path::new(wt), Some(&target), false).await
        {
            tracing::info!("keeping {wt}: {e}");
        }
        drop(_g);
        self.reindex_after_merge(&ws.id, touched);
        if let Some(w) = write(&self.ws).as_mut().filter(|w| w.id == ws.id) {
            w.head = Some(sha.clone());
        }
        Ok(sha)
    }
}

/// The process groups led by a child of this process: each agent CLI and the Codex
/// app-server lead their own (`proc::command` spawns with `process_group(0)`), and the group
/// takes along what they started in it (for Claude Code that is not its commands: see
/// `proc::command`). Shutdown SIGKILLs them.
///
/// ponytail: adapters do not hand out their child's pid, so this goes by "is our child" rather
/// than by run (verify commands, also in their own group, go too: fine at exit). Only live
/// children: a pgid recorded earlier could belong to an unrelated process by now.
async fn child_groups() -> Vec<i32> {
    let me = std::process::id() as i32;
    let own = nix::unistd::getpgrp().as_raw();
    let all = processes().await;
    all.iter().filter(|p| p.ppid == me && p.pgid == p.pid && p.pgid > 1 && p.pgid != own).map(|p| p.pgid).collect()
}

/// One `ps` row. `started` is epoch ms, to the second (`ps` prints how long ago in whole
/// seconds); None when that column could not be read.
pub(super) struct Proc {
    pid: i32,
    ppid: i32,
    pgid: i32,
    started: Option<i64>,
}

/// Every process, from one `ps` call (none if `ps` fails or takes over a second).
pub(super) async fn processes() -> Vec<Proc> {
    let ps = if Path::new("/bin/ps").exists() { "/bin/ps" } else { "ps" };
    let mut cmd = tokio::process::Command::new(ps);
    cmd.args(["-A", "-o", "pid=,ppid=,pgid=,etime="]).stdin(std::process::Stdio::null()).kill_on_drop(true);
    // `ps` reads the clock once, as it starts: elapsed times count back from about here.
    let now = now_ms();
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(1), cmd.output()).await else { return vec![] };
    parse_ps(&String::from_utf8_lossy(&out.stdout), now)
}

fn parse_ps(text: &str, now: i64) -> Vec<Proc> {
    text.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let mut id = || cols.next()?.parse::<i32>().ok();
            let (pid, ppid, pgid) = (id()?, id()?, id()?);
            Some(Proc { pid, ppid, pgid, started: cols.next().and_then(etime_secs).map(|s| now - s * 1000) })
        })
        .collect()
}

/// `ps` etime, `[[dd-]hh:]mm:ss`, in seconds.
fn etime_secs(etime: &str) -> Option<i64> {
    let (days, clock) = match etime.split_once('-') {
        Some((d, clock)) => (d.parse::<i64>().ok()?, clock),
        None => (0, etime),
    };
    let parts: Vec<i64> = clock.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        &[m, s] if days == 0 => Some(m * 60 + s),
        &[h, m, s] => Some(((days * 24 + h) * 60 + m) * 60 + s),
        _ => None,
    }
}

/// Of the recorded groups (`proc::recorded_group`: pgid, start in ms), those a crashed Codly
/// left running. All of these must hold, or the group is not touched:
/// - its leader is alive and still leads it;
/// - the leader's parent is pid 1, so the Codly that started it is gone (the agents of a
///   second, running Codly have that Codly as their parent);
/// - the leader started when the record says, within etime's whole seconds (a reused pid, and
///   anything after a reboot, started later).
///
/// ponytail: a group whose leader already exited is left alone, and so is an orphan adopted
/// by a subreaper other than pid 1 (Linux user sessions). Covering those means recording each
/// member's pid and start time, not loosening these checks.
pub(super) fn orphans(snapshot: &[Proc], wanted: &[(i32, i64)]) -> Vec<i32> {
    let mut found: Vec<i32> = Vec::new();
    for &(pgid, started) in wanted {
        let orphaned = snapshot
            .iter()
            .any(|p| p.pid == pgid && p.pgid == pgid && p.ppid == 1 && pgid > 1 && p.started.is_some_and(|s| (s - started).abs() <= 2000));
        if orphaned && !found.contains(&pgid) {
            found.push(pgid);
        }
    }
    found
}

/// What a worker reads when you send its task back from the diff. The header is Codly's; the
/// comments are yours (each one: the file and line, the quoted line, what you wrote).
fn review_comments(t: &Task, comments: &str) -> String {
    format!(
        "[Codly] The user reviewed your work on task {} \"{}\" and sent it back with the comments below. Address every comment, \
         stay within your task, re-run the verify commands and call codly_submit_report again.\n\n{comments}",
        t.key, t.title
    )
}

fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map_or_else(|| PathBuf::from(p), |h| h.join(rest)),
        None => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child in its own process group (like every agent CLI) is found, and killing its group
    /// takes what it started along; a child sharing our group (git) is not listed.
    #[tokio::test]
    async fn leftover_agent_groups_are_found() {
        let mut agent = tokio::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30 & sleep 30"])
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut ours = tokio::process::Command::new("/bin/sleep").arg("30").kill_on_drop(true).spawn().unwrap();
        let pgid = agent.id().unwrap() as i32;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let groups = child_groups().await;
        assert!(groups.contains(&pgid), "{groups:?}");
        assert!(!groups.contains(&(ours.id().unwrap() as i32)));
        // only our own test child: other tests in this process have children too
        nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::Signal::SIGKILL).unwrap();
        let status = tokio::time::timeout(Duration::from_secs(2), agent.wait()).await.unwrap().unwrap();
        assert!(!status.success());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), None).is_err(), "the background sleep is gone too");
        let _ = ours.kill().await;
    }

    /// Crash recovery kills a recorded group only when its leader is certainly the process
    /// Codly started and no running Codly owns it. Synthetic `ps` rows: nothing is signalled.
    #[test]
    fn only_orphaned_leaders_with_the_recorded_start_match() {
        let now = 1_800_000_000_000;
        let ago = |secs: i64| now - secs * 1000;
        // pid, ppid, pgid, etime
        let snapshot = parse_ps(
            "100   1 100       00:30\n\
             101 100 100       00:29\n\
             200 777 200       00:30\n\
             300   1 300    01:00:00\n\
             400   1 399       00:30\n\
             500   1 500 02-03:04:05\n\
             600   1 600           -\n\
             0     1   0       00:30\n",
            now,
        );
        assert_eq!(snapshot.len(), 8);
        let one = |pgid: i32, started: i64| orphans(&snapshot, &[(pgid, started)]);

        // launchd adopted the leader, and it started when the record says (to etime's second)
        assert_eq!(one(100, ago(30) + 400), [100]);
        assert_eq!(one(100, ago(32)), [100]);
        assert_eq!(one(300, ago(3600)), [300], "hh:mm:ss");
        assert_eq!(one(500, ago(((2 * 24 + 3) * 60 + 4) * 60 + 5)), [500], "dd-hh:mm:ss");

        assert!(one(100, ago(33)).is_empty(), "3 s off is another process");
        assert!(one(100, ago(3600)).is_empty(), "the pid was reused since");
        assert!(one(200, ago(30)).is_empty(), "its parent lives: a second Codly's agent");
        assert!(one(399, ago(30)).is_empty(), "the leader is gone");
        assert!(one(400, ago(30)).is_empty(), "not a group leader");
        assert!(one(600, ago(30)).is_empty(), "no start time to compare");
        assert!(one(0, ago(30)).is_empty(), "never our own group");

        // Codex runs share one server: its group comes once.
        let wanted = [(100, ago(30)), (200, ago(30)), (500, ago(183_845)), (100, ago(30))];
        assert_eq!(orphans(&snapshot, &wanted), [100, 500]);
    }
}
