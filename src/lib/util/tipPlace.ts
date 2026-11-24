// Where the one tooltip goes (components/TooltipHost.svelte). Above its control when there is
// room, else below. A control can ask for below (`data-tip-place="below"`): the Stop buttons of
// the agent sheet's lists do, so the tip does not cover the title of the section above them.
// Pure, so the host and the tests agree.

/** The space between a control and its tip, and the least room left at the window's edge. */
const GAP = 8;
const EDGE = 4;

export type TipBox = { x: number; y: number; below: boolean };

/**
 * The tip's place: `r` the control's box, `w`/`h` the tip's size, `place` what the control asks
 * for ('below', else ''), `win` the window's size. Below when asked or when there is no room
 * above, and above again when asked for below with no room there either.
 */
export function tipPlace(r: Pick<DOMRect, 'left' | 'top' | 'bottom' | 'width'>, w: number, h: number, place: string, win: { width: number; height: number }): TipBox {
  const x = Math.max(GAP, Math.min(r.left + r.width / 2 - w / 2, win.width - w - GAP));
  const roomAbove = r.top - h - GAP >= EDGE;
  const roomBelow = r.bottom + GAP + h <= win.height - EDGE;
  const below = place === 'below' ? roomBelow || !roomAbove : !roomAbove;
  return { x, y: below ? r.bottom + GAP : r.top - h - GAP, below };
}
