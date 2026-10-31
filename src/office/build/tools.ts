// Build mode's tools as pure edits of a lot: each takes the lot (and the furniture bought through
// Buy, which the shop keeps and the lot mirrors) and returns the lot after, or why it can't. The
// office (layout.ts) finds the rooms again from the walls, so a tool only ever edits walls,
// openings, floors, paint and where furniture stands. Pure: no three.js, tested in Node.
//
// Rooms are regions of tiles (`officeLayout().roomOf`); push/pull, move and knocking a room down
// relabel tiles and let `rewall` put walls where regions meet. Furniture that no longer fits goes
// to storage; a piece bought through Buy is sold back instead (for its full price: buying it
// again costs the same), since the shop has no storage.

import type { DoorKind } from '../../lib/gen/DoorKind';
import type { FloorRect } from '../../lib/gen/FloorRect';
import type { LotDoor } from '../../lib/gen/LotDoor';
import type { LotWall } from '../../lib/gen/LotWall';
import type { LotWindow } from '../../lib/gen/LotWindow';
import type { Placed } from '../../lib/gen/Placed';
import type { WallKind } from '../../lib/gen/WallKind';
import { BY_ID } from '../buy';
import type { OfficeLayout } from '../layout';
import { addEdges, adopt, edgeMap, ekey, freshId, g, isH, lenOf, openingsOf, parse, rewall, T, tidy, wallThrough, cutWall, type BLot, type Opening, type Orient } from './grid';

export type { BLot };

/** An edit: the lot after, and the pieces bought through Buy after (moved, sold or bought). */
export interface Edit {
  lot: BLot;
  bought: Placed[];
  /** Something worth saying (a door that didn't fit any more, pieces put in storage). */
  note?: string;
}
export type Outcome = Edit | { error: string };
export const failed = (o: Outcome): o is { error: string } => 'error' in o;

export const DOOR_WIDTH: Record<DoorKind, number> = { open: 1.4, single: 1.0, double: 1.8, glass: 1.0, rollup: 2.6 };
export const WINDOW_WIDTHS = [1.0, 1.6, 2.6];
const QUARTER = Math.PI / 2;

// ───────────── helpers ─────────────

const tilesOf = (lot: BLot) => ({ cols: g(lot.size.w), rows: g(lot.size.d) });
const tileAt = (lot: BLot, x: number, z: number) => {
  const { cols, rows } = tilesOf(lot);
  const c = Math.floor(x / T);
  const r = Math.floor(z / T);
  return c < 0 || r < 0 || c >= cols || r >= rows ? -1 : r * cols + c;
};

/** The tiles a piece stands on: its footprint for the office's own pieces, its middle for a model. */
export function pieceTiles(lot: BLot, p: Placed): number[] {
  const f = BY_ID.get(p.item);
  if (!f?.builtin || !f.w || !f.d || f.block === false) return [tileAt(lot, p.x, p.z)];
  const q = Math.abs(Math.round(p.rot / QUARTER)) % 2 === 1;
  const [w, d] = q ? [f.d, f.w] : [f.w, f.d];
  const out: number[] = [];
  for (let z = Math.floor((p.z - d / 2 + 0.02) / T); z <= Math.floor((p.z + d / 2 - 0.02) / T); z++)
    for (let x = Math.floor((p.x - w / 2 + 0.02) / T); x <= Math.floor((p.x + w / 2 - 0.02) / T); x++) out.push(tileAt(lot, x * T + T / 2, z * T + T / 2));
  return out;
}

export const labelsOf = (L: OfficeLayout) => Int32Array.from(L.roomOf);
export const roomTiles = (L: OfficeLayout, k: number) => {
  const out: number[] = [];
  L.roomOf.forEach((r, t) => r === k && out.push(t));
  return out;
};
/** A room's bounding box in tiles, and whether its outline is a plain rectangle (rooms inside
 *  it are fine: a garage with a booth in it still has four straight sides). */
export function roomBox(L: OfficeLayout, k: number) {
  const tiles = roomTiles(L, k);
  const cols = L.tiles.cols;
  let [c0, r0, c1, r1] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const t of tiles) {
    const c = t % cols;
    const r = (t - c) / cols;
    (c0 = Math.min(c0, c)), (r0 = Math.min(r0, r)), (c1 = Math.max(c1, c + 1)), (r1 = Math.max(r1, r + 1));
  }
  const mine = (c: number, r: number) => L.roomOf[r * cols + c] === k;
  let rect = tiles.length > 0;
  for (let c = c0; c < c1 && rect; c++) rect = mine(c, r0) && mine(c, r1 - 1);
  for (let r = r0; r < r1 && rect; r++) rect = mine(c0, r) && mine(c1 - 1, r);
  return { c0, r0, c1, r1, tiles, rect };
}

/** What a room's tiles become when it goes: the room it borders most (walls between them come
 *  down, so it merges into it), or outside (−1) when it borders none. */
function mergeLabel(L: OfficeLayout, k: number): number {
  const cols = L.tiles.cols;
  const rows = L.tiles.rows;
  const count = new Map<number, number>();
  for (const t of roomTiles(L, k)) {
    const c = t % cols;
    const r = (t - c) / cols;
    for (const [nc, nr] of [[c, r - 1], [c, r + 1], [c - 1, r], [c + 1, r]]) {
      if (nc < 0 || nr < 0 || nc >= cols || nr >= rows) continue;
      const n = L.roomOf[nr * cols + nc];
      if (n !== k && n >= 0) count.set(n, (count.get(n) ?? 0) + 1);
    }
  }
  let best = -1;
  for (const [n, v] of count) if (best < 0 || v > count.get(best)!) best = n;
  return best;
}

