import { afterEach, describe, expect, it } from 'vitest';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { BACKSPACE_REST, backspaceLeaves, composerEsc, denyKeyInField, ESC_PAIR_WINDOW, ESC_STOP_WINDOW, escPair, escStep, escStop, mayRewind, openOne, REWIND_HELD_LINE, rewindHeldByDraft, type EscState, type RewindHere } from '../state/esc';
import { keyTargetChanges } from '../state/keys';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

const home: EscState = { repeat: false, artifactOpen: false, selection: false, mode: 'blend', room: 'home', popupOpen: false, inOffice: false, findOpen: false, readingClosed: false, toastWaits: false, rewind: false, second: false };

describe('Esc outside a text field', () => {
  it('closes what is open first, one step per press, and only then stops the turn', () => {
    expect(escStep({ ...home, artifactOpen: true, selection: true, mode: 'office' })).toBe('close-artifact');
    expect(escStep({ ...home, selection: true, mode: 'office' })).toBe('deselect');
    expect(escStep({ ...home, mode: 'office' })).toBe('leave-office');
    expect(escStep({ ...home, mode: 'map' })).toBe('leave-office');
    expect(escStep(home)).toBe('stop');
    expect(escStep({ ...home, mode: 'work' })).toBe('stop');
  });

  it('a held key closes but never stops the turn', () => {
    expect(escStep({ ...home, repeat: true, selection: true })).toBe('deselect');
    expect(escStep({ ...home, repeat: true, findOpen: true })).toBe('close-find');
    expect(escStep({ ...home, repeat: true })).toBeNull();
  });

  it('with the find bar open it closes the bar, and the running turn goes on', () => {
    expect(escStep({ ...home, findOpen: true })).toBe('close-find');
    expect(escStep({ ...home, findOpen: true, mode: 'work' })).toBe('close-find');
    // The bar closes before a closed request is left: one step per press.
    expect(escStep({ ...home, findOpen: true, readingClosed: true })).toBe('close-find');
    // What is open on top of the conversation still closes first.
    expect(escStep({ ...home, findOpen: true, artifactOpen: true })).toBe('close-artifact');
    expect(escStep({ ...home, findOpen: true, selection: true })).toBe('deselect');
  });

  it('reading a closed request, it goes back to your requests and stops nothing', () => {
    expect(escStep({ ...home, readingClosed: true })).toBe('leave-closed');
    expect(escStep({ ...home, readingClosed: true, repeat: true })).toBe('leave-closed');
  });

  it('the find bar and a closed request are left alone where Esc belongs to something else', () => {
    // The full office hides the conversation: Esc leaves the office first.
    expect(escStep({ ...home, findOpen: true, mode: 'office' })).toBe('leave-office');
    expect(escStep({ ...home, findOpen: true, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, findOpen: true, inOffice: true })).toBeNull();
    expect(escStep({ ...home, findOpen: true, room: 'plan' })).toBeNull();
    expect(escStep({ ...home, readingClosed: true, popupOpen: true })).toBeNull();
  });

  it('stops only on Home, with no popup open, and not from inside the office', () => {
    expect(escStep({ ...home, room: 'plan' })).toBeNull();
    expect(escStep({ ...home, room: 'terminal' })).toBeNull();
    expect(escStep({ ...home, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, inOffice: true })).toBeNull();
  });
});

describe('Esc and a toast that waits for you', () => {
  it('dismisses the toast first and does nothing else: the sheet, the artifact and the turn wait for the next press', () => {
    expect(escStep({ ...home, toastWaits: true, selection: true })).toBe('dismiss-toast');
    expect(escStep({ ...home, toastWaits: true, artifactOpen: true })).toBe('dismiss-toast');
    expect(escStep({ ...home, toastWaits: true, mode: 'office' })).toBe('dismiss-toast');
    expect(escStep({ ...home, toastWaits: true })).toBe('dismiss-toast');
    // Once it is gone, the next press closes the sheet.
    expect(escStep({ ...home, selection: true })).toBe('deselect');
  });

  it('a dialog, a menu and the office keep their own Esc', () => {
    expect(escStep({ ...home, toastWaits: true, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, toastWaits: true, popupOpen: true, selection: true })).toBe('deselect');
    expect(escStep({ ...home, toastWaits: true, inOffice: true })).toBeNull();
  });

  it('a held key dismisses no more toasts and closes nothing while one waits', () => {
    expect(escStep({ ...home, toastWaits: true, repeat: true, selection: true })).toBeNull();
  });
});

