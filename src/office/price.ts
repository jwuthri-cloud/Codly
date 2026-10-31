// Build mode's prices, the same as the core's (crates/codly-core/src/price.rs), from the same
// table (lots/prices.json) and catalog (furniture.json), so build mode and the map can show live
// prices; the core charges. A lot is a list of priced parts, each with a key that stays the same
// while the part does (a wall keeps its id when it moves; a floor rect is its size and
// material); comparing two lots part by part is diff pricing. Both sides check themselves
// against `__tests__/price-fixture.json`.

import type { Lot } from '../lib/gen/Lot';
import type { LotSizeName } from '../lib/gen/LotSizeName';
import RAW from './furniture.json';
import PRICES from './lots/prices.json';

export type { LotSizeName };

const TILE = 0.5;
const SIZES: LotSizeName[] = ['small', 'medium', 'large'];

// The ready-made designs the core builds in (its build.rs knows the same names).
const KNOWN = ['garage', 'small-office', 'studio'];
const FILES = import.meta.glob<Lot>('./lots/*.json', { eager: true, import: 'default' });

/** Every ready-made design, by name. */
export const TEMPLATES: Record<string, Lot> = Object.fromEntries(KNOWN.flatMap((n) => (FILES[`./lots/${n}.json`] ? [[n, FILES[`./lots/${n}.json`]]] : [])));
export const TEMPLATE_NAMES = Object.keys(TEMPLATES);

const CATALOG = new Map((RAW as { id: string; price: number }[]).map((f) => [f.id, f.price]));

type Table = {
  lots: Record<LotSizeName, { w: number; d: number; price: number }>;
  wall_per_m: Record<string, number>;
  wall_h: Record<string, number>;
  door: Record<string, number>;
  window: Record<string, number>;
  floor_per_m2: number;
  paint_per_m2: number;
  bare_floor: string;
  sell: { furniture: number; build: number; paint: number; lot: number };
};
const T = PRICES as Table;

/** What a part sells back as: walls, doors and windows are `build`; wall paint and floors `paint`. */
export type PartKind = 'furniture' | 'build' | 'paint';

export interface Part {
  key: string;
  /** Its id or uid, for the gift marks (paint and floors have none). */
  id: string | null;
  kind: PartKind;
  price: number;
}

const coins = (v: number) => Math.round(Math.max(0, v));

/** A plot's metres. */
export const plotSize = (size: LotSizeName) => ({ w: T.lots[size].w, d: T.lots[size].d });
/** The empty plot's price. */
export const landPrice = (size: LotSizeName) => T.lots[size].price;
/** What selling a plot (not a gift) gives back for the land. */
export const landBack = (size: LotSizeName) => Math.floor(T.lots[size].price * T.sell.lot);
/** The smallest plot a `w` × `d` lot fits on (null: bigger than any). */
export const sizeFor = (w: number, d: number): LotSizeName | null => SIZES.find((s) => w <= T.lots[s].w + 1e-6 && d <= T.lots[s].d + 1e-6) ?? null;

/** What selling a part of this kind and price gives back (rounded down). */
export const refund = (kind: PartKind, price: number) => Math.floor(price * T.sell[kind]);

/** Every priced part of a lot. Gift marks don't change prices (only refunds). */
export function parts(lot: Lot): Part[] {
  const out: Part[] = [];
  const len = new Map<string, [number, string]>();
  for (const w of lot.walls) {
    const l = Math.abs(w.x1 - w.x0) + Math.abs(w.z1 - w.z0);
    len.set(w.id, [l, w.kind]);
    out.push({ key: `wall:${w.id}`, id: w.id, kind: 'build', price: coins(l * (T.wall_per_m[w.kind] ?? 0)) });
  }
  for (const d of lot.doors) out.push({ key: `door:${d.id}`, id: d.id, kind: 'build', price: T.door[d.kind] ?? 0 });
  for (const w of lot.windows) out.push({ key: `window:${w.id}`, id: w.id, kind: 'build', price: T.window[w.kind] ?? 0 });
  for (const p of [...lot.furniture, ...(lot.storage ?? [])]) out.push({ key: `piece:${p.uid}:${p.item}`, id: p.uid, kind: 'furniture', price: CATALOG.get(p.item) ?? 0 });
  // Paint and floors have no ids: the same one twice is two parts.
  const seen = new Map<string, number>();
  const nth = (key: string) => {
    const n = (seen.get(key) ?? 0) + 1;
    seen.set(key, n);
    return `${key}#${n}`;
  };
  for (const p of lot.paint) {
    const [l, kind] = len.get(p.wall) ?? [0, ''];
    out.push({ key: nth(`paint:${p.wall}:${p.side}:${p.paint}`), id: null, kind: 'paint', price: coins(l * (T.wall_h[kind] ?? 0) * T.paint_per_m2) });
  }
  if (lot.floors.default !== T.bare_floor) out.push({ key: `floor:all:${lot.floors.default}`, id: null, kind: 'paint', price: coins(lot.size.w * lot.size.d * T.floor_per_m2) });
  for (const r of lot.floors.rects) out.push({ key: nth(`floor:${r.w}x${r.d}:${r.m}`), id: null, kind: 'paint', price: coins(r.w * r.d * T.floor_per_m2) });
  return out;
}