/** The office's furniture and Buy's, once each (Buy's place wins), with which is which. */
function everyPiece(lot: BLot, bought: Placed[]) {
  const shop = new Map(bought.map((p) => [p.uid, p]));
  return [...lot.furniture.filter((p) => !shop.has(p.uid)).map((p) => ({ p, bought: false })), ...bought.map((p) => ({ p, bought: true }))];
}

/** Put pieces away: the office's go to storage, Buy's are sold back (and leave the lot's mirror). */
function putAway(e: Edit, uids: Set<string>): number {
  if (!uids.size) return 0;
  const lot = e.lot;
  const shop = new Set(e.bought.map((p) => p.uid));
  const away = lot.furniture.filter((p) => uids.has(p.uid) && !shop.has(p.uid));
  lot.furniture = lot.furniture.filter((p) => !uids.has(p.uid));
  lot.storage = [...(lot.storage ?? []), ...away];
  e.bought = e.bought.filter((p) => !uids.has(p.uid));
  return uids.size;
}

const storedNote = (n: number) => (n ? `${n === 1 ? 'A piece' : `${n} pieces`} didn’t fit: in storage now (pieces you bought are sold back for what you paid).` : undefined);

/** Interior grid points of [a, b] along a wall where another wall meets it (a door can't go there). */
function junctions(lot: BLot, w: LotWall, a: number, b: number): boolean {
  const m = edgeMap(lot);
  for (let s = Math.ceil((a + 1e-6) / T); s * T < b - 1e-6; s++) {
    const [x, z] = isH(w) ? [g(w.x0) + s, g(w.z0)] : [g(w.x0), g(w.z0) + s];
    const hit = isH(w) ? m.has(ekey('v', x, z - 1)) || m.has(ekey('v', x, z)) : m.has(ekey('h', x - 1, z)) || m.has(ekey('h', x, z));
    if (hit) return true;
  }
  return false;
}

/** Why an opening can't go `width` wide at `at` along a wall (null: it can). */
export function openingBlock(lot: BLot, wallId: string, at: number, width: number, window: boolean, skip?: string): string | null {
  const w = lot.walls.find((x) => x.id === wallId);
  if (!w) return 'Not on a wall';
  if (window && w.kind !== 'exterior') return 'Windows go in outside walls';
  const [a, b] = [at - width / 2, at + width / 2];
  if (a < -1e-6 || b > lenOf(w) + 1e-6) return 'It doesn’t fit in this wall';
  if (openingsOf(lot, wallId).some((o) => o.id !== skip && a < o.at + o.width / 2 - 1e-6 && b > o.at - o.width / 2 + 1e-6)) return 'There’s a door or window there';
  if (junctions(lot, w, a, b)) return 'Another wall meets this one there';
  return null;
}

/** Openings a wall now runs into (a new wall meeting one in its span). */
function cutOff(lot: BLot): Opening[] {
  return [...lot.doors, ...lot.windows].filter((o) => {
    const w = lot.walls.find((x) => x.id === o.wall);
    return !w || junctions(lot, w, o.at - o.width / 2, o.at + o.width / 2);
  });
}

/** Put openings back where they stood (a moved wall's doors): on the wall through each point, if it fits. */
function rehome(lot: BLot, ops: { op: Opening; window: boolean; x: number; z: number; o: Orient }[]): number {
  let lost = 0;
  for (const { op, window, x, z, o } of ops) {
    const hit = wallThrough(lot, x, z, o);
    const id = new Set([...lot.doors, ...lot.windows].map((d) => d.id)).has(op.id) ? freshId(lot, window ? 'win' : 'd') : op.id;
    if (!hit || openingBlock(lot, hit.wall.id, hit.at, op.width, window)) {
      lost++;
      continue;
    }
    const placed = { ...op, id, wall: hit.wall.id, at: hit.at };
    if (window) lot.windows.push(placed as LotWindow);
    else lot.doors.push(placed as LotDoor);
  }
  return lost;
}

/** Where an opening's middle is, and which way its wall runs. */
function openingAt(lot: BLot, op: Opening) {
  const w = lot.walls.find((x) => x.id === op.wall)!;
  const [x, z] = isH(w) ? [w.x0 + op.at, w.z0] : [w.x0, w.z0 + op.at];
  return { x, z, o: (isH(w) ? 'h' : 'v') as Orient };
}

const sameLot = (a: BLot, b: BLot) => JSON.stringify(a) === JSON.stringify(b);

// ───────────── floors ─────────────

/** Each tile's floor material. */
export function floorGrid(lot: BLot): string[] {
  const { cols, rows } = tilesOf(lot);
  const m = new Array<string>(cols * rows).fill(lot.floors.default);
  for (const r of lot.floors.rects) for (let z = g(r.z); z < g(r.z + r.d); z++) for (let x = g(r.x); x < g(r.x + r.w); x++) if (x >= 0 && z >= 0 && x < cols && z < rows) m[z * cols + x] = r.m;
  return m;
}

/** Tiles as few rects: runs along each row, merged down while they line up. */
export function rectsOf(tiles: number[], cols: number): { x: number; z: number; w: number; d: number }[] {
  const set = new Set(tiles);
  const rows = new Map<number, [number, number][]>();
  for (const t of [...set].sort((a, b) => a - b)) {
    const c = t % cols;
    const r = (t - c) / cols;
    const runs = rows.get(r) ?? (rows.set(r, []), rows.get(r)!);
    const last = runs[runs.length - 1];
    if (last && last[1] === c) last[1] = c + 1;
    else runs.push([c, c + 1]);
  }
  const open = new Map<string, { c0: number; c1: number; r0: number; r1: number }>();
  const out: { c0: number; c1: number; r0: number; r1: number }[] = [];
  for (const r of [...rows.keys()].sort((a, b) => a - b)) {
    for (const [c0, c1] of rows.get(r)!) {
      const k = `${c0}:${c1}`;
      const o = open.get(k);
      if (o && o.r1 === r) o.r1 = r + 1;
      else {
        const n = { c0, c1, r0: r, r1: r + 1 };
        open.set(k, n);
        out.push(n);
      }
    }
  }
  return out.map((b) => ({ x: b.c0 * T, z: b.r0 * T, w: (b.c1 - b.c0) * T, d: (b.r1 - b.r0) * T }));
}

