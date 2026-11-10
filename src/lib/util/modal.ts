// What every dialog owes the keyboard, as one action: focus moves in, Tab wraps inside, Esc
// closes, and focus goes back to where it was when the dialog goes away.
// ponytail: a selector decides what Tab can reach, and focus that falls to <body> (its element was
// removed) is not pulled back in. Make the background `inert`, or use <dialog>, once the build
// targets Safari 17 (before that the top layer hides toasts, tooltips and menus).
import { on } from 'svelte/events';

const TABBABLE = 'a[href], button, input, select, textarea, summary, [tabindex]';

export function modal(node: HTMLElement, onclose: () => void) {
  const prev = document.activeElement as HTMLElement | null;
  if (!node.hasAttribute('tabindex')) node.tabIndex = -1;
  if (!node.contains(prev)) node.focus();
  // `on`, not addEventListener: a field inside that handles its own Esc still goes first.
  const off = on(node, 'keydown', (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    } else if (e.key === 'Tab') {
      const stops = [...node.querySelectorAll<HTMLElement>(TABBABLE)].filter((el) => el.tabIndex >= 0 && !el.matches(':disabled') && el.getClientRects().length);
      const at = document.activeElement;
      if (at === node || at === stops[e.shiftKey ? 0 : stops.length - 1] || !stops.length) {
        e.preventDefault();
        (stops[e.shiftKey ? stops.length - 1 : 0] ?? node).focus();
      }
    }
  });
  return {
    destroy() {
      off();
      // Another dialog may have opened meanwhile: take focus back only if it is still here, or nowhere.
      const at = document.activeElement;
      if (prev?.isConnected && (!at || at === document.body || node.contains(at))) prev.focus();
    },
  };
}
