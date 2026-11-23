// The catalog's thumbnail renderer holds a WebGL context. It is made when a thumbnail is drawn
// and released a few seconds after the last one, not kept for the rest of the session.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { idleLease } from '../gfx/lease';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function lease() {
  let made = 0;
  const released: number[] = [];
  const take = idleLease(
    () => ++made,
    (n) => released.push(n),
    4000,
  );
  return { take, made: () => made, released };
}

describe('idleLease', () => {
  it('makes nothing until the first use, and one thing for a burst of uses', () => {
    const l = lease();
    expect(l.made()).toBe(0);
    expect([l.take(), l.take(), l.take()]).toEqual([1, 1, 1]);
    expect(l.made()).toBe(1);
  });

  it('releases once, after the idle time has passed since the last use', () => {
    const l = lease();
    l.take();
    vi.advanceTimersByTime(3000);
    l.take(); // still in use: the wait starts again
    vi.advanceTimersByTime(3999);
    expect(l.released).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(l.released).toEqual([1]);
    vi.advanceTimersByTime(60_000);
    expect(l.released).toEqual([1]);
  });

  it('makes a new one for the next use after a release', () => {
    const l = lease();
    l.take();
    vi.advanceTimersByTime(4000);
    expect(l.take()).toBe(2);
    vi.advanceTimersByTime(4000);
    expect(l.released).toEqual([1, 2]);
  });
});