/** Lay `m` on these tiles (those not `m` already); rects nothing shows any more are dropped. */
function layFloor(lot: BLot, tiles: number[], m: string): number {
  const { cols } = tilesOf(lot);
  const now = floorGrid(lot);
  const need = tiles.filter((t) => t >= 0 && now[t] !== m);
  if (!need.length) return 0;
  lot.floors.rects.push(...rectsOf(need, cols).map((r) => ({ ...r, m })));
  // Drop rects every tile of which a later one covers.
  const top = new Int32Array(now.length).fill(-1);
  lot.floors.rects.forEach((r, i) => {
    for (let z = g(r.z); z < g(r.z + r.d); z++) for (let x = g(r.x); x < g(r.x + r.w); x++) top[z * cols + x] = i;
  });
  const shown = new Set(top);
  lot.floors.rects = lot.floors.rects.filter((_, i) => shown.has(i));
  return need.length;
}

export function paintFloor(lot: BLot, bought: Placed[], tiles: number[], m: string): Outcome {
  const next = tidy(lot);
  if (!layFloor(next, tiles, m)) return { error: 'It’s that floor already' };
  return { lot: next, bought };
}

// ───────────── room, wall ─────────────

/** Snap a dragged wall: from the grid point nearest the start, straight along the axis it moves
 *  most along, in `step` metres (1, or 0.5 with Alt). */
export function snapWall(a: [number, number], b: [number, number], step: number): [number, number, number, number] | null {
  const s = (v: number) => Math.round(v / step) * step;
  const [x0, z0] = [s(a[0]), s(a[1])];
  const dx = b[0] - x0;
  const dz = b[1] - z0;
  const [x1, z1] = Math.abs(dx) >= Math.abs(dz) ? [s(b[0]), z0] : [x0, s(b[1])];
  if (x1 === x0 && z1 === z0) return null;
  return [Math.min(x0, x1), Math.min(z0, z1), Math.max(x0, x1), Math.max(z0, z1)];
}

/** Snap a dragged rectangle to whole tiles (corners on the grid). */
export function snapRect(a: [number, number], b: [number, number]): [number, number, number, number] {
  const s = (v: number) => Math.round(v / T) * T;
  return [Math.min(s(a[0]), s(b[0])), Math.min(s(a[1]), s(b[1])), Math.max(s(a[0]), s(b[0])), Math.max(s(a[1]), s(b[1]))];
}

const inLot = (lot: BLot, x: number, z: number) => x >= -1e-6 && z >= -1e-6 && x <= lot.size.w + 1e-6 && z <= lot.size.d + 1e-6;

function wallEdges(x0: number, z0: number, x1: number, z1: number, kind: WallKind) {
  const out: { k: string; kind: WallKind }[] = [];
  if (z0 === z1) for (let c = g(x0); c < g(x1); c++) out.push({ k: ekey('h', c, g(z0)), kind });
  else for (let r = g(z0); r < g(z1); r++) out.push({ k: ekey('v', g(x0), r), kind });
  return out;
}

/** A wall on any of these edges would run through a piece of furniture. */
function throughPiece(lot: BLot, bought: Placed[], keys: string[]): boolean {
  const { cols } = tilesOf(lot);
  const sides = keys.map((k) => {
    const e = parse(k);
    return e.o === 'h' ? [(e.r - 1) * cols + e.c, e.r * cols + e.c] : [e.r * cols + e.c - 1, e.r * cols + e.c];
  });
  return everyPiece(lot, bought).some(({ p }) => {
    const ts = new Set(pieceTiles(lot, p));
    return ts.size > 1 && sides.some(([a, b]) => ts.has(a) && ts.has(b));
  });
}

function addWalls(lot: BLot, bought: Placed[], edges: { k: string; kind: WallKind }[]): Outcome {
  const next = tidy(lot);
  if (throughPiece(next, bought, edges.map((e) => e.k))) return { error: 'There’s furniture in the way' };
  addEdges(next, edges);
  if (cutOff(next).length) return { error: 'That wall would run into a door or window' };
  if (sameLot(next, tidy(lot))) return { error: 'There’s a wall there already' };
  return { lot: next, bought };
}

/** Room: four walls around the rectangle (where there are none yet), and its floor. */
export function drawRoom(lot: BLot, bought: Placed[], x0: number, z0: number, x1: number, z1: number, kind: WallKind, floor: string | null): Outcome {
  if (x1 - x0 < 1 || z1 - z0 < 1) return { error: 'A room is at least 1 × 1 m' };
  if (!inLot(lot, x0, z0) || !inLot(lot, x1, z1)) return { error: 'Keep it on the lot' };
  const next = tidy(lot);
  const edges = [...wallEdges(x0, z0, x1, z0, kind), ...wallEdges(x0, z1, x1, z1, kind), ...wallEdges(x0, z0, x0, z1, kind), ...wallEdges(x1, z0, x1, z1, kind)];
  if (throughPiece(next, bought, edges.map((e) => e.k))) return { error: 'There’s furniture in the way' };
  addEdges(next, edges);
  if (cutOff(next).length) return { error: 'That wall would run into a door or window' };
  const { cols } = tilesOf(next);
  const tiles: number[] = [];
  for (let r = g(z0); r < g(z1); r++) for (let c = g(x0); c < g(x1); c++) tiles.push(r * cols + c);
  if (floor) layFloor(next, tiles, floor);
  if (sameLot(next, tidy(lot))) return { error: 'That room is there already' };
  return { lot: next, bought };
}

