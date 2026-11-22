import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import type { Agent } from '../../lib/gen/Agent';
import type { Rank } from '../../lib/gen/Rank';
import { toasts } from '../../lib/state/toasts.svelte';
import { TrustPick } from '../trustPick.svelte';

describe('the Trust choice in the agent sheet', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
  });
  const core = () => vi.spyOn(api, 'trustAgent').mockImplementation(async (id: string, trust: Rank) => ({ id, name: 'Ana', career: { trust } }) as unknown as Agent);

  it('a click on a higher level changes nothing until you confirm it', async () => {
    const saved = core();
    const pick = new TrustPick();
    await pick.pick('a1', 'intern', 'staff');
    expect(saved).not.toHaveBeenCalled();
    expect(pick.asking).toBe('staff');

    await pick.confirm('a1');
    expect(saved.mock.calls).toEqual([['a1', 'staff']]);
    expect(pick.asking).toBeNull();
    expect(pick.saving).toBe(false);
  });

  it('every raise asks, one step up too', async () => {
    const saved = core();
    const pick = new TrustPick();
    await pick.pick('a1', 'junior', 'senior');
    expect(pick.asking).toBe('senior');
    expect(saved).not.toHaveBeenCalled();
  });

  it('"Keep" leaves the trust as it is', async () => {
    const saved = core();
    const pick = new TrustPick();
    await pick.pick('a1', 'intern', 'senior');
    pick.keep();
    expect(pick.asking).toBeNull();
    await pick.confirm('a1'); // nothing is waiting any more
    expect(saved).not.toHaveBeenCalled();
  });

  it('taking trust back is one click, and drops a raise that was waiting', async () => {
    const saved = core();
    const pick = new TrustPick();
    await pick.pick('a1', 'senior', 'staff');
    await pick.pick('a1', 'senior', 'intern');
    expect(saved.mock.calls).toEqual([['a1', 'intern']]);
    expect(pick.asking).toBeNull();
  });

  it('a click on the level it has changes nothing, and answers "Keep" to a waiting raise', async () => {
    const saved = core();
    const pick = new TrustPick();
    await pick.pick('a1', 'junior', 'staff');
    await pick.pick('a1', 'junior', 'junior');
    expect(pick.asking).toBeNull();
    expect(saved).not.toHaveBeenCalled();
  });

  it('nothing else is saved while a change is on its way to the core', async () => {
    let done!: (a: Agent) => void;
    const saved = vi.spyOn(api, 'trustAgent').mockImplementation(() => new Promise<Agent>((r) => (done = r)));
    const pick = new TrustPick();
    const lowering = pick.pick('a1', 'staff', 'junior');
    expect(pick.saving).toBe(true);
    await pick.pick('a1', 'staff', 'intern');
    expect(saved.mock.calls).toEqual([['a1', 'junior']]);
    done({ id: 'a1', name: 'Ana', career: { trust: 'junior' } } as unknown as Agent);
    await lowering;
    expect(pick.saving).toBe(false);
  });
});
