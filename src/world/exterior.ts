// A lot's building seen from outside: its outer walls at full height with the doors and windows
// cut in, and a flat roof over the footprint. Only what shows from the street; the inside is the
// office view's. Pure: boxes in the lot's own metres (x east, z south, (0, 0) its north-west
// corner), which the scene merges into one mesh per material.

import type { Lot } from '../lib/gen/Lot';
import type { DoorKind } from '../lib/gen/DoorKind';

export type Box = [x0: number, y0: number, z0: number, x1: number, y1: number, z1: number];

export interface Exterior {
  /** The footprint (the outer walls' bounding box). */
  x0: number;
  z0: number;
  x1: number;
  z1: number;
  walls: Box[];
  /** Window panes (lit from inside at night). */
  glass: Box[];
  doors: { box: Box; kind: DoorKind }[];
  roof: Box[];
}

export const WALL_T = 0.2;
/** A storey seen from outside: taller than the office's cutaway walls (2.3 m), so a building
 *  reads as one from the map; sills, lintels and doors scale with it. */
export const STOREY = 3.2;
const SILL = 1.05;
const LINTEL = 2.6;
const DOOR_H = 2.75;
const ROOF_T = 0.18;
const PARAPET = 0.32;

/** The outer walls (those on the footprint's edge), cut by their doors and windows; null for a lot with no walls yet. */
export function exteriorOf(lot: Pick<Lot, 'walls' | 'doors' | 'windows'>): Exterior | null {
  if (!lot.walls.length) return null;
  const xs = lot.walls.flatMap((w) => [w.x0, w.x1]);
  const zs = lot.walls.flatMap((w) => [w.z0, w.z1]);
  const X0 = Math.min(...xs);
  const X1 = Math.max(...xs);
  const Z0 = Math.min(...zs);
  const Z1 = Math.max(...zs);
  const ex: Exterior = { x0: X0, z0: Z0, x1: X1, z1: Z1, walls: [], glass: [], doors: [], roof: [] };
  const H = STOREY;
  const t = WALL_T / 2;
  for (const w of lot.walls) {
    const alongX = w.z0 === w.z1;
    const onEdge = alongX ? w.z0 === Z0 || w.z0 === Z1 : w.x0 === X0 || w.x0 === X1;
    if (!onEdge) continue;
    const a = alongX ? Math.min(w.x0, w.x1) : Math.min(w.z0, w.z1);
    const b = alongX ? Math.max(w.x0, w.x1) : Math.max(w.z0, w.z1);
    // `at` is measured from (x0, z0); flip it when the wall runs the other way.
    const from = (at: number) => (alongX ? (w.x1 >= w.x0 ? w.x0 + at : w.x0 - at) : w.z1 >= w.z0 ? w.z0 + at : w.z0 - at);
    const line = alongX ? w.z0 : w.x0;
    const box = (s0: number, s1: number, y0: number, y1: number, inset = 0): Box =>
      alongX ? [s0, y0, line - t + inset, s1, y1, line + t - inset] : [line - t + inset, y0, s0, line + t - inset, y1, s1];
    const holes = [
      ...lot.doors.filter((d) => d.wall === w.id).map((d) => ({ c: from(d.at), half: d.width / 2, y0: 0, y1: DOOR_H, door: d.kind })),
      ...lot.windows.filter((o) => o.wall === w.id).map((o) => ({ c: from(o.at), half: o.width / 2, y0: SILL, y1: LINTEL, door: null as DoorKind | null })),
    ]
      .map((h) => ({ ...h, s0: Math.max(a, h.c - h.half), s1: Math.min(b, h.c + h.half) }))
      .filter((h) => h.s1 > h.s0)
      .sort((p, q) => p.s0 - q.s0);
    // Corners: extend along-x walls over the corner so the outline closes.
    let s = alongX ? a - t : a;
    const end = alongX ? b + t : b;
    for (const h of holes) {
      if (h.s0 > s) ex.walls.push(box(s, h.s0, 0, H));
      if (h.y0 > 0) ex.walls.push(box(h.s0, h.s1, 0, h.y0));
      ex.walls.push(box(h.s0, h.s1, h.y1, H));
      if (h.door) ex.doors.push({ box: box(h.s0, h.s1, 0, h.y1, 0.05), kind: h.door });
      else ex.glass.push(box(h.s0, h.s1, h.y0, h.y1, 0.07));
      s = Math.max(s, h.s1);
    }
    if (end > s) ex.walls.push(box(s, end, 0, H));
  }
  // A flat roof with a low parapet all round.
  ex.roof.push([X0 - t, H, Z0 - t, X1 + t, H + ROOF_T, Z1 + t]);
  const p = H + PARAPET;
  ex.roof.push([X0 - t, H, Z0 - t, X1 + t, p, Z0 + t], [X0 - t, H, Z1 - t, X1 + t, p, Z1 + t], [X0 - t, H, Z0 - t, X0 + t, p, Z1 + t], [X1 - t, H, Z0 - t, X1 + t, p, Z1 + t]);
  // A row of skylights down the middle (they glow with the windows, seen from any side), and
  // a small rooftop unit in a corner.
  const top = H + ROOF_T;
  const n = Math.max(1, Math.floor((X1 - X0) / 7));
  const zc = (Z0 + Z1) / 2;
  for (let i = 0; i < n; i++) {
    const xc = X0 + ((i + 0.5) * (X1 - X0)) / n;
    ex.roof.push([xc - 1.25, top, zc - 0.95, xc + 1.25, top + 0.1, zc + 0.95]);
    ex.glass.push([xc - 1.05, top + 0.1, zc - 0.75, xc + 1.05, top + 0.16, zc + 0.75]);
  }
  ex.roof.push([X1 - 2.6, top, Z0 + 1, X1 - 1, top + 0.9, Z0 + 2.2]);
  return ex;
}
