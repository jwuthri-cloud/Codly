// Build mode's live price: what a change to the lot costs, as the core will charge it
// (`price.ts`, the shared table: something new costs its price, something gone sells back at its
// kind's rate, something kept costs the difference if it grew; gifts give back nothing), split
// by kind of thing for the preview, and what an edit buys or sells through Buy.

import type { Lot } from '../../lib/gen/Lot';
import type { Placed } from '../../lib/gen/Placed';
import { BY_ID } from '../buy';
import { diff, parts, refund, type Part } from '../price';

export interface Cost {
  /** Coins it takes (negative: what it gives back). */
  net: number;
  /** Per kind of thing: what it costs (negative: gives back). */
  lines: { what: string; coins: number }[];
}

const WHAT: Record<string, string> = { wall: 'walls', door: 'doors', window: 'windows', piece: 'furniture', paint: 'paint', floor: 'floors' };
const whatOf = (p: Part) => WHAT[p.key.slice(0, p.key.indexOf(':'))] ?? 'other';

/** What changing `before` into `after` costs; gifts are `before`'s. */
export function costOf(before: Lot, after: Lot): Cost {
  const gift = new Set(before.gift);
  const was = new Map(parts(before).map((p) => [p.key, p]));
  const now = new Map(parts(after).map((p) => [p.key, p]));
  const by = new Map<string, number>();
  const add = (p: Part, v: number) => v && by.set(whatOf(p), (by.get(whatOf(p)) ?? 0) + v);
  for (const [k, n] of now) {
    const o = was.get(k);
    if (!o) add(n, n.price);
    else if (n.price > o.price) add(n, n.price - o.price);
    else if (n.price < o.price && !(o.id && gift.has(o.id))) add(o, -refund(o.kind, o.price - n.price));
  }
  for (const [k, o] of was) if (!now.has(k) && !(o.id && gift.has(o.id))) add(o, -refund(o.kind, o.price));
  return { net: diff(before, after), lines: [...by].map(([what, coins]) => ({ what, coins })).filter((l) => l.coins !== 0) };
}

const piecePrice = (item: string) => BY_ID.get(item)?.price ?? 0;

/** Pieces bought through Buy that an edit buys (a saved room's) or sells back (one that no longer
 *  fits): the shop's prices, at its rate (the full price back). Moving one is free. */
export function boughtCost(before: Placed[], after: Placed[]): number {
  const had = new Set(before.map((p) => p.uid));
  const has = new Set(after.map((p) => p.uid));
  const buy = after.filter((p) => !had.has(p.uid)).reduce((s, p) => s + piecePrice(p.item), 0);
  const sell = before.filter((p) => !has.has(p.uid)).reduce((s, p) => s + refund('furniture', piecePrice(p.item)), 0);
  return buy - sell;
}

/** "🪙 12" to pay, "+🪙 5 back", or "free". */
export function costLabel(net: number): string {
  if (net > 0) return `🪙 ${net.toLocaleString('en-US')}`;
  if (net < 0) return `+🪙 ${(-net).toLocaleString('en-US')} back`;
  return 'free';
}
