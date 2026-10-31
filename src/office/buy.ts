// Build/Buy: the furniture catalog (CC0 models in public/furniture, prices shared with the core
// via furniture.json) and where a piece may go. Pure: no three.js, so it is testable in Node.
//
// Rules: a piece snaps to the walking grid and turns in quarter turns. Rugs go anywhere on a
// floor. Small things (lamps, plants, a laptop) go on top of a table or cabinet you bought, or
// on the floor. Everything else blocks walking: it may not cover a wall, built-in furniture,
// another piece, a seat or use-slot, nor cut any of those off from the front door.
//
// Some pieces are earned (`needs`): the PS5 or the pool table once someone on the crew is a
// Senior, a coffee machine once the office has a kitchen (a room with a counter or a fridge),
// and in it. The core checks the same rule when it sells one (`career::locked`).

import type { Lot } from '../lib/gen/Lot';
import type { Placed } from '../lib/gen/Placed';
import type { Rank } from '../lib/gen/Rank';
import { atLeast, rankLabel } from '../lib/util/career';
import RAW from './furniture.json';
import { cellOf, CELL, footprint, keepRects, markRect, roomAt, type ItemType, type Job, type OfficeLayout, type Rect, type RoomId, type Slot } from './layout';

export interface Furniture {
  id: string;
  label: string;
  cat: string;
  price: number;
  /** Model path under public/furniture (none for the office's own code-built pieces). */
  src: string;
  /** A surface: small things can go on top. */
  top?: boolean;
  /** Goes on a surface (or the floor). */
  small?: boolean;
  /** Flat on the floor; never blocks. */
  rug?: boolean;
  /** Its height in the office (packs without one office scale: Poly Pizza models). */
  h?: number;
  /** Not sold (a built-in piece's model, like the pool table: it only plays where it is). */
  hidden?: boolean;
  /** The turn that makes the model face +z, when its pack's default doesn't. */
  yaw?: number;
  /** One of the office's own pieces (layout.ts builds it): its type, footprint and look. */
  builtin?: ItemType;
  w?: number;
  d?: number;
  variant?: number;
  /** Built-ins: false when it doesn't block walking (a rug, a poster). */
  block?: boolean;
  /** Hung on a wall. */
  hung?: boolean;
  /** What it does for the work: a desk is a seat for one agent, a rack runs the tests… */
  job?: Job;
  /** Earned: someone on the crew at this rank or above, or a kitchen in the office. */
  needs?: Rank | 'kitchen';
  /** A counter or a fridge: the room it stands in is a kitchen. */
  kitchen?: boolean;
}

export const FURNITURE = RAW as Furniture[];
export const BY_ID = new Map(FURNITURE.map((f) => [f.id, f]));
export const CATEGORIES = [...new Set(FURNITURE.map((f) => f.cat))];
/** What the Buy catalog sells. */
export const FOR_SALE = FURNITURE.filter((f) => !f.hidden);

/** The rooms with a counter or a fridge (the office's own, or bought). */
export function kitchens(L: OfficeLayout, pieces: Placed[]): Set<RoomId> {
  const out = new Set<RoomId>();
  for (const it of L.items) if ((it.type === 'counter' || it.type === 'fridge') && it.room) out.add(it.room);
  for (const p of pieces) {
    const room = BY_ID.get(p.item)?.kitchen ? roomAt(L, p.x, p.z) : null;
    if (room) out.add(room);
  }
  return out;
}

const article = (r: Rank) => (r === 'staff' ? 'on Staff' : r === 'intern' ? 'an Intern' : `a ${rankLabel(r)}`);

/** Why a piece can't be bought yet (`crew`: the ranks on the crew), or null when it can. */
export function locked(f: Furniture, crew: Rank[], kitchen: boolean): string | null {
  if (!f.needs) return null;
  if (f.needs === 'kitchen') return kitchen ? null : 'Needs a kitchen: a counter or a fridge first';
  const need = f.needs;
  return crew.some((r) => atLeast(r, need)) ? null : `Someone on the crew must be ${article(need)}`;
}

/**
 * The lot the office is made from, with the office's own kind of pieces that were bought (the
 * foosball, the PS5…): those play where they stand like the lot's, so they go in its furniture.
 * The shop has the truth for them (copies of bought pieces the core keeps in the lot are left
 * out); everything else bought is drawn as a model (`decorOf`).
 */
export function withBought(lot: Lot, placed: Placed[]): Lot {
  const gift = new Set(lot.gift);
  const sold = (p: Placed) => !!BY_ID.get(p.item)?.builtin && !BY_ID.get(p.item)?.hidden;
  const bought = placed.filter(sold);
  const mirrored = lot.furniture.filter((p) => sold(p) && !gift.has(p.uid));
  if (!bought.length && !mirrored.length) return lot;
  return { ...lot, furniture: [...lot.furniture.filter((p) => !mirrored.includes(p)), ...bought] };
}

/** The bought pieces drawn as models (the office's own kind go in the lot: `withBought`). */
export const decorOf = (placed: Placed[]) => placed.filter((p) => !BY_ID.get(p.item)?.builtin);

/** A model's size once scaled (w along x, d along z, before rotation; h = its top). */
export interface Dims {
  w: number;
  d: number;
  h: number;
}

export interface Piece extends Placed {
  dims: Dims;
}

const QUARTER = Math.PI / 2;
const quarter = (rot: number) => (((Math.round(rot / QUARTER) % 4) + 4) % 4) * QUARTER;

