// Build mode's walls on the tile grid. A wall is a run of 0.5 m unit edges, so the tools add, cut
// and move walls edge by edge while the lot keeps whole walls: a cut wall keeps its id on its
// longest piece (with the doors, windows and paint that still fit), a new run of wall extends a
// wall it continues, and a wall that moved keeps the id of the one it replaced (`adopt`), so
// moving costs nothing and gifts stay gifts. Pure: no three.js.
//
// Edges: `h{c},{r}` runs along x from tile corner (c, r) to (c + 1, r); `v{c},{r}` along z from
// (c, r) to (c, r + 1). Tile (c, r) has `h{c},{r}` on its north side and `v{c},{r}` on its west.

import type { Lot } from '../../lib/gen/Lot';
import type { LotDoor } from '../../lib/gen/LotDoor';
import type { LotWall } from '../../lib/gen/LotWall';
import type { LotWindow } from '../../lib/gen/LotWindow';
import type { WallKind } from '../../lib/gen/WallKind';

export const T = 0.5;
/** A lot as build mode edits it (its `storage` holds what no longer fit). */
export type BLot = Lot;
export type Orient = 'h' | 'v';
export type Opening = LotDoor | LotWindow;

export const ekey = (o: Orient, c: number, r: number) => `${o}${c},${r}`;
export function parse(k: string): { o: Orient; c: number; r: number } {
  const [c, r] = k.slice(1).split(',').map(Number);
  return { o: k[0] as Orient, c, r };
}
/** Metres to grid steps. */
export const g = (v: number) => Math.round(v / T);
export const isH = (w: Pick<LotWall, 'z0' | 'z1'>) => Math.abs(w.z0 - w.z1) < 1e-9;
export const lenOf = (w: Pick<LotWall, 'x0' | 'x1' | 'z0' | 'z1'>) => Math.abs(w.x1 - w.x0) + Math.abs(w.z1 - w.z0);

/** A copy of the lot with every wall running east or south (openings follow), as the core keeps it. */
export function tidy(lot: Lot): BLot {
  const out = structuredClone(lot) as BLot;
  for (const w of out.walls) {
    if (w.x1 < w.x0 || w.z1 < w.z0) {
      [w.x0, w.x1, w.z0, w.z1] = [w.x1, w.x0, w.z1, w.z0];
      const l = lenOf(w);
      for (const o of [...out.doors, ...out.windows]) if (o.wall === w.id) o.at = l - o.at;
    }
  }
  return out;
}

/** A wall's unit edges, from its start. */
export function edgesOf(w: LotWall): string[] {
  const n = g(lenOf(w));
  const out: string[] = [];
  for (let i = 0; i < n; i++) out.push(isH(w) ? ekey('h', g(w.x0) + i, g(w.z0)) : ekey('v', g(w.x0), g(w.z0) + i));
  return out;
}

/** Edge → the wall on it (the first, if two overlap). */
export function edgeMap(lot: Lot): Map<string, LotWall> {
  const m = new Map<string, LotWall>();
  for (const w of lot.walls) for (const k of edgesOf(w)) if (!m.has(k)) m.set(k, w);
  return m;
}

/** Everything in the lot with an id (and every id that was ever a gift: never reuse one). */
export function idsOf(lot: BLot): Set<string> {
  return new Set([...lot.walls.map((w) => w.id), ...lot.doors.map((d) => d.id), ...lot.windows.map((w) => w.id), ...lot.furniture.map((p) => p.uid), ...(lot.storage ?? []).map((p) => p.uid), ...lot.gift]);
}

let seq = 0;
/** A new id, unused in the lot. */
export function freshId(lot: BLot, prefix: string): string {
  const taken = idsOf(lot);
  for (;;) {
    const id = `${prefix}${Date.now().toString(36).slice(-4)}${(seq++).toString(36)}`;
    if (!taken.has(id)) return id;
  }
}

/** The world point `at` metres along a wall, from its start. */
export function along(w: LotWall, at: number): [number, number] {
  return isH(w) ? [w.x0 + at, w.z0] : [w.x0, w.z0 + at];
}

export const openingsOf = (lot: Lot, wall: string): Opening[] => [...lot.doors, ...lot.windows].filter((o) => o.wall === wall);

