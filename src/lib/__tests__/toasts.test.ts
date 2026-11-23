// The time a toast stays (success 4 s, info 6 s, an error until you close it), and what stops it:
// the pointer on the toast, and a folded stack for the toasts it has put away.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { toasts } from '../state/toasts.svelte';

const titles = () => toasts.list.map((t) => t.title);

describe('a toast that a folded stack has put away', () => {
  beforeEach(() => vi.useFakeTimers());
  // The toasts are one singleton: each test puts back what it set.
  afterEach(() => {
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    toasts.putAway([]);
    vi.useRealTimers();
  });

  it('does not go behind an error that stays, and has its whole time when it shows', () => {
    toasts.show({ kind: 'error', title: 'Push failed' });
    const saved = toasts.show({ kind: 'success', title: 'Saved' });
    toasts.putAway([saved]);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Push failed', 'Saved']);

    toasts.putAway([]);
    vi.advanceTimersByTime(3999);
    expect(titles()).toEqual(['Push failed', 'Saved']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual(['Push failed']);
  });

  it('goes on with the time it had left, 1.2 s at least', () => {
    const early = toasts.show({ kind: 'info', title: 'Early' });
    vi.advanceTimersByTime(2000);
    toasts.putAway([early]);
    vi.advanceTimersByTime(60_000);
    toasts.putAway([]);
    vi.advanceTimersByTime(3999);
    expect(titles()).toEqual(['Early']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);

    const late = toasts.show({ kind: 'info', title: 'Late' });
    vi.advanceTimersByTime(5900);
    toasts.putAway([late]);
    toasts.putAway([]);
    vi.advanceTimersByTime(1199);
    expect(titles()).toEqual(['Late']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);
  });

  it('stays put away while other toasts come and go', () => {
    const first = toasts.show({ kind: 'info', title: 'First' });
    toasts.putAway([first]);
    const second = toasts.show({ kind: 'success', title: 'Second' });
    toasts.putAway([first, second]);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['First', 'Second']);

    // The second one shows (the stack's front toast went): only its time runs.
    toasts.putAway([first]);
    vi.advanceTimersByTime(4000);
    expect(titles()).toEqual(['First']);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['First']);
  });

  it('does not start its time when the pointer leaves it', () => {
    const id = toasts.show({ kind: 'success', title: 'Saved' });
    toasts.pause(id);
    toasts.putAway([id]);
    toasts.resume(id);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Saved']);

    toasts.putAway([]);
    vi.advanceTimersByTime(4000);
    expect(titles()).toEqual([]);
  });

  it('gets its time when it shows, also when the work it told of ended while it was put away', () => {
    const id = toasts.show({ kind: 'progress', title: 'Pushing' });
    toasts.putAway([id]);
    toasts.update(id, { kind: 'success', title: 'Pushed' });
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Pushed']);

    toasts.putAway([]);
    vi.advanceTimersByTime(3999);
    expect(titles()).toEqual(['Pushed']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);
  });

  it('stays when it turned into an error while it was put away', () => {
    const id = toasts.show({ kind: 'info', title: 'Pushing' });
    vi.advanceTimersByTime(1000);
    toasts.putAway([id]);
    toasts.update(id, { kind: 'error', title: 'Push failed' });
    toasts.putAway([]);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Push failed']);
  });

  it('starts no time for a toast that went while it was put away', () => {
    const id = toasts.show({ kind: 'success', title: 'Saved' });
    toasts.putAway([id]);
    toasts.dismiss(id);
    toasts.putAway([]);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the pointer on a toast', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    vi.useRealTimers();
  });

  it('stops its time, which goes on when the pointer leaves', () => {
    const id = toasts.show({ kind: 'success', title: 'Saved' });
    vi.advanceTimersByTime(1000);
    toasts.pause(id);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Saved']);
    toasts.resume(id);
    vi.advanceTimersByTime(2999);
    expect(titles()).toEqual(['Saved']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);
  });

  it('does not give an error the time it had before it was an error', () => {
    const id = toasts.show({ kind: 'info', title: 'Pushing' });
    toasts.pause(id);
    toasts.update(id, { kind: 'error', title: 'Push failed' });
    toasts.resume(id);
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Push failed']);
  });

  it('starts no time for a toast that went', () => {
    const id = toasts.show({ kind: 'success', title: 'Saved' });
    toasts.pause(id);
    toasts.dismiss(id);
    toasts.resume(id);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the same message', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    toasts.putAway([]);
    vi.useRealTimers();
  });

  it('shows once: three equal toasts are one toast with a count of 3', () => {
    const first = toasts.show({ kind: 'error', title: 'Couldn’t copy', body: 'The clipboard is not available.' });
    expect(toasts.show({ kind: 'error', title: 'Couldn’t copy', body: 'The clipboard is not available.' })).toBe(first);
    toasts.show({ kind: 'error', title: 'Couldn’t copy', body: 'The clipboard is not available.' });
    expect(toasts.list.map((t) => [t.title, t.count])).toEqual([['Couldn’t copy', 3]]);
  });

  it('another text, title or kind is another toast', () => {
    toasts.show({ kind: 'error', title: 'Couldn’t copy', body: 'The clipboard is not available.' });
    toasts.show({ kind: 'error', title: 'Couldn’t copy', body: 'Nothing to copy.' });
    toasts.show({ kind: 'info', title: 'Couldn’t copy', body: 'Nothing to copy.' });
    expect(toasts.list.map((t) => [t.kind, t.body, t.count])).toEqual([
      ['error', 'The clipboard is not available.', 1],
      ['error', 'Nothing to copy.', 1],
      ['info', 'Nothing to copy.', 1],
    ]);
  });

  it('keeps its place in the stack', () => {
    toasts.show({ kind: 'error', title: 'Push failed' });
    toasts.show({ kind: 'error', title: 'Couldn’t copy' });
    toasts.show({ kind: 'error', title: 'Push failed' });
    expect(titles()).toEqual(['Push failed', 'Couldn’t copy']);
  });

  it('starts its time again, and takes the newest action', () => {
    const run = vi.fn();
    toasts.show({ kind: 'success', title: 'Copied' });
    vi.advanceTimersByTime(3000);
    toasts.show({ kind: 'success', title: 'Copied', action: { label: 'Open', run } });
    vi.advanceTimersByTime(3999);
    expect(titles()).toEqual(['Copied']);
    toasts.list[0].action!.run();
    expect(run).toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);
  });

  it('under the pointer its time stands still, and it gets its whole time when the pointer leaves', () => {
    const id = toasts.show({ kind: 'success', title: 'Copied' });
    vi.advanceTimersByTime(3000);
    toasts.pause(id);
    toasts.show({ kind: 'success', title: 'Copied' });
    vi.advanceTimersByTime(60_000);
    expect(titles()).toEqual(['Copied']);
    toasts.resume(id);
    vi.advanceTimersByTime(3999);
    expect(titles()).toEqual(['Copied']);
    vi.advanceTimersByTime(1);
    expect(titles()).toEqual([]);
  });

  it('a progress toast is never joined: each one tells of its own work', () => {
    toasts.show({ kind: 'progress', title: 'Pushing…' });
    toasts.show({ kind: 'progress', title: 'Pushing…' });
    expect(toasts.list.map((t) => t.count)).toEqual([1, 1]);
  });
});

