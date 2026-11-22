// Workspace trust, from the screen's side: the open repo's own CLI configuration, your choice
// about it, and whether the question is on screen. The question comes when a repo with such
// files opens, and again when the core records that an agent started without files you have
// not decided about (they changed since). Closing it without an answer puts it off for those
// files: agents keep starting without them, and Settings → Safety has the same two choices.
import { api } from '../api';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';
import { shouldAsk } from '../util/workspaceTrust';
import { store } from './store.svelte';
import { toast } from './toasts.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

class Trust {
  /** The open repo's files and your choice, as last read (null: not read yet). */
  value = $state.raw<WorkspaceTrust | null>(null);
  /** Why the last read failed (null: it worked, or none ran yet). */
  error = $state<string | null>(null);
  /** The question is on screen. */
  asking = $state(false);
  /** The office `value` belongs to. */
  private ws: string | null = null;
  /** The fingerprint of the files you closed the question on. */
  private putOff: string | null = null;
  private started = false;

  /** Listen for the core's record that an agent started without files you have not decided about (once). */
  start() {
    if (this.started) return;
    this.started = true;
    store.onEvent((ev) => {
      if (ev.kind.type === 'decision' && ev.kind.decision_type === 'workspace_trust' && ev.kind.method === 'rule') void this.check();
    });
  }

  /** Read the open repo's state. `ask`: put the question on screen when there is something
   *  to decide (Settings → Safety reads without it: it shows the choices itself). */
  async check(ask = true) {
    const ws = store.workspace?.id ?? null;
    if (ws !== this.ws) {
      // The files of the office you left are not this one's.
      this.ws = ws;
      this.value = null;
      this.error = null;
      this.asking = false;
      this.putOff = null;
    }
    if (!ws) return;
    try {
      const value = await api.workspaceTrust();
      if (store.isOpen(ws)) this.show(value, ask);
    } catch (e) {
      if (!store.isOpen(ws)) return;
      // Not read: agents start without the repo's files meanwhile. Settings → Safety retries.
      this.error = msg(e);
      toast({ kind: 'error', title: 'Couldn’t read this repo’s own CLI configuration', body: this.error });
    }
  }

  /** Trust the files shown (`trust` = their fingerprint) or open the repo without them (null). */
  async choose(trust: string | null) {
    const ws = store.workspace?.id ?? null;
    try {
      const value = await api.workspaceTrust({ trust });
      if (store.isOpen(ws)) this.show(value, true);
    } catch (e) {
      // Refused: the files changed since they were shown. Show the new ones.
      toast({ kind: 'error', title: 'Couldn’t save your choice', body: msg(e) });
      await this.check();
    }
  }

  /** Close the question without an answer: not asked again until the files change. */
  notNow() {
    this.putOff = this.value?.fingerprint ?? null;
    this.asking = false;
  }

  private show(value: WorkspaceTrust, ask: boolean) {
    this.value = value;
    this.error = null;
    // A read that does not ask still takes down a question that is answered by now.
    this.asking = (ask || this.asking) && shouldAsk(value, this.putOff);
  }
}

export const workspaceTrust = new Trust();