/** What everything on a lot costs to build (the plot aside), gifts at their price too. */
export const value = (lot: Lot) => parts(lot).reduce((n, p) => n + p.price, 0);

/** What selling everything on a lot gives back, at each kind's rate; gifts give nothing. */
export function sellValue(lot: Lot): number {
  const gift = new Set(lot.gift);
  return parts(lot).reduce((n, p) => n + (p.id != null && gift.has(p.id) ? 0 : refund(p.kind, p.price)), 0);
}

/** What changing a lot from `old` to `next` costs (negative: it gives back), as the core
 *  charges it: new things cost their price, gone ones sell back at their kind's rate, kept ones
 *  (moved, turned, put away) cost the difference if they grew and give some back if they
 *  shrank. Gifts (`old.gift`) give back nothing. */
export function diff(old: Lot, next: Lot): number {
  const gift = new Set(old.gift);
  const gifted = (p: Part) => p.id != null && gift.has(p.id);
  const before = new Map(parts(old).map((p) => [p.key, p]));
  const after = new Map(parts(next).map((p) => [p.key, p]));
  let cost = 0;
  for (const [k, n] of after) {
    const o = before.get(k);
    if (!o) cost += n.price;
    else if (n.price > o.price) cost += n.price - o.price;
    else if (n.price < o.price && !gifted(o)) cost -= refund(o.kind, o.price - n.price);
  }
  for (const [k, o] of before) if (!after.has(k) && !gifted(o)) cost -= refund(o.kind, o.price);
  return cost;
}

/** A design put down on a `w` × `d` plot, as the core does: centred across, against the south
 *  edge; its own floor becomes a rect on the bare plot. Null if it's too big. */
export function fit(lot: Lot, w: number, d: number): Lot | null {
  const { w: lw, d: ld } = lot.size;
  if (lw > w + 1e-6 || ld > d + 1e-6) return null;
  if (Math.abs(lw - w) < 1e-6 && Math.abs(ld - d) < 1e-6) return lot;
  const dx = Math.floor((w - lw) / 2 / TILE) * TILE;
  const dz = d - ld;
  const at = <P extends { x: number; z: number }>(p: P): P => ({ ...p, x: p.x + dx, z: p.z + dz });
  const rects = lot.floors.rects.map(at);
  if (lot.floors.default !== T.bare_floor) rects.unshift({ x: dx, z: dz, w: lw, d: ld, m: lot.floors.default });
  return {
    ...lot,
    size: { w, d },
    walls: lot.walls.map((x) => ({ ...x, x0: x.x0 + dx, x1: x.x1 + dx, z0: x.z0 + dz, z1: x.z1 + dz })),
    furniture: lot.furniture.map(at),
    ...(lot.storage ? { storage: lot.storage.map(at) } : {}),
    labels: lot.labels.map(at),
    floors: { ...lot.floors, default: T.bare_floor, rects },
  };
}

/** A bare plot: nothing built, the bare floor. */
export function bareLot(size: LotSizeName): Lot {
  const { w, d } = plotSize(size);
  return { v: 1, template: '', size: { w, d }, walls: [], doors: [], windows: [], floors: { tile: TILE, default: T.bare_floor, rects: [] }, paint: [], labels: [], furniture: [], gift: [] };
}

/** Nothing stands on it (no walls, doors, windows or furniture, nothing stored). */
export const isBare = (lot: Lot) => !lot.walls.length && !lot.doors.length && !lot.windows.length && !lot.furniture.length && !lot.storage?.length;

/** Every id and uid in a lot (what can be a gift). */
export const lotIds = (lot: Lot) => new Set([...lot.walls.map((w) => w.id), ...lot.doors.map((d) => d.id), ...lot.windows.map((w) => w.id), ...[...lot.furniture, ...(lot.storage ?? [])].map((p) => p.uid)]);

/** The gift marks a saved lot keeps, as the core does: what came free and is still there (the
 *  same piece of furniture). */
export function keptGifts(old: Lot, next: Lot): string[] {
  const item = (lot: Lot, uid: string) => [...lot.furniture, ...(lot.storage ?? [])].find((p) => p.uid === uid)?.item;
  const now = lotIds(next);
  return old.gift.filter((id) => now.has(id) && item(old, id) === item(next, id));
}

/** A plot with a ready-made design on it (null: empty): the plot plus everything on it, at the
 *  prices of building it yourself. Null if the design doesn't exist or doesn't fit. */
export function readyMadePrice(size: LotSizeName, template: string | null): number | null {
  if (template == null) return landPrice(size);
  const t = TEMPLATES[template];
  const { w, d } = plotSize(size);
  const lot = t && fit(t, w, d);
  return lot ? landPrice(size) + value(lot) : null;
}
