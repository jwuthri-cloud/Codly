//! Workspace-level operations: opening a repo (and its code index), the UI snapshot,
//! solo agents, stopping agents, resolving approvals, diffs, and merging a finished plan
//! into the user's branch.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{anyhow, bail};
use serde_json::json;

use super::{Inner, guard, lock, names, prompts, row, scheduler, write};
use crate::adapters::AgentCmd;
use crate::index::Index;
use crate::memory::{self, MemoryView};
use crate::model::*;
use crate::store;

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

    pub(super) fn open_index(self: &Arc<Self>, ws: &Workspace) {
        *write(&self.index) = None;
        let repo = PathBuf::from(&ws.repo_path);
        let db = self.data("index").join(format!("{}.sqlite", ws.id));
        let me = self.clone();
        let ws_id = ws.id.clone();
        tokio::task::spawn_blocking(move || {
            let idx = match guard("code index", || Index::open(&repo, &db)) {
                Ok(i) => Arc::new(i),
                Err(e) => return tracing::warn!("{e}"),
            };
            if me.workspace().map(|w| w.id) != Some(ws_id) {
                return;
            }
            *write(&me.index) = Some(idx.clone());
            match guard("code index", || idx.refresh()) {
                Ok(n) => tracing::info!("indexed {n} files in {}", repo.display()),
                Err(e) => tracing::warn!("index refresh failed: {e}"),
            }
        });
    }

    pub(super) async fn search(&self, query: &str, k: usize) -> anyhow::Result<SearchResult> {
        let idx = self.index().ok_or_else(|| anyhow!("the code index is not ready; use grep/rg instead"))?;
        let q = query.to_string();
        tokio::task::spawn_blocking(move || guard("code index", || idx.search(&q, k))).await?
    }

    /// The pointer pack for a task prompt, bounded in time so a long index refresh never
    /// delays a worker start.
    pub(super) async fn pointer_pack(&self, query: &str) -> Option<(String, SearchResult)> {
        let idx = self.index()?;
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
        let slug = format!("{}-{}", names::slug(&agent.name, 16), &agent.id[agent.id.len().saturating_sub(4)..]);
        (format!("codly/solo/{slug}"), self.worktree_root(ws).join("solo").join(slug))
    }

    pub(super) async fn stop_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        let agent = self.agent(agent_id)?;
        let stopped = self.stop_run(agent_id);
        if let Some(task_id) = agent.task_id.as_deref()
            && let Ok(t) = self.task(task_id)
            && scheduler::is_active(t.status)
            && agent.role == Role::Worker
        {
            self.update_task(task_id, |t| {
                t.status = TaskStatus::Blocked;
                t.status_reason = Some(format!("stopped by you; move it to Ready to resume with {}", agent.name));
                Ok(())
            })?;
        }
        self.update_agent(agent_id, |a| {
            a.status = AgentStatus::Offline;
            a.activity = None;
            a.activity_label = None;
            if a.role != Role::Reviewer {
                a.task_id = None;
            }
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
        let (base, _) = self.store.task_commits(task_id)?;
        let base = match base {
            Some(b) => b,
            None => match t.plan_id.as_deref().map(|p| self.plan(p)).transpose()?.and_then(|p| p.base_commit) {
                Some(b) => b,
                None => return Ok(String::new()),
            },
        };
        self.git.diff(Path::new(&ws.repo_path), &base, branch).await
    }

    pub(super) async fn plan_diff(&self, plan_id: &str) -> anyhow::Result<String> {
        let p = self.plan(plan_id)?;
        let (Some(base), Some(branch)) = (p.base_commit.as_deref(), p.integration_branch.as_deref()) else { return Ok(String::new()) };
        let ws = self.store.workspace(&p.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        self.git.diff(Path::new(&ws.repo_path), base, branch).await
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
        if self.git.dirty(&repo, false).await? {
            bail!("your checkout has uncommitted changes; commit or stash them, then merge again");
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
        if let Some(idx) = self.index().filter(|_| !touched.is_empty()) {
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
    let ps = if Path::new("/bin/ps").exists() { "/bin/ps" } else { "ps" };
    let mut cmd = tokio::process::Command::new(ps);
    cmd.args(["-A", "-o", "pid=,ppid=,pgid="]).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(1), cmd.output()).await else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let v: Vec<i32> = line.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            match v.as_slice() {
                &[pid, ppid, pgid] if ppid == me && pgid == pid && pgid > 1 && pgid != own => Some(pgid),
                _ => None,
            }
        })
        .collect()
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
}
