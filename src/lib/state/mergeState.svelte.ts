// A merge that cannot go straight in, from the buttons' side: what stands between a finished
// plan (or a solo agent's branch) and your branch, and the two ways out of a conflict. The state
// is read from the core when a card shows it and after a merge that did not go through; nothing
// is polled. An agent resolves a conflict in Codly's own worktree: your checkout is not touched.
import { api } from '../api';
import type { MergeState } from '../gen/MergeState';
import type { MergeWarning } from '../gen/MergeWarning';
import type { PrSource } from '../gen/PrSource';
import type { TaskStatus } from '../gen/TaskStatus';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

class MergeUi {
  /** What the core last said, by `mergeKey` (missing: not read, or it could not be read). */
  states = $state<Record<string, MergeState>>({});
  /** The plans whose merge is under way (`mergePlan`): no button starts a second one. */
  merging = $state<string[]>([]);
}
export const mergeUi = new MergeUi();

export const mergeKey = (source: PrSource) => `${source.kind}:${source.id}`;

/**
 * The branch that a finished plan (or a solo agent's branch) conflicts with, while the core's
 * last word is that the conflict is not resolved. Null: it can merge, as far as Codly knows.
 * A header or a button that offers the merge reads this first: it must not say "ready".
 */
export function conflictsWith(source: PrSource): string | null {
  const merge = mergeUi.states[mergeKey(source)];
  return merge?.conflicts.length ? merge.target : null;
}

/**
 * What stands between a finished plan and your branch, as the core last said (null: not read,
 * or nothing): files that conflict, checks that failed on the merge result, or checks that
 * could not run there. While one stands, "Merge into" is a plain button: the way out (an agent
 * resolves it, an agent fixes the checks) is the one main button, and checks that could not run
 * have no main button at all. Pure.
 */
export function mergeBlock(merge: Pick<MergeState, 'conflicts' | 'failed_checks' | 'checks_not_run'> | null | undefined): 'conflict' | 'checks-failed' | 'checks-not-run' | null {
  if (!merge) return null;
  if (merge.conflicts.length) return 'conflict';
  if (merge.failed_checks.length) return 'checks-failed';
  return merge.checks_not_run ? 'checks-not-run' : null;
}

/** A task that is not at work any more: done, failed or cancelled. One the window does not know
 *  yet (its row comes after the answer that named it) is not over. Pure. */
export const taskOver = (status: TaskStatus | undefined): boolean => status === 'done' || status === 'failed' || status === 'cancelled';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** "a.rs, b.rs, c.rs and 9 more". */
export function fileList(files: string[]): string {
  const shown = files.slice(0, 3).join(', ');
  return files.length > 3 ? `${shown} and ${files.length - 3} more` : shown;
}

/** "This plan is behind main by 3 commits": your branch has commits the Codly branch was not
 *  built on, and whether git can bring the two together by itself. Said before you merge. */
export function behindLine(merge: Pick<MergeState, 'target' | 'behind' | 'conflicts'>, who: string | null = null): string {
  const commits = merge.behind === 1 ? '1 commit' : `${merge.behind} commits`;
  const end = merge.conflicts.length ? '.' : ': they merge without a conflict.';
  return `${who ? `${who}’s branch` : 'This plan'} is behind ${merge.target} by ${commits}${end}`;
}

