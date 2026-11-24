// What every dialog owes the keyboard, as one action: focus moves in, Tab wraps inside, Esc
// closes, and focus goes back to where it was when the dialog goes away.
// ponytail: a selector decides what Tab can reach, and focus that falls to <body> (its element was
// removed) comes back only on the next Esc or Tab. Make the background `inert`, or use <dialog>,
// once the build targets Safari 17 (before that the top layer hides toasts, tooltips and menus).
import { on } from 'svelte/events';

/** What marks a closing dialog from the moment its close starts: Svelte makes an element with an
 *  `out:` transition inert while it goes, and takes that off again when it comes back mid-way. */
export const CLOSING = '[inert]';

/**
 * Where Tab moves the focus so that it stays inside the dialog (`stops`: what Tab can reach in
 * it, in order), or null when the browser's own move stays inside. A field inside that took the
 * key (`defaultPrevented`: the IDE's editor indents with Tab) keeps it, and the focus stays.
 */
export function tabWrap<T>(key: Pick<KeyboardEvent, 'shiftKey' | 'defaultPrevented'>, at: unknown, stops: readonly T[], node: T): T | null {
  if (key.defaultPrevented) return null;
  if (at !== node && stops.length && at !== stops[key.shiftKey ? 0 : stops.length - 1]) return null;
  return stops[key.shiftKey ? stops.length - 1 : 0] ?? node;
}

const TABBABLE = 'a[href], button, input, select, textarea, summary, [tabindex]';

export function modal(node: HTMLElement, onclose: () => void) {
  const prev = document.activeElement as HTMLElement | null;
  if (!node.hasAttribute('tabindex')) node.tabIndex = -1;
  if (!node.contains(prev)) node.focus();
  const stops = () => [...node.querySelectorAll<HTMLElement>(TABBABLE)].filter((el) => el.tabIndex >= 0 && !el.matches(':disabled') && el.getClientRects().length);
  // A dialog that is closing (it fades out) takes no key: an Esc then goes to what is under it.
  const closing = () => !!node.closest(CLOSING);
  // `on`, not addEventListener: a field inside that handles its own Esc still goes first.
  const off = on(node, 'keydown', (e) => {
    if (closing()) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    } else if (e.key === 'Tab') {
      const to = tabWrap(e, document.activeElement, stops(), node);
      if (to) {
        e.preventDefault();
        to.focus();
      }
    }
  });
  // The focused control can go away (a button swapped for its confirm): focus is then on <body>
  // and no key reaches the node. The dialog on top still closes on Esc and takes Tab back in.
  const offDoc = on(document, 'keydown', (e) => {
    if ((e.key !== 'Escape' && e.key !== 'Tab') || node.contains(document.activeElement) || closing()) return;
    const open = document.querySelectorAll('[aria-modal="true"]');
    if (open[open.length - 1] !== node) return;
    e.preventDefault();
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    } else {
      const s = stops();
      (s[e.shiftKey ? s.length - 1 : 0] ?? node).focus();
    }
  });
  return {
    destroy() {
      off();
      offDoc();
      // Another dialog may have opened meanwhile: take focus back only if it is still here, or nowhere.
      const at = document.activeElement;
      if (prev?.isConnected && (!at || at === document.body || node.contains(at))) prev.focus();
    },
  };
}
