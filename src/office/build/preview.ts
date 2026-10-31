// What an edit would change, as overlay shapes: walls going up in green, walls coming down in red,
// doors and windows going in, floors being laid. Every tool previews the same way: it works out
// the lot after, and this shows the difference. Pure.

import type { Lot } from '../../lib/gen/Lot';
import type { WallKind } from '../../lib/gen/WallKind';
import { edgeMap, isH, parse, T, type Opening } from './grid';
import type { Shape, Tone } from './gfx';

const SHOW_H: Record<WallKind, number> = { exterior: 1.2, solid: 0.95, glass: 1.0, half: 0.32 };

/** Unit edges as runs along their lines, one shape each. */
function runs(keys: string[], h: (k: string) => number, tone: Tone): Shape[] {
  const lines = new Map<string, number[]>();
  for (const k of keys) {
    const e = parse(k);
    const id = `${e.o}${e.o === 'h' ? e.r : e.c}:${h(k)}`;
    (lines.get(id) ?? (lines.set(id, []), lines.get(id)!)).push(e.o === 'h' ? e.c : e.r);
  }
  const out: Shape[] = [];
  for (const [id, at] of lines) {
    const o = id[0];
    const [line, hh] = id.slice(1).split(':').map(Number);
    at.sort((a, b) => a - b);
    let s = at[0];
    for (let i = 1; i <= at.length; i++) {
      if (i < at.length && at[i] === at[i - 1] + 1) continue;
      const e = at[i - 1] + 1;
      out.push(o === 'h' ? { kind: 'wall', x0: s * T, z0: line * T, x1: e * T, z1: line * T, h: hh, tone } : { kind: 'wall', x0: line * T, z0: s * T, x1: line * T, z1: e * T, h: hh, tone });
      s = at[i];
    }
  }
  return out;
}

function openingShape(lot: Lot, o: Opening, tone: Tone): Shape | null {
  const w = lot.walls.find((x) => x.id === o.wall);
  if (!w) return null;
  const [a, b] = [o.at - o.width / 2, o.at + o.width / 2];
  const h = 0.5;
  return isH(w) ? { kind: 'wall', x0: w.x0 + a, z0: w.z0, x1: w.x0 + b, z1: w.z0, h, tone } : { kind: 'wall', x0: w.x0, z0: w.z0 + a, x1: w.x0, z1: w.z0 + b, h, tone };
}

/** The difference between two lots, to draw. `bad`: the edit can't be done (all red). */
export function diffShapes(before: Lot, after: Lot, bad = false): Shape[] {
  const was = edgeMap(before);
  const now = edgeMap(after);
  const up = [...now.keys()].filter((k) => was.get(k)?.kind !== now.get(k)!.kind);
  const down = [...was.keys()].filter((k) => !now.has(k));
  const out: Shape[] = [...runs(up, (k) => SHOW_H[now.get(k)!.kind], bad ? 'bad' : 'ok'), ...runs(down, (k) => SHOW_H[was.get(k)!.kind] + 0.05, 'bad')];
  const key = (o: Opening) => `${o.id}:${o.wall}:${o.at}:${o.width}:${'kind' in o ? o.kind : ''}`;
  const had = new Set([...before.doors, ...before.windows].map(key));
  const has = new Set([...after.doors, ...after.windows].map(key));
  for (const o of [...after.doors, ...after.windows]) if (!had.has(key(o))) out.push(openingShape(after, o, bad ? 'bad' : 'hot')!);
  for (const o of [...before.doors, ...before.windows]) if (!has.has(key(o)) && ![...after.doors, ...after.windows].some((n) => n.id === o.id)) out.push(openingShape(before, o, 'bad')!);
  const rect = (r: { x: number; z: number; w: number; d: number; m: string }) => `${r.x},${r.z},${r.w},${r.d},${r.m}`;
  const laid = new Set(before.floors.rects.map(rect));
  for (const r of after.floors.rects) if (!laid.has(rect(r))) out.push({ kind: 'floor', x: r.x, z: r.z, w: r.w, d: r.d, tone: bad ? 'bad' : 'ok' });
  return out.filter(Boolean);
}
