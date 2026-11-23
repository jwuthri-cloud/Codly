// Where the conversation scrolls while a card waits for you. Pure: the component reads the boxes
// and applies the answer.
//
// While a card waits, new rows do not move it: the thread keeps the oldest waiting card where it
// is (a click or a key aimed at it lands on it), and the rows that come in below are counted
// ("2 new below"). When a card becomes the oldest waiting one, the thread brings it into view.
import type { Item } from './spine';

/** The key of the oldest card that waits for your answer in these rows (null: none waits). */
export function oldestWaiting(items: Item[]): string | null {
  for (const it of items) if (it.kind === 'approval' && it.approval.status === 'pending') return it.key;
  return null;
}

/** A vertical box on the screen, in px. */
export type Span = { top: number; bottom: number };

/**
 * The scroll position that shows `box` whole inside `view` (each edge `pad` px inside), moving as
 * little as it can. A box taller than that room shows its top. `scrollTop`: the position now.
 */
export function revealTop(scrollTop: number, view: Span, box: Span, pad: number): number {
  const top = view.top + pad;
  const bottom = view.bottom - pad;
  if (box.top < top) return scrollTop - (top - box.top);
  if (box.bottom <= bottom) return scrollTop;
  return scrollTop + Math.min(box.bottom - bottom, box.top - top);
}

/** How many of the rows below the view are new: not among the keys you had seen. */
export function newBelow(rows: { key: string; top: number }[], seen: ReadonlySet<string>, viewBottom: number): number {
  return rows.filter((r) => !seen.has(r.key) && r.top >= viewBottom).length;
}
