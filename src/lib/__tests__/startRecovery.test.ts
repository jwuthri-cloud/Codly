import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { StartProblem } from '../gen/StartProblem';
import { startRecovery } from '../state/startRecovery.svelte';
import { store } from '../state/store.svelte';

const BACKUP = '/data/backups/codly-1700000000000-v6-daily.sqlite';
const problem: StartProblem = {
  message: 'Codly’s database is damaged and cannot be read (file is not a database).',
  database: '/data/codly.sqlite',
  backups: [{ path: BACKUP, kind: 'daily', created_at: 1_700_000_000_000, schema: 6, bytes: 4_000_000 }],
  start_empty: true,
};

describe('The start-up screen when the database cannot be used', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    startRecovery.problem = null;
    startRecovery.failed = null;
  });

  it('shows what the app says about the database, and nothing when the app has no answer', async () => {
    const ask = vi.spyOn(api, 'startProblem').mockResolvedValue(problem);
    await startRecovery.load();
    expect(startRecovery.problem).toEqual(problem);
    // Another reason, or an app that does not know the command: the plain error screen.
    ask.mockRejectedValue('start_problem is not allowed');
    await startRecovery.load();
    expect(startRecovery.problem).toBeNull();
  });

  it('starts from the backup the app offered, then connects as "Try again" does', async () => {
    const order: string[] = [];
    vi.spyOn(api, 'recoverDatabase').mockImplementation(async (backup) => void order.push(`recover ${backup}`));
    vi.spyOn(store, 'connect').mockImplementation(async () => {
      order.push(`connect, ready=${store.ready}`);
      store.ready = true;
      store.error = null;
    });
    store.ready = true;
    store.error = problem.message;

    const done = startRecovery.recover(BACKUP);
    expect(startRecovery.busy).toBe(true);
    await done;
    // The loading screen was up while the core started again.
    expect(order).toEqual([`recover ${BACKUP}`, 'connect, ready=false']);
    expect([startRecovery.busy, startRecovery.failed, store.error]).toEqual([false, null, null]);

    await startRecovery.recover(null);
    expect(order[2]).toBe('recover null');
  });

  it('says why when the app could not recover, and stays on the screen', async () => {
    vi.spyOn(api, 'recoverDatabase').mockRejectedValue('move /data/codly.sqlite aside: Permission denied');
    const connect = vi.spyOn(store, 'connect');
    store.ready = true;
    store.error = problem.message;

    await startRecovery.recover(BACKUP);
    expect(connect).not.toHaveBeenCalled();
    expect([startRecovery.busy, startRecovery.failed]).toEqual([false, 'move /data/codly.sqlite aside: Permission denied']);
    expect([store.ready, store.error]).toEqual([true, problem.message]);
  });
});
