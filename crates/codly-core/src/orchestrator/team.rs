//! The standing team and consultants. Team members are configured globally in Settings (a
//! repo can switch one off or add its own) and get one agent per repo: a persistent
//! identity with its own model, role, private memory, CLI session and one stable worktree
//! (`<worktrees>/team/<member id>`) that switches to each task's branch, so the member can
//! resume the same conversation task after task. Its CLI only runs while it has work or a
//! message. Consultants are workers hired for one task (harness and model chosen by the Lead)
//! and let go (archived) when the task ends.
//!
//! The Lead delegates through `codly_submit_plan` (assignee / consultant / session per task),
//! `codly_quick_task`, `codly_reassign` and `codly_message_member`; members' answers come back
//! to the Lead through `Live::answers`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::planner::{self, PlanInput, TaskInput};
use super::runs::{RunReq, new_session};
use super::{Answer, Inner, lock, names, prompts};
use crate::adapters::SessionMode;
use crate::model::*;
use crate::store;

/// Names a member cannot take: they mean something else in an assignee.
const RESERVED: [&str; 4] = [planner::CONSULTANT, "lead", "any", "codly"];

/// CLIs a consultant can run on (the ones the Lead is told about).
const CONSULTANT_CLIS: [Provider; 4] = [Provider::Claude, Provider::Codex, Provider::Gemini, Provider::Opencode];

fn role_of(kind: MemberKind) -> Role {
    match kind {
        MemberKind::Worker => Role::Worker,
        MemberKind::Reviewer => Role::Reviewer,
    }
}

/// Trim and check a team list; fill in missing ids. `others` = the members it must not clash
/// with (the global team, for a repo's own members).
pub fn normalize_team(list: Vec<TeamMember>, others: &[TeamMember]) -> Result<Vec<TeamMember>> {
    let mut out: Vec<TeamMember> = vec![];
    for mut m in list {
        m.name = m.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if m.name.is_empty() {
            bail!("every team member needs a name");
        }
        if m.name.chars().count() > 24 {
            bail!("the name '{}' is too long (max 24 characters)", m.name);
        }
        if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&m.name)) {
            bail!("'{}' cannot be a member's name: it means something else to the Lead", m.name);
        }
        if out.iter().chain(others).any(|x| x.name.eq_ignore_ascii_case(&m.name) && (x.id != m.id || x.id.is_empty())) {
            bail!("two team members are called '{}'", m.name);
        }
        m.model = m.model.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(model) = m.model.as_deref().filter(|s| !planner::valid_model(s)) {
            bail!("{}'s model '{model}' must be one word (e.g. sonnet, opus:high)", m.name);
        }
        m.role = prompts::clip(m.role.trim(), 400);
        m.title = match m.title.trim() {
            "" => String::new(),
            t => role_title(t).ok_or_else(|| anyhow!("{}'s title '{t}' is not one of: {}", m.name, ROLE_TITLES.join(", ")))?.into(),
        };
        let id = m.id.trim().to_string();
        m.id = if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            let uid = new_id();
            format!("{}-{}", names::slug(&m.name, 12), &uid[uid.len() - 4..])
        } else {
            id
        };
        if out.iter().chain(others).any(|x| x.id == m.id) {
            bail!("two team members share the id {}", m.id);
        }
        out.push(m);
    }
    Ok(out)
}

impl Inner {
    // ───────────── the roster ─────────────

    /// The repo's team, in your order: global members it did not switch off, then its own.
    pub(super) fn effective_team(&self, ws_id: &str) -> Vec<TeamMember> {
        let repo = self.store.workspace_team(ws_id).unwrap_or_default();
        let mut out: Vec<TeamMember> = vec![];
        for m in self.settings().team.into_iter().filter(|m| !repo.off.contains(&m.id)).chain(repo.members) {
            if m.enabled && !m.id.is_empty() && !out.iter().any(|x| x.name.eq_ignore_ascii_case(&m.name)) {
                out.push(m);
            }
        }
        out
    }

    /// The team configuration of a member agent (None for everyone else, or a member that
    /// left the team).
    pub(super) fn team_member(&self, agent: &Agent) -> Option<TeamMember> {
        let id = agent.member.as_deref()?;
        let ws = self.store.agent_workspace(&agent.id).ok().flatten()?;
        self.effective_team(&ws).into_iter().find(|m| m.id == id)
    }