describe('Esc Esc with an agent that rests', () => {
  /** An idle 1:1 agent in its own worktree, nothing typed. */
  const idle: RewindHere = { oneToOne: true, agent: { role: 'solo', worktree: '/wt/ada' }, turnRuns: false, waits: false, inTerminal: false, composerEmpty: true };

  it('the second of two presses opens "Undo a turn"; the first does what it did', () => {
    const here = { ...home, rewind: mayRewind(idle) };
    expect(escStep({ ...here, second: false })).toBe('stop');
    expect(escStep({ ...here, second: true })).toBe('rewind');
    expect(mayRewind({ ...idle, agent: { role: 'worker', worktree: '/wt/t1' } })).toBe(true);
  });

  it('a running turn keeps stopping: the two never mix', () => {
    expect(mayRewind({ ...idle, turnRuns: true })).toBe(false);
    expect(escStep({ ...home, rewind: mayRewind({ ...idle, turnRuns: true }), second: true })).toBe('stop');
  });

  it('nothing new in a request’s thread, or while the agent cannot rest its files', () => {
    expect(mayRewind({ ...idle, oneToOne: false, agent: { role: 'lead', worktree: null } })).toBe(false);
    expect(mayRewind({ ...idle, oneToOne: false })).toBe(false);
    expect(mayRewind({ ...idle, agent: { role: 'solo', worktree: null } })).toBe(false);
    expect(mayRewind({ ...idle, agent: { role: 'reviewer', worktree: '/wt/r' } })).toBe(false);
    expect(mayRewind({ ...idle, agent: null })).toBe(false);
    expect(mayRewind({ ...idle, waits: true })).toBe(false);
    expect(mayRewind({ ...idle, inTerminal: true })).toBe(false);
    expect(mayRewind({ ...idle, composerEmpty: false })).toBe(false);
  });

  it('a held key opens nothing, and what is open still closes first', () => {
    expect(escStep({ ...home, rewind: true, second: true, repeat: true })).toBeNull();
    expect(escStep({ ...home, rewind: true, second: true, selection: true })).toBe('deselect');
    expect(escStep({ ...home, rewind: true, second: true, toastWaits: true })).toBe('dismiss-toast');
    expect(escStep({ ...home, rewind: true, second: true, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, rewind: true, second: true, room: 'plan' })).toBeNull();
  });

  it('a pair is two presses in the same thread within the window', () => {
    expect(escPair(null, 'agent:a1', 1000)).toBe(false);
    expect(escPair({ thread: 'agent:a1', at: 1000 }, 'agent:a1', 1000 + ESC_PAIR_WINDOW)).toBe(true);
    expect(escPair({ thread: 'agent:a1', at: 1000 }, 'agent:a1', 1001 + ESC_PAIR_WINDOW)).toBe(false);
    expect(escPair({ thread: 'agent:a1', at: 1000 }, 'agent:b2', 1001)).toBe(false);
    expect(escPair({ thread: 'agent:a1', at: 1000 }, 'agent:a1', 999)).toBe(false);
  });
});

describe('Esc in the composer', () => {
  const field = { repeat: false, menuOpen: false, empty: true, toastWaits: false, findOpen: false, turnRuns: false };

  it('on an empty line dismisses the toast that waits for you first, then stops the turn', () => {
    expect(composerEsc({ ...field, toastWaits: true, turnRuns: true, findOpen: true })).toBe('dismiss-toast');
    expect(composerEsc({ ...field, turnRuns: true })).toBe('stop');
    // Nothing to do: the key leaves the field (and may begin a pair).
    expect(composerEsc(field)).toBeNull();
  });

  it('the menu closes before all; with text typed the toast and the turn stay', () => {
    expect(composerEsc({ ...field, menuOpen: true, toastWaits: true })).toBe('close-menu');
    expect(composerEsc({ ...field, empty: false, toastWaits: true, turnRuns: true })).toBeNull();
    expect(composerEsc({ ...field, empty: false, findOpen: true })).toBe('close-find');
  });

  it('a held key dismisses no toast and stops nothing', () => {
    expect(composerEsc({ ...field, repeat: true, toastWaits: true, turnRuns: true })).toBeNull();
  });
});