/** Wall: one straight run along the grid. */
export function drawWall(lot: BLot, bought: Placed[], x0: number, z0: number, x1: number, z1: number, kind: WallKind): Outcome {
  if (!inLot(lot, x0, z0) || !inLot(lot, x1, z1)) return { error: 'Keep it on the lot' };
  return addWalls(lot, bought, wallEdges(x0, z0, x1, z1, kind));
}

// ───────────── doors and windows ─────────────

/** The place along a wall nearest (x, z), snapped to 0.25 m and kept inside the wall for `width`. */
export function atOnWall(w: LotWall, x: number, z: number, width: number): number {
  const raw = isH(w) ? x - w.x0 : z - w.z0;
  const at = Math.round(raw / 0.25) * 0.25;
  return Math.min(lenOf(w) - width / 2, Math.max(width / 2, at));
}

/** Door or window: put one in a wall (one already there of the same sort changes kind and width). */
export function placeOpening(lot: BLot, bought: Placed[], wallId: string, x: number, z: number, what: { door: DoorKind } | { window: number }): Outcome {
  const next = tidy(lot);
  const w = next.walls.find((x) => x.id === wallId);
  if (!w) return { error: 'Not on a wall' };
  const window = 'window' in what;
  const width = window ? what.window : DOOR_WIDTH[what.door];
  const list: Opening[] = window ? next.windows : next.doors;
  const raw = isH(w) ? x - w.x0 : z - w.z0;
  const there = list.find((o) => o.wall === wallId && Math.abs(o.at - raw) <= o.width / 2);
  if (there) {
    // Changing one: same place (moved in a little if it grew), new kind and width.
    const at = Math.min(lenOf(w) - width / 2, Math.max(width / 2, there.at));
    const why = openingBlock(next, wallId, at, width, window, there.id);
    if (why) return { error: why };
    if (!window && (there as LotDoor).kind === what.door && there.width === width) return { error: 'It’s that door already' };
    if (window && there.width === width) return { error: 'It’s that window already' };
    Object.assign(there, { at, width }, window ? {} : { kind: what.door });
    return { lot: next, bought };
  }
  const at = atOnWall(w, x, z, width);
  const why = openingBlock(next, wallId, at, width, window);
  if (why) return { error: why };
  if (window) next.windows.push({ id: freshId(next, 'win'), wall: wallId, at, width, kind: 'standard' });
  else next.doors.push({ id: freshId(next, 'd'), wall: wallId, at, width, kind: what.door });
  return { lot: next, bought };
}

// ───────────── sledgehammer ─────────────

export function removeOpening(lot: BLot, bought: Placed[], id: string): Outcome {
  const next = tidy(lot);
  const n = next.doors.length + next.windows.length;
  next.doors = next.doors.filter((d) => d.id !== id);
  next.windows = next.windows.filter((w) => w.id !== id);
  if (next.doors.length + next.windows.length === n) return { error: 'Nothing to knock out there' };
  return { lot: next, bought };
}

/** The stretch of a wall around (x, z) between the walls that meet it (or its ends), as unit indices. */
export function stretchAt(lot: BLot, wallId: string, x: number, z: number): [number, number] | null {
  const w = lot.walls.find((v) => v.id === wallId);
  if (!w) return null;
  const n = g(lenOf(w));
  const at = Math.min(n - 1, Math.max(0, Math.floor((isH(w) ? x - w.x0 : z - w.z0) / T)));
  const m = edgeMap(lot);
  const meets = (s: number) => {
    const [cx, cz] = isH(w) ? [g(w.x0) + s, g(w.z0)] : [g(w.x0), g(w.z0) + s];
    return isH(w) ? m.has(ekey('v', cx, cz - 1)) || m.has(ekey('v', cx, cz)) : m.has(ekey('h', cx - 1, cz)) || m.has(ekey('h', cx, cz));
  };
  let a = at;
  while (a > 0 && !meets(a)) a--;
  let b = at + 1;
  while (b < n && !meets(b)) b++;
  return [a, b];
}

/** Sledgehammer on a wall: the stretch between the walls that meet it goes (its doors too). */
export function knockDown(lot: BLot, bought: Placed[], wallId: string, x: number, z: number): Outcome {
  const next = tidy(lot);
  const w = next.walls.find((v) => v.id === wallId);
  const s = stretchAt(next, wallId, x, z);
  if (!w || !s) return { error: 'Not on a wall' };
  const drop = new Set<number>();
  for (let i = s[0]; i < s[1]; i++) drop.add(i);
  cutWall(next, w, drop);
  return { lot: next, bought };
}

/** Sledgehammer on a room's floor: its walls go, but those another room still needs; a room
 *  inside another merges into it, a room on its own leaves bare ground and its furniture goes
 *  to storage. */
export function removeRoom(lot: BLot, bought: Placed[], L: OfficeLayout, k: number): Outcome {
  const tiles = roomTiles(L, k);
  if (!tiles.length) return { error: 'Not a room' };
  const next = tidy(lot);
  const labels = labelsOf(L);
  const into = mergeLabel(L, k);
  for (const t of tiles) labels[t] = into;
  rewall(next, L.tiles.cols, L.tiles.rows, labels, tiles, () => 'solid');
  const inRoom = new Set(tiles);
  const e: Edit = { lot: next, bought };
  next.labels = next.labels.filter((l) => !inRoom.has(tileAt(next, l.x, l.z)));
  if (into >= 0) return e;
  const n = putAway(e, new Set(everyPiece(next, bought).filter(({ p }) => inRoom.has(tileAt(next, p.x, p.z))).map(({ p }) => p.uid)));
  return { ...e, note: storedNote(n) };
}

