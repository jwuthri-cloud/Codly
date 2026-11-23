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
 * stack goes down to the bottom edge of the lowest such control and looks again: `maxMoves`
 * times at most, and never further than `maxDistance` px under `top`. With no free place found
 * it stays at the last place it tried.
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
    const below = Math.max(...under.map((c) => c.bottom));
    if (below > top + maxDistance) return at;
    at = below;
  }
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

/** What you press or type in: a toast must not lie on one of these. */
const CONTROLS = 'button, a[href], input, select, textarea, [role="button"], [role="link"], [role="tab"], [role="switch"]';

const boxOf = (el: Element): Box => {
  const r = el.getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
};

/**
 * The boxes of the page's controls that show inside `area` and that a toast could cover.
 * Not counted: a control of the toast stack itself, a disabled one, one that is not shown or
 * takes no press, the part of one that is scrolled out of its list, and one in a layer that is
 * drawn above the stack (a dialog, the palette): the stack never lies on that.
 */
export function controlBoxes(stack: HTMLElement, area: Box): Box[] {
  const stackZ = Number(getComputedStyle(stack).zIndex);
  const boxes: Box[] = [];
  for (const el of document.querySelectorAll<HTMLElement>(CONTROLS)) {
    if (stack.contains(el) || el.matches(':disabled, [aria-disabled="true"]')) continue;
    const box = boxOf(el);
    if (!overlap(box, area)) continue;
    const own = getComputedStyle(el);
    if (own.visibility !== 'visible' || own.pointerEvents === 'none') continue;
    // Up the tree: what clips the control, and the z-index of the layer it is drawn in (the
    // outermost one that has a z-index of its own).
    const frames: Box[] = [];
    let clipped = own.position !== 'fixed';
    let layerZ = own.position !== 'static' && own.zIndex !== 'auto' ? Number(own.zIndex) : 0;
    for (let up = el.parentElement; up; up = up.parentElement) {
      const s = getComputedStyle(up);
      if (clipped && (s.overflowX !== 'visible' || s.overflowY !== 'visible')) frames.push(boxOf(up));
      // What is fixed to the window is not clipped by the boxes around it.
      if (s.position === 'fixed') clipped = false;
      if (s.position !== 'static' && s.zIndex !== 'auto') layerZ = Number(s.zIndex);
    }
    if (layerZ > stackZ) continue;
    const part = visiblePart(box, frames);
    if (part && overlap(part, area)) boxes.push(part);
  }
  return boxes;
}
