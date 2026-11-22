import { describe, expect, it } from 'vitest';
import { Loader } from '../state/loader.svelte';

describe('Loader', () => {
  it('a failed load says why instead of loading for ever, and Try again recovers', async () => {
    let fail = true;
    const l = new Loader(async () => {
      if (fail) throw new Error('core unreachable');
      return 42;
    });
    expect(await l.load()).toBe(false);
    expect([l.value, l.error, l.loading, l.fails]).toEqual([null, 'core unreachable', false, 1]);
    fail = false;
    expect(await l.load()).toBe(true);
    expect([l.value, l.error, l.loading, l.fails]).toEqual([42, null, false, 0]);
  });

  it('a failed reload keeps the last good value and counts failures in a row', async () => {
    let fail = false;
    const l = new Loader<string>(() => (fail ? Promise.reject('offline') : Promise.resolve('ok')));
    await l.load();
    fail = true;
    await l.load();
    await l.load();
    expect([l.value, l.error, l.fails]).toEqual(['ok', 'offline', 2]);
  });

  it("a save's answer wins over a load that was still on its way", async () => {
    let answer!: (v: string) => void;
    const l = new Loader(() => new Promise<string>((r) => (answer = r)));
    const loading = l.load();
    expect(l.loading).toBe(true);
    l.set('saved');
    answer('stale');
    await loading;
    expect([l.value, l.loading]).toEqual(['saved', false]);
  });
});
