//! An office as a `codly/office@1` file (PROGRESSION.md §9 step 1): the lot as built (walls,
//! doors, windows, floors, paint, furniture) and, per agent, what a visitor could see: its name,
//! look, rank, stamina, traits and public stats. Nothing of the work: no code, file paths,
//! branch names, prompts, memories or session ids. A read-only viewer (and later visiting and
//! leaderboards) is made from it.

use serde::Serialize;

use super::Inner;
use crate::model::*;

pub const OFFICE_SCHEMA: &str = "codly/office@1";

#[derive(Serialize)]
struct OfficeFile {
    schema: &'static str,
    exported_at: i64,
    /// A cheat was used in this wallet (cheated offices are not ranked).
    cheated: bool,
    lot: Lot,
    crew: Vec<CrewMember>,
}

#[derive(Serialize)]
struct CrewMember {
    name: String,
    role: Role,
    /// Its badge ("Backend"), if it has one.
    title: Option<String>,
    look: Look,
    rank: Rank,
    legend: bool,
    stamina: i64,
    traits: Vec<String>,
    xp: i64,
    stats: PublicStats,
}

/// What its character looks like (`lookFor` in the office: seed, colour and engine).
#[derive(Serialize)]
struct Look {
    seed: String,
    color: i64,
    provider: Provider,
}

#[derive(Serialize)]
struct PublicStats {
    merged: i64,
    commits: i64,
    checks_passed: i64,
    checks_failed: i64,
    reviews_done: i64,
    approved: i64,
    changes_requested: i64,
}

impl Inner {
    /// The open repo's office as a `codly/office@1` file.
    pub(super) fn export_office(&self) -> anyhow::Result<String> {
        let ws = self.require_ws()?;
        let crew = self
            .store
            .agents(&ws.id)?
            .into_iter()
            .map(|a| {
                let s = &a.career.stats;
                CrewMember {
                    stats: PublicStats {
                        merged: s.merged,
                        commits: s.commits,
                        checks_passed: s.checks_passed,
                        checks_failed: s.checks_failed,
                        reviews_done: s.reviews_done,
                        approved: s.approved,
                        changes_requested: s.changes_requested,
                    },
                    look: Look { seed: a.look_seed, color: a.color, provider: a.provider },
                    name: a.name,
                    role: a.role,
                    title: a.title.filter(|t| role_title(t).is_some()),
                    rank: a.career.rank,
                    legend: a.career.legend,
                    stamina: a.career.stamina,
                    traits: crate::career::clean_traits(&a.career.traits),
                    xp: a.career.xp,
                }
            })
            .collect();
        let file = OfficeFile { schema: OFFICE_SCHEMA, exported_at: now_ms(), cheated: super::wallet::wallet(&self.store)?.cheated, lot: self.lot(&ws.id)?, crew };
        Ok(serde_json::to_string_pretty(&file)?)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Core, CoreConfig, StartOptions};
    use crate::model::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_office_exports_without_the_work() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("secret-project");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: root.path().join("data"), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        assert_eq!(core.lot().unwrap().template, "garage", "a new repo's");

        // An agent in the middle of work: its session, worktree, branch, task and a prompt.
        let worktree = root.path().join("data/worktrees/ada-feature-login");
        let session = "sess-7f3e9b2c-4d1a-4c8e-9f00-123456789abc";
        let mut career = Career { xp: 300, rank: Rank::Senior, legend: false, stamina: 42, traits: vec!["bookworm".into()], ..Default::default() };
        career.stats.merged = 12;
        career.stats.checks_passed = 30;
        career.stats.reviews_done = 7;
        career.milestones.push(Milestone { at: 1, text: "Merged codly/ada/feature-login".into() });
        let ada = Agent {
            id: "ada".into(),
            name: "Ada".into(),
            provider: Provider::Claude,
            model: Some("opus".into()),
            role: Role::Worker,
            status: AgentStatus::Working,
            activity: Some(ActivityKind::Edit),
            activity_label: Some("Editing src/secret/payments.rs".into()),
            task_id: Some("task-1".into()),
            run_id: Some("run-1".into()),
            session_id: Some(session.into()),
            worktree: Some(worktree.to_string_lossy().into()),
            billing: Billing::Subscription,
            account: Some("/Users/someone/.claude-work".into()),
            color: 0x3f8077,
            look_seed: "ada".into(),
            thread: None,
            member: None,
            title: Some("Backend".into()),
            consultant: false,
            kept: false,
            archived: false,
            usage: Usage::default(),
            career,
            away_until: None,
            away_since: None,
            tired_since: None,
            created_at: 0,
            updated_at: 0,
        };
        let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind: EventKind::AgentChanged { agent: ada } };
        core.inner.store.append_event(Some(&ws.id), &row).unwrap();
        crate::memory::panel::write_memory_file(&core.inner.data("memory"), "ada", "Ada", "# Ada\n\n- the staging password lives in vault\n").unwrap();

        let json = core.export_office().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["schema"], "codly/office@1");
        assert_eq!(v["lot"]["template"], "garage");
        assert!(v["lot"]["furniture"].as_array().unwrap().iter().any(|p| p["item"] == "office.leadDesk"));
        let a = v["crew"].as_array().unwrap().iter().find(|a| a["name"] == "Ada").unwrap();
        assert_eq!((&a["rank"], &a["stamina"], &a["stats"]["merged"], &a["stats"]["reviews_done"]), (&"senior".into(), &42.into(), &12.into(), &7.into()));
        assert_eq!((&a["look"]["seed"], &a["traits"][0], &a["title"]), (&"ada".into(), &"bookworm".into(), &"Backend".into()));

        // Nothing of the work.
        let real = std::fs::canonicalize(&repo).unwrap();
        for leak in [repo.to_str().unwrap(), real.to_str().unwrap(), worktree.to_str().unwrap(), session, "feature-login", "payments", "staging password", "task-1", "run-1", ".claude-work", "secret-project"] {
            assert!(!json.contains(leak), "the export has {leak:?}:\n{json}");
        }
        for key in ["session_id", "worktree", "repo_path", "memory", "milestones", "goal", "description", "prompt", "branch"] {
            assert!(!json.contains(&format!("\"{key}\"")), "the export has a {key:?} field");
        }
        core.shutdown().await;
    }
}