    /// A repo's own team changes, saved and applied.
    pub(super) fn set_workspace_team(&self, team: WorkspaceTeam) -> Result<WorkspaceTeam> {
        let ws = self.require_ws()?;
        let global = self.settings().team;
        let mut team = team;
        team.members = normalize_team(std::mem::take(&mut team.members), &global)?;
        team.off.retain(|id| global.iter().any(|m| &m.id == id));
        team.off.dedup();
        self.store.save_workspace_team(&ws.id, &team)?;
        self.sync_team(&ws)?;
        self.kick();
        Ok(team)
    }

    /// Bring the repo's member agents in line with its team: create new members, bring back
    /// switched-on ones (same agent, memory and session), apply renames and models, and let
    /// go of members who left (once they are not working).
    pub(super) fn sync_team(&self, ws: &Workspace) -> Result<()> {
        let team = self.effective_team(&ws.id);
        let members = self.store.member_agents(&ws.id)?;
        let live: HashSet<Id> = lock(&self.live).keys().cloned().collect();
        let mut colors: Vec<i64> = self.store.agents(&ws.id)?.iter().map(|a| a.color).collect();
        for m in &team {
            let title = Some(m.title());
            let Some(a) = members.iter().find(|a| a.member.as_deref() == Some(&m.id)) else {
                let color = names::next_color(&colors);
                colors.push(color);
                let mut a = self.new_agent(&m.name, m.provider, role_of(m.kind), color, m.model.clone());
                a.member = Some(m.id.clone());
                a.title = title;
                self.insert_agent(&ws.id, a)?;
                continue;
            };
            let free = !live.contains(&a.id) && a.task_id.is_none();
            let billing = self.clis().iter().find(|c| c.provider == m.provider).map_or(Billing::Unknown, |c| c.billing);
            let moved = free && a.provider != m.provider;
            let was_archived = a.archived;
            let a = self.update_agent(&a.id, |a| {
                a.name = m.name.clone();
                a.title = title.clone();
                if a.archived {
                    a.archived = false;
                    a.status = AgentStatus::Idle;
                }
                if free {
                    a.role = role_of(m.kind);
                    a.model = m.model.clone();
                }
                if moved {
                    // Sessions are per CLI: another harness starts a new conversation.
                    a.provider = m.provider;
                    a.session_id = None;
                    a.account = None;
                    a.billing = billing;
                }
            })?;
            if was_archived {
                self.decide(Some(&ws.id), None, "staff", format!("{} is back on the team", a.name), json!({ "agent": a.id }))?;
            }
            if moved {
                let summary = format!("{} now runs on {}, in a new session", a.name, m.provider.label());
                self.decide(Some(&ws.id), None, "staff", summary, json!({ "agent": a.id, "provider": m.provider }))?;
            }
        }
        for a in members.iter().filter(|a| !a.archived && !team.iter().any(|m| a.member.as_deref() == Some(&m.id))) {
            if live.contains(&a.id) || a.task_id.is_some() {
                continue; // let go once its current work is done
            }
            let mut freed = vec![];
            for t in self.store.tasks(&ws.id)?.into_iter().filter(|t| t.assignee.as_deref() == Some(&a.id)) {
                if matches!(t.status, TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Blocked | TaskStatus::Failed) {
                    self.update_task(&t.id, |t| {
                        t.assignee = None;
                        t.fresh_session = false;
                        Ok(())
                    })?;
                    freed.push(t.key);
                }
            }
            let why = if freed.is_empty() {
                "left the team".to_string()
            } else {
                format!("left the team; {} go to consultants unless the Lead reassigns them", freed.join(", "))
            };
            self.retire(&ws.id, a, &why)?;
        }
        Ok(())
    }

    /// Let an agent go: it leaves the board and the office; its memory and history stay.
    fn retire(&self, ws_id: &str, a: &Agent, why: &str) -> Result<()> {
        self.update_agent(&a.id, |a| {
            a.archived = true;
            a.status = AgentStatus::Offline;
            a.task_id = None;
            a.activity = None;
            a.activity_label = None;
            a.worktree = None; // the task's worktree goes with the plan; a ghost brought back gets a new one
        })?;
        let who = if a.consultant { "consultant " } else { "" };
        self.decide(Some(ws_id), None, "staff", format!("let {who}{} go: {why}", a.name), json!({ "agent": a.id, "consultant": a.consultant }))?;
        Ok(())
    }

