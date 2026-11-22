// What Esc does outside a text field and outside an overlay: one step per press. It closes what
// is open first (the find bar and a closed request you are reading count); with nothing left to
// close, on Home, it stops the running turn.
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
};
export type EscStep = 'close-artifact' | 'deselect' | 'leave-office' | 'close-find' | 'leave-closed' | 'stop';

export function escStep(s: EscState): EscStep | null {
  if (s.artifactOpen) return 'close-artifact';
  if (s.selection) return 'deselect';
  if (s.mode === 'office' || s.mode === 'map') return 'leave-office';
  if (s.room !== 'home' || s.popupOpen || s.inOffice) return null;
  if (s.findOpen) return 'close-find';
  if (s.readingClosed) return 'leave-closed';
  // A held key never stops a turn: the repeats of the Esc that closed a sheet end here.
  return s.repeat ? null : 'stop';
}
