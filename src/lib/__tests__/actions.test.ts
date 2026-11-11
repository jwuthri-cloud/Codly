import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { isStarting, runPlan } from '../state/actions';

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
