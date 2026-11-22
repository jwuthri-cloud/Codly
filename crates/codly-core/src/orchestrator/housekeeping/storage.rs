//! Core glue for Settings → Storage (the rules and the git and file work are in
//! `crate::housekeeping::storage`): who owns each worktree and branch, from the store and the
//! live runs; the report; the user's own deletes; and the storage pass of the background
//! sweep. Every removal is recorded as a `storage` decision with the commit it was at.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, bail};
use serde_json::json;

use super::super::{Core, Inner, lock, prompts};
use crate::housekeeping::archive::DAY_MS;
use crate::housekeeping::storage::{self, OfficeStorage, Owner, Owners, PrunedRuns, Removed, Site, StorageCleanup, StoragePolicy, StorageReport};
use crate::model::*;

const POLICY_KEY: &str = "storage_policy";

/// The background sweep cleans up after a plan (or a task without one) only when it has been
/// over for this long: time to look at it, to undo a merge, to take something from it.
const SETTLE_MS: i64 = DAY_MS;

fn plan_branch_key(plan_id: &str) -> String {
    format!("deleted_plan_branch:{plan_id}")
}

/// "1 worktree" / "3 worktrees".
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// "2 worktrees and 1 branch" (what is zero is left out).
fn tally(removed: &[Removed]) -> String {
    let worktrees = removed.iter().filter(|r| r.worktree.is_some()).count();
    let branches = removed.iter().filter(|r| r.branch.is_some()).count();
    let parts = [(worktrees, count(worktrees, "worktree", "worktrees")), (branches, count(branches, "branch", "branches"))];
    parts.into_iter().filter(|(n, _)| *n > 0).map(|(_, text)| text).collect::<Vec<_>>().join(" and ")
}

impl Inner {
    fn storage_policy(&self) -> StoragePolicy {
        let stored = self.store.kv_get(POLICY_KEY).ok().flatten().and_then(|s| serde_json::from_str::<StoragePolicy>(&s).ok());
        stored.unwrap_or_default().normalized()
    }

    /// The last commit of a plan's integration branch that a storage cleanup deleted. It still
    /// resolves while the work is merged somewhere (the sweep deletes nothing else).
    pub(in crate::orchestrator) fn deleted_plan_branch(&self, plan_id: &str) -> anyhow::Result<Option<String>> {
        self.store.kv_get(&plan_branch_key(plan_id))
    }