/** Remove the unit edges `drop` (indices from the wall's start) from a wall, in place: the longest
 *  piece left keeps its id; openings go to the piece they fit in, or are gone; paint goes to every piece. */
export function cutWall(lot: BLot, w: LotWall, drop: Set<number>) {
  const n = g(lenOf(w));
  const runs: [number, number][] = [];
  for (let i = 0; i < n; i++) {
    if (drop.has(i)) continue;
    const last = runs[runs.length - 1];
    if (last && last[1] === i) last[1] = i + 1;
    else runs.push([i, i + 1]);
  }
  const ops = openingsOf(lot, w.id);
  const wins = new Set<Opening>(lot.windows);
  const paint = lot.paint.filter((p) => p.wall === w.id);
  lot.walls = lot.walls.filter((x) => x !== w);
  lot.doors = lot.doors.filter((d) => d.wall !== w.id);
  lot.windows = lot.windows.filter((x) => x.wall !== w.id);
  lot.paint = lot.paint.filter((p) => p.wall !== w.id);
  if (!runs.length) return;
  const keep = runs.reduce((a, b) => (b[1] - b[0] > a[1] - a[0] ? b : a));
  for (const run of runs) {
    const [s, e] = run;
    const id = run === keep ? w.id : freshId(lot, 'w');
    const piece: LotWall = isH(w) ? { id, x0: w.x0 + s * T, z0: w.z0, x1: w.x0 + e * T, z1: w.z0, kind: w.kind } : { id, x0: w.x0, z0: w.z0 + s * T, x1: w.x0, z1: w.z0 + e * T, kind: w.kind };
    lot.walls.push(piece);
    for (const o of ops) {
      if (o.at - o.width / 2 >= s * T - 1e-6 && o.at + o.width / 2 <= e * T + 1e-6) {
        const moved = { ...o, wall: id, at: o.at - s * T };
        if (wins.has(o)) lot.windows.push(moved as LotWindow);
        else lot.doors.push(moved as LotDoor);
      }
    }
    for (const p of paint) lot.paint.push({ ...p, wall: id });
  }
}

/** Take these unit edges out of whatever walls stand on them. */
export function removeEdges(lot: BLot, keys: Set<string>) {
  for (const w of [...lot.walls]) {
    const drop = new Set<number>();
    edgesOf(w).forEach((k, i) => keys.has(k) && drop.add(i));
    if (drop.size) cutWall(lot, w, drop);
  }
}

/** Put walls on these unit edges (those without one yet): a run that continues a wall of the same
 *  kind lengthens it, others become new walls. Returns the walls added or lengthened. */
export function addEdges(lot: BLot, edges: { k: string; kind: WallKind }[]): string[] {
  const have = edgeMap(lot);
  const lines = new Map<string, { o: Orient; line: number; kind: WallKind; at: number[] }>();
  for (const { k, kind } of edges) {
    if (have.has(k)) continue;
    const e = parse(k);
    const line = e.o === 'h' ? e.r : e.c;
    const id = `${e.o}:${line}:${kind}`;
    let l = lines.get(id);
    if (!l) lines.set(id, (l = { o: e.o, line, kind, at: [] }));
    const pos = e.o === 'h' ? e.c : e.r;
    if (!l.at.includes(pos)) l.at.push(pos);
  }
  const out: string[] = [];
  for (const l of lines.values()) {
    l.at.sort((a, b) => a - b);
    const runs: [number, number][] = [];
    for (const p of l.at) {
      const last = runs[runs.length - 1];
      if (last && last[1] === p) last[1] = p + 1;
      else runs.push([p, p + 1]);
    }
    for (const [s, e] of runs) {
      const on = (w: LotWall) => w.kind === l.kind && (l.o === 'h' ? isH(w) && g(w.z0) === l.line : !isH(w) && g(w.x0) === l.line);
      const start = (w: LotWall) => (l.o === 'h' ? g(w.x0) : g(w.z0));
      const end = (w: LotWall) => (l.o === 'h' ? g(w.x1) : g(w.z1));
      const before = lot.walls.find((w) => on(w) && end(w) === s);
      const after = lot.walls.find((w) => on(w) && start(w) === e);
      if (before) {
        if (l.o === 'h') before.x1 = e * T;
        else before.z1 = e * T;
        out.push(before.id);
      } else if (after) {
        const grow = (start(after) - s) * T;
        if (l.o === 'h') after.x0 = s * T;
        else after.z0 = s * T;
        for (const o of openingsOf(lot, after.id)) o.at += grow;
        out.push(after.id);
      } else {
        const id = freshId(lot, 'w');
        lot.walls.push(l.o === 'h' ? { id, x0: s * T, z0: l.line * T, x1: e * T, z1: l.line * T, kind: l.kind } : { id, x0: l.line * T, z0: s * T, x1: l.line * T, z1: e * T, kind: l.kind });
        out.push(id);
      }
    }
  }
  return out;
}

