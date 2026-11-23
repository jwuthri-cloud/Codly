import { afterEach, describe, expect, it } from 'vitest';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { denyKeyInField, ESC_STOP_WINDOW, escStep, escStop, openOne, type EscState } from '../state/esc';
import { keyTargetChanges } from '../state/keys';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

const home: EscState = { repeat: false, artifactOpen: false, selection: false, mode: 'blend', room: 'home', popupOpen: false, inOffice: false, findOpen: false, readingClosed: false, toastWaits: false };

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
