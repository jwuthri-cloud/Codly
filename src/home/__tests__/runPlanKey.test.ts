// The "Run plan" button's ⌘↵ keycap (`watchRunPlan`): a new watch starts with it hidden, whatever
// the watch before it last said, so the keycap never shows while ⌘↵ would do nothing.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { watchRunPlan } from '../cards/runPlanKey';

afterEach(() => vi.unstubAllGlobals());

describe('the "Run plan" keycap', () => {
  it('a new watch hides the keycap at once, before it has looked at the button', () => {
    vi.stubGlobal('requestAnimationFrame', () => 1);
    vi.stubGlobal('cancelAnimationFrame', () => {});
    const onCap = vi.fn();
    const stop = watchRunPlan({ isConnected: false } as HTMLElement, 'p2', onCap);
    expect(onCap).toHaveBeenCalledOnce();
    expect(onCap).toHaveBeenCalledWith(false);
    stop();
  });
});
