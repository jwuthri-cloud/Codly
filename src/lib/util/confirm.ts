// What an inline question ("Kill / Keep", "Discard it / Keep") owes the keyboard, the same as the
// request tabs' close question: it opens with the keyboard on the safe button, so Enter keeps;
// Esc anywhere in the question is the safe choice too (and closes no sheet, stops no turn); Enter
// on the other button does what it says. When the question goes, the keyboard goes back to the
// control that asked it (`back`), or to where it was before the question opened.

/** What a key does in an inline question: 'keep' for the safe choice, null for the default. */
export function confirmKey(e: Pick<KeyboardEvent, 'key' | 'isComposing' | 'repeat'>): 'keep' | null {
  if (e.isComposing || e.repeat) return null;
  return e.key === 'Escape' ? 'keep' : null;
}

export type SafeChoice = {
  /** The safe choice: what Keep does. */
  keep: () => void;
  /** Where the keyboard goes when the question goes (the control that opened it, shown again). */
  back?: () => HTMLElement | null | undefined;
};

/**
 * Put on the question's safe button (Keep). It takes the keyboard when it appears; an Esc on any
 * button of the question (its parent) keeps.
 */
export function safeChoice(node: HTMLElement, opts: SafeChoice) {
  let o = opts;
  const prev = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const scope = node.parentElement ?? node;
  node.focus();
  const onKey = (e: KeyboardEvent) => {
    if (confirmKey(e) !== 'keep') return;
    e.preventDefault();
    e.stopPropagation();
    o.keep();
  };
  scope.addEventListener('keydown', onKey);
  return {
    update(next: SafeChoice) {
      o = next;
    },
    destroy() {
      scope.removeEventListener('keydown', onKey);
      // After the update that removed the question: the control that asked it is there again.
      queueMicrotask(() => {
        const at = document.activeElement;
        // Something else took the keyboard meanwhile: it keeps it.
        if (at && at !== document.body && at.isConnected) return;
        const to = o.back?.() ?? (prev?.isConnected ? prev : null);
        to?.focus();
      });
    },
  };
}
