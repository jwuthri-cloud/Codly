// Esc through the app's key map (installKeys): a toast that waits for you goes first, a dialog
// that is closing no longer holds the key, and two presses with an agent that rests open "Undo a turn". The page is a stand-in: the dialogs it has, and a key
// pressed outside every text field.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { rewindLayer } from '../../home/rewindLayer.svelte';
import type { Agent } from '../gen/Agent';
import { ESC_PAIR_WINDOW } from '../state/esc';
import { installKeys } from '../state/keys';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

/** A part of the page; `closing`: it is inside a dialog whose close has started. */
const part = (closing = false) => ({ closest: (selector: string) => (selector === '[inert]' && closing ? {} : null) });

let onKey: ((e: KeyboardEvent) => void) | null = null;
let modals: ReturnType<typeof part>[] = [];
/** The composers on the page whose draft holds files (marked `data-files`). */
let withFiles: object[] = [];
let off: () => void = () => {};

/** Esc on `target`; `used`: a field or a menu already took it (`defaultPrevented`). */
function press(target: object, used = false) {
  onKey!({ key: 'Escape', repeat: false, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, defaultPrevented: used, target, preventDefault() {} } as unknown as KeyboardEvent);
}

/** Esc, pressed on the page's body. */
const esc = (used = false) => press({ tagName: 'BODY', isContentEditable: false, closest: () => null }, used);

/** The composer's text field, holding `value`; `files`: the draft has attached files too. */
function composer(value: string, files = false) {
  return {
    tagName: 'TEXTAREA',
    isContentEditable: false,
    value,
    blurred: false,
    closest: (selector: string) => (selector === '[aria-label="Composer"]' || (files && selector === '[aria-label="Composer"][data-files]') ? {} : null),
    blur() {
      this.blurred = true;
    },
  };
}

/** Esc, pressed in the composer's field. */
const escIn = (field: ReturnType<typeof composer>) => press(field);

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('window', { addEventListener: (_: string, fn: (e: KeyboardEvent) => void) => (onKey = fn), removeEventListener: () => (onKey = null) });
  // The modal dialogs are the only dialogs of this page.
  vi.stubGlobal('document', { querySelectorAll: (selector: string) => (selector.includes('aria-modal') || selector.includes('role="dialog"') ? modals : selector === '[aria-label="Composer"][data-files]' ? withFiles : []) });
  off = installKeys();
  ui.go('home');
  ui.mode = 'blend';
  ui.selection = { kind: 'agent', id: 'a1' };
});

afterEach(() => {
  off();
  for (const t of [...toasts.list]) toasts.dismiss(t.id);
  ui.selection = null;
  modals = [];
  withFiles = [];
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('Esc with the agent sheet open', () => {
  it('with an error toast: the first press closes the toast, the second the sheet', () => {
    toasts.show({ kind: 'error', title: 'Couldn’t copy' });
    esc();
    expect(toasts.list).toEqual([]);
    expect(ui.selection).toEqual({ kind: 'agent', id: 'a1' });
    esc();
    expect(ui.selection).toBeNull();
  });

  it('with a success toast, which goes by itself: Esc closes the sheet and leaves the toast', () => {
    toasts.show({ kind: 'success', title: 'Copied' });
    esc();
    expect(ui.selection).toBeNull();
    expect(toasts.list.map((t) => t.title)).toEqual(['Copied']);
  });

  it('with two errors: one press closes the newest only', () => {
    toasts.show({ kind: 'error', title: 'Push failed' });
    toasts.show({ kind: 'error', title: 'Couldn’t copy' });
    esc();
    expect(toasts.list.map((t) => t.title)).toEqual(['Push failed']);
    expect(ui.selection).not.toBeNull();
  });
});

describe('Esc right after a dialog closed', () => {
  it('a dialog that is closing (it fades out) does not take the key: one press closes the sheet', () => {
    modals = [part(true)];
    esc();
    expect(ui.selection).toBeNull();
  });

  it('a dialog that is open keeps its own Esc', () => {
    modals = [part(false)];
    esc();
    expect(ui.selection).toEqual({ kind: 'agent', id: 'a1' });
  });
});

describe('Esc Esc in a 1:1 chat with an agent that rests', () => {
  const ada = { id: 'ada', name: 'Ada', role: 'solo', status: 'idle', worktree: '/wt/ada', provider: 'claude' } as Agent;

  beforeEach(() => {
    ui.selection = null;
    store.agents = [ada];
    ui.thread = 'agent:ada';
  });
  afterEach(() => {
    rewindLayer.close();
    store.agents = [];
    ui.thread = 'lead';
  });

  it('two presses open "Undo a turn" of that agent', () => {
    esc();
    expect(rewindLayer.agentId).toBeNull();
    vi.advanceTimersByTime(300);
    esc();
    expect(rewindLayer.agentId).toBe('ada');
  });

  it('two presses too far apart open nothing', () => {
    esc();
    vi.advanceTimersByTime(ESC_PAIR_WINDOW + 1);
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });

  it('the Esc that dismissed a toast is not the first of the pair', () => {
    toasts.show({ kind: 'error', title: 'Couldn’t copy' });
    esc();
    esc();
    expect(toasts.list).toEqual([]);
    expect(rewindLayer.agentId).toBeNull();
    esc();
    expect(rewindLayer.agentId).toBe('ada');
  });

  it('the first Esc in the empty composer begins the pair: it lets go of the field, the second opens the list', () => {
    const field = composer('');
    escIn(field);
    expect(field.blurred).toBe(true);
    expect(rewindLayer.agentId).toBeNull();
    vi.advanceTimersByTime(300);
    esc();
    expect(rewindLayer.agentId).toBe('ada');
  });

  it('an Esc in a composer that holds text begins no pair', () => {
    escIn(composer('fix the header'));
    vi.advanceTimersByTime(300);
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });

  it('files attached with no text are a draft too: an Esc in that composer begins no pair', () => {
    const field = composer('', true);
    escIn(field);
    expect(field.blurred).toBe(true);
    vi.advanceTimersByTime(300);
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });

  it('with files in the composer, two presses outside it open nothing either', () => {
    withFiles = [{}];
    esc();
    vi.advanceTimersByTime(300);
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });

  it('an Esc the composer used (a menu closed) begins no pair', () => {
    escIn(composer(''));
    vi.advanceTimersByTime(100);
    esc(true);
    vi.advanceTimersByTime(100);
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });

  it('in a request’s thread two presses open nothing', () => {
    ui.thread = 'lead';
    esc();
    esc();
    expect(rewindLayer.agentId).toBeNull();
  });
});