/** One shell word: as it is when it needs no quotes, else in single quotes. */
function word(s: string): string {
  return /^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

/** I resolve it myself: the commands that merge the Codly branch into your branch by hand. */
export function mergeByHand(merge: Pick<MergeState, 'repo' | 'branch' | 'conflicts'>): string {
  return [
    `cd ${word(merge.repo)}`,
    `git merge ${word(merge.branch)}`,
    '# fix the conflicts in the files git names, then:',
    `git add ${merge.conflicts.map(word).join(' ')}`,
    'git commit',
  ].join('\n');
}

/** Read it again. When it cannot be read (no repository open, a detached HEAD) there is nothing
 *  to show: the merge itself still says what is wrong. */
export async function loadMergeState(source: PrSource): Promise<MergeState | null> {
  const key = mergeKey(source);
  try {
    const merge = await api.mergeState(source);
    mergeUi.states[key] = merge;
    return merge;
  } catch {
    delete mergeUi.states[key];
    return null;
  }
}

/** Have an agent resolve it. `who`: the solo agent's name; a plan gets a task of its own. When
 *  your checkout is not on the branch the plan started from, nothing starts yet: a toast says
 *  what that brings and offers to go on (`into` is then the branch you confirmed). The same for
 *  a plan whose checks failed on the merge result with no conflict: its task makes them pass. */
export async function resolveMergeConflict(source: PrSource, who: string | null = null, into: string | null = null): Promise<boolean> {
  if (source.kind === 'plan' && !into) {
    let warning: MergeWarning | null;
    try {
      warning = await api.mergePlanWarning(source.id);
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t start the agent', body: msg(e) });
      return false;
    }
    if (warning) {
      const target = warning.target;
      const goOn = { label: 'Go on', run: () => void resolveMergeConflict(source, who, target) };
      // What the agent is for, from what the core last said about this merge: files that
      // conflict, or checks that failed on the merge result with no conflict.
      const last = mergeUi.states[mergeKey(source)];
      const opening = last?.conflicts.length ? 'To resolve the conflict, an agent' : last?.failed_checks.length ? 'To make the checks pass, an agent' : 'An agent';
      const body = `${warning.message} ${opening} brings ${target} into the plan’s branch: from then on the plan builds on ${target}.`;
      toast({ kind: 'attention', title: `Bring ${target} into this plan?`, body, action: goOn, duration: 0 });
      return false;
    }
  }
  const id = toast({ kind: 'progress', title: 'Bringing your branch into Codly’s own worktree…' });
  try {
    const merge = await api.resolveMergeConflict(source, into);
    mergeUi.states[mergeKey(source)] = merge;
    // The plan's new task is on the board: one click away. (A solo agent has no task.)
    const task = source.kind === 'plan' ? merge.resolving : null;
    // No conflict, and still a task: it makes the checks that failed on the merge result pass.
    const fixing = !!task && !merge.conflicts.length;
    const body = fixing
      ? 'A task of the plan makes the checks pass with your branch merged in. When the plan is checked again, you can merge it.'
      : !merge.conflicts.length
        ? `No conflict after all.${source.kind === 'plan' ? ' The plan’s checks run again; then you can merge it.' : ' You can merge it now.'}`
        : source.kind === 'plan'
          ? 'A task of the plan resolves it. When the plan is checked again, you can merge it.'
          : `${who ?? 'The agent'} resolves it in its conversation. Merge again when its turn ends.`;
    const title = fixing ? 'An agent is fixing the checks' : merge.conflicts.length ? 'An agent is resolving the conflict' : `${merge.target} is in Codly’s branch now`;
    // The toast says an agent is on it: it goes once that task is over.
    const action = task ? { label: 'Open the task', run: () => ui.selectTask(task), gone: () => taskOver(store.taskMap.get(task)?.status) } : undefined;
    toasts.update(id, { kind: 'success', title, body, action });
    return true;
  } catch (e) {
    toasts.update(id, { kind: 'error', title: 'Couldn’t start the agent', body: msg(e) });
    return false;
  }
}

/** Stop a solo agent's conflict resolution: the merge is taken out of its worktree. */
export async function stopResolvingMerge(agentId: string, who: string | null = null): Promise<boolean> {
  try {
    mergeUi.states[mergeKey({ kind: 'agent', id: agentId })] = await api.stopResolvingMerge(agentId);
    toast({ kind: 'success', title: 'Stopped resolving', body: `${who ?? 'The agent'}’s branch is as it was before the merge. The conflict is still there.` });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t stop resolving', body: msg(e) });
    return false;
  }
}
