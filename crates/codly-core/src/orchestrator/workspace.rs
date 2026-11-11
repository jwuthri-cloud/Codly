//! Workspace-level operations: opening a repo (and its code index), the UI snapshot,
//! solo agents, stopping agents, resolving approvals, diffs, and merging a finished plan
//! into the user's branch.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail};
use serde_json::json;

use super::{Inner, guard, lock, names, prompts, row, scheduler, write};
use crate::adapters::AgentCmd;
use crate::index::Index;
use crate::memory::{self, MemoryView};
use crate::model::*;
use crate::store;

/// Answering an approval is read-check-write over the event log: one at a time.
static RESOLVING: Mutex<()> = Mutex::new(());

impl Inner {
    // ───────────── workspace ─────────────

    pub(super) async fn open_workspace(self: &Arc<Self>, repo_path: &str) -> anyhow::Result<Workspace> {
        let path = expand_home(repo_path);
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

    /// `ws_id`: the asking agent's office (None = the open one).
    pub(super) async fn search(&self, ws_id: Option<&str>, query: &str, k: usize) -> anyhow::Result<SearchResult> {
        let idx = self.index_of(ws_id).ok_or_else(|| anyhow!("the code index is not ready; use grep/rg instead"))?;
        let q = query.to_string();
        tokio::task::spawn_blocking(move || guard("code index", || idx.search(&q, k))).await?
    }

    /// The pointer pack for a task prompt, bounded in time so a long index refresh never
    /// delays a worker start.
    pub(super) async fn pointer_pack(&self, ws_id: &str, query: &str) -> Option<(String, SearchResult)> {
        let idx = self.index_of(Some(ws_id))?;
        let q = query.to_string();
        let job = tokio::task::spawn_blocking(move || guard("code index", || idx.pointer_pack(&q)));
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
            events: self.store.latest_events(id, 300)?,
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
        let agent = self.agent(agent_id)?;
        // The task it holds, as its worker or its reviewer, whatever its own role (a 1:1 agent
        // on staff takes tasks too). The row first: it has the task before a run exists.
        let held = agent.task_id.clone().or_else(|| lock(&self.live).get(agent_id).and_then(|l| l.task_id.clone()));
        if let Some(t) = held.and_then(|id| self.task(&id).ok()) {
            let reason = if t.assignee.as_deref() == Some(agent_id) {
                format!("stopped by you; move it to Ready to resume with {}", agent.name)
            } else {
                "review stopped by you; Retry to re-check".to_string()
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

    /// App exit, in about 6 s at most. Each adapter's shutdown ladder (interrupt ≤ 5 s → close
    /// stdin ≤ 3 s → SIGTERM → SIGKILL) can take ~11 s and Codex's `close()` ~6 s, longer than
    /// the app waits for us; what is still running at the deadline is SIGKILLed, so no agent
    /// (or the shells and servers it started) outlives the app.
    pub(super) async fn shutdown(&self) {
        const RUNS_GRACE: Duration = Duration::from_secs(4);
        const CODEX_GRACE: Duration = Duration::from_millis(1500);
        self.shutting_down.store(true, Ordering::SeqCst);
        let ids: Vec<Id> = lock(&self.live).keys().cloned().collect();
        for id in &ids {
            self.stop_run(id);
        }
        self.wait_for_runs(tokio::time::Instant::now() + RUNS_GRACE).await;
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
            live.get(&a.agent_id)
                .filter(|l| l.run_id == a.run_id)
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
        if self.git.rev_parse(repo, branch).await?.is_none() {
            bail!("the plan's branch {branch} was deleted, so its combined diff is gone; each task's diff still works");
        }
        self.git.diff(repo, base, branch).await
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

    pub(super) async fn merge_plan(&self, plan_id: &str) -> anyhow::Result<String> {
        let p = self.plan(plan_id)?;
        if matches!(p.status, PlanStatus::Running | PlanStatus::Integrating) {
            bail!("the plan is still running; wait until it is done (or cancel it)");
        }
        let branch = p.integration_branch.clone().ok_or_else(|| anyhow!("the plan never ran, so there is nothing to merge"))?;
        let ws = self.store.workspace(&p.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let repo = PathBuf::from(&ws.repo_path);
        let _g = self.repo_lock.lock().await;
        // Uncommitted work the plan doesn't touch is fine: git itself refuses a merge that would
        // overwrite a dirty file, and `merge --abort` on conflict keeps unrelated local changes.
        if self.git.dirty(&repo, false).await? {
            let dirty = self.git.changed_paths(&repo, &["HEAD"]).await.unwrap_or_default();
            let ours = match p.base_commit.as_deref() {
                Some(base) => self.git.changed_paths(&repo, &[base, &branch]).await.unwrap_or_default(),
                None => vec![],
            };
            let overlap: Vec<&String> = dirty.iter().filter(|f| ours.contains(f)).collect();
            if !overlap.is_empty() {
                bail!("your checkout has uncommitted changes in {}, which this plan also changes; commit them, then merge again", name_list(&overlap));
            }
        }
        let target = self
            .git
            .current_branch(&repo)
            .await?
            .ok_or_else(|| anyhow!("your checkout is on a detached HEAD; switch to a branch first"))?;
        let before = self.git.head(&repo).await?;
        let msg = format!("Merge Codly plan: {}", prompts::clip(&p.summary, 72));
        let sha = match self.git.merge(&repo, &branch, &msg, false).await? {
            crate::git::MergeOutcome::Merged { sha } | crate::git::MergeOutcome::AlreadyMerged { sha } => sha,
            crate::git::MergeOutcome::Conflict { files, .. } => {
                bail!("merging into {target} conflicts in {}; the merge was aborted and your branch is unchanged", files.join(", "))
            }
        };
        let ws_id = Some(ws.id.as_str());
        self.emit(ws_id, row(None, None, None, EventKind::Merged { sha: sha.clone(), branch: target.clone() }))?;
        self.update_plan(&p.id, |plan| {
            plan.merged_sha = Some(sha.clone());
            Ok(())
        })?;
        self.decide_by(
            ws_id,
            None,
            "merge",
            "human",
            format!("you merged {branch} into {target}"),
            json!({ "plan_id": p.id, "sha": sha }),
        )?;
        self.chat(
            ws_id,
            &p.thread,
            "codly",
            &format!("Merged \"{}\" into {target} ({}).", p.summary, &sha[..sha.len().min(10)]),
            Some(json!({ "plan_id": p.id })),
        )?;
        // The user's checkout changed under the index: re-index what the merge touched.
        let touched = match before.as_deref() {
            Some(b) => self.git.changed_files(&repo, b).await.unwrap_or_default(),
            None => vec![],
        };
        // The integration worktree has served its purpose (the branch stays for the record).
        if let Some(wt) = p.integration_worktree.as_deref()
            && let Err(e) = self.git.worktree_remove(&repo, Path::new(wt), Some(&target), false).await
        {
            tracing::info!("keeping {wt}: {e}");
        }
        drop(_g);
        if let Some(idx) = self.index_of(Some(&ws.id)).filter(|_| !touched.is_empty()) {
            let paths: Vec<PathBuf> = touched.into_iter().map(PathBuf::from).collect();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = guard("code index", || idx.update_paths(&paths)) {
                    tracing::warn!("index update after merge: {e}");
                }
            });
        }
        if let Some(w) = write(&self.ws).as_mut().filter(|w| w.id == ws.id) {
            w.head = Some(sha.clone());
        }
        Ok(sha)
    }
}

/// The process groups led by a child of this process: each agent CLI and the Codex
/// app-server lead their own (`proc::command` spawns with `process_group(0)`), and the group
/// takes the shells, test runners and dev servers they started along. Shutdown SIGKILLs them.
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

/// "a.rs, b.rs, c.rs and 9 more".
fn name_list<S: AsRef<str>>(files: &[S]) -> String {
    let shown: Vec<&str> = files.iter().take(3).map(AsRef::as_ref).collect();
    match files.len().saturating_sub(3) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
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
