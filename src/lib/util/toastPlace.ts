// Where the toast stack goes (components/Toasts.svelte). Nothing on the page moves for a toast:
// the stack is laid over the page, at the right under the top bar, and it is put where it lies
// on nothing you can press. The geometry is here, apart from the page, so it has tests.

/** A box on screen, in px from the window's top left corner. */
export type Box = { left: number; top: number; right: number; bottom: number };

/** Do the two boxes share an area? Boxes that only touch do not. */
const overlap = (a: Box, b: Box): boolean => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

/** Less than this many px of overlap, on either axis, is not covering (see `covers`). */
const MIN_COVER = 8;

/** The air the stack keeps between a control and itself when it moves off it, in px. */
export const AIR = 4;

/**
 * Does the stack's `box` cover the control `c`? Only when their boxes share at least `MIN_COVER`
 * px on both axes: a few pixels at an edge leave the control free to press. The boxes are the
 * real ones; the air the stack keeps is a rule of its moves (`toastTop`), not of this one.
 */
export const covers = (c: Box, box: Box): boolean =>
  Math.min(c.right, box.right) - Math.max(c.left, box.left) >= MIN_COVER && Math.min(c.bottom, box.bottom) - Math.max(c.top, box.top) >= MIN_COVER;

/** The tops the stack tries, in order, the first one `top` and the last one where it stays (see `toastTop`). */
function tries(stack: Box, controls: Box[], top: number, maxMoves: number, maxDistance: number): number[] {
  const height = stack.bottom - stack.top;
  const tried = [top];
  for (let moves = 0; moves < maxMoves; moves++) {
    const at = tried[tried.length - 1];
    const box: Box = { left: stack.left, right: stack.right, top: at, bottom: at + height };
    const inReach = (c: Box) => c.bottom + AIR <= top + maxDistance;
    const under = controls.filter((c) => covers(c, box));
    if (!under.some(inReach)) break;
    // Under every control in reach that it covers or comes nearer to than its air.
    const near = controls.filter((c) => inReach(c) && overlap(grown(c, AIR), box));
    tried.push(Math.max(...near.map((c) => c.bottom + AIR)));
  }
  return tried;
}

/**
 * The top edge of the toast stack, in px from the window's top.
 *
 * It starts at `top` (right under the top bar). While a control lies under the stack's box
 * (`covers`), the stack goes down and looks again: `maxMoves` times at most, and never further
 * than `maxDistance` px under `top`. A move goes `AIR` px under the lowest control in that reach
 * that the box covers or comes nearer to than `AIR` px: a control that ends further down (a tall
 * list of rows) does not keep the stack on the row of buttons above it, and the stack does not
 * land 2 px from a control beside the one it moved off. With no control under it in reach it
 * stays where it is, and with no free place found it stays at the last place it tried.
 *
 * `stack` gives the stack's left and right edge and its height (where its top is now does not
 * count). `controls` are the boxes of what a toast must not cover.
 */
export function toastTop(stack: Box, controls: Box[], top: number, maxMoves: number, maxDistance: number): number {
  const tried = tries(stack, controls, top, maxMoves, maxDistance);
  return tried[tried.length - 1];
}

/** A box the stack must not cover. `card`: a card that waits for your answer (`WAITING_CARD`). */
export type Control = Box & { card?: boolean };

/**
 * A height the stack can have, how far under `top` a stack of that height may go (`reachDown`),
 * and how far it may go to get off a card that waits for your answer (to the window's bottom,
 * less the room kept there), in px.
 */
export type Shape = { height: number; reach: number; cardReach: number };