/** Snap to the grid: whole cells for the footprint, quarter turns. */
export function snap(x: number, z: number, rot: number, dims: Dims): { x: number; z: number; rot: number } {
  const r = quarter(rot);
  const f = footprint({ x: 0, z: 0, rot: r, w: dims.w, d: dims.d });
  const cw = Math.max(1, Math.round(f.w / CELL));
  const cd = Math.max(1, Math.round(f.d / CELL));
  return { x: (Math.round(x / CELL - cw / 2) + cw / 2) * CELL, z: (Math.round(z / CELL - cd / 2) + cd / 2) * CELL, rot: r };
}

/** A piece's footprint, shrunk a hair so a piece snapped to the grid covers only its own cells. */
function rect(p: Pick<Piece, 'x' | 'z' | 'rot' | 'dims'>): Rect {
  const f = footprint({ x: p.x, z: p.z, rot: p.rot, w: p.dims.w, d: p.dims.d });
  return { x: f.x + 0.02, z: f.z + 0.02, w: Math.max(0.01, f.w - 0.04), d: Math.max(0.01, f.d - 0.04) };
}

const blocks = (p: Piece) => !BY_ID.get(p.item)?.rug && !(BY_ID.get(p.item)?.small && p.y > 0);

/** Cells reachable on foot from the front door. */
function reach(L: OfficeLayout, grid: Uint8Array): Uint8Array {
  const [sx, sz] = cellOf(L, L.door.inside.x, L.door.inside.z);
  const COLS = L.cols;
  const seen = new Uint8Array(L.cols * L.rows);
  const queue = [sz * COLS + sx];
  seen[queue[0]] = 1;
  for (let i = 0; i < queue.length; i++) {
    const c = queue[i];
    const cx = c % COLS;
    for (const n of [c - 1, c + 1, c - COLS, c + COLS]) {
      if (n < 0 || n >= seen.length || seen[n] || grid[n] || Math.abs((n % COLS) - cx) > 1) continue;
      seen[n] = 1;
      queue.push(n);
    }
  }
  return seen;
}

/** The empty office's grid, the cells to keep free, and which of those can be walked to. */
const base = new WeakMap<OfficeLayout, { grid: Uint8Array; keep: Uint8Array; reached: Uint8Array }>();
function baseOf(L: OfficeLayout) {
  let b = base.get(L);
  if (!b) {
    const keep = new Uint8Array(L.cols * L.rows);
    for (const r of keepRects(L)) markRect(keep, r, 0, L.cols, L.rows);
    const grid = L.blocked.slice();
    b = { grid, keep, reached: reach(L, grid) };
    base.set(L, b);
  }
  return b;
}

/** Re-block the walking grid for the furniture placed (call before the first change too).
 *  `seats`: spots on furniture to keep reachable (the seat and the way in from the front). */
export function applyDecor(L: OfficeLayout, pieces: Piece[], seats: Slot[] = []) {
  const b = baseOf(L);
  L.blocked.set(b.grid);
  for (const p of pieces) if (blocks(p)) markRect(L.blocked, rect(p), 0, L.cols, L.rows);
  for (const s of seats)
    for (let d = 0; d <= 0.75; d += 0.25) {
      const [cx, cz] = cellOf(L, s.x + Math.sin(s.face) * d, s.z + Math.cos(s.face) * d);
      L.blocked[cz * L.cols + cx] = 0;
    }
}

/** The surface under (x, z) a small thing would sit on: its top, or null for the floor. */
export function surfaceAt(pieces: Piece[], x: number, z: number, skip?: string): Piece | null {
  for (const p of pieces) {
    if (p.uid === skip || !BY_ID.get(p.item)?.top) continue;
    const r = rect(p);
    if (x >= r.x && x <= r.x + r.w && z >= r.z && z <= r.z + r.d) return p;
  }
  return null;
}

/** Things sitting on a surface (they'd float if it moved). */
export const onTop = (pieces: Piece[], s: Piece) => pieces.filter((p) => p.uid !== s.uid && p.y > 0 && surfaceAt([s], p.x, p.z) === s);

/**
 * Can `p` go there, with the others (minus `p` itself when moving it) already down? Returns
 * null when it can, else why not.
 */
export function blocker(L: OfficeLayout, others: Piece[], p: Piece): string | null {
  const it = BY_ID.get(p.item);
  if (!it) return 'Not in the catalog';
  const room = roomAt(L, p.x, p.z);
  if (!room) return 'Outside the office';
  if (it.needs === 'kitchen' && !kitchens(L, others).has(room)) return 'Put it in the kitchen';
  if (!blocks(p)) return null;
  const b = baseOf(L);
  const grid = b.grid.slice();
  for (const o of others) if (o.uid !== p.uid && blocks(o)) markRect(grid, rect(o), 0, L.cols, L.rows);
  const r = rect(p);
  const [x0, z0] = cellOf(L, r.x, r.z);
  const [x1, z1] = cellOf(L, r.x + r.w, r.z + r.d);
  for (let z = z0; z <= z1; z++)
    for (let x = x0; x <= x1; x++) {
      if (b.keep[z * L.cols + x]) return 'In the way of a seat or the door';
      if (grid[z * L.cols + x]) return 'Something is already there';
    }
  markRect(grid, r, 0, L.cols, L.rows);
  // Every seat, use-slot and desk the door reaches now must stay reachable.
  const seen = reach(L, grid);
  for (let c = 0; c < seen.length; c++) if (b.keep[c] && b.reached[c] && !seen[c]) return 'It would block the way';
  return null;
}
