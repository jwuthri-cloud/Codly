// When you come back, the summary of your time away sits above the view: the row above the
// composer offers a chip to it while it is out of view, and only then.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { awayShown } from '../awayShown.svelte';

/** A row of the thread, as the browser gives it. */
const rowEl = () => ({ isConnected: true, scrollIntoView: vi.fn() }) as unknown as HTMLElement & { scrollIntoView: ReturnType<typeof vi.fn> };

afterEach(() => {
  awayShown.row = null;
});

describe('the chip to the summary of your time away', () => {
  it('shows while the summary is out of view, and goes once you see it', () => {
    const el = rowEl();
    const gone = awayShown.show(el, 93);
    expect(awayShown.hidden).toBe(93);
    awayShown.seen(el, true);
    expect(awayShown.hidden).toBeNull();
    awayShown.seen(el, false);
    expect(awayShown.hidden).toBe(93);
    gone();
    expect(awayShown.hidden).toBeNull();
  });

  it('takes you to the summary', () => {
    const el = rowEl();
    awayShown.show(el, 12);
    awayShown.reveal(true);
    expect(el.scrollIntoView).toHaveBeenCalledWith({ block: 'center', behavior: 'auto' });
  });

  it('a summary of another thread that went does not take away the one on screen', () => {
    const old = rowEl();
    const now = rowEl();
    const goneOld = awayShown.show(old, 40);
    awayShown.show(now, 12);
    goneOld();
    awayShown.seen(old, true);
    expect(awayShown.hidden).toBe(12);
  });
});