/**
 * Where the stack goes, and whether it folds.
 *
 * The whole stack looks for a place first (`toastTop`). When it finds none that is free within
 * its reach, it folds: one toast shows, with a line "+2 more" under it, and that smaller stack
 * looks for a place the same way. With no free place for it either it stays folded at the last
 * place it tried: it covers less than the whole stack would. `folded` is the shape of the folded
 * stack, null for a stack that cannot fold (one toast).
 *
 * `stay`: the top the stack has now, when it may keep it (`lookFor` said 'keep': a toast went or
 * changed, nothing of the page's layout moved). The whole stack stays there while it is free there
 * and within its reach: the toasts that stay do not move. Null: it looks from the top bar down.
 *
 * Nothing of the stack, its line under it included, lies on a card that waits for your answer
 * (`Control.card`): who asks, and what, is read before a button is pressed. With no free place,
 * the stack (folded when it can fold) goes to the last place it tried that is off every waiting
 * card, though it covers another control there; with none, under the cards, as far as the
 * window lets it go (`Shape.cardReach`), when that place covers no control either (the composer
 * under a tall card). Only a card that leaves no such room is covered. `controls` must hold what
 * lies down to `cardReach` under the stack, not only what lies in its usual reach.
 */
export function stackPlace(sides: { left: number; right: number }, whole: Shape, folded: Shape | null, controls: Control[], top: number, maxMoves: number, stay: number | null = null): { top: number; folded: boolean } {
  const boxAt = (at: number, shape: Shape): Box => ({ ...sides, top: at, bottom: at + shape.height });
  const freeAt = (at: number, shape: Shape, of: Box[] = controls) => !of.some((c) => covers(c, boxAt(at, shape)));
  if (stay !== null && stay >= top && stay - top <= whole.reach && freeAt(stay, whole)) return { top: stay, folded: false };
  const look = (shape: Shape) => tries(boxAt(top, shape), controls, top, maxMoves, shape.reach);
  const all = look(whole);
  const allAt = all[all.length - 1];
  if (freeAt(allAt, whole)) return { top: allAt, folded: false };
  const shape = folded ?? whole;
  const tried = folded ? look(folded) : all;
  const last = tried[tried.length - 1];
  if (folded && freeAt(last, folded)) return { top: last, folded: true };
  const cards = controls.filter((c) => c.card);
  const offCards = tried.filter((at) => freeAt(at, shape, cards));
  if (offCards.length) return { top: offCards[offCards.length - 1], folded: !!folded };
  const below = underCards(boxAt(last, shape), cards, top, shape.cardReach);
  return { top: below !== null && freeAt(below, shape) ? below : last, folded: !!folded };
}

/** The top under the waiting `cards` that `box` covers, `AIR` px under them, when it is no
 *  further than `reach` px under `top`; null when the cards leave no room there. */
function underCards(box: Box, cards: Box[], top: number, reach: number): number | null {
  const height = box.bottom - box.top;
  let at = box.top;
  for (;;) {
    const here: Box = { ...box, top: at, bottom: at + height };
    const under = cards.filter((c) => covers(c, here));
    if (!under.length) return at;
    at = Math.max(...under.map((c) => c.bottom + AIR));
    if (at - top > reach) return null;
  }
}

/**
 * The one toast of a folded stack: the newest, or the newest error when the stack has one (an
 * error is never the one that is hidden). `newestFirst`: the stack's toasts, the newest first.
 */
export const frontToast = <T extends { kind: string }>(newestFirst: T[]): T | undefined => newestFirst.find((t) => t.kind === 'error') ?? newestFirst[0];

/**
 * How far under `top` the stack may go, in px: `part` of the window's height, and never so far
 * that its bottom edge comes nearer than `edge` px to the window's bottom (a toast moved down
 * must not leave the window or lie on the composer's Send). 0: it has no room to go down.
 */
export function reachDown(windowHeight: number, part: number, top: number, height: number, edge: number): number {
  return Math.max(0, Math.min(part * windowHeight, windowHeight - edge - top - height));
}

/**
 * How the stack fits the window: the height of its list in px, and whether the list scrolls.
 *
 * `heights`: the stack's rows as they are drawn, the newest first (each toast, and the place a
 * dismissed toast keeps: `stackRows`); `front`: the index of the one toast a folded stack shows.
 * `foot`: the height of the line under the stack ("+2 more", "Dismiss all"), 0 with none; `gap`:
 * the gap between two rows and over that line. A stack that is higher than the room from `top`
 * to `bottom` (the window's bottom less the room kept there) ends at `bottom`, and its list scrolls.
 */