    /// What Codly's records say about the worktrees and branches of `ws`. Work that ended
    /// before `settled_before` is finished for the sweep.
    fn owners(&self, ws: &Workspace, settled_before: i64) -> anyhow::Result<Owners> {
        let mut owners = Owners::default();
        let plans = self.store.plans(&ws.id, i64::MAX)?;
        let ended = |p: &Plan| matches!(p.status, PlanStatus::Done | PlanStatus::Failed | PlanStatus::Cancelled);
        // A done plan waits for your Merge, and one you chose to keep is yours: their branches
        // are the work. A failed or cancelled plan never runs again.
        let over = |p: &Plan| !p.kept && p.updated_at <= settled_before && ended(p) && (p.status != PlanStatus::Done || p.merged_sha.is_some());
        for p in &plans {
            let owner = Owner {
                label: Some(format!("Plan: {}", prompts::clip(&p.summary, 60))),
                locked: (!ended(p)).then(|| "its plan is not finished".to_string()),
                finished: over(p),
                merged_into: None,
            };
            if let Some(wt) = p.integration_worktree.as_deref() {
                owners.add_worktree(Path::new(wt), owner.clone());
            }
            if let Some(branch) = p.integration_branch.as_deref() {
                owners.add_branch(branch, owner);
            }
        }
        for t in self.store.tasks(&ws.id)? {
            let plan = t.plan_id.as_deref().map(|id| plans.iter().find(|p| p.id == id));
            let (locked, finished) = match (plan, t.status) {
                // A task of a plan is over with its plan: nothing restarts it then.
                (Some(Some(p)), _) => ((!ended(p)).then(|| "its plan is not finished".to_string()), over(p)),
                // Its plan is not on record: keep what it left.
                (Some(None), _) => (None, false),
                // On its own, a failed or cancelled task can be retried: only a done one is over.
                (None, TaskStatus::Done) => (None, t.updated_at <= settled_before),
                (None, TaskStatus::Failed | TaskStatus::Cancelled) => (None, false),
                (None, _) => (Some(format!("task {} is not finished", t.key)), false),
            };
            let owner = Owner {
                label: Some(format!("Task {}: {}", t.key, prompts::clip(&t.title, 60))),
                locked,
                finished,
                merged_into: plan.flatten().and_then(|p| p.integration_branch.clone()),
            };
            // A team member's task runs in the member's own folder, which outlives the task.
            if let Some(wt) = t.worktree.as_deref().map(Path::new).filter(|wt| !self.is_member_worktree(ws, wt)) {
                owners.add_worktree(wt, owner.clone());
            }
            if let Some(branch) = t.branch.as_deref() {
                owners.add_branch(branch, owner);
            }
        }
        // The folders agents keep between tasks (a 1:1 agent's, a team member's) are never
        // finished; they only get a name here.
        let root = self.worktree_root(ws);
        let mut agents = self.store.agents(&ws.id)?;
        agents.extend(self.store.archived_agents(&ws.id)?);
        for a in &agents {
            let Some(wt) = a.worktree.as_deref().map(Path::new) else { continue };
            if wt.starts_with(root.join("solo")) || self.is_member_worktree(ws, wt) {
                let label = if a.archived { format!("{}'s own folder (let go)", a.name) } else { format!("{}'s own folder", a.name) };
                owners.add_worktree(wt, Owner { label: Some(label), ..Default::default() });
            }
        }
        // A folder that a running agent has open stays, whatever the records say.
        let open: Vec<(Id, PathBuf)> = lock(&self.live).iter().map(|(id, live)| (id.clone(), live.cwd.clone())).collect();
        for (agent_id, cwd) in open {
            let name = agents.iter().find(|a| a.id == agent_id).map_or("an agent", |a| a.name.as_str());
            owners.lock_worktree(&cwd, format!("{name} has it open now"));
        }
        Ok(owners)
    }

    async fn office_storage(&self, office: &Office) -> OfficeStorage {
        let read = async {
            let ws = self.store.workspace(&office.id)?.ok_or_else(|| anyhow!("unknown office"))?;
            if !Path::new(&ws.repo_path).is_dir() {
                bail!("its folder {} is gone", ws.repo_path);
            }
            let root = self.worktree_root(&ws);
            let site = Site { git: &self.git, repo: Path::new(&ws.repo_path), root: &root };
            let (worktrees, branches) = site.inspect(&self.owners(&ws, i64::MIN)?).await?;
            anyhow::Ok((storage::with_sizes(worktrees).await, branches))
        };
        let (worktrees, branches, problem) = match read.await {
            Ok((worktrees, branches)) => (worktrees, branches, None),
            Err(e) => (vec![], vec![], Some(e.to_string())),
        };
        OfficeStorage { office_id: office.id.clone(), name: office.name.clone(), repo_path: office.repo_path.clone(), worktrees, branches, problem }
    }

    /// Put removals on record: a decision that names each one with its last commit, and the
    /// last commit of a deleted plan branch where `plan_diff` finds it. `confirmed`: the user
    /// agreed to lose what it had.
    fn record_removed(&self, ws: &Workspace, removed: &[Removed], method: &str, summary: String, confirmed: bool) -> anyhow::Result<()> {
        let plans = self.store.plans(&ws.id, i64::MAX)?;
        let heads: Vec<(String, Option<String>)> = removed
            .iter()
            .filter_map(|r| {
                let plan = plans.iter().find(|p| p.integration_branch.is_some() && p.integration_branch == r.branch)?;
                Some((plan_branch_key(&plan.id), Some(r.head.clone()?)))
            })
            .collect();
        self.store.kv_write(&heads)?;
        self.decide_by(Some(&ws.id), None, "storage", method, summary, json!({ "removed": removed, "confirmed": confirmed }))?;
        Ok(())
    }

