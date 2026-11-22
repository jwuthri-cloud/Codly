// What Esc does outside a text field and outside an overlay: one step per press. It closes what
// is open first; with nothing left to close, on Home, it stops the running turn.
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
};
export type EscStep = 'close-artifact' | 'deselect' | 'leave-office' | 'stop';

export function escStep(s: EscState): EscStep | null {
  if (s.artifactOpen) return 'close-artifact';
  if (s.selection) return 'deselect';
  if (s.mode === 'office' || s.mode === 'map') return 'leave-office';
  // A held key never stops a turn: the repeats of the Esc that closed a sheet end here.
  if (s.room === 'home' && !s.popupOpen && !s.inOffice && !s.repeat) return 'stop';
  return null;
}