    /// Workers hired for tasks (consultants, and older workers from before consultants) are let
    /// go once their tasks are all over (done, failed or cancelled); team members and workers
    /// you brought back stay. A blocked task keeps its worker, so a retry resumes the same
    /// session. A worktree that is gone from disk is forgotten. Caller holds `sched_lock`.
    pub(super) fn sweep_staff(&self, ws: &Workspace, tasks: &[Task], agents: &[Agent]) -> Result<()> {
        let live: HashSet<Id> = lock(&self.live).keys().cloned().collect();
        for a in agents.iter().filter(|a| a.worktree.as_deref().is_some_and(|w| !Path::new(w).exists()) && !live.contains(&a.id)) {
            self.update_agent(&a.id, |a| a.worktree = None)?;
        }
        // ponytail: "Came back" in the scrapbook marks ghosts brought back before `kept` existed.
        let kept = |a: &Agent| a.kept || a.career.milestones.iter().any(|m| m.text == "Came back");
        let hired = |a: &Agent| a.role == Role::Worker && a.member.is_none() && (a.consultant || !kept(a));
        for a in agents.iter().filter(|a| hired(a) && !live.contains(&a.id) && a.task_id.is_none()) {
            let mine: Vec<&Task> = tasks.iter().filter(|t| t.assignee.as_deref() == Some(&a.id)).collect();
            if mine.iter().all(|t| matches!(t.status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled)) {
                let why = match mine.last() {
                    Some(t) => format!("{} is {}", t.key, store::enum_str(&t.status)),
                    None => "no task".into(),
                };
                self.retire(&ws.id, a, &why)?;
            }
        }
        Ok(())
    }

    // ───────────── member worktrees ─────────────

    fn team_root(&self, ws: &Workspace) -> PathBuf {
        self.worktree_root(ws).join("team")
    }

    pub(super) fn is_member_worktree(&self, ws: &Workspace, path: &Path) -> bool {
        path.starts_with(self.team_root(ws))
    }

    /// The member's stable worktree, created (detached at HEAD, deps seeded) on first use.
    pub(super) async fn member_worktree(&self, ws: &Workspace, agent: &Agent) -> Result<PathBuf> {
        let id = agent.member.as_deref().ok_or_else(|| anyhow!("{} is not on the team", agent.name))?;
        let wt = self.team_root(ws).join(id);
        if !wt.exists() {
            let repo = PathBuf::from(&ws.repo_path);
            let head = self.git.head(&repo).await?.ok_or_else(|| anyhow!("the repository has no commits yet"))?;
            {
                let _g = self.repo_lock.lock().await;
                self.git.worktree_add_detached(&repo, &wt, &head).await?;
            }
            self.bootstrap_worktree(ws, &repo, &wt, None).await;
        }
        Ok(wt)
    }

    /// Put the member's worktree on a task's branch (created at `start` if new). Leftovers of
    /// the previous task are committed on its branch first; a branch checked out in another
    /// Codly worktree is released there. Returns a note for the member, if anything happened.
    pub(super) async fn switch_member_worktree(&self, ws: &Workspace, agent: &Agent, key: &str, branch: &str, start: &str) -> Result<(PathBuf, Option<String>)> {
        let wt = self.member_worktree(ws, agent).await?;
        let repo = PathBuf::from(&ws.repo_path);
        let _g = self.repo_lock.lock().await;
        self.git.check_worktree(&repo, &wt).await?;
        let current = self.git.current_branch(&wt).await?;
        if current.as_deref() == Some(branch) {
            return Ok((wt, None));
        }
        // Resolved in the repo: "HEAD" means the user's checkout, not the member's.
        let start = self.git.rev_parse(&repo, start).await?.ok_or_else(|| anyhow!("{start} does not resolve"))?;
        let mut note = None;
        if self.git.in_merge(&wt).await {
            self.git.abort_merge(&wt).await?;
        }
        if self.git.dirty(&wt, true).await? {
            let on = match current {
                Some(b) => b,
                None => {
                    let b = format!("codly/team/{}/wip-{}", agent.member.as_deref().unwrap_or("x"), now_ms());
                    self.git.switch(&wt, &b, "HEAD").await?;
                    b
                }
            };
            if let Some(c) = self.git.commit_all(&wt, &format!("codly(wip): {} leftovers before {key}", agent.name)).await? {
                note = Some(format!("Codly committed your uncommitted leftovers on {on} ({}).", &c.sha[..c.sha.len().min(10)]));
            }
        }
        let mine = std::fs::canonicalize(&wt).unwrap_or_else(|_| wt.clone());
        let root = self.worktree_root(ws);
        for w in self.git.worktrees(&repo).await? {
            if w.branch.as_deref() != Some(branch) || std::fs::canonicalize(&w.path).unwrap_or_else(|_| w.path.clone()) == mine {
                continue;
            }
            if !w.path.starts_with(&root) && !std::fs::canonicalize(&w.path).is_ok_and(|p| p.starts_with(&root)) {
                bail!("{branch} is checked out in {}; switch that checkout to another branch", w.path.display());
            }
            self.git.check_worktree(&repo, &w.path).await?;
            self.git.commit_all(&w.path, &format!("codly(wip): leftovers before {} takes {key}", agent.name)).await?;
            self.git.detach(&w.path).await?;
        }
        self.git.switch(&wt, branch, &start).await?;
        Ok((wt, note))
    }

