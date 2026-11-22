import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Memory } from '../gen/Memory';
import type { Plan } from '../gen/Plan';
import { isStarting, keepBranch, runPlan, togglePinMemory } from '../state/actions';
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
