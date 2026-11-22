// A merge that cannot go straight in, from the buttons' side: what stands between a finished
// plan (or a solo agent's branch) and your branch, and the two ways out of a conflict. The state
// is read from the core when a card shows it and after a merge that did not go through; nothing
// is polled. An agent resolves a conflict in Codly's own worktree: your checkout is not touched.
import { api } from '../api';
import type { MergeState } from '../gen/MergeState';
import type { PrSource } from '../gen/PrSource';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

class MergeUi {
  /** What the core last said, by `mergeKey` (missing: not read, or it could not be read). */
  states = $state<Record<string, MergeState>>({});
}
export const mergeUi = new MergeUi();

export const mergeKey = (source: PrSource) => `${source.kind}:${source.id}`;

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** "a.rs, b.rs, c.rs and 9 more". */
export function fileList(files: string[]): string {
  const shown = files.slice(0, 3).join(', ');
  return files.length > 3 ? `${shown} and ${files.length - 3} more` : shown;
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

/** Have an agent resolve it. `who`: the solo agent's name; a plan gets a task of its own. */
export async function resolveMergeConflict(source: PrSource, who: string | null = null): Promise<boolean> {
  const id = toast({ kind: 'progress', title: 'Bringing your branch into Codly’s own worktree…' });
  try {
    const merge = await api.resolveMergeConflict(source);
    mergeUi.states[mergeKey(source)] = merge;
    const body = !merge.conflicts.length
      ? `No conflict after all.${source.kind === 'plan' ? ' The plan’s checks run again; then you can merge it.' : ' You can merge it now.'}`
      : source.kind === 'plan'
        ? 'A task of the plan resolves it. When the plan is checked again, you can merge it.'
        : `${who ?? 'The agent'} resolves it in its conversation. Merge again when its turn ends.`;
    // The plan's new task is on the board: one click away.
    const task = merge.resolving;
    const action = task ? { label: 'Open the task', run: () => ui.selectTask(task) } : undefined;
    toasts.update(id, { kind: 'success', title: merge.conflicts.length ? 'An agent is resolving the conflict' : `${merge.target} is in Codly’s branch now`, body, action });
    return true;
  } catch (e) {
    toasts.update(id, { kind: 'error', title: 'Couldn’t have an agent resolve it', body: msg(e) });
    return false;
  }
}