describe('a dialog that is closing', () => {
  /** An element of the page: `closing` says whether it is inside a dialog whose close has started. */
  const el = (closing: boolean) => ({ closest: (selector: string) => (selector === '[inert]' && closing ? {} : null) });

  it('is not open: an Esc while it fades out goes to what is under it', () => {
    expect(openOne([el(true)])).toBe(false);
    expect(openOne([])).toBe(false);
  });

  it('another dialog that is open still counts', () => {
    expect(openOne([el(true), el(false)])).toBe(true);
    expect(openOne([el(false)])).toBe(true);
  });
});

describe('Esc on a running turn', () => {
  it('asks on the first press and stops on a second one for the same thread inside the window', () => {
    expect(escStop(null, 'lead', 1000)).toBe('ask');
    expect(escStop({ thread: 'lead', at: 1000 }, 'lead', 1001)).toBe('stop');
    expect(escStop({ thread: 'lead', at: 1000 }, 'lead', 1000 + ESC_STOP_WINDOW)).toBe('stop');
  });

  it('asks again once the window is over, in another thread, or when the clock went back', () => {
    expect(escStop({ thread: 'lead', at: 1000 }, 'lead', 1001 + ESC_STOP_WINDOW)).toBe('ask');
    expect(escStop({ thread: 'lead', at: 1000 }, 'agent:a1', 1001)).toBe('ask');
    expect(escStop({ thread: 'lead', at: 1000 }, 'lead', 999)).toBe('ask');
  });
});

describe('⌘⌫ in a text field', () => {
  it('denies only from the empty composer: with text in it, and in every other field, it deletes text', () => {
    expect(denyKeyInField({ composer: true, empty: true })).toBe(true);
    expect(denyKeyInField({ composer: true, empty: false })).toBe(false);
    expect(denyKeyInField({ composer: false, empty: true })).toBe(false);
  });
});

describe('G C', () => {
  const plan = (id: string, thread: string, status: Plan['status']) => ({ id, thread, status, created_at: 1 }) as Plan;
  const task = (id: string, planId: string, status: Task['status']) => ({ id, plan_id: planId, status, branch: null }) as Task;

  afterEach(() => {
    store.plans = [];
    store.tasks = [];
    ui.diffTarget = null;
    ui.thread = 'lead';
    ui.go('home');
  });

  it('opens Changes on the plan of the request you are on, not on what you looked at last', () => {
    store.plans = [plan('p1', 'lead', 'done'), plan('p2', 'lead:b', 'running')];
    store.tasks = [task('t1', 'p1', 'done'), task('u1', 'p2', 'done')];
    ui.diffTarget = { kind: 'plan', id: 'p1' };
    ui.thread = 'lead:b';
    expect(keyTargetChanges()).toEqual({ kind: 'plan', id: 'p2' });
  });

  it('leaves the choice to the room for a request with nothing to show yet', () => {
    store.plans = [plan('p1', 'lead', 'done'), plan('p2', 'lead:b', 'awaiting_approval')];
    store.tasks = [task('t1', 'p1', 'done'), task('u1', 'p2', 'pending')];
    ui.diffTarget = { kind: 'plan', id: 'p1' };
    ui.thread = 'lead:b';
    expect(keyTargetChanges()).toBeNull();
    ui.thread = 'lead:draft';
    expect(keyTargetChanges()).toBeNull();
  });

  it('keeps what you looked at last from a 1:1 chat and from another room', () => {
    store.plans = [plan('p2', 'lead:b', 'done')];
    store.tasks = [task('u1', 'p2', 'done')];
    ui.diffTarget = { kind: 'task', id: 'u1' };
    ui.thread = 'agent:a1';
    expect(keyTargetChanges()).toEqual({ kind: 'task', id: 'u1' });
    ui.thread = 'lead:b';
    ui.go('plan');
    expect(keyTargetChanges()).toEqual({ kind: 'task', id: 'u1' });
  });
});

