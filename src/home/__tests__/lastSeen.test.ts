// When you last had each thread on screen is kept across a restart: one key per office, the 50
// threads you left most recently, and a closed request leaves it.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Workspace } from '../../lib/gen/Workspace';
import { store } from '../../lib/state/store.svelte';
import { ui } from '../../lib/state/ui.svelte';
import { capSeen, forgetSeen, loadSeen, readSeen, seenAt, SEEN_CAP, stampable, stampSeen } from '../lastSeen';

/** A storage that keeps what it is given, like the browser's. */
function fakeStorage(kept: Record<string, string> = {}) {
  const map = new Map(Object.entries(kept));
  vi.stubGlobal('localStorage', { getItem: (key: string) => map.get(key) ?? null, setItem: (key: string, value: string) => void map.set(key, value) });
  return map;
}

beforeEach(() => {
  ui.lastSeen = {};
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('lastSeen', () => {
  it('one key per office, read back after a restart', () => {
    const kept = fakeStorage();
    stampSeen('acme', 'lead', 100);
    stampSeen('acme', 'agent:ada', 200);
    stampSeen('billing', 'lead', 300);
    expect([...kept.keys()]).toEqual(['codly.seen.acme', 'codly.seen.billing']);
    // A restart: nothing in memory, the storage as it was.
    ui.lastSeen = {};
    loadSeen('acme');
    expect([seenAt('acme', 'lead'), seenAt('acme', 'agent:ada'), seenAt('billing', 'lead'), seenAt(null, 'lead')]).toEqual([100, 200, undefined, undefined]);
  });

  it('keeps the 50 threads you left most recently', () => {
    const many = Object.fromEntries(Array.from({ length: SEEN_CAP + 5 }, (_, i) => [`lead:${i}`, i]));
    const kept = capSeen(many);
    expect(Object.keys(kept)).toHaveLength(SEEN_CAP);
    expect(kept['lead:0']).toBeUndefined();
    expect(kept[`lead:${SEEN_CAP + 4}`]).toBe(SEEN_CAP + 4);
  });

  it('a closed request leaves the map, in memory and in the storage', () => {
    const kept = fakeStorage();
    stampSeen('acme', 'lead', 100);
    stampSeen('acme', 'lead:x', 200);
    forgetSeen('acme', ['lead:x']);
    expect(seenAt('acme', 'lead:x')).toBeUndefined();
    expect(JSON.parse(kept.get('codly.seen.acme')!)).toEqual({ lead: 100 });
  });

  it('a request that is not open any more is not stamped again: it would come back in the map', () => {
    const kept = fakeStorage();
    const open = new Set(['lead', 'agent:ada']);
    const isOpen = (_office: string, thread: string) => open.has(thread);
    stampSeen('acme', 'lead:x', 100, isOpen);
    stampSeen('acme', 'lead', 200, isOpen);
    expect([seenAt('acme', 'lead:x'), seenAt('acme', 'lead')]).toEqual([undefined, 200]);
    expect(JSON.parse(kept.get('codly.seen.acme')!)).toEqual({ lead: 200 });
  });

  it('only a request of the open office must be one of its tabs', () => {
    // No office is open in this test, and its request list is empty.
    expect(stampable('acme', 'lead:x')).toBe(true);
    store.workspace = { id: 'acme' } as Workspace;
    try {
      expect(stampable('acme', 'lead:x')).toBe(false);
      expect(stampable('acme', 'agent:ada')).toBe(true);
      expect(stampable('billing', 'lead:x')).toBe(true);
    } finally {
      store.workspace = null;
    }
  });

  it('a storage that holds something else, or none at all, gives an empty map', () => {
    expect([readSeen(null), readSeen('not json'), readSeen('[1,2]'), readSeen('{"lead":"x","lead:a":5}')]).toEqual([{}, {}, {}, { 'lead:a': 5 }]);
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    });
    loadSeen('acme');
    stampSeen('acme', 'lead', 100);
    // Kept for this session only.
    expect(seenAt('acme', 'lead')).toBe(100);
  });
});