    /// The member's latest finished tasks, for a new session's brief.
    pub(super) fn member_history(&self, ws_id: &str, agent_id: &str) -> Vec<String> {
        let mut done: Vec<Task> = self
            .store
            .tasks(ws_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.assignee.as_deref() == Some(agent_id) && t.status == TaskStatus::Done)
            .collect();
        done.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        done.iter()
            .take(5)
            .map(|t| format!("{} \"{}\": {}", t.key, prompts::clip(&t.title, 60), prompts::clip(t.summary.as_deref().unwrap_or("(no summary)"), 160)))
            .collect()
    }

    // ───────────── what the Lead sees ─────────────

    /// Model names a CLI reported (cached; empty = unknown or any name).
    pub(super) fn models_of(&self, p: Provider) -> Vec<String> {
        lock(&self.models).get(&p).cloned().unwrap_or_default()
    }

    /// Ask the CLIs for their model lists in the background (once per CLI).
    pub(super) fn warm_models(self: &Arc<Self>) {
        for p in CONSULTANT_CLIS.into_iter().filter(|p| self.cli_usable(*p) && !lock(&self.models).contains_key(p)) {
            let me = self.clone();
            tokio::spawn(async move {
                let job = crate::env::list_models(p, &me.base_env);
                let models = match tokio::time::timeout(Duration::from_secs(10), job).await {
                    Ok(Ok(m)) => m,
                    _ => vec![],
                };
                lock(&me.models).insert(p, models);
            });
        }
    }

    pub(super) fn consultant_clis(&self) -> Vec<Provider> {
        CONSULTANT_CLIS.into_iter().filter(|p| self.cli_usable(*p)).collect()
    }

    pub(super) fn consultant_options(&self) -> Vec<Value> {
        self.consultant_clis().into_iter().map(|p| json!({ "provider": p, "models": self.models_of(p) })).collect()
    }

    /// `codly_board`'s team section for the Lead.
    pub(super) fn team_view(&self, ws_id: &str, agents: &[Agent], tasks: &[Task]) -> Vec<Value> {
        let team = self.effective_team(ws_id);
        let key = |id: &Option<Id>| id.as_deref().and_then(|i| tasks.iter().find(|t| t.id == i)).map(|t| t.key.clone());
        agents
            .iter()
            .filter(|a| a.on_staff() || a.consultant)
            .map(|a| {
                let m = team.iter().find(|m| a.member.as_ref() == Some(&m.id));
                let queue: Vec<&str> = tasks
                    .iter()
                    .filter(|t| t.assignee.as_deref() == Some(&a.id) && matches!(t.status, TaskStatus::Pending | TaskStatus::Ready))
                    .map(|t| t.key.as_str())
                    .collect();
                let ctx = match (a.usage.context_used, a.usage.context_window.filter(|w| *w > 0)) {
                    (Some(u), Some(w)) => Some((u * 100 / w).clamp(0, 100)),
                    _ => None,
                };
                json!({
                    "name": a.name, "consultant": a.consultant, "kind": if a.role == Role::Reviewer { "reviewer" } else { "worker" },
                    "title": a.title, "provider": a.provider, "model": a.model, "role": m.map(|m| prompts::untrusted_inline(&m.role, 400)),
                    "status": a.status, "task": key(&a.task_id), "queue": queue,
                    "session": { "resumable": a.session_id.is_some(), "context_pct": ctx },
                    "cli_ready": self.cli_usable(a.provider),
                    "at_home_until": super::at_home(a, now_ms()),
                    "energy": super::tired(a, now_ms()).then_some(TIRED_NOTE),
                })
            })
            .collect()
    }

