import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { store } from '../state/store.svelte';

describe('Try again after a failed start', () => {
  afterEach(() => vi.restoreAllMocks());

  it('asks the app to start its core again, then connects', async () => {
    const order: string[] = [];
    vi.spyOn(api, 'retryCore').mockImplementation(async () => void order.push('retry_core'));
    vi.spyOn(store, 'connect').mockImplementation(async () => {
      order.push('connect');
      store.ready = true;
      store.error = null;
    });
    store.ready = true;
    store.error = 'unable to open database file';

    const done = store.retry();
    expect(store.ready).toBe(false); // the loading screen is back while the core starts
    await done;
    expect(order).toEqual(['retry_core', 'connect']);
    expect([store.ready, store.error]).toEqual([true, null]);
  });

  it('says why when the app refused the retry, and does not pretend to connect', async () => {
    vi.spyOn(api, 'retryCore').mockRejectedValue('retry_core is not allowed');
    const connect = vi.spyOn(store, 'connect');
    await store.retry();
    expect(connect).not.toHaveBeenCalled();
    expect([store.ready, store.error]).toEqual([true, 'retry_core is not allowed']);
  });
});
