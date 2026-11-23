// The start-up error screen when the database cannot be used (src-tauri/src/boot.rs): what the
// app says about it, and the ways out it offers. The app moves the database aside for each of
// them; nothing here, and nothing there, deletes it.
import { api } from '../api';
import type { StartProblem } from '../gen/StartProblem';
import { store } from './store.svelte';

class StartRecovery {
  /** What the app says about the database. null = the start failed for another reason. */
  problem = $state.raw<StartProblem | null>(null);
  /** A recovery is running. */
  busy = $state(false);
  /** Why the last recovery did not work. */
  failed = $state<string | null>(null);

  /** Ask the app whether the failed start is about the database. */
  async load(): Promise<void> {
    try {
      this.problem = await api.startProblem();
    } catch {
      // No answer: the screen keeps the start error it already shows, with "Try again".
      this.problem = null;
    }
  }

  /** Start from `backup` (the path of one the app offered) or with no data (null), then connect again as "Try again" does. */
  async recover(backup: string | null): Promise<void> {
    this.busy = true;
    this.failed = null;
    try {
      await api.recoverDatabase(backup);
    } catch (e) {
      this.failed = e instanceof Error ? e.message : String(e);
      return;
    } finally {
      this.busy = false;
    }
    store.ready = false; // the loading screen, while the core starts
    await store.connect();
  }
}

export const startRecovery = new StartRecovery();
