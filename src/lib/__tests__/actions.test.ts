import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { LastMerge } from '../gen/LastMerge';
import type { Memory } from '../gen/Memory';
import type { Plan } from '../gen/Plan';
import type { RepoSync } from '../gen/RepoSync';
import { isStarting, keepBranch, mergePlan, recheckTask, runPlan, togglePinMemory, undoMerge } from '../state/actions';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

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

describe('actions that used to report success before the core answered', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    ui.keptPlans = [];
    ui.pinned = [];
    store.memories = [];
  });

  it('"Branch kept" is said, and the merge prompt hidden, only once the core has recorded it', async () => {
    const core = vi.spyOn(api, 'keepPlan').mockRejectedValue(new Error('no such plan'));
    expect(await keepBranch('p1')).toBe(false);
    expect(ui.keptPlans).toEqual([]);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t keep the branch', 'no such plan']]);
    core.mockResolvedValue({} as Plan);
    expect(await keepBranch('p1')).toBe(true);
    expect(ui.keptPlans).toEqual(['p1']);
    expect(toasts.list.map((t) => t.title)).toContain('Branch kept');
  });

  it('a pin shows only once the core has saved it, and unpins again', async () => {
    const m = { id: 'm1', pinned: false } as Memory;
    store.memories = [m];
    const core = vi.spyOn(api, 'pinMemory').mockRejectedValue(new Error('read-only'));
    expect(await togglePinMemory(m)).toBe(false);
    expect(store.memories[0].pinned).toBe(false);
    expect(ui.pinned).toEqual([]);
    expect(toasts.list.map((t) => t.title)).toEqual(['Couldn’t pin the memory']);
    core.mockResolvedValue();
    expect(await togglePinMemory(m)).toBe(true);
    expect(core).toHaveBeenLastCalledWith('m1', true);
    expect(store.memories[0].pinned).toBe(true);
    expect(ui.pinned).toEqual(['m1']);
    expect(await togglePinMemory(store.memories[0])).toBe(true);
    expect(core).toHaveBeenLastCalledWith('m1', false);
    expect(store.memories[0].pinned).toBe(false);
    expect(ui.pinned).toEqual([]);
  });
});

describe('recheckTask', () => {
  afterEach(() => vi.restoreAllMocks());

  it('answers the checks that need your OK, and none when the core refuses', async () => {
    const held = [{ command: 'rm -rf dist && npm test', risks: ['deletes files'] }];
    const core = vi.spyOn(api, 'recheckTask').mockResolvedValueOnce(held).mockRejectedValueOnce(new Error('t1 is done'));
    // The first click allows nothing: the core says what waits.
    expect(await recheckTask('t1')).toEqual(held);
    expect(core).toHaveBeenLastCalledWith('t1', []);
    // A refusal is a toast, not a card that asks for an OK.
    expect(await recheckTask('t1', [held[0].command])).toEqual([]);
    expect(core).toHaveBeenLastCalledWith('t1', [held[0].command]);
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