/**
 * Walls follow regions: on every edge of the tiles in `touched`, a wall stands where the tiles on
 * its two sides are in different regions of `labels` (−1 outside; off the lot is outside) and
 * goes where they are in the same one. New walls are `kindOf(edge)`.
 */
export function rewall(lot: BLot, cols: number, rows: number, labels: ArrayLike<number>, touched: Iterable<number>, kindOf: (k: string) => WallKind) {
  const lab = (c: number, r: number) => (c < 0 || r < 0 || c >= cols || r >= rows ? -1 : labels[r * cols + c]);
  const have = edgeMap(lot);
  const drop = new Set<string>();
  const add: { k: string; kind: WallKind }[] = [];
  const seen = new Set<string>();
  const look = (k: string, a: number, b: number) => {
    if (seen.has(k)) return;
    seen.add(k);
    const want = a !== b;
    if (want && !have.has(k)) add.push({ k, kind: kindOf(k) });
    else if (!want && have.has(k)) drop.add(k);
  };
  for (const t of touched) {
    const c = t % cols;
    const r = (t - c) / cols;
    look(ekey('h', c, r), lab(c, r - 1), lab(c, r));
    look(ekey('h', c, r + 1), lab(c, r), lab(c, r + 1));
    look(ekey('v', c, r), lab(c - 1, r), lab(c, r));
    look(ekey('v', c + 1, r), lab(c, r), lab(c + 1, r));
  }
  if (drop.size) removeEdges(lot, drop);
  if (add.length) addEdges(lot, add);
}

/** Walls that went (in `before`, not in `after`) lend their ids to new walls of the same kind and
 *  direction, longest first and nearest in length: a wall that moved is the same wall. */
export function adopt(before: Lot, after: BLot) {
  const had = new Set(before.walls.map((w) => w.id));
  const now = new Set(after.walls.map((w) => w.id));
  const gone = before.walls.filter((w) => !now.has(w.id));
  const fresh = after.walls.filter((w) => !had.has(w.id)).sort((a, b) => lenOf(b) - lenOf(a));
  for (const w of fresh) {
    let best: LotWall | null = null;
    for (const o of gone) if (o.kind === w.kind && isH(o) === isH(w) && (!best || Math.abs(lenOf(o) - lenOf(w)) < Math.abs(lenOf(best) - lenOf(w)))) best = o;
    if (!best) continue;
    gone.splice(gone.indexOf(best), 1);
    const old = w.id;
    w.id = best.id;
    for (const o of [...after.doors, ...after.windows]) if (o.wall === old) o.wall = best.id;
    for (const p of after.paint) if (p.wall === old) p.wall = best.id;
  }
}

/** The wall standing on the grid line through (x, z) along `o`, and how far along it the point is. */
export function wallThrough(lot: Lot, x: number, z: number, o: Orient): { wall: LotWall; at: number } | null {
  for (const w of lot.walls) {
    if (isH(w) !== (o === 'h')) continue;
    if (o === 'h' ? Math.abs(w.z0 - z) < 1e-6 && x >= w.x0 - 1e-6 && x <= w.x1 + 1e-6 : Math.abs(w.x0 - x) < 1e-6 && z >= w.z0 - 1e-6 && z <= w.z1 + 1e-6) return { wall: w, at: o === 'h' ? x - w.x0 : z - w.z0 };
  }
  return null;
}
