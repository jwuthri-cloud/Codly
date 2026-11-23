// Where the conversation scrolls while a card waits for you. Pure: the component reads the boxes
// and applies the answer.
//
// While a card waits in view, new rows do not move it: the thread keeps the oldest waiting card
// where it is (a click or a key aimed at it lands on it), and the rows that come in below are
// counted ("2 new below"). When a card becomes the oldest waiting one, the thread brings it into
// view. A card out of view holds nothing: the thread follows the newest line as it does with no
// card, while you are there.
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

/** Whether some of `box` shows in `view`. */
export function meets(view: Span, box: Span): boolean {
  return box.top < view.bottom && box.bottom > view.top;
}

/**
 * What the thread does after its rows changed: hold the waiting card where it is, bring it into
 * view, go to the newest line, mark that something came in below, or nothing.
 */
export type Move = 'hold' | 'reveal' | 'bottom' | 'mark' | 'none';

export type ScrollState = {
  /** The oldest waiting card is on the page. */
  card: boolean;
  /** It became the oldest waiting one with this change. */
  fresh: boolean;
  /** The thread held a card before this change. */
  held: boolean;
  /** The held card showed in the view before this change. */
  inView: boolean;
  /** You are at the newest line. */
  atBottom: boolean;
  /** You were at the newest line when the thread began to hold a card. */
  followed: boolean;
  /** Rows came in below, or the newest one grows. */
  added: boolean;
};

/**
 * The rule. A card that becomes the oldest waiting one comes into view while you were at the
 * newest line or at the card before it (while you read further up, it does not pull you down).
 * A held card stays where it is while it shows. With no card in view, the thread follows the
 * newest line while you are there; when the last card is answered, it goes back there if you
 * were there when the hold began.
 */
export function nextMove(s: ScrollState): Move {
  if (s.card && s.fresh) return s.atBottom || s.held ? 'reveal' : s.added ? 'mark' : 'none';
  if (s.card && s.inView) return 'hold';
  if (s.atBottom || (!s.card && s.held && s.followed)) return 'bottom';
  return s.added ? 'mark' : 'none';
}