export function stackFit(fit: { heights: number[]; folded: boolean; front: number; foot: number; gap: number; top: number; bottom: number }): { height: number; scrolls: boolean } {
  const shown = fit.folded ? [fit.heights[fit.front]] : fit.heights;
  const rows = shown.reduce((sum, h) => sum + h, 0) + fit.gap * Math.max(0, shown.length - 1);
  const height = rows + (fit.foot ? fit.gap + fit.foot : 0);
  const room = Math.max(0, fit.bottom - fit.top);
  return height > room ? { height: room, scrolls: true } : { height, scrolls: false };
}

/** The place a toast that was dismissed keeps while you work on the stack: its id and its height in px. */
export type Gap = { id: number; height: number };

/** A row of the stack, by the id of its toast: the toast, or the empty place of `gap` px that it
 *  keeps after it was dismissed (`Gap`). */
export type Row<T> = { id: number; toast: T } | { id: number; gap: number };

/**
 * What the stack shows, the newest first. `newestFirst`: its toasts; `gaps`: the toasts that were
 * dismissed while the pointer was on the stack or the focus in it; `held`: that is still so.
 * While it is held, each dismissed toast keeps an empty place of its own height where it was, so
 * the toasts under it keep their place under your pointer. Once you leave the stack the places close.
 */
export function stackRows<T extends { id: number }>(newestFirst: T[], gaps: Gap[], held: boolean): Row<T>[] {
  const rows: Row<T>[] = newestFirst.map((toast) => ({ id: toast.id, toast }));
  if (!held) return rows;
  const shown = new Set(newestFirst.map((t) => t.id));
  for (const g of gaps) if (!shown.has(g.id)) rows.push({ id: g.id, gap: g.height });
  // A newer toast has a higher id: it is nearer the top.
  return rows.sort((a, b) => b.id - a.id);
}

/**
 * The part of `box` that shows through every one of `frames` (the boxes of what clips it: a
 * scrolling list, a room with cut corners). Null: nothing of it shows.
 */
export function visiblePart(box: Box, frames: Box[]): Box | null {
  let part = box;
  for (const f of frames) {
    part = { left: Math.max(part.left, f.left), top: Math.max(part.top, f.top), right: Math.min(part.right, f.right), bottom: Math.min(part.bottom, f.bottom) };
    if (part.left >= part.right || part.top >= part.bottom) return null;
  }
  return part;
}

/** `box` with `by` px more on each side. */
export const grown = (box: Box, by: number): Box => ({ left: box.left - by, top: box.top - by, right: box.right + by, bottom: box.bottom + by });

/** What the page says of one control. */
export type Seen = {
  /** It is part of the toast stack itself. */
  inStack: boolean;
  /** It is in a layer that is drawn above the toast stack (a dialog, the palette). */
  above: boolean;
  disabled: boolean;
  /** Its computed `visibility` and `pointer-events`. */
  visibility: string;
  pointerEvents: string;
  /** A press at the centre of the part of it that shows reaches it (see `onTop`). */
  onTop: boolean;
};

/**
 * Must a toast keep off this control? Not off a control of the toast stack itself, one in a
 * layer above the stack (the stack never lies on that), a disabled one, one that is not shown
 * or takes no press, and one that lies under another layer of the page (the office's buttons
 * under the first-run quest or the IDE): a press does not reach that one.
 */
export const counts = (c: Seen): boolean => !c.inStack && !c.above && !c.disabled && c.visibility === 'visible' && c.pointerEvents !== 'none' && c.onTop;

/**
 * The box a control counts with, null when it does not count.
 *
 * A control that is wider than the stack, and that the stack could cover for less than half of
 * its width (`sides`: the stack's left and right edge), counts by what it shows: `content()`
 * gives the smallest box around its text and its pictures, null when it shows nothing of its own
 * (the press area that lies over a whole row). So a toast at the right stays at the top over a
 * whole-width row whose label is at the left (a task row of Home, 632 px wide). Any other control
 * counts with its whole box (`part`: the part of it that shows), and so does a field you type in:
 * its text can be anywhere in it.
 */
