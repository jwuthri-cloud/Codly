// Whose "Undo a turn" list is open over the composer (Esc Esc, `/undo`, the palette's row), and
// the restore that list and the sheet's Last turn share: a toast says what went back, with Undo.
import { on } from 'svelte/events';
import { api } from '../lib/api';
import { ESC_PAIR_WINDOW } from '../lib/state/esc';
import { toast } from '../lib/state/toasts.svelte';
import { plural } from '../lib/util/format';
import { heldAfterClose, putBackTitle } from './rewind';

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

/**
 * An Esc closed the list, and the focus went back to the composer. The next Esc there, within
 * `ESC_PAIR_WINDOW`, only leaves the composer: it begins no pair in keys.ts, so a third fast press
 * does not open the list again (`heldAfterClose`). It runs after the composer's own Esc (a menu,
 * a toast that waits), on the document, before keys.ts on the window. Armed after the closing
 * Esc is done, so that one is not counted.
 */
export function closedByEsc() {
  const closedAt = Date.now();
  setTimeout(() => {
    const off = on(document, 'keydown', (e) => {
      if (e.key !== 'Escape') return;
      off();
      const field = e.target as HTMLElement;
      if (!heldAfterClose(closedAt, Date.now()) || field.tagName !== 'TEXTAREA' || !field.closest('[aria-label="Composer"]')) return;
      e.stopPropagation();
      if (!e.defaultPrevented) field.blur();
    });
    setTimeout(off, ESC_PAIR_WINDOW);
  });
}

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