    // ───────────── staffing checks ─────────────

    /// Check a plan's staffing against the repo's team and CLIs. Errors reject the plan;
    /// notes go back to the Lead as adjustments.
    pub(super) fn check_team(&self, ws: &Workspace, tasks: &mut [TaskInput]) -> Result<Vec<String>> {
        let agents = self.store.agents(&ws.id)?;
        let members: Vec<&Agent> = agents.iter().filter(|a| a.on_staff() && a.role != Role::Reviewer).collect();
        let names = || members.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
        // At home: the Lead gives it no new work (you can call it in), unless the whole team
        // is home; the game never stops the crew from shipping.
        let now = now_ms();
        // Rested first: a tired member is only a second choice (never refused).
        let mut at_work: Vec<&&Agent> = members.iter().filter(|a| super::at_home(a, now).is_none()).collect();
        at_work.sort_by_key(|a| super::tired(a, now));
        let at_work: Vec<String> =
            at_work.iter().map(|a| if super::tired(a, now) { format!("{} (tired)", a.name) } else { a.name.clone() }).collect();
        let mut errors = vec![];
        let mut notes = vec![];
        for t in tasks.iter_mut() {
            if let Some(name) = t.member() {
                match agents.iter().find(|a| a.on_staff() && a.name.eq_ignore_ascii_case(name)) {
                    None if members.is_empty() => errors.push(format!(
                        "task {}: there is no team member '{name}' (the team is empty): use assignee \"consultant\" or omit it",
                        t.id
                    )),
                    None => errors.push(format!("task {}: there is no team member '{name}' (members: {})", t.id, names())),
                    Some(a) if a.role == Role::Reviewer => {
                        errors.push(format!("task {}: {} reviews code; assign one of: {}", t.id, a.name, names()))
                    }
                    Some(a) if !at_work.is_empty() && super::at_home(a, now).is_some() => errors.push(format!(
                        "task {}: {}; assign someone at work ({}) or a consultant",
                        t.id,
                        away_note(a, now),
                        at_work.join(", ")
                    )),
                    Some(a) => {
                        t.assignee = Some(a.name.clone());
                        if !self.cli_usable(a.provider) {
                            notes.push(format!("{}: {}'s CLI ({}) is not signed in; the task waits until it is, or reassign it", t.id, a.name, a.provider.label()));
                        }
                    }
                }
            } else {
                if t.session.is_some() {
                    t.session = None;
                    notes.push(format!("{}: session only applies to team members; ignored", t.id));
                }
                // Every consultant the Lead hires has a job title (its badge).
                if t.consultant_spec().is_none_or(|c| c.title.is_none()) {
                    errors.push(format!("task {}: a consultant needs consultant.title, one of: {}", t.id, ROLE_TITLES.join(", ")));
                }
            }
            if let Some(spec) = t.consultant_spec()
                && let Some(provider) = spec.provider
            {
                if !self.cli_usable(provider) {
                    let ok: Vec<&str> = self.consultant_clis().iter().map(|p| p.as_str()).collect();
                    errors.push(format!("task {}: {} is not available for a consultant (available: {})", t.id, provider.label(), ok.join(", ")));
                } else if let Some(m) = spec.model.as_deref() {
                    let known = self.models_of(provider);
                    let base = m.split(':').next().unwrap_or(m);
                    let claude_id = provider == Provider::Claude && base.starts_with("claude-");
                    if !known.is_empty() && !claude_id && !known.iter().any(|k| k == base || k == m) {
                        notes.push(format!("{}: {} has no model '{m}'; the consultant uses its default (known: {})", t.id, provider.label(), known.join(", ")));
                        if let Some(c) = t.consultant.as_mut() {
                            c.model = None;
                        }
                    }
                }
            }
        }
        if !errors.is_empty() {
            let list: Vec<String> = errors.iter().map(|e| format!("- {e}")).collect();
            bail!("Plan rejected:\n{}\nFix these and call codly_submit_plan again.", list.join("\n"));
        }
        let mut per: Vec<(&str, usize)> = vec![];
        for t in tasks.iter() {
            if let Some(n) = t.member() {
                match per.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(n)) {
                    Some(e) => e.1 += 1,
                    None => per.push((n, 1)),
                }
            }
        }
        for (n, c) in per.into_iter().filter(|(_, c)| *c > 1) {
            notes.push(format!("{n} has {c} tasks in this plan: they run one after the other"));
        }
        Ok(notes)
    }

    /// The member agent a checked assignee names.
    pub(super) fn member_by_name(&self, ws_id: &str, name: &str) -> Option<Agent> {
        self.store.agents(ws_id).ok()?.into_iter().find(|a| a.on_staff() && a.name.eq_ignore_ascii_case(name))
    }

    // ───────────── the Lead's tools ─────────────

    /// `codly_quick_task`: a one-task plan (the user starts it unless plans auto-run). It
    /// does not replace the Lead's pending plan.
    pub(super) async fn quick_task(self: &Arc<Self>, lead_id: &str, args: Value) -> Result<Value> {
        let mut args = args;
        args["id"] = json!("t1"); // one task: the Lead never names it
        let task: TaskInput = serde_json::from_value(args).map_err(|e| anyhow!("Quick task rejected: {e}"))?;
        let summary = task.title.clone();
        self.submit_plan_inner(lead_id, PlanInput { summary, tasks: vec![task], ..Default::default() }, false).await
    }

    /// `codly_reassign`: move a task of this Lead's that has not started.
    pub(super) fn reassign(&self, lead_id: &str, args: &Value) -> Result<Value> {
        let lead = self.agent(lead_id)?;
        let thread = super::lead::thread_of(&lead).to_string();
        let ws_id = self.store.agent_workspace(lead_id)?.ok_or_else(|| anyhow!("no workspace"))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let mut input: TaskInput = serde_json::from_value(args.clone()).map_err(|e| anyhow!("Reassign rejected: {e}"))?;
        let key = args["task"].as_str().map(str::trim).unwrap_or_default().to_string();
        let plan_id = args["plan_id"].as_str().map(str::trim).filter(|p| !p.is_empty());
        let plans: Vec<Plan> = self
            .store
            .plans(&ws.id, 50)?
            .into_iter()
            .filter(|p| p.thread == thread && matches!(p.status, PlanStatus::AwaitingApproval | PlanStatus::Running))
            .filter(|p| plan_id.is_none_or(|id| p.id == id))
            .collect();
        let found: Vec<Task> =
            plans.iter().flat_map(|p| self.store.plan_tasks(&p.id).unwrap_or_default()).filter(|t| t.key.eq_ignore_ascii_case(&key)).collect();
        let task = match found.as_slice() {
            [] => bail!("none of your live plans has a task '{key}'"),
            [t] => t.clone(),
            many => bail!("several of your plans have a task '{key}': pass plan_id ({})", many.iter().filter_map(|t| t.plan_id.clone()).collect::<Vec<_>>().join(", ")),
        };
        if !matches!(task.status, TaskStatus::Pending | TaskStatus::Ready) {
            bail!("{} is {}: only a task that has not started can be reassigned (ask the user to stop it first)", task.key, store::enum_str(&task.status));
        }
        input.id = task.key.clone();
        let mut errors = vec![];
        let mut fixes = vec![];
        planner::check_staffing(&mut input, &mut errors, &mut fixes);
        if !errors.is_empty() {
            bail!("Reassign rejected: {}", errors.join("; "));
        }
        let mut one = [input];
        let notes = self.check_team(&ws, &mut one)?;
        let [input] = one;
        let member = input.member().and_then(|n| self.member_by_name(&ws.id, n));
        let from = task.assignee.as_deref().and_then(|a| self.agent(a).ok()).map_or_else(|| "nobody".into(), |a| a.name);
        let to = match (&member, input.consultant_spec()) {
            (Some(m), _) => m.name.clone(),
            (None, Some(c)) => {
                let on = c.provider.map(|p| format!(" on {}{}", p.label(), c.model.as_deref().map(|m| format!(" {m}")).unwrap_or_default()));
                format!("a {} consultant{}", c.title.as_deref().unwrap_or("new"), on.unwrap_or_default())
            }
            (None, None) => "a consultant".into(),
        };
        let t = self.update_task(&task.id, |t| {
            if !matches!(t.status, TaskStatus::Pending | TaskStatus::Ready) {
                bail!("{} started meanwhile", t.key);
            }
            t.assignee = member.as_ref().map(|m| m.id.clone());
            t.consultant = input.consultant_spec();
            t.fresh_session = input.fresh_session() && member.is_some();
            t.status_reason = None;
            Ok(())
        })?;
        self.decide(
            Some(&ws.id),
            Some(&t.id),
            "route",
            format!("{} reassigned {} from {from} to {to}", lead.name, t.key),
            json!({ "plan_id": t.plan_id, "assignee": t.assignee, "consultant": t.consultant }),
        )?;
        self.kick();
        Ok(json!({ "ok": true, "task": t.key, "assignee": to, "notes": notes }))
    }

    /// `codly_message_member`: a Lead's message to a member (or a working consultant). The
    /// reply is forwarded to that Lead when the member's turn that reads it ends.
    pub(super) async fn message_member(self: &Arc<Self>, lead_id: &str, to: &str, text: &str) -> Result<Value> {
        let lead = self.agent(lead_id)?;
        let thread = super::lead::thread_of(&lead).to_string();
        let ws_id = self.store.agent_workspace(lead_id)?.ok_or_else(|| anyhow!("no workspace"))?;
        let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
        let agents = self.store.agents(&ws.id)?;
        let agent = agents
            .iter()
            .find(|a| (a.member.is_some() || (a.consultant && self.is_live(&a.id))) && a.name.eq_ignore_ascii_case(to))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = agents.iter().filter(|a| a.member.is_some() || a.consultant).map(|a| a.name.as_str()).collect();
                anyhow!("no team member or working consultant is called '{to}' (team: {})", if names.is_empty() { "none".into() } else { names.join(", ") })
            })?;
        if super::at_home(&agent, now_ms()).is_some() {
            bail!("{}", away_note(&agent, now_ms()));
        }
        let hands_off = agent
            .task_id
            .as_deref()
            .and_then(|t| self.store.task(t).ok().flatten())
            .is_some_and(|t| matches!(t.status, TaskStatus::Verifying | TaskStatus::Reviewing | TaskStatus::Merging));
        let prompt = prompts::lead_message(text, hands_off);
        self.chat(Some(&ws.id), &format!("agent:{}", agent.id), lead_id, &prompts::clip(text, 4000), None)?;
        let delivery = if self.is_live(&agent.id) {
            "delivered into its session"
        } else {
            if agent.member.is_none() {
                bail!("{} is not running", agent.name);
            }
            if !self.cli_usable(agent.provider) {
                bail!("{}'s CLI ({}) is not available", agent.name, agent.provider.label());
            }
            self.spawn_member_chat(&ws, &agent).await?;
            if let Some(l) = lock(&self.live).get_mut(&agent.id) {
                l.answer_only = true;
            }
            "started a read-only chat"
        };
        let steer = self.would_steer(&agent.id);
        self.send_prompt(&agent.id, prompt)?;
        if let Some(l) = lock(&self.live).get_mut(&agent.id) {
            let due = if steer { l.turns + 1 } else { l.turns + l.turn_active as u64 + l.queued as u64 };
            l.answers.push(Answer { due, ws_id: ws.id.clone(), thread });
        }
        Ok(json!({ "ok": true, "delivery": delivery, "note": format!("{}'s answer arrives as a later [Codly] message.", agent.name) }))
    }

    /// A read-only conversation in a member's own worktree, resuming its own session.
    pub(super) async fn spawn_member_chat(self: &Arc<Self>, ws: &Workspace, agent: &Agent) -> Result<()> {
        let cwd = self.member_worktree(ws, agent).await?;
        let system = match self.team_member(agent) {
            Some(m) => prompts::member_system(prompts::CHAT_SYSTEM, &m),
            None => prompts::CHAT_SYSTEM.to_string(),
        };
        let req = |session: SessionMode| RunReq {
            agent: agent.clone(),
            role: agent.role,
            cwd: cwd.clone(),
            read_only: true,
            task_id: None,
            session,
            system_prompt: system.clone(),
            workspace_id: Some(ws.id.clone()),
            account: None,
        };
        match &agent.session_id {
            Some(s) => {
                if let Err(e) = self.spawn_run(req(SessionMode::Resume { session_id: s.clone() })).await {
                    tracing::warn!("resuming {} failed ({e}); starting a new session", agent.name);
                    self.spawn_run(req(new_session(agent.provider))).await?;
                }
            }
            None => {
                self.spawn_run(req(new_session(agent.provider))).await?;
            }
        }
        Ok(())
    }

    /// A member's turn ended: forward its reply to each Lead whose message that turn answered,
    /// then shut a run down that was started only to answer.
    pub(super) fn forward_answers(self: &Arc<Self>, ctx_agent: &str, run_id: &str, reply: Option<&str>) {
        let (due, done) = {
            let mut live = lock(&self.live);
            let Some(l) = live.get_mut(ctx_agent).filter(|l| l.run_id == run_id) else { return };
            if l.answers.is_empty() {
                return;
            }
            let turns = l.turns;
            let (due, rest): (Vec<Answer>, Vec<Answer>) = l.answers.drain(..).partition(|a| a.due <= turns);
            l.answers = rest;
            (due, l.answer_only && l.answers.is_empty() && l.queued == 0 && !l.turn_active)
        };
        if due.is_empty() {
            return;
        }
        let name = self.agent(ctx_agent).map_or_else(|_| "A member".into(), |a| a.name);
        let reply = reply.map(str::trim).filter(|r| !r.is_empty()).unwrap_or("(no answer)").to_string();
        for a in due {
            let me = self.clone();
            let body = format!("\n{}", prompts::member_answer(&name, &reply));
            tokio::spawn(async move {
                let Ok(Some(ws)) = me.store.workspace(&a.ws_id) else { return };
                if let Err(e) = me.send_to_lead(&ws, &a.thread, body, None).await {
                    tracing::warn!("member answer not delivered: {e}");
                }
            });
        }
        if done {
            self.stop_run(ctx_agent);
        }
    }

    /// A run exited with answers still owed: tell those Leads.
    pub(super) fn answers_lost(self: &Arc<Self>, agent_id: &str, answers: &[Answer]) {
        if answers.is_empty() {
            return;
        }
        let name = self.agent(agent_id).map_or_else(|_| "The member".into(), |a| a.name);
        for a in answers {
            let me = self.clone();
            let body = format!("\n[Codly] {name} stopped before answering your message.");
            let (ws_id, thread) = (a.ws_id.clone(), a.thread.clone());
            tokio::spawn(async move {
                let Ok(Some(ws)) = me.store.workspace(&ws_id) else { return };
                let _ = me.send_to_lead(&ws, &thread, body, None).await;
            });
        }
    }

    /// Is this live run only an idle chat (no task, nothing in flight, no answer owed)? A
    /// task for that agent may stop it and start.
    pub(super) fn preemptible(&self, agent_id: &str) -> bool {
        lock(&self.live).get(agent_id).is_some_and(|l| {
            l.task_id.is_none() && !l.turn_active && l.queued == 0 && l.answers.is_empty() && !matches!(l.role, Role::Lead | Role::Solo)
        })
    }

    /// Agents a task cannot go to right now: every live run but quiet chats.
    pub(super) fn busy_agents(&self) -> HashSet<Id> {
        let ids: Vec<Id> = lock(&self.live).keys().cloned().collect();
        ids.into_iter().filter(|id| !self.preemptible(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> TeamMember {
        TeamMember { name: name.into(), ..Default::default() }
    }

    #[test]
    fn members_get_ids_and_clashes_are_refused() {
        let list = normalize_team(vec![TeamMember { role: "Backend dev: owns crates/".into(), ..m("  Ana   B ") }], &[]).unwrap();
        assert_eq!(list[0].name, "Ana B");
        assert!(list[0].id.starts_with("ana-b-") && list[0].id.len() == "ana-b-".len() + 4, "{}", list[0].id);
        assert_eq!(list[0].title(), "Backend dev");
        let again = normalize_team(list.clone(), &[]).unwrap();
        assert_eq!(again[0].id, list[0].id, "an id is kept");
        for (bad, why) in [(vec![m("Ana"), m("ana")], "two team members"), (vec![m("Consultant")], "cannot be a member"), (vec![m(" ")], "needs a name")] {
            let e = normalize_team(bad, &[]).unwrap_err().to_string();
            assert!(e.contains(why), "{e}");
        }
        assert!(normalize_team(vec![m("Ana")], &list).is_ok(), "a different name never clashes");
        let e = normalize_team(vec![TeamMember { model: Some("two words".into()), ..m("Leo") }], &[]).unwrap_err().to_string();
        assert!(e.contains("one word"), "{e}");
        assert_eq!(TeamMember { kind: MemberKind::Reviewer, ..m("Rex") }.title(), "Reviewer");
    }
}

/// How `codly_board` and the Lead's team lines show a tired member.
pub(super) const TIRED_NOTE: &str = "tired, prefer someone rested";

/// "Ana is at home until 14:05; the user can call it in".
fn away_note(a: &Agent, now: i64) -> String {
    format!("{} is at home until {}; the user can call it in", a.name, super::at_home(a, now).unwrap_or_default())
}
