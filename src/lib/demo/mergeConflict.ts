// The browser demo's answer to `merge_state`, `resolve_merge_conflict` and a merge that is
// refused. Nobody commits on the demo's main while a plan runs, so nothing conflicts, unless
// the demo starts with `?merge=conflict` (docs/DEMO.md): then your branch changed a file that
// each finished plan changed too. The plan's card shows the conflict, Merge is refused in the
// core's words, and "Have an agent resolve it" gives the plan one more task that resolves it
// (story.ts `resolveConflict`). A 1:1 agent's branch always merges cleanly here.
import type { MergeState } from '../gen/MergeState';
import type { PrSource } from '../gen/PrSource';
import { mockAgentGit } from './agentgit';
import type { Sim } from './sim';
import type { Story } from './story';

export class MergeConflictDemo {
  /** `on`: `?merge=conflict`. */
  constructor(private on: boolean, private sim: Sim, private story: Story) {}

  /** `merge_state`: what stands between the plan's branch (or the agent's) and your branch. */
  state(source: PrSource): MergeState {
    const sim = this.sim;
    const clean = { target: 'main', repo: sim.workspace?.repo_path ?? '', conflicts: [], resolving: null, failed_checks: [], checks_not_run: null, behind: 0 };
    if (source.kind === 'agent') {
      const branch = mockAgentGit(sim.agents.get(source.id), sim.now()).branch;
      if (!branch) throw new Error('this agent has no worktree yet, so there is nothing to merge');
      return { ...clean, branch };
    }
    const plan = sim.plans.get(source.id);
    const branch = plan?.integration_branch;
    if (!plan || !branch) throw new Error('the plan never ran, so there is nothing to merge');
    // Like the core: a task that resolves the conflict works on the plan's own branch. Done, the
    // conflict is gone; until then it is still there, and that task is the one on it.
    const resolutions = [...sim.tasks.values()].filter((t) => t.plan_id === plan.id && t.branch === branch);
    const resolved = resolutions.some((t) => t.status === 'done');
    const open = resolutions.find((t) => t.status !== 'done' && t.status !== 'cancelled');
    const conflicts = this.on && !resolved ? this.story.conflictFiles(plan.id) : [];
    // A conflict means your branch moved since the plan started: one commit in the demo.
    return { ...clean, branch, conflicts, resolving: open?.id ?? null, behind: conflicts.length ? 1 : 0 };
  }

  /** Before `merge_plan`: a plan that conflicts is not merged, and your branch does not move. */
  refuse(planId: string) {
    const m = this.state({ kind: 'plan', id: planId });
    if (!m.conflicts.length) return;
    throw new Error(
      `merging into ${m.target} conflicts in ${m.conflicts.join(', ')}; the merge was aborted and your branch is unchanged. ` +
        `An agent (on the plan's card) can resolve it, or you can do it yourself: git merge ${m.branch}`,
    );
  }

  /** `resolve_merge_conflict`: an agent resolves the plan's conflict. Without one there is nothing to do. */
  resolve(source: PrSource): MergeState {
    const m = this.state(source);
    if (source.kind === 'plan') {
      if (!m.conflicts.length) throw new Error(`${m.branch} already has everything on ${m.target}, so there is nothing to resolve; merge it`);
      this.story.resolveConflict(source.id, m.target, m.conflicts);
    }
    return this.state(source);
  }
}
