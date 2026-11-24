// The keys of a question card, as pure rules (the card and the global key map call them):
// 1-9 pick an option while the card is the one waiting and the caret is in no text field,
// ↵ in the answer field sends (⇧↵ is a new line), ⌘↵ sends from anywhere.

/** Whether keys typed at `el` write text there: a radio or a checkbox does not (digits pick an option on it). */
export function isTextField(el: Element | null): boolean {
  if (!el) return false;
  const h = el as HTMLElement;
  if (h.isContentEditable || !!h.closest?.('.xterm')) return true;
  if (h.tagName === 'TEXTAREA' || h.tagName === 'SELECT') return true;
  if (h.tagName !== 'INPUT') return false;
  return !/^(radio|checkbox|button|submit|reset|range|color|file|image)$/i.test((h as HTMLInputElement).type);
}

type Press = { key: string; mod: boolean; alt: boolean; shift: boolean };

/** The option a key picks, 0-based; null for any other key, a key with a modifier, or a press in a text field. */
export function digitPick(p: Press, inTextField: boolean, options: number): number | null {
  if (p.mod || p.alt || inTextField || !/^[1-9]$/.test(p.key)) return null;
  const n = Number(p.key) - 1;
  return n < options ? n : null;
}

/** Whether a key press in the answer field sends the answer: ↵ (not ⇧↵, not while an IME composes) and ⌘↵. */
export function sendsFromField(p: Press & { composing?: boolean }): boolean {
  return p.key === 'Enter' && !p.shift && !p.alt && !p.composing;
}

/**
 * Which question the digits pick in: the first one with options that has no answer yet, or the
 * last one with options when all have one; -1 when none has options.
 */
export function digitQuestion(qs: { options: number; answered: boolean }[]): number {
  const open = qs.findIndex((q) => q.options > 0 && !q.answered);
  if (open >= 0) return open;
  for (let i = qs.length - 1; i >= 0; i--) if (qs[i].options > 0) return i;
  return -1;
}
