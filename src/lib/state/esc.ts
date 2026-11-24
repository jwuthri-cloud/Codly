// What Esc does outside a text field and outside an overlay: one step per press. A toast that
// waits for you goes first; then it closes what is open (the find bar and a closed request you
// are reading count); with nothing left to close, on Home, it stops the running turn: the first
// press only asks, a second press stops. In a 1:1 chat with an agent that rests, two presses
// within a moment open "Undo a turn" instead (`rewind`). The composer's own Esc is `composerEsc`.
import { CLOSING } from '../util/modal';
import type { Role } from '../gen/Role';
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
  /** A turn of the conversation's agent can be undone from here now (`mayRewind`). */
  rewind: boolean;
  /** This press is the second of a pair (`escPair`). */
  second: boolean;
};
export type EscStep = 'dismiss-toast' | 'close-artifact' | 'deselect' | 'leave-office' | 'close-find' | 'leave-closed' | 'stop' | 'rewind';

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
  if (s.repeat) return null;
  // No turn runs where a turn can be undone (`mayRewind`): the two never mix.
  return s.rewind && s.second ? 'rewind' : 'stop';
}

/** How soon a second Esc must follow the first to open "Undo a turn". */
export const ESC_PAIR_WINDOW = 600;

/**
 * Is this Esc the second of a pair: `first` (the Esc that may begin one, its thread and when) was
 * pressed in the same thread within `ESC_PAIR_WINDOW`. Only an Esc that did nothing else begins
 * a pair: one that left the empty composer, or found nothing to close or stop.
 */
export function escPair(first: { thread: string; at: number } | null, thread: string, now: number): boolean {
  return !!first && first.thread === thread && now >= first.at && now - first.at <= ESC_PAIR_WINDOW;
}

/** The conversation, as far as "Undo a turn" by Esc Esc goes. */
export type RewindHere = {
  /** A 1:1 chat: in a request's thread the Lead changes no file. */
  oneToOne: boolean;
  /** The conversation's agent, null: none. */
  agent: { role: Role; worktree: string | null } | null;
  /** A turn of the agent runs. */
  turnRuns: boolean;
  /** A request of the agent waits for you. */
  waits: boolean;
  /** The agent is in your terminal: its files may change there at any moment. */
  inTerminal: boolean;
  /** Nothing is typed in the composer. */
  composerEmpty: boolean;
};

/** Esc Esc may open "Undo a turn": a 1:1 chat with an agent that works in a worktree of its own and rests, and an empty composer. */
export function mayRewind(h: RewindHere): boolean {
  const writes = h.agent?.role === 'solo' || h.agent?.role === 'worker';
  return h.oneToOne && writes && !!h.agent?.worktree && !h.turnRuns && !h.waits && !h.inTerminal && h.composerEmpty;
}

/**
 * Esc Esc where "Undo a turn" would open but for what the composer holds (text or files): the
 * list does not open over a draft that ↵ would send, and the composer says why in one quiet
 * line (`REWIND_HELD_LINE`) instead of doing nothing.
 */
export function rewindHeldByDraft(h: RewindHere): boolean {
  return !h.composerEmpty && mayRewind({ ...h, composerEmpty: true });
}

/** The composer's line when Esc Esc did not open "Undo a turn" over a draft. */
export const REWIND_HELD_LINE = 'Undo a turn opens when the message field is empty';

export type ComposerEscStep = 'close-menu' | 'dismiss-toast' | 'close-find' | 'stop';

/**
 * What Esc does in the composer, one step per press: the open menu closes; on an empty line the
 * toast that waits for you goes; the find bar closes; on an empty line the running turn is asked
 * to stop (`escStop`). Null: the key leaves the field, and on an empty line that Esc may begin a
 * pair (`escPair`). A held key dismisses no toast and stops nothing.
 */
export function composerEsc(s: { repeat: boolean; menuOpen: boolean; empty: boolean; toastWaits: boolean; findOpen: boolean; turnRuns: boolean }): ComposerEscStep | null {
  if (s.menuOpen) return 'close-menu';
  if (s.empty && s.toastWaits && !s.repeat) return 'dismiss-toast';
  if (s.findOpen) return 'close-find';
  if (s.empty && s.turnRuns && !s.repeat) return 'stop';
  return null;
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

/** How long the composer's field must stay as it is before ⌫ on it may leave the chat. */
export const BACKSPACE_REST = 600;

/**
 * ⌫ in a 1:1 chat's composer goes back to the request only when the field was already empty
 * when the key went down, on a press that is not a held key's repeat, with no modifier (⌘⌫
 * denies), and `sinceChange` ms after the field last changed: the presses that cleared a draft
 * never leave the chat by running one press too far.
 */
export function backspaceLeaves(k: { empty: boolean; repeat: boolean; modifier: boolean; sinceChange: number }): boolean {
  return k.empty && !k.repeat && !k.modifier && k.sinceChange >= BACKSPACE_REST;
}
