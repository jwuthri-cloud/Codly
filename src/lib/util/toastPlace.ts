// Where the toast stack goes (components/Toasts.svelte). Nothing on the page moves for a toast:
// the stack is laid over the page, at the right under the top bar, and it is put where it lies
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

/** A height the stack can have, and how far under `top` a stack of that height may go (`reachDown`), in px. */
export type Shape = { height: number; reach: number };

/**
 * Where the stack goes, and whether it folds.
 *
 * The whole stack looks for a place first (`toastTop`). When it finds none that is free within
 * its reach, it folds: one toast shows, with a line "+2 more" under it, and that smaller stack
 * looks for a place the same way. With no free place for it either it stays folded at the last
 * place it tried: it covers less than the whole stack would. `folded` is the shape of the folded
 * stack, null for a stack that cannot fold (one toast).
 */
export function stackPlace(sides: { left: number; right: number }, whole: Shape, folded: Shape | null, controls: Box[], top: number, maxMoves: number): { top: number; folded: boolean } {
  const look = (shape: Shape) => {
    const at = toastTop({ ...sides, top, bottom: top + shape.height }, controls, top, maxMoves, shape.reach);
    const box: Box = { ...sides, top: at, bottom: at + shape.height };
    return { at, free: !controls.some((c) => overlap(c, box)) };
  };
  const all = look(whole);
  if (all.free || !folded) return { top: all.at, folded: false };
  return { top: look(folded).at, folded: true };
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
 * A control that is wider than twice the stack can never be more than half under it. It counts
 * by what it shows: `content()` gives the smallest box around its text and its pictures, null
 * when it shows nothing of its own (the press area that lies over a whole row). So a toast at
 * the right stays at the top over a whole-width row whose label is at the left. Any narrower
 * control counts with its whole box (`part`: the part of it that shows), and so does a field you
 * type in: its text can be anywhere in it.
 */
export function countedBox(part: Box, stackWidth: number, field: boolean, content: () => Box | null): Box | null {
  if (field || part.right - part.left <= 2 * stackWidth) return part;
  const shown = content();
  return shown && visiblePart(shown, [part]);
}

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
 * (see `counts`). A box is the part of the control that shows: not what is scrolled out of its
 * list or lies outside the window. A control much wider than the stack counts by what it shows
 * (see `countedBox`).
 */
export function controlBoxes(stack: HTMLElement, area: Box): Box[] {
  const above = layers(stack);
  const stackWidth = stack.offsetWidth;
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
      onTop: reached(el, part, stack, above),
    };
    if (!counts(seen)) continue;
    const counted = countedBox(part, stackWidth, el.matches(FIELDS), () => contentBox(el));
    if (counted && overlap(counted, area)) boxes.push(counted);
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
 * the first time, and not when the page scrolls.
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
      const before = heights.get(entry.target);
      const now = entry.contentRect.height;
      if (before !== undefined && before !== now) any = true;
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
        heights.set(el, undefined);
        observer.observe(el);
      }
    },
    stop() {
      observer.disconnect();
      heights.clear();
    },
  };
}
