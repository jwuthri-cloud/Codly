// Esc through the app's key map (installKeys): a toast that waits for you goes first, and a dialog
// that is closing no longer holds the key. The page is a stand-in: the dialogs it has, and a key
// pressed outside every text field.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installKeys } from '../state/keys';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

/** A part of the page; `closing`: it is inside a dialog whose close has started. */
const part = (closing = false) => ({ closest: (selector: string) => (selector === '[inert]' && closing ? {} : null) });

let onKey: ((e: KeyboardEvent) => void) | null = null;
let modals: ReturnType<typeof part>[] = [];
let off: () => void = () => {};

/** Esc, pressed on the page's body. */
function esc() {
  const target = { tagName: 'BODY', isContentEditable: false, closest: () => null };
  onKey!({ key: 'Escape', repeat: false, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, defaultPrevented: false, target, preventDefault() {} } as unknown as KeyboardEvent);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('window', { addEventListener: (_: string, fn: (e: KeyboardEvent) => void) => (onKey = fn), removeEventListener: () => (onKey = null) });
  // The modal dialogs are the only dialogs of this page.
  vi.stubGlobal('document', { querySelectorAll: (selector: string) => (selector.includes('aria-modal') || selector.includes('role="dialog"') ? modals : []) });
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
