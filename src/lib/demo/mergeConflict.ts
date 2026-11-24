// The browser demo's answer to `merge_state`, `resolve_merge_conflict` and a merge that is
// refused. Nobody commits on the demo's main while a plan runs, so nothing conflicts, unless
// the demo starts with `?merge=conflict` (docs/DEMO.md): then your branch changed a file that
// each finished plan changed too. The plan's card shows the conflict, Merge is refused in the
// core's words, and "Have an agent resolve it" gives the plan one more task that resolves it
// (story.ts `resolveConflict`). With `?merge=checks` your branch moved without a conflict, but a
// check of the plan fails on the merge result: Merge is refused, and "Have an agent fix it"
// gives the plan one more task that makes the checks pass (story.ts `fixChecks`). A 1:1 agent's
// branch always merges cleanly here.
import type { MergeState } from '../gen/MergeState';
import type { PrSource } from '../gen/PrSource';
import { mockAgentGit } from './agentgit';
import type { Sim } from './sim';
import type { Story } from './story';

/** What stands between each finished plan and your branch: `?merge=conflict`, `?merge=checks`, or nothing. */
export type MergeTrouble = 'conflict' | 'checks' | null;

export class MergeConflictDemo {
  /** By plan: its checks that failed on the merge result the last time you tried the merge. */
  private failed = new Map<string, string[]>();
  constructor(private trouble: MergeTrouble, private sim: Sim, private story: Story) {}

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
    const conflicts = this.trouble === 'conflict' && !resolved ? this.story.conflictFiles(plan.id) : [];
    const failed_checks = this.trouble === 'checks' && !resolved ? (this.failed.get(plan.id) ?? []) : [];
    // A conflict, or checks that fail on the merge, mean your branch moved since the plan started: one commit in the demo.
    const moved = conflicts.length > 0 || (this.trouble === 'checks' && !resolved);
    return { ...clean, branch, conflicts, resolving: open?.id ?? null, failed_checks, behind: moved ? 1 : 0 };
  }

  /**
   * Before `merge_plan`: a plan that conflicts is not merged, and your branch does not move.
   * Like the core, the plan's checks run on the merge result first (not with `unchecked`: you
   * merge without them): one that fails stops the merge, said in the core's words.
   */
  refuse(planId: string, unchecked = false) {
    const m = this.state({ kind: 'plan', id: planId });
    if (m.conflicts.length) {
      throw new Error(
        `merging into ${m.target} conflicts in ${m.conflicts.join(', ')}; the merge was aborted and your branch is unchanged. ` +
          `An agent (on the plan's card) can resolve it, or you can do it yourself: git merge ${m.branch}`,
      );
    }
    if (this.trouble !== 'checks' || unchecked || !m.behind) return;
    const failed = this.story.failingChecks(planId);
    if (!failed.length) return;
    const sim = this.sim;
    const plan = sim.plans.get(planId)!;
    this.failed.set(planId, failed);
    for (const command of failed) sim.emit({ type: 'verified', command, ok: false, exit_code: 1, tail: '1 failed' });
    const list = failed.map((c) => `\`${c}\``).join(', ');
    const text =
      `Not merged: with "${plan.summary}" merged into ${m.target}, ${list} failed. ${m.target} was not changed. ${m.target} has commits the plan ` +
      `was never checked with; have an agent fix it (on the plan's card), or merge anyway if you know why it fails.`;
    sim.chat(plan.thread, 'codly', text);
    sim.decision('merge', 'rule', text.length > 200 ? `${text.slice(0, 199)}…` : text, { plan_id: planId, target: m.target, failed_checks: failed, passed_checks: [], refused_checks: [] });
    throw new Error(`with the plan merged into ${m.target}, ${list} failed, so nothing was merged and ${m.target} is unchanged`);
  }

  /** `resolve_merge_conflict`: an agent resolves the plan's conflict, or makes its checks pass on the merge. Without either there is nothing to do. */
  resolve(source: PrSource): MergeState {
    const m = this.state(source);
    if (source.kind === 'plan') {
      if (m.conflicts.length) this.story.resolveConflict(source.id, m.target, m.conflicts);
      else if (m.failed_checks.length) this.story.fixChecks(source.id, m.target, m.failed_checks);
      else throw new Error(`${m.branch} already has everything on ${m.target}, so there is nothing to resolve; merge it`);
    }
    return this.state(source);
  }
}