export function countedBox(part: Box, sides: { left: number; right: number }, field: boolean, content: () => Box | null): Box | null {
  const width = part.right - part.left;
  const under = Math.min(part.right, sides.right) - Math.max(part.left, sides.left);
  if (field || width <= sides.right - sides.left || 2 * under >= width) return part;
  const shown = content();
  return shown && visiblePart(shown, [part]);
}

/**
 * What a change asks of the stack: 'fresh', look for its place from the top bar down; 'keep',
 * keep the place it has while that is free (`stackPlace`'s `stay`); 'wait', no look now, one
 * when you stop working on the stack.
 *
 * 'layout': the page's layout or the window changed: its boxes moved, the place is looked for anew.
 * 'added': a new toast came: a new stack, it goes to the place a stack of its size has.
 * 'changed': a toast went, joined an equal one or changed its text, or the stack folded or unfolded.
 * 'page': what is in the page's main column got higher or lower.
 * `working`: the pointer is on the stack or the keyboard's focus is in it: the stack does not
 * move under your hand for a change of its own content or of the page's.
 */
export type StackChange = 'layout' | 'added' | 'changed' | 'page';
export function lookFor(change: StackChange, working: boolean): 'fresh' | 'keep' | 'wait' {
  if (change === 'layout' || change === 'added') return 'fresh';
  return working ? 'wait' : 'keep';
}

/**
 * The room kept over the stack for the Resume bar, in px: `room` while the bar is offered and
 * shows. The editor overlay is drawn over the bar: while it is open the stack starts at its usual place.
 */
export const barRoom = (offered: boolean, ideOpen: boolean, room: number): number => (offered && !ideOpen ? room : 0);

/**
 * Is a report of a watched box a change of its height (`contentWatch`)? `before`: the height of
 * its last report (`watchedHeight` before the first one); undefined: none to compare with.
 */
export const heightChanged = (before: number | undefined, now: number): boolean => before !== undefined && before !== now;

/**
 * The height a box is known to have when the watch starts, given its height on screen then. A
 * box that is 0 px high may get no first report (a ResizeObserver need not report it before it
 * grows), so its first report can be its growth: a card that arrives in an empty list is a change. Any other
 * box: unknown until its first report, which is only its size then, not a change.
 */
export const watchedHeight = (heightNow: number): number | undefined => (heightNow === 0 ? 0 : undefined);

/**
 * Is `el` what a press at a point reaches? `hits` is everything at that point, the topmost
 * first (document.elementsFromPoint). What is `aside` is left out: the toast stack itself (the
 * question is what is there without it) and the layers above it (a dialog goes away again, and
 * the stack must then not lie on a button that was under it). True when the first thing left is
 * `el` or a part of it.
 */
export function onTop<N extends { contains(other: N): boolean }>(el: N, hits: N[], aside: (hit: N) => boolean): boolean {
  const top = hits.find((hit) => !aside(hit));
  return top !== undefined && el.contains(top);
}

/** A CSS time ("380ms", "0.38s") in ms. */
export function ms(time: string): number {
  const read = /^\s*(\d*\.?\d+)(ms|s)\s*$/.exec(time);
  if (!read) throw new Error(`Not a CSS time: "${time}"`);
  return Number(read[1]) * (read[2] === 's' ? 1000 : 1);
}

/**
 * `run`, at most once in `every` ms. The first call runs at once; calls in the `every` ms after a
 * run make one more run when that time is up. `stop` drops a run that waits.
 */