// ───────────── wall paint ─────────────

function setPaint(lot: BLot, wall: string, side: number, paint: string): boolean {
  const w = lot.walls.find((x) => x.id === wall);
  if (!w || w.kind === 'glass') return false;
  const was = lot.paint.find((p) => p.wall === wall && p.side === side);
  if (was?.paint === paint) return false;
  lot.paint = lot.paint.filter((p) => !(p.wall === wall && p.side === side));
  lot.paint.push({ wall, side, paint });
  return true;
}

/** Wall paint on one side of a wall (1: the +z or +x side). */
export function paintSide(lot: BLot, bought: Placed[], wall: string, side: number, paint: string): Outcome {
  const next = tidy(lot);
  if (next.walls.find((w) => w.id === wall)?.kind === 'glass') return { error: 'Glass can’t be painted' };
  if (!setPaint(next, wall, side, paint)) return { error: 'It’s that paint already' };
  return { lot: next, bought };
}

/** The walls around a room, each with the side that faces in. */
export function roomSides(lot: BLot, L: OfficeLayout, k: number): { wall: string; side: number }[] {
  const cols = L.tiles.cols;
  const m = edgeMap(lot);
  const out = new Map<string, { wall: string; side: number }>();
  for (const t of roomTiles(L, k)) {
    const c = t % cols;
    const r = (t - c) / cols;
    for (const [key, side] of [
      [ekey('h', c, r), 1],
      [ekey('h', c, r + 1), -1],
      [ekey('v', c, r), 1],
      [ekey('v', c + 1, r), -1],
    ] as [string, number][]) {
      const w = m.get(key);
      if (w) out.set(`${w.id}:${side}`, { wall: w.id, side });
    }
  }
  return [...out.values()];
}

/** Wall paint on the inside of every wall around a room (whole walls: a wall shared along two rooms is painted along both). */
export function paintRoom(lot: BLot, bought: Placed[], L: OfficeLayout, k: number, paint: string): Outcome {
  const next = tidy(lot);
  let any = false;
  for (const s of roomSides(next, L, k)) any = setPaint(next, s.wall, s.side, paint) || any;
  if (!any) return { error: 'It’s that paint already' };
  return { lot: next, bought };
}

// ───────────── blueprints: a room as data (move, turn, the library) ─────────────

/** A room relative to its bounding box's north-west corner: its tiles, the walls around it (with
 *  the paint on each side), its doors and windows, floor, furniture and name. */
export interface Blueprint {
  /** Size of the bounding box, in tiles. */
  w: number;
  d: number;
  tiles: [number, number][];
  /** Its floor, per tile (same order as `tiles`). */
  floor: string[];
  edges: { o: Orient; c: number; r: number; kind: WallKind; inside?: string; outside?: string }[];
  openings: { op: Opening; window: boolean; x: number; z: number; o: Orient }[];
  furniture: (Placed & { bought: boolean })[];
  name: string | null;
}

export function blueprint(lot: BLot, bought: Placed[], L: OfficeLayout, k: number): Blueprint {
  const box = roomBox(L, k);
  const cols = L.tiles.cols;
  const [ox, oz] = [box.c0 * T, box.r0 * T];
  const m = edgeMap(lot);
  const fl = floorGrid(lot);
  const inRoom = new Set(box.tiles);
  const paintOf = (wall: string, side: number) => lot.paint.find((p) => p.wall === wall && p.side === side)?.paint;
  const edges: Blueprint['edges'] = [];
  for (const t of box.tiles) {
    const c = t % cols;
    const r = (t - c) / cols;
    for (const [o, ec, er, side, other] of [
      ['h', c, r, 1, t - cols],
      ['h', c, r + 1, -1, t + cols],
      ['v', c, r, 1, c > 0 ? t - 1 : -1],
      ['v', c + 1, r, -1, c < cols - 1 ? t + 1 : -1],
    ] as [Orient, number, number, number, number][]) {
      if (inRoom.has(other)) continue;
      const w = m.get(ekey(o, ec, er));
      if (!w) continue;
      edges.push({ o, c: ec - box.c0, r: er - box.r0, kind: w.kind, inside: paintOf(w.id, side), outside: paintOf(w.id, -side) });
    }
  }
  const onEdge = new Set(edges.map((e) => ekey(e.o, e.c + box.c0, e.r + box.r0)));
  const openings: Blueprint['openings'] = [];
  for (const [op, window] of [...lot.doors.map((d) => [d, false] as const), ...lot.windows.map((w) => [w, true] as const)]) {
    const at = openingAt(lot, op);
    const k2 = at.o === 'h' ? ekey('h', Math.floor(at.x / T), g(at.z)) : ekey('v', g(at.x), Math.floor(at.z / T));
    if (onEdge.has(k2)) openings.push({ op: { ...op }, window, x: at.x - ox, z: at.z - oz, o: at.o });
  }
  const furniture = everyPiece(lot, bought)
    .filter(({ p }) => inRoom.has(tileAt(lot, p.x, p.z)))
    .map(({ p, bought: b }) => ({ ...p, x: p.x - ox, z: p.z - oz, bought: b }));
  const label = lot.labels.find((l) => inRoom.has(tileAt(lot, l.x, l.z)));
  return {
    w: box.c1 - box.c0,
    d: box.r1 - box.r0,
    tiles: box.tiles.map((t) => [(t % cols) - box.c0, Math.floor(t / cols) - box.r0]),
    floor: box.tiles.map((t) => fl[t]),
    edges,
    openings,
    furniture,
    name: label?.name ?? null,
  };
}

