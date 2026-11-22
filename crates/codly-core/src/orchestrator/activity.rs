//! The work under way right now, for the shell: it keeps the Mac awake while there is some
//! (`Settings::keep_awake`), and asks before a quit that would stop agents in the middle of
//! their work (`Settings::confirm_quit`).

use super::*;

/// Work under way at one moment (`Core::activity`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// Agents in a turn, or with a prompt on its way to one.
    pub agents: usize,
    /// Tasks whose work Codly checks or merges right now: no agent is in a turn for those.
    pub checks: usize,
    /// Plans that go on by themselves: one of their tasks runs or is about to start.
    pub plans: usize,
}

impl Activity {
    /// Is anything under way that a Mac going to sleep would stop?
    pub fn busy(&self) -> bool {
        self.agents + self.checks + self.plans > 0
    }
}

/// Does a running plan go on by itself? It does while Codly runs its integration checks, while
/// a task of it is in work, and while one is ready to start (nothing starts in a paused
/// office). A plan whose tasks all wait for you does not: there is nothing to stay awake for.
fn goes_on(plan: &Plan, tasks: &[Task], paused: bool) -> bool {
    plan.status == PlanStatus::Integrating
        || (plan.status == PlanStatus::Running && tasks.iter().any(|t| scheduler::is_active(t.status) || (t.status == TaskStatus::Ready && !paused)))
}

impl Core {
    /// The work under way right now, in every office.
    pub fn activity(&self) -> anyhow::Result<Activity> {
        let i = &self.inner;
        let agents = lock(&i.live).values().filter(|l| !l.stopping && (l.turn_active || l.queued > 0 || l.starting.is_some())).count();
        let checks = lock(&i.gating).len();
        let settings = i.settings();
        let mut plans = 0;
        for plan in i.store.plans_with_status(&[PlanStatus::Running, PlanStatus::Integrating])? {
            let paused = settings.paused || settings.paused_offices.contains(&plan.workspace_id);
            if goes_on(&plan, &i.store.plan_tasks(&plan.id)?, paused) {
                plans += 1;
            }
        }
        Ok(Activity { agents, checks, plans })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(status: &str) -> Plan {
        serde_json::from_value(json!({
            "id": "p", "workspace_id": "ws", "request": "", "summary": "Sign-in page", "assumptions": [], "open_questions": [],
            "status": status, "base_commit": null, "integration_branch": null, "integration_worktree": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    fn task(status: &str) -> Task {
        serde_json::from_value(json!({
            "id": "t", "plan_id": "p", "key": "t1", "title": "Add sign-in", "goal": "", "acceptance": [], "write_scope": [],
            "verify": [], "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
            "attempt": 1, "status_reason": null, "summary": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    #[test]
    fn a_plan_goes_on_while_a_task_of_it_works_or_is_about_to_start() {
        let running = plan("running");
        for status in ["running", "verifying", "reviewing", "changes_requested", "merging", "ready"] {
            assert!(goes_on(&running, &[task("done"), task(status)], false), "{status}");
        }
        // Every task waits for you, is done or waits for one that is stopped: nothing goes on.
        assert!(!goes_on(&running, &[task("done"), task("blocked"), task("failed"), task("pending"), task("cancelled")], false));
        // A paused office starts nothing, and lets the work under way finish.
        assert!(!goes_on(&running, &[task("ready")], true));
        assert!(goes_on(&running, &[task("running")], true));
        // The integration checks are Codly's own work.
        assert!(goes_on(&plan("integrating"), &[task("done")], true));
        for status in ["planning", "awaiting_approval", "done", "failed", "cancelled"] {
            assert!(!goes_on(&plan(status), &[task("ready")], false), "{status}");
        }
    }

    /// A settings file written before the two switches existed loads with both on.
    #[test]
    fn settings_saved_before_the_switches_load_with_both_on() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        let fields = old.as_object_mut().unwrap();
        assert!(fields.remove("keep_awake").is_some() && fields.remove("confirm_quit").is_some());
        let settings: Settings = serde_json::from_value(old).unwrap();
        assert!(settings.keep_awake && settings.confirm_quit);
    }

    #[test]
    fn any_work_under_way_is_busy() {
        assert!(!Activity::default().busy());
        for busy in [Activity { agents: 1, ..Default::default() }, Activity { checks: 1, ..Default::default() }, Activity { plans: 1, ..Default::default() }] {
            assert!(busy.busy(), "{busy:?}");
        }
    }
}