export function paced(run: () => void, every: number): { (): void; stop(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let again = false;
  const go = () => {
    again = false;
    run();
    timer = setTimeout(() => {
      timer = undefined;
      if (again) go();
    }, every);
  };
  const call = () => {
    if (timer === undefined) go();
    else again = true;
  };
  call.stop = () => {
    clearTimeout(timer);
    timer = undefined;
    again = false;
  };
  return call;
}

/**
 * The look that comes after the look at once: when the page has come to rest, `wait()` ms later.
 *
 * `after(false)`: the stack changed and the page's layout did not (a toast came, one changed its
 * text). A look that already waits is kept: it comes at the end of the first wait, once, also
 * when toasts change faster than the wait is long. `after(true)`: the page's layout changed, and
 * its boxes move from now on, so the wait starts over. `stop` drops a look that waits.
 */
export function restLook(wait: () => number, look: () => void): { after(moved: boolean): void; stop(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  return {
    after(moved) {
      if (moved) stop();
      timer ??= setTimeout(() => {
        timer = undefined;
        look();
      }, wait());
    },
    stop,
  };
}

// From here on: what reads the page.

/** What you press or type in: a toast must not lie on one of these. */
const CONTROLS = 'button, a[href], input, select, textarea, [role="button"], [role="link"], [role="tab"], [role="switch"]';
/**
 * An approval or question card that waits for your answer (home/cards/ApprovalCard.svelte names
 * it with this attribute; an answered one is a row, not a card). A toast keeps off the whole
 * card: who asks, what and why are read before a button is pressed.
 */
export const WAITING_CARD = '[data-waiting-card]';

const boxOf = (el: Element): Box => {
  const r = el.getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
};

const windowBox = (): Box => ({ left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight });

/** The z-index of the layer `el` is drawn in: that of the outermost box around it (or of itself)
 *  that has a z-index of its own; 0 with none. */
function layerOf(el: Element): number {
  let z = 0;
  for (let up: Element | null = el; up; up = up.parentElement) {
    const s = getComputedStyle(up);
    if (s.position !== 'static' && s.zIndex !== 'auto') z = Number(s.zIndex);
  }
  return z;
}

/** How this stack tells what is in a layer above it. */
function layers(stack: Element): (el: Element) => boolean {
  const own = layerOf(stack);
  return (el) => layerOf(el) > own;
}

/** Does a press at the centre of `part` (a box of `el`) reach `el`, with the toast stack and the
 *  layers above it aside? */
const reached = (el: Element, part: Box, stack: Element, above: (el: Element) => boolean): boolean =>
  onTop<Element>(
    el,
    document.elementsFromPoint((part.left + part.right) / 2, (part.top + part.bottom) / 2),
    (hit) => stack.contains(hit) || above(hit),
  );

/** The boxes that clip `el`: every scrolling or cutting box around it, up to the first one that
 *  is fixed to the window (what is fixed is not clipped by the boxes around it). */
function clips(el: Element): Box[] {
  const frames: Box[] = [];
  let clipped = getComputedStyle(el).position !== 'fixed';
  for (let up = el.parentElement; up && clipped; up = up.parentElement) {
    const s = getComputedStyle(up);
    if (s.overflowX !== 'visible' || s.overflowY !== 'visible') frames.push(boxOf(up));
    if (s.position === 'fixed') clipped = false;
  }
  return frames;
}

/** What you type in: it counts with its whole box (see `countedBox`). */
const FIELDS = 'input, select, textarea';
/** What a control shows beside its text. */
const PICTURES = 'svg, img, canvas, video';

/** Puts into `out` the boxes of what `el` shows: each line of its text, and its pictures. */
function shownIn(el: Element, range: Range, out: DOMRect[]) {
  for (const node of el.childNodes) {
    if (node instanceof Text) {
      if (!node.data.trim()) continue;
      range.selectNodeContents(node);
      out.push(...range.getClientRects());
    } else if (node instanceof Element) {
      if (node.matches(PICTURES)) out.push(node.getBoundingClientRect());
      else shownIn(node, range, out);
    }
  }
}

/** The smallest box around what `el` shows (its text and its pictures); null: it shows nothing of its own. */
function contentBox(el: Element): Box | null {
  const found: DOMRect[] = [];
  shownIn(el, document.createRange(), found);
  const boxes = found.filter((b) => b.width > 0 && b.height > 0);
  if (!boxes.length) return null;
  return {
    left: Math.min(...boxes.map((b) => b.left)),
    top: Math.min(...boxes.map((b) => b.top)),
    right: Math.max(...boxes.map((b) => b.right)),
    bottom: Math.max(...boxes.map((b) => b.bottom)),
  };
}

/**
 * The boxes of the page's controls that show inside `area` and that a toast must keep off
 * (see `counts`), and those of the cards that wait for your answer (`WAITING_CARD`). A box is the
 * part that shows: not what is scrolled out of its list or lies outside the window. A control
 * much wider than the stack counts by what it shows (see `countedBox`); a waiting card counts
 * whole, and is marked as one (`Control.card`).
 */
export function controlBoxes(stack: HTMLElement, area: Box): Control[] {
  const above = layers(stack);
  const own = stack.getBoundingClientRect();
  const sides = { left: own.left, right: own.right };
  const cards = new Set(document.querySelectorAll<HTMLElement>(WAITING_CARD));
  const boxes: Control[] = [];
  for (const el of [...document.querySelectorAll<HTMLElement>(CONTROLS), ...cards]) {
    const box = boxOf(el);
    if (!overlap(box, area)) continue;
    const part = visiblePart(box, [windowBox(), ...clips(el)]);
    if (!part || !overlap(part, area)) continue;
    const own = getComputedStyle(el);
    const seen: Seen = {
      inStack: stack.contains(el),
      above: above(el),
      disabled: el.matches(':disabled, [aria-disabled="true"]'),
      visibility: own.visibility,
      pointerEvents: own.pointerEvents,
      onTop: reached(el, part, stack, above),
    };
    if (!counts(seen)) continue;
    if (cards.has(el)) boxes.push({ ...part, card: true });
    else {
      const counted = countedBox(part, sides, el.matches(FIELDS), () => contentBox(el));
      if (counted && overlap(counted, area)) boxes.push(counted);
    }
  }
  return boxes;
}

/** Puts into `out` every box in `el` (or `el` itself) that scrolls up and down, and what is in it.
 *  It does not look inside such a box: what scrolls in it is its content. */
function scrollBoxes(el: Element, out: Element[]) {
  const flow = getComputedStyle(el).overflowY;
  if (flow === 'auto' || flow === 'scroll') out.push(el, ...el.children);
  else for (const child of el.children) scrollBoxes(child, out);
}

/**
 * Tells when what is in the page's main column gets higher or lower: text that streams in, a
 * card that arrives, rows that load. A control can then be under a toast that was put there
 * before. It watches the boxes that scroll in each `main` of the page and what is in them
 * (`scrollBoxes`), and calls `changed` when the height of one changes. Not when one is seen for
 * the first time (unless it was 0 px high then: `watchedHeight`), and not when the page scrolls.
 *
 * `look()` reads the page again (after a change of layout other boxes scroll); `stop()` ends
 * the watch.
 */
export function contentWatch(changed: () => void): { look(): void; stop(): void } {
  /** What is watched, with the height it had last (none before its first report). */
  const heights = new Map<Element, number | undefined>();
  const observer = new ResizeObserver((entries) => {
    let any = false;
    for (const entry of entries) {
      const now = entry.contentRect.height;
      if (heightChanged(heights.get(entry.target), now)) any = true;
      heights.set(entry.target, now);
    }
    if (any) changed();
  });
  return {
    look() {
      const found: Element[] = [];
      for (const main of document.querySelectorAll('main')) scrollBoxes(main, found);
      const now = new Set(found);
      for (const el of [...heights.keys()]) {
        if (now.has(el)) continue;
        observer.unobserve(el);
        heights.delete(el);
      }
      for (const el of now) {
        if (heights.has(el)) continue;
        heights.set(el, watchedHeight(el.getBoundingClientRect().height));
        observer.observe(el);
      }
    },
    stop() {
      observer.disconnect();
      heights.clear();
    },
  };
}
