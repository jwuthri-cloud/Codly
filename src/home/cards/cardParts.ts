// What the approval keys read of a request card on the page (the rule is `keysView` in
// lib/util/approvalCard.ts): the boxes of its three parts (who asks, what it asks, its buttons),
// what is drawn over them, and the visible box of the scroller that shows it.
import { keysView, probePoints, type Box, type KeysView, type SeenPart } from '../../lib/util/approvalCard';

/** Of what a card asks that is long (a plan, a list of paths), its first lines are what you must see: about three. */
const FIRST_LINES = 64;

/** The nearest box around `node` that scrolls up and down. */
export function scrollerOf(node: HTMLElement): HTMLElement | null {
  for (let p = node.parentElement; p; p = p.parentElement) if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
  return null;
}

const px = (v: string) => (v === 'auto' ? 0 : parseFloat(v));

/**
 * The part of the scroller where a card is seen: its box, less at each end its scroll padding
 * where it has one (the sheet's pinned bar lies over its top 48 px; the thread fades out over
 * the last 10 px and keeps 20), else its padding (the thread fades in over its top 28 px), and
 * inside the window.
 */
export function viewOf(sc: HTMLElement): Box {
  const r = sc.getBoundingClientRect();
  const s = getComputedStyle(sc);
  const inset = (scrollPadding: string, padding: string) => (scrollPadding === 'auto' ? px(padding) : px(scrollPadding));
  const top = inset(s.scrollPaddingTop, s.paddingTop);
  const bottom = inset(s.scrollPaddingBottom, s.paddingBottom);
  return { left: Math.max(r.left, 0), right: Math.min(r.right, window.innerWidth), top: Math.max(r.top + top, 0), bottom: Math.min(r.bottom - bottom, window.innerHeight) };
}

/** The card's title (who asks), what it asks (`data-asks`) and its button row (`data-answers`); null while one is missing. */
export function partsOf(card: HTMLElement): HTMLElement[] | null {
  const parts = [card.querySelector<HTMLElement>(':scope > header'), card.querySelector<HTMLElement>('[data-asks]'), card.querySelector<HTMLElement>('[data-answers]')];
  return parts.every((p) => p !== null) ? (parts as HTMLElement[]) : null;
}

/** The box a part counts with: the first lines of what is long (`data-asks="lines"`), else its whole box. */
export function boxOf(part: HTMLElement): Box {
  const r = part.getBoundingClientRect();
  const bottom = part.dataset.asks === 'lines' ? Math.min(r.bottom, r.top + FIRST_LINES) : r.bottom;
  return { left: r.left, top: r.top, right: r.right, bottom };
}

/** The part, with a press tested at each of its points: does it reach the part, or something drawn over it? */
function seen(part: HTMLElement): SeenPart {
  const box = boxOf(part);
  const free = probePoints(box).map(([x, y]) => {
    const top = document.elementFromPoint(x, y);
    return top !== null && part.contains(top);
  });
  return { box, free };
}

/**
 * Where the card stands with the keys now, and the boxes of its parts (to tell when it moves).
 * `need`: the height from its title to its buttons with its body (`body`) at its smallest (`bodyMin`).
 */
export function readCard(card: HTMLElement, sc: HTMLElement, body: HTMLElement | null, bodyMin: number): { view: KeysView; boxes: Box[] } {
  const parts = partsOf(card);
  if (!parts) return { view: 'out', boxes: [] };
  const all = parts.map(seen);
  const [title, , answers] = all;
  const shrinks = body && body.isConnected ? body.offsetHeight - Math.min(body.scrollHeight, bodyMin) : 0;
  const need = answers.box.bottom - title.box.top - Math.max(0, shrinks);
  return { view: keysView(all, viewOf(sc), need), boxes: all.map((p) => p.box) };
}
