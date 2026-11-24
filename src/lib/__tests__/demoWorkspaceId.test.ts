// The browser demo keeps one id for its repo per browser profile (demo/workspaceId.ts): after a
// reload it opens on the request you left, and the office keeps its look under that id only.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { demoWorkspaceId } from '../demo/workspaceId';

/** A storage that keeps what it is given, like the browser's. */
function fakeStorage(kept: Record<string, string> = {}) {
  const map = new Map(Object.entries(kept));
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
    removeItem: (key: string) => void map.delete(key),
    key: (i: number) => [...map.keys()][i] ?? null,
    get length() {
      return map.size;
    },
  });
  return map;
}

afterEach(() => vi.unstubAllGlobals());

describe('the demo repo’s id', () => {
  it('is the same on every page load of one browser', () => {
    const kept = fakeStorage();
    const first = demoWorkspaceId();
    expect(first).toMatch(/^ws-/);
    expect(demoWorkspaceId()).toBe(first);
    expect(kept.get('codly.demo.workspace')).toBe(first);
  });

  it('keeps the office look of its own id, takes away that of every other demo id, and keeps every other key', () => {
    const kept = fakeStorage({
      'codly.demo.workspace': 'ws-0198-7abc',
      'codly.sims.ws-0198-7abc': 'mine',
      'codly.sims.ws-0197-1111': 'old',
      'codly.sims.ws-0196-2222': 'old',
      'codly.sims.demo': 'kept',
      'codly.request.ws-0198-7abc': 'lead:lockout',
      'codly.theme': 'dark',
    });
    expect(demoWorkspaceId()).toBe('ws-0198-7abc');
    // The demo's agents keep their ids from one load to the next: the look is theirs again.
    // The looks an older demo left under an id of each load go; the request you were on is kept.
    expect([...kept.keys()].sort()).toEqual(['codly.demo.workspace', 'codly.request.ws-0198-7abc', 'codly.sims.demo', 'codly.sims.ws-0198-7abc', 'codly.theme']);
    expect(kept.get('codly.sims.ws-0198-7abc')).toBe('mine');
  });

  it('is a new one for the page when the browser keeps nothing', () => {
    const refuse = () => {
      throw new Error('SecurityError');
    };
    vi.stubGlobal('localStorage', { getItem: refuse, setItem: refuse });
    expect(demoWorkspaceId()).toMatch(/^ws-/);
  });
});