    /// One office's storage pass. The repository lock is held from the read of the records to
    /// the last removal, so no task starts in a worktree while it is judged.
    async fn clean_office(&self, ws: &Workspace, settled_before: i64) -> anyhow::Result<Vec<Removed>> {
        let root = self.worktree_root(ws);
        let _g = self.repo_lock.lock().await;
        let owners = self.owners(ws, settled_before)?;
        let removed = Site { git: &self.git, repo: Path::new(&ws.repo_path), root: &root }.sweep(&owners).await?;
        if !removed.is_empty() {
            self.record_removed(ws, &removed, "rule", format!("Codly removed {} that finished, merged work left behind", tally(&removed)), false)?;
        }
        Ok(removed)
    }

    /// Remove the run folders nothing was written to for `days` (0 = keep all). A live run
    /// keeps its folder, and so does one the database still counts as running: crash recovery
    /// reads the process group recorded there.
    async fn prune_logs(&self, days: i64) -> anyhow::Result<PrunedRuns> {
        if days <= 0 {
            return Ok(PrunedRuns::default());
        }
        let mut keep: HashSet<String> = lock(&self.live).values().map(|live| live.run_id.clone()).collect();
        keep.extend(self.store.runs_with_status(&["starting", "running"])?.into_iter().map(|r| r.id));
        let dir = self.data("runs");
        let max_age = Duration::from_secs(days as u64 * 86_400);
        let pruned = tokio::task::spawn_blocking(move || storage::prune_runs(&dir, max_age, &keep, SystemTime::now())).await?;
        if !pruned.runs.is_empty() {
            let summary = format!("Codly deleted {} older than {}", count(pruned.runs.len(), "run log", "run logs"), count(days as usize, "day", "days"));
            self.decide(None, None, "storage", summary, json!({ "runs": pruned.runs, "bytes": pruned.bytes }))?;
        }
        Ok(pruned)
    }

    /// The storage pass over every office: worktrees and branches of work that ended before
    /// `settled_before` and has nothing to lose (None = leave them all), and run logs older
    /// than `log_days`. An office that fails is named in `problems`; the others still go on.
    async fn clean_storage(&self, settled_before: Option<i64>, log_days: i64) -> StorageCleanup {
        let mut done = StorageCleanup::default();
        if let Some(settled_before) = settled_before {
            let offices = match self.store.offices() {
                Ok(offices) => offices,
                Err(e) => {
                    done.problems.push(format!("the offices could not be read: {e}"));
                    vec![]
                }
            };
            for office in offices {
                // An office whose folder is gone has no repository to clean.
                if !Path::new(&office.repo_path).is_dir() {
                    continue;
                }
                let cleaned = match self.store.workspace(&office.id) {
                    Ok(Some(ws)) => self.clean_office(&ws, settled_before).await,
                    Ok(None) => continue,
                    Err(e) => Err(e),
                };
                match cleaned {
                    Ok(removed) => {
                        done.worktrees += removed.iter().filter(|r| r.worktree.is_some()).count() as i64;
                        done.branches += removed.iter().filter(|r| r.branch.is_some()).count() as i64;
                        done.bytes += removed.iter().map(|r| r.bytes).sum::<i64>();
                    }
                    Err(e) => done.problems.push(format!("{}: {e}", office.name)),
                }
            }
        }
        match self.prune_logs(log_days).await {
            Ok(pruned) => {
                done.logs = pruned.runs.len() as i64;
                done.bytes += pruned.bytes;
            }
            Err(e) => done.problems.push(format!("run logs: {e}")),
        }
        done
    }

    /// The background sweep's storage pass at `now`.
    pub(super) async fn storage_sweep_at(&self, now: i64) -> StorageCleanup {
        let policy = self.storage_policy();
        self.clean_storage(policy.clean_merged.then_some(now - SETTLE_MS), policy.log_days).await
    }

