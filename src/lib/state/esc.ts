// What Esc does outside a text field and outside an overlay: one step per press. A toast that
// waits for you goes first; then it closes what is open (the find bar and a closed request you
// are reading count); with nothing left to close, on Home, it stops the running turn: the first
// press only asks, a second press stops.
import { CLOSING } from '../util/modal';
import type { Mode, Room } from './ui.svelte';

export type EscState = {
  /** The key is held down: this press is an auto-repeat. */
  repeat: boolean;
  artifactOpen: boolean;
  selection: boolean;
  mode: Mode;
  room: Room;
  /** A dialog or a menu is open: this Esc closes it. */
  popupOpen: boolean;
  /** The key was pressed inside the office, where Esc clears the office's own selection. */
  inOffice: boolean;
  /** The find bar (⌘F) is open over the conversation. */
  findOpen: boolean;
  /** Home shows the closed requests, not a conversation. */
  readingClosed: boolean;
  /** A toast that does not go by itself shows (an error, or one with no time limit). */
  toastWaits: boolean;
};
export type EscStep = 'dismiss-toast' | 'close-artifact' | 'deselect' | 'leave-office' | 'close-find' | 'leave-closed' | 'stop';

export function escStep(s: EscState): EscStep | null {
  // The toast and nothing else. A held key dismisses no more of them: its repeats do nothing while
  // one waits. A dialog, a menu and the office keep their own Esc.
  if (s.toastWaits && !s.popupOpen && !s.inOffice) return s.repeat ? null : 'dismiss-toast';
  if (s.artifactOpen) return 'close-artifact';
  if (s.selection) return 'deselect';
  if (s.mode === 'office' || s.mode === 'map') return 'leave-office';
  if (s.room !== 'home' || s.popupOpen || s.inOffice) return null;
  if (s.findOpen) return 'close-find';
  if (s.readingClosed) return 'leave-closed';
  // A held key never stops a turn: the repeats of the Esc that closed a sheet end here.
  return s.repeat ? null : 'stop';
}

/**
 * Is one of `found` (the page's dialogs, menus or modal layers) open? One that is closing is
 * not: it fades out for a moment, and an Esc in that moment goes to what is under it.
 */
export const openOne = (found: Iterable<{ closest(selector: string): unknown }>): boolean => [...found].some((el) => !el.closest(CLOSING));

/** How long "Press Esc again to stop …" stays, and how long a second Esc counts as its answer. */
export const ESC_STOP_WINDOW = 2000;

/**
 * Esc on a running turn with nothing left to close. One press never stops work: the first one
 * asks, next to that thread's composer (`asked`: the thread and when). A second press for the
 * same thread inside the window stops the turn; a later one, or one in another thread, asks again.
 */
export function escStop(asked: { thread: string; at: number } | null, thread: string, now: number): 'ask' | 'stop' {
  return asked && asked.thread === thread && now >= asked.at && now - asked.at <= ESC_STOP_WINDOW ? 'stop' : 'ask';
}

/**
 * ⌘⌫ / ⇧⌘⌫ (deny) while the caret is in a text field: only in the composer, and only while it
 * is empty. With text in it the keys keep their text meaning (delete to the start of the line).
 */
export function denyKeyInField(field: { composer: boolean; empty: boolean }): boolean {
  return field.composer && field.empty;
}
