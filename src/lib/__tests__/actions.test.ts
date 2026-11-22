import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { LastMerge } from '../gen/LastMerge';
import type { RepoSync } from '../gen/RepoSync';
import { isStarting, mergePlan, runPlan, undoMerge } from '../state/actions';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';

describe('runPlan', () => {
  afterEach(() => vi.restoreAllMocks());

  it('a plan is starting until the core answers, whichever way it was started, and starts once', async () => {
    let answer!: () => void;
    const core = vi.spyOn(api, 'runPlan').mockReturnValue(new Promise<void>((r) => (answer = r)));
    // ⌘↵, the palette and the buttons all come through here; no button's own flag is involved.
    const first = runPlan('a');
    void runPlan('a');
    expect(core).toHaveBeenCalledTimes(1);
    expect(isStarting('a')).toBe(true);
    expect(isStarting('b')).toBe(false);
    answer();
    await first;
    expect(isStarting('a')).toBe(false);
  });
});

describe('merging into your branch', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    store.planMerges = {};
  });

  it('a plan that started from another branch is merged only after you choose to go on', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue({ target: 'main', message: 'This plan started from feature-x.' });
    const merge = vi.spyOn(api, 'mergePlan').mockResolvedValue('abc1234def');
    await mergePlan('p1');
    expect(merge).not.toHaveBeenCalled();
    const ask = toasts.list.find((t) => t.title === 'Merge into main?');
    expect(ask?.body).toBe('This plan started from feature-x.');
    expect(ask?.action?.label).toBe('Merge anyway');

    ask!.action!.run();
    await vi.waitFor(() => expect(store.mergeOf('p1')?.sha).toBe('abc1234def'));
    // The branch you confirmed goes along: the core refuses if the checkout moved again.
    expect(merge).toHaveBeenCalledWith('p1', 'main');
  });

  it('a plan merges straight away on the branch it started from', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(null);
    const merge = vi.spyOn(api, 'mergePlan').mockResolvedValue('abc1234def');
    await mergePlan('p2');
    expect(merge).toHaveBeenCalledWith('p2', null);
    expect(store.mergeOf('p2')?.sha).toBe('abc1234def');
  });

  it('an undone merge is no longer merged; a refused undo keeps it and says why', async () => {
    store.markPlanMerged('p3', 'abc1234def');
    const merge: LastMerge = { sha: 'abc1234def', branch: 'main', plan_id: 'p3', agent_id: null, blocked: null };
    const after: RepoSync = { branch: 'main', upstream: null, ahead: 0, behind: 0, dirty: [], last_merge: null };
    const undo = vi
      .spyOn(api, 'undoMerge')
      .mockRejectedValueOnce(new Error('the merge cannot be undone: the merge is already on origin/main'))
      .mockResolvedValueOnce(after);

    expect(await undoMerge(merge)).toBeUndefined();
    expect(store.mergeOf('p3')?.sha).toBe('abc1234def');
    expect(toasts.list.some((t) => t.kind === 'error' && t.body?.includes('already on origin/main'))).toBe(true);

    expect(await undoMerge(merge)).toEqual(after);
    expect(undo).toHaveBeenLastCalledWith('abc1234def');
    expect(store.mergeOf('p3')).toBeNull();
  });
});
