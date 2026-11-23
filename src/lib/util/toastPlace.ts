// Where the toast stack goes (components/Toasts.svelte). Nothing on the page moves for a toast:
// the stack is laid over the page, at the right under the top bar. In a wide window it stands
// beside the page's panel, as wide as the free room there. Everywhere it is put where it lies
// on nothing you can press. The geometry is here, apart from the page, so it has tests.

/** A box on screen, in px from the window's top left corner. */
export type Box = { left: number; top: number; right: number; bottom: number };

/** Do the two boxes share an area? Boxes that only touch do not. */
const overlap = (a: Box, b: Box): boolean => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

/**
 * The top edge of the toast stack, in px from the window's top.
 *
 * It starts at `top` (right under the top bar). While a control lies under the stack's box, the
 * stack goes down and looks again: `maxMoves` times at most, and never further than
 * `maxDistance` px under `top`. A move goes to the bottom edge of the lowest control under the
 * box that is within that reach: a control that ends further down (a tall list of rows) does not
 * keep the stack on the row of buttons above it. With no control under it in reach it stays
 * where it is, and with no free place found it stays at the last place it tried.
 *
 * `stack` gives the stack's left and right edge and its height (where its top is now does not
 * count). `controls` are the boxes of what a toast must not cover.
 */
export function toastTop(stack: Box, controls: Box[], top: number, maxMoves: number, maxDistance: number): number {
  const height = stack.bottom - stack.top;
  let at = top;
  for (let moves = 0; ; moves++) {
    const box: Box = { left: stack.left, right: stack.right, top: at, bottom: at + height };
    const under = controls.filter((c) => overlap(c, box));
    if (!under.length || moves === maxMoves) return at;
    const reachable = under.map((c) => c.bottom).filter((bottom) => bottom <= top + maxDistance);
    if (!reachable.length) return at;
    at = Math.max(...reachable);
  }
}

/**
 * How far under `top` the stack may go, in px: `part` of the window's height, and never so far
 * that its bottom edge comes nearer than `edge` px to the window's bottom (a toast moved down
 * must not leave the window or lie on the composer's Send). 0: it has no room to go down.
 */
export function reachDown(windowHeight: number, part: number, top: number, height: number, edge: number): number {
  return Math.max(0, Math.min(part * windowHeight, windowHeight - edge - top - height));
}

/**
 * How wide the stack is when it stands beside the page's panel, in the free room at the panel's
 * right: from `gap` px after the panel's right edge to the stack's own right edge, and `most`
 * px at most. Null: less than `least` px are free there (a narrow window, an open sheet, a room
 * as wide as the window), so the stack keeps its own width and lies over the panel.
 */
export function widthBeside(panelRight: number, stackRight: number, gap: number, least: number, most: number): number | null {
  const room = stackRight - panelRight - gap;
  return room >= least ? Math.min(room, most) : null;
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

// From here on: what reads the page.

/** The attribute on a panel the stack may stand beside: Home's conversation column and the
 *  first-run quest's level. A test checks that both carry it. */
export const BESIDE = 'data-toast-beside';

/** What you press or type in: a toast must not lie on one of these. */
const CONTROLS = 'button, a[href], input, select, textarea, [role="button"], [role="link"], [role="tab"], [role="switch"]';

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

/** Does a press at a point of `part` (a box of `el`) reach `el`, with the toast stack and the
 *  layers above it aside? The point is `across` of the way from left to right and `along` of the
 *  way from top to bottom: 0.5 and 0.5 is the centre. */
const reached = (el: Element, part: Box, across: number, along: number, stack: Element, above: (el: Element) => boolean): boolean =>
  onTop<Element>(
    el,
    document.elementsFromPoint(part.left + across * (part.right - part.left), part.top + along * (part.bottom - part.top)),
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

/**
 * The boxes of the page's controls that show inside `area` and that a toast must keep off
 * (see `counts`). A box is the part of the control that shows: not what is scrolled out of its
 * list or lies outside the window.
 */
export function controlBoxes(stack: HTMLElement, area: Box): Box[] {
  const above = layers(stack);
  const boxes: Box[] = [];
  for (const el of document.querySelectorAll<HTMLElement>(CONTROLS)) {
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
      onTop: reached(el, part, 0.5, 0.5, stack, above),
    };
    if (counts(seen)) boxes.push(part);
  }
  return boxes;
}

/** Is this panel on screen, and not under another layer of the page (the office's column under
 *  the first-run quest or the IDE)? A dialog over it does not count: it goes away again. Asked
 *  at the centre and around it: a menu that is open over one place hides no panel. */
export function panelShows(panel: Element, stack: HTMLElement): boolean {
  const part = visiblePart(boxOf(panel), [windowBox()]);
  if (!part) return false;
  const above = layers(stack);
  const places = [0.5, 0.25, 0.75];
  return places.some((across) => places.some((along) => reached(panel, part, across, along, stack, above)));
}