describe('⌘F', () => {
  afterEach(() => {
    ui.mode = 'blend';
    ui.go('home');
  });

  it('finds where a conversation is on screen: Home, but not the full office or the map', () => {
    ui.go('home');
    expect(ui.canFind).toBe(true);
    ui.toggleMode('work');
    expect(ui.canFind).toBe(true);
    ui.toggleMode('office');
    expect(ui.canFind).toBe(false);
    ui.toggleMode('map');
    expect(ui.canFind).toBe(false);
    ui.toggleMode('map'); // back to the blend
    expect(ui.canFind).toBe(true);
    ui.go('plan');
    expect(ui.canFind).toBe(false);
  });
});

describe('Esc Esc over a draft', () => {
  const idle: RewindHere = { oneToOne: true, agent: { role: 'solo', worktree: '/wt/ada' }, turnRuns: false, waits: false, inTerminal: false, composerEmpty: true };

  it('where "Undo a turn" would open, a draft holds it, and the composer says why', () => {
    expect(rewindHeldByDraft({ ...idle, composerEmpty: false })).toBe(true);
    expect(REWIND_HELD_LINE).toBe('Undo a turn opens when the message field is empty');
  });

  it('says nothing where the list would not open anyway, or does open', () => {
    expect(rewindHeldByDraft(idle)).toBe(false);
    expect(rewindHeldByDraft({ ...idle, composerEmpty: false, turnRuns: true })).toBe(false);
    expect(rewindHeldByDraft({ ...idle, composerEmpty: false, oneToOne: false })).toBe(false);
  });
});

describe('⌫ on an empty line of a 1:1 chat', () => {
  const key = { empty: true, repeat: false, modifier: false, sinceChange: BACKSPACE_REST, sinceKey: BACKSPACE_REST };

  it('goes back to the request only on a field that was empty and rested', () => {
    expect(backspaceLeaves(key)).toBe(true);
    // The press that empties the field: it held a letter when the key went down.
    expect(backspaceLeaves({ ...key, empty: false })).toBe(false);
    // The presses right after it, of a hand still clearing the draft.
    expect(backspaceLeaves({ ...key, sinceChange: BACKSPACE_REST - 1 })).toBe(false);
    expect(backspaceLeaves({ ...key, sinceChange: 0 })).toBe(false);
    // Taps that run on past the end of the draft: each one comes soon after the one before.
    expect(backspaceLeaves({ ...key, sinceChange: 5000, sinceKey: BACKSPACE_REST - 1 })).toBe(false);
  });

  it('25 taps at a hand’s pace clear a 19-letter draft and stay in the chat', () => {
    // A press every 150 ms; the field empties with the 19th.
    let length = 19;
    let changedAt = 0;
    let keyAt = -10_000;
    const left: number[] = [];
    for (let n = 1; n <= 25; n++) {
      const now = n * 150;
      if (backspaceLeaves({ empty: !length, repeat: false, modifier: false, sinceChange: now - changedAt, sinceKey: now - keyAt })) left.push(n);
      keyAt = now;
      if (length) {
        length--;
        changedAt = now;
      }
    }
    expect(left).toEqual([]);
    // A pause, then one more press: now it goes back to the request.
    const now = 25 * 150 + BACKSPACE_REST;
    expect(backspaceLeaves({ empty: true, repeat: false, modifier: false, sinceChange: now - changedAt, sinceKey: now - keyAt })).toBe(true);
  });

  it('a held key or a modifier never leaves', () => {
    expect(backspaceLeaves({ ...key, repeat: true, sinceChange: 5000 })).toBe(false);
    expect(backspaceLeaves({ ...key, modifier: true, sinceChange: 5000 })).toBe(false);
    expect(BACKSPACE_REST).toBe(600);
  });
});