/** A blueprint turned a quarter (clockwise seen from above) `turns` times. */
export function turned(bp: Blueprint, turns: number): Blueprint {
  let b = bp;
  for (let i = 0; i < ((turns % 4) + 4) % 4; i++) {
    const D = b.d;
    const pt = (x: number, z: number): [number, number] => [D * T - z, x];
    const edge = (e: Blueprint['edges'][number]) => (e.o === 'h' ? { ...e, o: 'v' as Orient, c: D - e.r, r: e.c } : { ...e, o: 'h' as Orient, c: D - e.r - 1, r: e.c });
    b = {
      ...b,
      w: b.d,
      d: b.w,
      tiles: b.tiles.map(([c, r]) => [D - 1 - r, c]),
      edges: b.edges.map(edge),
      openings: b.openings.map((o) => {
        const [x, z] = pt(o.x, o.z);
        return { ...o, x, z, o: o.o === 'h' ? 'v' : 'h' };
      }),
      furniture: b.furniture.map((p) => {
        const [x, z] = pt(p.x, p.z);
        return { ...p, x, z, rot: Math.atan2(Math.sin(p.rot - QUARTER), Math.cos(p.rot - QUARTER)) };
      }),
    };
  }
  return b;
}

/**
 * Put a blueprint down with its corner on tile (c, r), on free ground (`labels` −1 there): walls
 * where it meets anything else, its doors, windows, paint, floor and furniture. `keep`: the same
 * ids (moving a room); otherwise everything is new (a room from the library), and the pieces
 * bought through Buy are bought again.
 */
function stamp(lot: BLot, bought: Placed[], labels: Int32Array, bp: Blueprint, c: number, r: number, keep: boolean, mirror = new Set<string>(), rects: FloorRect[] = []): Outcome {
  const { cols, rows } = tilesOf(lot);
  const tiles: number[] = [];
  for (const [tc, tr] of bp.tiles) {
    const [x, y] = [tc + c, tr + r];
    if (x < 0 || y < 0 || x >= cols || y >= rows) return { error: 'Keep it on the lot' };
    tiles.push(y * cols + x);
  }
  // On one piece of ground (outside, or the floor of one room) with nothing standing there.
  if (tiles.some((t) => labels[t] !== labels[tiles[0]])) return { error: 'Put it down inside one room, or outside' };
  const under = new Set(tiles);
  const mine = new Set(bp.furniture.map((p) => p.uid));
  if (everyPiece(lot, bought).some(({ p }) => !mine.has(p.uid) && pieceTiles(lot, p).some((t) => under.has(t)))) return { error: 'Something is in the way' };
  const ROOM = 1 << 20;
  const after = labels.slice();
  for (const t of tiles) after[t] = ROOM;
  const kinds = new Map(bp.edges.map((e) => [ekey(e.o, e.c + c, e.r + r), e]));
  rewall(lot, cols, rows, after, tiles, (k) => kinds.get(k)?.kind ?? 'solid');
  if (cutOff(lot).length) return { error: 'A wall would run into a door or window there' };
  // Paint, on whichever wall now stands on each edge.
  const m = edgeMap(lot);
  const inRoom = new Set(tiles);
  for (const [k, e] of kinds) {
    const w = m.get(k);
    if (!w || w.kind === 'glass') continue;
    const { o, c: ec, r: er } = parse(k);
    const plus = o === 'h' ? er * cols + ec : er * cols + ec; // the tile on the +z / +x side
    const side = inRoom.has(plus) && (o === 'h' ? er < rows : ec < cols) ? 1 : -1;
    if (e.inside) setPaint(lot, w.id, side, e.inside);
    if (e.outside) setPaint(lot, w.id, -side, e.outside);
  }
  const ox = c * T;
  const oz = r * T;
  const lost = rehome(
    lot,
    bp.openings.map((o) => ({ ...o, op: keep ? o.op : { ...o.op, id: freshId(lot, o.window ? 'win' : 'd') }, x: o.x + ox, z: o.z + oz })),
  );
  // Floors: the room's own floor rects come along; then, by material, the tiles that aren't that yet.
  lot.floors.rects.push(...rects.map((f) => ({ ...f, x: f.x + ox, z: f.z + oz })));
  const byM = new Map<string, number[]>();
  bp.tiles.forEach(([tc, tr], i) => (byM.get(bp.floor[i]) ?? (byM.set(bp.floor[i], []), byM.get(bp.floor[i])!)).push((tr + r) * cols + tc + c));
  for (const [mat, ts] of byM) layFloor(lot, ts, mat);
  // Furniture.
  let shop = bought;
  for (const p of bp.furniture) {
    const { bought: b, ...rest } = p;
    const q: Placed = { ...rest, x: p.x + ox, z: p.z + oz, uid: keep ? p.uid : freshId(lot, 'p') };
    if (b) shop = [...shop.filter((s) => s.uid !== q.uid), q];
    if (!b || mirror.has(p.uid)) lot.furniture.push(q);
  }
  if (bp.name) lot.labels.push({ x: (bp.tiles[0][0] + c + 0.5) * T, z: (bp.tiles[0][1] + r + 0.5) * T, name: bp.name });
  return { lot, bought: shop, note: lost ? `${lost === 1 ? 'A door or window' : `${lost} doors and windows`} didn’t fit and went.` : undefined };
}