    /// One office with its worktree folder and its records, for a delete the user asked for.
    fn office_owners(&self, office_id: &str) -> anyhow::Result<(Workspace, PathBuf, Owners)> {
        let ws = self.store.workspace(office_id)?.ok_or_else(|| anyhow!("unknown office"))?;
        let (root, owners) = (self.worktree_root(&ws), self.owners(&ws, i64::MIN)?);
        Ok((ws, root, owners))
    }
}

impl Core {
    /// How long run logs are kept and whether the sweep cleans up after finished plans
    /// (defaults until you change them).
    pub async fn storage_policy(&self, set: Option<StoragePolicy>) -> anyhow::Result<StoragePolicy> {
        if let Some(p) = set {
            self.inner.store.kv_set(POLICY_KEY, &serde_json::to_string(&p.normalized())?)?;
        }
        Ok(self.inner.storage_policy())
    }

    /// What Codly holds on disk and in each repository: worktrees with their state and size,
    /// `codly/…` branches without a worktree, run logs and artifact copies.
    pub async fn storage(&self) -> anyhow::Result<StorageReport> {
        let mut offices = vec![];
        for office in self.inner.store.offices()? {
            offices.push(self.inner.office_storage(&office).await);
        }
        Ok(StorageReport {
            offices,
            logs: storage::measure(self.inner.data("runs")).await,
            artifacts: storage::measure(self.inner.data("artifacts")).await,
        })
    }

    /// "Clean up now": what the background sweep removes, without its day of waiting and
    /// whether or not the sweep is switched on. Never anything dirty, unmerged or in use.
    pub async fn clean_storage(&self) -> StorageCleanup {
        self.inner.clean_storage(Some(now_ms()), self.inner.storage_policy().log_days).await
    }

    /// The background sweep's storage pass with the clock given (tests).
    pub async fn storage_sweep_at(&self, now: i64) -> StorageCleanup {
        self.inner.storage_sweep_at(now).await
    }

    /// Delete one of Codly's worktrees. `head` is the commit the list showed. One with nothing
    /// to lose goes with its branch. A dirty or unmerged one is refused unless `force` (you
    /// confirmed what is lost), and then keeps its branch unless `delete_branch`.
    pub async fn delete_worktree(&self, office_id: &str, path: &str, head: Option<&str>, force: bool, delete_branch: bool) -> anyhow::Result<()> {
        let inner = &self.inner;
        let _g = inner.repo_lock.lock().await;
        let (ws, root, owners) = inner.office_owners(office_id)?;
        let site = Site { git: &inner.git, repo: Path::new(&ws.repo_path), root: &root };
        let removed = site.delete_worktree(&owners, Path::new(path), head, force, delete_branch).await?;
        let name = Path::new(path).strip_prefix(&root).unwrap_or(Path::new(path)).display().to_string();
        let summary = match &removed.branch {
            Some(branch) => format!("you deleted the worktree {name} and its branch {branch}"),
            None => format!("you deleted the worktree {name}"),
        };
        inner.record_removed(&ws, &[removed], "human", summary, force)
    }