describe('Dismiss all, and the toast that Esc dismisses', () => {
  afterEach(() => {
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    toasts.putAway([]);
  });

  it('dismisses every toast but a progress toast', () => {
    toasts.show({ kind: 'error', title: 'Push failed' });
    toasts.show({ kind: 'progress', title: 'Merging…' });
    toasts.show({ kind: 'info', title: 'Branch kept' });
    expect(toasts.dismissible.map((t) => t.title)).toEqual(['Push failed', 'Branch kept']);
    toasts.dismissAll();
    expect(titles()).toEqual(['Merging…']);
    expect(toasts.dismissible).toEqual([]);
  });

  it('Esc takes the newest toast that waits for you: an error or one with no time limit, never a progress toast', () => {
    expect(toasts.waiting).toBeUndefined();
    toasts.show({ kind: 'success', title: 'Copied' });
    toasts.show({ kind: 'progress', title: 'Merging…' });
    expect(toasts.waiting).toBeUndefined();
    toasts.show({ kind: 'error', title: 'Push failed' });
    expect(toasts.waiting?.title).toBe('Push failed');
    toasts.show({ kind: 'info', title: 'Not sent', duration: 0 });
    expect(toasts.waiting?.title).toBe('Not sent');
  });

  it('Esc does not take a toast that a folded stack has put away: it is not seen', () => {
    const hidden = toasts.show({ kind: 'info', title: 'Not sent', duration: 0 });
    toasts.show({ kind: 'error', title: 'Push failed' });
    toasts.putAway([hidden]);
    expect(toasts.waiting?.title).toBe('Push failed');
  });
});
