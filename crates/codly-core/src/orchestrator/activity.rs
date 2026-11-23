//! The work under way right now, for the shell: it keeps the Mac awake while there is some
//! (`Settings::keep_awake`), and asks before a quit that would stop agents in the middle of
//! their work (`Settings::confirm_quit`).
//!
//! Work that waits for you is not work under way: a turn that asked you to approve an action or
//! to answer a question, a worker you or the circuit breaker paused, a task that cannot start
//! because the office is paused or a budget is used up. Nothing of that goes on before you act,
//! so a Mac that is left alone with it may go to sleep, and Codly quits without a question.

use super::*;

/// Work under way at one moment (`Core::activity`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// Agents in a turn, or with a prompt on its way to one. Not one whose turn waits for your
    /// answer to an approval or a question.
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
/// a task of it is in work, and while one is ready to start. No task starts while the office is
/// paused or a budget is used up (`held`), and a task whose worker waits for you (`waiting`) is
/// not in work. A plan whose tasks all wait for you does not go on: there is nothing to stay
/// awake for.
fn goes_on(plan: &Plan, tasks: &[Task], held: bool, waiting: &HashSet<Id>) -> bool {
    let moves = |t: &Task| if t.status == TaskStatus::Ready { !held } else { scheduler::is_active(t.status) && !waiting.contains(&t.id) };
    plan.status == PlanStatus::Integrating || (plan.status == PlanStatus::Running && tasks.iter().any(moves))
}

impl Core {
    /// The work under way right now, in every office.
    pub fn activity(&self) -> anyhow::Result<Activity> {
        let i = &self.inner;
        // Every run: its agent, its task, and whether it is in a turn or about to be.
        let runs: Vec<(Id, Id, Option<Id>, bool)> = lock(&i.live)
            .iter()
            .filter(|(_, l)| !l.stopping)
            .map(|(agent, l)| (agent.clone(), l.run_id.clone(), l.task_id.clone(), l.turn_active || l.queued > 0 || l.starting.is_some()))
            .collect();
        let mut agents = 0;
        // The tasks nobody works on until you answer their worker or resume it.
        let mut waiting: HashSet<Id> = HashSet::new();
        for (agent, run, task, in_turn) in runs {
            // An approval, or a question the agent asked you: its turn stands still until you answer.
            let asks = !i.store.pending_approvals_for_run(&run)?.is_empty();
            if in_turn && !asks {
                agents += 1;
            }
            if asks || i.control_holding(&agent) {
                waiting.extend(task);
            }
        }
        let checks = lock(&i.gating).len();
        let settings = i.settings();
        let over_budget = i.budget_hold().is_some();
        let mut plans = 0;
        for plan in i.store.plans_with_status(&[PlanStatus::Running, PlanStatus::Integrating])? {
            let held = over_budget || settings.paused || settings.paused_offices.contains(&plan.workspace_id);
            if goes_on(&plan, &i.store.plan_tasks(&plan.id)?, held, &waiting) {
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

    fn task(id: &str, status: &str) -> Task {
        serde_json::from_value(json!({
            "id": id, "plan_id": "p", "key": "t1", "title": "Add sign-in", "goal": "", "acceptance": [], "write_scope": [],
            "verify": [], "depends_on": [], "status": status, "assignee": null, "agent_hint": null, "branch": null, "worktree": null,
            "attempt": 1, "status_reason": null, "summary": null, "created_at": 0, "updated_at": 0,
        }))
        .unwrap()
    }

    #[test]
    fn a_plan_goes_on_while_a_task_of_it_works_or_is_about_to_start() {
        let running = plan("running");
        let nobody = HashSet::new();
        for status in ["running", "verifying", "reviewing", "changes_requested", "merging", "ready"] {
            assert!(goes_on(&running, &[task("a", "done"), task("b", status)], false, &nobody), "{status}");
        }
        // Every task waits for you, is done or waits for one that is stopped: nothing goes on.
        let stopped = [task("a", "done"), task("b", "blocked"), task("c", "failed"), task("d", "pending"), task("e", "cancelled")];
        assert!(!goes_on(&running, &stopped, false, &nobody));
        // A paused office, or a budget that is used up, starts nothing, and lets the work under
        // way finish.
        assert!(!goes_on(&running, &[task("a", "ready")], true, &nobody));
        assert!(goes_on(&running, &[task("a", "running")], true, &nobody));
        // The integration checks are Codly's own work.
        assert!(goes_on(&plan("integrating"), &[task("a", "done")], true, &nobody));
        for status in ["planning", "awaiting_approval", "done", "failed", "cancelled"] {
            assert!(!goes_on(&plan(status), &[task("a", "ready")], false, &nobody), "{status}");
        }
    }

    /// A task whose worker waits for your answer, or for your Resume, is in nobody's hands.
    #[test]
    fn a_task_whose_worker_waits_for_you_does_not_keep_its_plan_going() {
        let running = plan("running");
        let waiting = HashSet::from(["a".to_string()]);
        for status in ["running", "changes_requested", "reviewing"] {
            assert!(!goes_on(&running, &[task("a", status), task("b", "done")], false, &waiting), "{status}");
            assert!(goes_on(&running, &[task("a", status), task("b", "running")], false, &waiting), "another task is in work ({status})");
        }
        assert!(goes_on(&running, &[task("a", "running"), task("b", "ready")], false, &waiting), "another task is about to start");
        assert!(!goes_on(&running, &[task("a", "running"), task("b", "ready")], true, &waiting));
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