/** Take a room off the lot (for moving it): walls only it needs, its openings, furniture and name. */
function lift(lot: BLot, bought: Placed[], L: OfficeLayout, k: number, bp: Blueprint) {
  const tiles = roomTiles(L, k);
  const ids = new Set(bp.openings.map((o) => o.op.id));
  lot.doors = lot.doors.filter((d) => !ids.has(d.id));
  lot.windows = lot.windows.filter((w) => !ids.has(w.id));
  const labels = labelsOf(L);
  const into = mergeLabel(L, k);
  for (const t of tiles) labels[t] = into;
  rewall(lot, L.tiles.cols, L.tiles.rows, labels, tiles, () => 'solid');
  const uids = new Set(bp.furniture.map((p) => p.uid));
  lot.furniture = lot.furniture.filter((p) => !uids.has(p.uid));
  const inRoom = new Set(tiles);
  lot.labels = lot.labels.filter((l) => !inRoom.has(tileAt(lot, l.x, l.z)));
  // Floor rects wholly inside the room go with it (a moved rect costs nothing).
  const cols = L.tiles.cols;
  const inside = (f: FloorRect) => {
    for (let z = g(f.z); z < g(f.z + f.d); z++) for (let x = g(f.x); x < g(f.x + f.w); x++) if (!inRoom.has(z * cols + x)) return false;
    return true;
  };
  const rects = lot.floors.rects.filter(inside);
  lot.floors.rects = lot.floors.rects.filter((f) => !inside(f));
  return { labels, bought: bought.filter((p) => !uids.has(p.uid)), rects };
}

/** A floor rect (relative to a blueprint `w` × `d` tiles) turned like `turned`. */
function turnRect(f: FloorRect, w: number, d: number, turns: number): FloorRect {
  let out = f;
  let [W, D] = [w, d];
  for (let i = 0; i < ((turns % 4) + 4) % 4; i++) {
    out = { ...out, x: D * T - (out.z + out.d), z: out.x, w: out.d, d: out.w };
    [W, D] = [D, W];
  }
  return out;
}

/** Move room: pick a room up with what's in it and put it down `dc`, `dr` tiles away, turned `turns` quarters. Free. */
export function moveRoom(lot: BLot, bought: Placed[], L: OfficeLayout, k: number, dc: number, dr: number, turns: number): Outcome {
  if (!roomTiles(L, k).length) return { error: 'Not a room' };
  if (!dc && !dr && !(turns % 4)) return { error: 'Move it somewhere' };
  const start = tidy(lot);
  const bp = blueprint(start, bought, L, k);
  const box = roomBox(L, k);
  const next = tidy(lot);
  const lifted = lift(next, bought, L, k, bp);
  const t = turned(bp, turns);
  // Turning keeps the middle where it was.
  const c = box.c0 + dc + Math.round((bp.w - t.w) / 2);
  const r = box.r0 + dr + Math.round((bp.d - t.d) / 2);
  // Pieces bought through Buy stay in the lot's mirror if they were in it.
  const rects = lifted.rects.map((f) => turnRect({ ...f, x: f.x - box.c0 * T, z: f.z - box.r0 * T }, bp.w, bp.d, turns));
  const o = stamp(next, lifted.bought, lifted.labels, t, c, r, true, new Set(start.furniture.map((p) => p.uid)), rects);
  if (failed(o)) return o;
  adopt(start, o.lot);
  return o;
}

/** Place saved room: a library blueprint on free ground with its corner at tile (c, r), turned. */
export function placeBlueprint(lot: BLot, bought: Placed[], L: OfficeLayout, bp: Blueprint, c: number, r: number, turns: number): Outcome {
  return stamp(tidy(lot), bought, labelsOf(L), turned(bp, turns), c, r, false);
}

// ───────────── push / pull ─────────────

export type Side = 'n' | 's' | 'e' | 'w';

/** Push/pull: move one side of a rectangular room `steps` tiles out (+) or in (−). Its doors and
 *  windows move with it; furniture that no longer fits goes to storage. */