    /// Delete a `codly/…` branch that no worktree has checked out. `head` is the commit the
    /// list showed. Commits that no other branch has are refused unless `force`.
    pub async fn delete_branch(&self, office_id: &str, branch: &str, head: &str, force: bool) -> anyhow::Result<()> {
        let inner = &self.inner;
        let _g = inner.repo_lock.lock().await;
        let (ws, root, owners) = inner.office_owners(office_id)?;
        let site = Site { git: &inner.git, repo: Path::new(&ws.repo_path), root: &root };
        let removed = site.delete_branch(&owners, branch, head, force).await?;
        inner.record_removed(&ws, &[removed], "human", format!("you deleted the branch {branch}"), force)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::*;
    use crate::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
    use crate::housekeeping::storage::WorkState;
    use crate::orchestrator::{CoreConfig, SpawnFuture, Spawner, StartOptions, row};
    use crate::store::RunRow;

    /// No test here starts an agent; if one did, it would get this idle stand-in.
    struct Idle;

    impl Spawner for Idle {
        fn spawn(&self, spec: SpawnSpec, _events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            Box::pin(async move { Ok(AgentHandle { agent_id: spec.agent_id, run_id: spec.run_id, tx }) })
        }
    }

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    async fn has_branch(core: &Core, repo: &Path, name: &str) -> bool {
        core.inner.git.branch_exists(repo, name).await.unwrap()
    }

    fn commit(wt: &Path, file: &str) {
        std::fs::write(wt.join(file), file).unwrap();
        sh(wt, &["add", "-A"]);
        sh(wt, &["commit", "-qm", file]);
    }

    /// A Core with one office: a repository on `main` with one commit.
    async fn start() -> (tempfile::TempDir, Arc<Core>, Workspace, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "T"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        let opts = StartOptions { spawner: Some(Arc::new(Idle)), env: Some(std::env::vars().collect()), clis: Some(vec![]), index: false };
        let core = Core::start_with(CoreConfig { data_dir: dir.path().join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(&repo.to_string_lossy()).await.unwrap();
        (dir, core, ws, repo)
    }

    /// A plan on record that ran on `codly/<short>/integration`, with its worktree on disk.
    /// The branch has a commit of its own before the plan is on record: the scheduler takes a
    /// done plan whose branch is contained in `main` for one that was merged outside Codly.
    async fn plan(core: &Core, ws: &Workspace, repo: &Path, short: &str, status: PlanStatus) -> Plan {
        let inner = &core.inner;
        let wt = inner.worktree_root(ws).join(short).join("integration");
        let branch = format!("codly/{short}/integration");
        inner.git.worktree_add(repo, &wt, &branch, "main").await.unwrap();
        commit(&wt, &format!("{short}.txt"));
        let plan = Plan {
            id: new_id(),
            workspace_id: ws.id.clone(),
            thread: "lead".into(),
            request: "do it".into(),
            summary: format!("Plan {short}"),
            assumptions: vec![],
            open_questions: vec![],
            status,
            base_commit: inner.git.head(repo).await.unwrap(),
            base_branch: Some("main".into()),
            integration_branch: Some(branch),
            integration_worktree: Some(wt.to_string_lossy().into_owned()),
            merged_sha: None,
            kept: false,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        inner.emit(Some(&ws.id), row(None, None, None, EventKind::PlanChanged { plan: plan.clone() })).unwrap();
        plan
    }

    /// A done task of `plan` on record, with its worktree on disk on `codly/<short>/<key>`.
    async fn task(core: &Core, ws: &Workspace, repo: &Path, plan: &Plan, short: &str, key: &str) -> PathBuf {
        let inner = &core.inner;
        let wt = inner.worktree_root(ws).join(short).join(key);
        let branch = format!("codly/{short}/{key}");
        inner.git.worktree_add(repo, &wt, &branch, "main").await.unwrap();
        let t = core.create_task(&format!("Task {key}"), "do a part").await.unwrap();
        inner
            .update_task_as(&t.id, true, |t| {
                t.plan_id = Some(plan.id.clone());
                t.key = key.to_string();
                t.branch = Some(branch.clone());
                t.worktree = Some(wt.to_string_lossy().into_owned());
                t.status = TaskStatus::Done;
                Ok(())
            })
            .unwrap();
        wt
    }

    fn storage_decisions(core: &Core, ws: Option<&str>) -> Vec<(String, String, serde_json::Value)> {
        core.inner
            .store
            .with(|db| {
                let mut st = db.prepare("SELECT method, summary, detail FROM decision WHERE type = 'storage' AND workspace_id IS ?1 ORDER BY created_at, rowid")?;
                let rows = st.query_map([ws], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .unwrap()
            .into_iter()
            .map(|(method, summary, detail)| (method, summary, serde_json::from_str(&detail).unwrap()))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sweep_cleans_up_after_a_merged_plan_and_keeps_dirty_and_unmerged_work() {
        let (_dir, core, ws, repo) = start().await;
        let inner = &core.inner;
        let p = plan(&core, &ws, &repo, "p1", PlanStatus::Done).await;
        let integration = PathBuf::from(p.integration_worktree.clone().unwrap());
        let merged = task(&core, &ws, &repo, &p, "p1", "t1").await;
        commit(&merged, "t1.txt");
        sh(&integration, &["merge", "-q", "--no-ff", "-m", "merge t1", "codly/p1/t1"]);
        let dirty = task(&core, &ws, &repo, &p, "p1", "t2").await;
        std::fs::write(dirty.join("notes.txt"), "not committed").unwrap();
        let unmerged = task(&core, &ws, &repo, &p, "p1", "t3").await;
        commit(&unmerged, "t3.txt");
        // You merged the plan into your branch. (Kept while the test does it by hand, so the
        // scheduler does not record the merge, and remove the plan's worktree, by itself.)
        inner
            .update_plan(&p.id, |plan| {
                plan.kept = true;
                Ok(())
            })
            .unwrap();
        sh(&repo, &["merge", "-q", "--no-ff", "-m", "Merge Codly plan", "codly/p1/integration"]);
        let merged_sha = inner.git.head(&repo).await.unwrap();
        inner
            .update_plan(&p.id, |plan| {
                plan.merged_sha = merged_sha.clone();
                plan.kept = false;
                Ok(())
            })
            .unwrap();
        let before = core.plan_diff(&p.id).await.unwrap();
        assert!(before.contains("t1.txt"), "{before}");
        let later = now_ms() + 2 * DAY_MS;

        // The plan ended a moment ago: the background pass waits a day.
        assert_eq!(core.storage_sweep_at(now_ms()).await, StorageCleanup::default());
        // Switched off: the background pass leaves worktrees and branches alone.
        core.storage_policy(Some(StoragePolicy { clean_merged: false, ..Default::default() })).await.unwrap();
        assert_eq!(core.storage_sweep_at(later).await, StorageCleanup::default());
        assert!(merged.exists() && integration.exists());
        core.storage_policy(Some(StoragePolicy::default())).await.unwrap();

        let cleaned = core.storage_sweep_at(later).await;
        assert_eq!((cleaned.worktrees, cleaned.branches, cleaned.logs), (2, 2, 0), "{cleaned:?}");
        assert!(cleaned.bytes > 0 && cleaned.problems.is_empty(), "{cleaned:?}");
        assert!(!merged.exists() && !integration.exists());
        for gone in ["codly/p1/t1", "codly/p1/integration"] {
            assert!(!inner.git.branch_exists(&repo, gone).await.unwrap(), "{gone} is merged and its plan is over");
        }
        // The test this package is about: work that is not safe elsewhere survives the sweep.
        assert_eq!(std::fs::read_to_string(dirty.join("notes.txt")).unwrap(), "not committed");
        assert!(unmerged.join("t3.txt").exists());
        for kept in ["codly/p1/t2", "codly/p1/t3"] {
            assert!(inner.git.branch_exists(&repo, kept).await.unwrap(), "{kept} must survive the sweep");
        }

        // What went is on record with the commit it was at, and the plan's diff still opens.
        let decisions = storage_decisions(&core, Some(&ws.id));
        assert_eq!(decisions.len(), 1, "{decisions:?}");
        let (method, summary, detail) = &decisions[0];
        assert_eq!((method.as_str(), summary.as_str()), ("rule", "Codly removed 2 worktrees and 2 branches that finished, merged work left behind"));
        let removed = detail["removed"].as_array().unwrap();
        assert!(removed.len() == 2 && removed.iter().all(|r| r["head"].is_string() && r["branch"].is_string() && r["worktree"].is_string()), "{detail}");
        assert_eq!(core.plan_diff(&p.id).await.unwrap(), before);

        // The list names what is left and what each would lose.
        let report = core.storage().await.unwrap();
        let office = report.offices.iter().find(|o| o.office_id == ws.id).unwrap();
        assert_eq!(office.problem, None);
        let states: Vec<(Option<&str>, WorkState)> = office.worktrees.iter().map(|w| (w.owner.as_deref(), w.state)).collect();
        assert_eq!(states, [(Some("Task t2: Task t2"), WorkState::Dirty), (Some("Task t3: Task t3"), WorkState::Unmerged)]);
        assert!(office.worktrees.iter().all(|w| w.size_bytes > 0 && w.locked.is_none()));
        assert!(office.branches.is_empty(), "{:?}", office.branches);

        // Your own delete: refused until you confirm what is lost, and the branch stays.
        let path = dirty.to_string_lossy().into_owned();
        let head = office.worktrees[0].head.clone();
        let err = core.delete_worktree(&ws.id, &path, head.as_deref(), false, false).await.unwrap_err().to_string();
        assert!(err.contains("notes.txt"), "{err}");
        assert!(dirty.exists());
        core.delete_worktree(&ws.id, &path, head.as_deref(), true, false).await.unwrap();
        assert!(!dirty.exists() && inner.git.branch_exists(&repo, "codly/p1/t2").await.unwrap());
        let decisions = storage_decisions(&core, Some(&ws.id));
        let (method, summary, detail) = decisions.last().unwrap();
        assert_eq!((method.as_str(), summary.as_str(), &detail["confirmed"]), ("human", "you deleted the worktree p1/t2", &json!(true)));
        // Its branch is listed now; it has nothing of its own, so it goes without a confirm.
        let report = core.storage().await.unwrap();
        let branch = &report.offices.iter().find(|o| o.office_id == ws.id).unwrap().branches[0];
        assert_eq!((branch.name.as_str(), branch.unmerged_commits, branch.owner.as_deref()), ("codly/p1/t2", 0, Some("Task t2: Task t2")));
        core.delete_branch(&ws.id, &branch.name, &branch.head, false).await.unwrap();
        assert!(!inner.git.branch_exists(&repo, "codly/p1/t2").await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_plan_that_waits_for_you_or_that_you_kept_is_left_alone() {
        let (_dir, core, ws, repo) = start().await;
        let inner = &core.inner;
        // A done plan that waits for your Merge. Its task is merged into the plan's branch
        // and clean: git alone would call that worktree and its branch safe to remove.
        let waiting = plan(&core, &ws, &repo, "p2", PlanStatus::Done).await;
        let waiting_wt = PathBuf::from(waiting.integration_worktree.clone().unwrap());
        let t1 = task(&core, &ws, &repo, &waiting, "p2", "t1").await;
        commit(&t1, "t1.txt");
        sh(&waiting_wt, &["merge", "-q", "--no-ff", "-m", "merge t1", "codly/p2/t1"]);
        // A plan you merged by hand and chose to keep.
        let kept = plan(&core, &ws, &repo, "p3", PlanStatus::Done).await;
        let kept_wt = PathBuf::from(kept.integration_worktree.clone().unwrap());
        inner
            .update_plan(&kept.id, |plan| {
                plan.kept = true;
                Ok(())
            })
            .unwrap();
        sh(&repo, &["merge", "-q", "--no-ff", "-m", "Merge Codly plan", "codly/p3/integration"]);
        let head = inner.git.head(&repo).await.unwrap();
        inner
            .update_plan(&kept.id, |plan| {
                plan.merged_sha = head.clone();
                Ok(())
            })
            .unwrap();
        let running = plan(&core, &ws, &repo, "p4", PlanStatus::Running).await;
        let running_wt = PathBuf::from(running.integration_worktree.clone().unwrap());
        let later = now_ms() + 2 * DAY_MS;

        assert_eq!(core.storage_sweep_at(later).await, StorageCleanup::default());
        assert!(t1.exists() && kept_wt.exists() && has_branch(&core, &repo, "codly/p2/t1").await && has_branch(&core, &repo, "codly/p3/integration").await);

        // A running plan's worktree cannot be deleted by hand either.
        let running_head = inner.git.head(&running_wt).await.unwrap();
        let err = core.delete_worktree(&ws.id, &running_wt.to_string_lossy(), running_head.as_deref(), true, true).await.unwrap_err().to_string();
        assert!(err.contains("its plan is not finished"), "{err}");

        // You cancel the first plan and stop keeping the second: "Clean up now" does not wait
        // a day.
        inner
            .update_plan(&waiting.id, |plan| {
                plan.status = PlanStatus::Cancelled;
                Ok(())
            })
            .unwrap();
        inner
            .update_plan(&kept.id, |plan| {
                plan.kept = false;
                Ok(())
            })
            .unwrap();
        let cleaned = core.clean_storage().await;
        assert_eq!((cleaned.worktrees, cleaned.branches), (2, 2), "{cleaned:?}");
        assert!(!t1.exists() && !kept_wt.exists() && !has_branch(&core, &repo, "codly/p2/t1").await && !has_branch(&core, &repo, "codly/p3/integration").await);
        // The cancelled plan's own branch holds work that is nowhere else: it stays.
        assert!(waiting_wt.exists() && has_branch(&core, &repo, "codly/p2/integration").await && running_wt.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn old_run_logs_go_with_the_sweep_and_a_running_run_keeps_its_folder() {
        let (dir, core, ws, _repo) = start().await;
        let inner = &core.inner;
        let runs = dir.path().join("data/runs");
        let already = core.storage().await.unwrap().logs.items;
        // 0 days = keep every log (set before an old log exists: the background sweep runs too).
        core.storage_policy(Some(StoragePolicy { log_days: 0, ..Default::default() })).await.unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(40 * 86_400);
        let old_run = |id: &str| {
            let d = runs.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("raw.ndjson"), vec![b'x'; 4096]).unwrap();
            std::fs::File::open(d.join("raw.ndjson")).unwrap().set_modified(long_ago).unwrap();
            std::fs::File::open(&d).unwrap().set_modified(long_ago).unwrap();
            d
        };
        let (old, running) = (old_run("old-run"), old_run("running-run"));
        let fresh = runs.join("fresh-run");
        std::fs::create_dir_all(&fresh).unwrap();
        // The database still counts this run as running: crash recovery will read its folder.
        let agent = inner.create_agent(&ws.id, "Ada", Provider::Claude, Role::Worker, 0, None).unwrap();
        inner
            .store
            .insert_run(&RunRow {
                id: "running-run".into(),
                agent_id: agent.id,
                task_id: None,
                role: Role::Worker,
                provider: Provider::Claude,
                model: None,
                cwd: "/tmp".into(),
                session_id: None,
                account: None,
                status: "running".into(),
                raw_log: running.join("raw.ndjson").to_string_lossy().into_owned(),
                report: None,
                started_at: 0,
                ended_at: None,
            })
            .unwrap();
        assert_eq!(core.storage().await.unwrap().logs.items, already + 3);

        assert_eq!(core.storage_sweep_at(now_ms()).await.logs, 0);
        assert!(old.exists());

        core.storage_policy(Some(StoragePolicy::default())).await.unwrap();
        assert_eq!(core.storage_policy(None).await.unwrap().log_days, 30);
        let cleaned = core.storage_sweep_at(now_ms()).await;
        assert_eq!(cleaned.logs, 1, "{cleaned:?}");
        assert!(cleaned.bytes > 0 && cleaned.problems.is_empty(), "{cleaned:?}");
        assert!(!old.exists() && running.exists() && fresh.exists());
        let decisions = storage_decisions(&core, None);
        assert_eq!(decisions.len(), 1, "{decisions:?}");
        assert_eq!(decisions[0].1, "Codly deleted 1 run log older than 30 days");
        assert_eq!(decisions[0].2["runs"], json!(["old-run"]));
    }
}
