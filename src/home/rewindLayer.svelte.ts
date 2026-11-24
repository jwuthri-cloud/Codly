// Whose "Undo a turn" list is open over the composer (Esc Esc, `/undo`, the palette's row), and
// the restore that list and the sheet's Last turn share: a toast says what went back, with Undo.
import { api } from '../lib/api';
import { toast } from '../lib/state/toasts.svelte';
import { plural } from '../lib/util/format';
import { putBackTitle } from './rewind';

/** How long the toast after a restore offers Undo. Later, Checkpoints in the agent's sheet still can. */
const UNDO_MS = 10_000;

class RewindLayer {
  /** The agent whose list is open; null: none is. */
  agentId = $state<string | null>(null);
  /** Counts the restores done here and their undos: what shows checkpoints or the last turn reads them again. */
  restored = $state(0);

  open(agentId: string) {
    this.agentId = agentId;
  }

  close() {
    this.agentId = null;
  }

  /**
   * The list of `agentId` left the screen (you went to another conversation or room with it
   * open): it closes, so it never opens again by itself when you come back. A list opened since
   * for another agent stays.
   */
  release(agentId: string) {
    if (this.agentId === agentId) this.agentId = null;
  }
}

export const rewindLayer = new RewindLayer();

const words = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * Put the agent's files back to checkpoint `n` (`path`: that one file, the others stay). The
 * core keeps the present files first; the toast's Undo restores that point. A refusal (a turn
 * runs in that worktree, Codly checks a task there) is thrown in the core's words, and nothing
 * changed.
 */
export async function putBack(agentId: string, n: number, path: string | null, label: string): Promise<void> {
  const r = await api.restoreCheckpoint(agentId, n, path);
  rewindLayer.restored++;
  toast({ kind: 'success', title: putBackTitle(r.files.length, label), duration: UNDO_MS, action: { label: 'Undo', run: () => void undoPutBack(agentId, r.undo) } });
}

async function undoPutBack(agentId: string, undo: number) {
  try {
    const r = await api.restoreCheckpoint(agentId, undo, null);
    rewindLayer.restored++;
    toast({ kind: 'success', title: `${plural(r.files.length, 'file')} back as ${r.files.length === 1 ? 'it was' : 'they were'}` });
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t undo the restore', body: words(e) });
  }
}