export function pushPull(lot: BLot, bought: Placed[], L: OfficeLayout, k: number, side: Side, steps: number): Outcome {
  const box = roomBox(L, k);
  if (!box.tiles.length) return { error: 'Not a room' };
  if (!box.rect) return { error: 'Push/pull works on rectangular rooms' };
  if (!steps) return { error: 'Drag the arrow' };
  const { cols, rows } = L.tiles;
  const { c0, r0, c1, r1 } = box;
  const sign = side === 'e' || side === 's' ? 1 : -1;
  const along = side === 'e' || side === 'w';
  const line = { e: c1, w: c0, s: r1, n: r0 }[side];
  const to = line + sign * steps;
  const [lo, hi] = along ? [c0, c1] : [r0, r1];
  const [nlo, nhi] = side === 'e' || side === 's' ? [lo, to] : [to, hi];
  if (nhi - nlo < 2) return { error: 'A room is at least 1 m across' };
  if (to < 0 || to > (along ? cols : rows)) return { error: 'Keep it on the lot' };

  const start = tidy(lot);
  const bp = blueprint(start, bought, L, k);
  const next = tidy(lot);
  const labels = labelsOf(L);
  const after = labels.slice();
  const changed: number[] = [];
  const [p0, p1] = along ? [r0, r1] : [c0, c1];
  for (let p = p0; p < p1; p++) {
    const tile = (q: number) => (along ? p * cols + q : q * cols + p);
    if (steps > 0) {
      for (let q = Math.min(line, to); q < Math.max(line, to); q++) (after[tile(q)] = k), changed.push(tile(q));
    } else {
      // Shrinking: the strip joins whatever is beyond the old side.
      const beyondQ = sign > 0 ? line : line - 1;
      const beyond = beyondQ < 0 || beyondQ >= (along ? cols : rows) ? -1 : labels[tile(beyondQ)];
      for (let q = Math.min(line, to); q < Math.max(line, to); q++) {
        if (labels[tile(q)] !== k) return { error: 'Another room is in the way' };
        (after[tile(q)] = beyond), changed.push(tile(q));
      }
    }
  }
  // The moved side's doors and windows, to put back on the new side.
  const sideOrient: Orient = along ? 'v' : 'h';
  const onSide = (o: { x: number; z: number; o: Orient }) => o.o === sideOrient && Math.abs((along ? o.x : o.z) - line * T + (along ? c0 : r0) * T) < 1e-6;
  const moving = bp.openings.filter(onSide);
  const ids = new Set(moving.map((o) => o.op.id));
  next.doors = next.doors.filter((d) => !ids.has(d.id));
  next.windows = next.windows.filter((w) => !ids.has(w.id));
  // New walls are like the room's own: the moved side's, moved; the others where they are.
  const shift = sign * steps;
  const like = bp.edges.map((e) => {
    const [c, r] = [e.c + c0, e.r + r0];
    const moved = e.o === sideOrient && (along ? c : r) === line;
    return { ...e, c: moved && along ? c + shift : c, r: moved && !along ? r + shift : r };
  });
  const kindOf = (key: string) => {
    const e = parse(key);
    let best: (typeof like)[number] | null = null;
    let bd = Infinity;
    for (const l of like) {
      if (l.o !== e.o) continue;
      const d = Math.abs((e.o === 'h' ? l.r - e.r : l.c - e.c) * 1000) + Math.abs(e.o === 'h' ? l.c - e.c : l.r - e.r);
      if (d < bd) (bd = d), (best = l);
    }
    return best?.kind ?? 'solid';
  };
  rewall(next, cols, rows, after, changed, kindOf);
  if (cutOff(next).length) return { error: 'A wall would run into a door or window there' };
  const lost = rehome(
    next,
    moving.map((o) => ({ ...o, x: o.x + c0 * T + (along ? shift * T : 0), z: o.z + r0 * T + (along ? 0 : shift * T) })),
  );
  adopt(start, next);
  // Furniture where tiles changed hands: it stays if it still stands in one room.
  const e: Edit = { lot: next, bought };
  const touched = new Set([...box.tiles, ...changed]);
  const away = new Set<string>();
  for (const { p } of everyPiece(next, bought)) {
    const ts = pieceTiles(next, p);
    if (!ts.some((t) => touched.has(t))) continue;
    const home = after[ts[0]];
    if (home === -1 && labels[tileAt(next, p.x, p.z)] !== -1) away.add(p.uid);
    else if (ts.some((t) => t < 0 || after[t] !== home)) away.add(p.uid);
  }
  const n = putAway(e, away);
  const notes = [storedNote(n), lost ? `${lost === 1 ? 'A door or window' : `${lost} doors and windows`} didn’t fit and went.` : undefined].filter(Boolean);
  return { ...e, note: notes.join(' ') || undefined };
}

// ───────────── storage ─────────────

/** Take a piece out of storage and put it at (x, z), turned `rot`. */
export function fromStorage(lot: BLot, bought: Placed[], uid: string, x: number, z: number, rot: number): Outcome {
  const next = tidy(lot);
  const p = next.storage?.find((s) => s.uid === uid);
  if (!p) return { error: 'Not in storage' };
  next.storage = next.storage!.filter((s) => s.uid !== uid);
  next.furniture.push({ ...p, x, z, y: 0, rot });
  return { lot: next, bought };
}

/** Why a stored piece can't go at (x, z) (null: it can): in one room, on free floor, nothing in the way. */
export function storageBlock(L: OfficeLayout, lot: BLot, p: Placed): string | null {
  const ts = pieceTiles(lot, p);
  if (ts.some((t) => t < 0)) return 'Keep it on the lot';
  const room = L.roomOf[ts[0]];
  if (room < 0 || ts.some((t) => L.roomOf[t] !== room)) return 'It has to stand inside one room';
  const f = BY_ID.get(p.item);
  if (!f) return 'Not in the catalog';
  const q = Math.abs(Math.round(p.rot / QUARTER)) % 2 === 1;
  const [w, d] = q ? [f.d ?? 0.5, f.w ?? 0.5] : [f.w ?? 0.5, f.d ?? 0.5];
  const x0 = Math.max(0, Math.floor((p.x - w / 2) / 0.25 + 1e-6));
  const z0 = Math.max(0, Math.floor((p.z - d / 2) / 0.25 + 1e-6));
  const x1 = Math.min(L.cols - 1, Math.ceil((p.x + w / 2) / 0.25 - 1e-6) - 1);
  const z1 = Math.min(L.rows - 1, Math.ceil((p.z + d / 2) / 0.25 - 1e-6) - 1);
  for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) if (L.blocked[z * L.cols + x]) return 'Something is in the way';
  return null;
}

/** Snap a stored piece to the walking grid like Buy does: whole cells, quarter turns. */
export function snapPiece(item: string, x: number, z: number, rot: number): { x: number; z: number; rot: number } {
  const f = BY_ID.get(item);
  const r = (((Math.round(rot / QUARTER) % 4) + 4) % 4) * QUARTER;
  const q = r / QUARTER === 1 || r / QUARTER === 3;
  const [w, d] = q ? [f?.d ?? 0.5, f?.w ?? 0.5] : [f?.w ?? 0.5, f?.d ?? 0.5];
  const cw = Math.max(1, Math.round(w / 0.25));
  const cd = Math.max(1, Math.round(d / 0.25));
  return { x: (Math.round(x / 0.25 - cw / 2) + cw / 2) * 0.25, z: (Math.round(z / 0.25 - cd / 2) + cd / 2) * 0.25, rot: r };
}

