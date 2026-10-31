// What a lot and a ready-made building cost on the map, from the one price table
// (src/office/lots/prices.json) and the furniture catalog. A ready-made lot costs exactly the
// empty lot plus everything on it (PROGRESSION §4); the core is the authority, this is what the
// buy panel shows before you ask it.
//
// TEMPORARY: src/office/price.ts (the core's mirror, same names) is being written in parallel;
// once it lands, `readyMadePrice` and `TEMPLATE_NAMES` come from there and this file keeps only
// the labels.

import type { Lot } from '../lib/gen/Lot';
import type { LotSlot } from '../lib/gen/LotSlot';
import { BY_ID } from '../office/buy';
import { TEMPLATES } from '../office/layout';
import PRICES from '../office/lots/prices.json';

export type LotSizeName = LotSlot['size'];

/** Each lot size's plot (metres) and empty price. */
export const LOT_SIZES: Record<LotSizeName, { w: number; d: number; price: number }> = PRICES.lots;

/** The ready-made designs this build ships, smallest first. */
export const TEMPLATE_NAMES: string[] = Object.keys(TEMPLATES).sort((a, b) => area(a) - area(b));

/** How a ready-made design is called on the map. */
export const TEMPLATE_LABEL: Record<string, string> = { garage: 'Garage', 'small-office': 'Small office', studio: 'Studio' };
export const templateLabel = (t: string): string => TEMPLATE_LABEL[t] ?? t.replace(/-/g, ' ').replace(/^./, (c) => c.toUpperCase());

/** A saved design by name, when this build ships it. */
export function templateLot(name: string): Lot | null {
  return (TEMPLATES as Record<string, Lot>)[name] ?? null;
}

function area(t: string): number {
  const l = templateLot(t);
  return l ? l.size.w * l.size.d : Infinity;
}

/** A design fits any plot at least its size. */
export function fits(template: string, size: LotSizeName): boolean {
  const l = templateLot(template);
  const p = LOT_SIZES[size];
  return !!l && l.size.w <= p.w && l.size.d <= p.d;
}

const len = (w: { x0: number; z0: number; x1: number; z1: number }) => Math.abs(w.x1 - w.x0) + Math.abs(w.z1 - w.z0);

/** Everything built on a lot at build-mode prices: walls per metre, doors, windows, floor and furniture. */
export function lotValue(lot: Lot): number {
  const P = PRICES;
  let n = 0;
  for (const w of lot.walls) n += len(w) * (P.wall_per_m[w.kind] ?? P.wall_per_m.solid);
  for (const d of lot.doors) n += P.door[d.kind] ?? P.door.single;
  for (const w of lot.windows) n += P.window[w.kind] ?? P.window.standard;
  n += lot.size.w * lot.size.d * P.floor_per_m2;
  for (const p of lot.furniture) n += BY_ID.get(p.item)?.price ?? 0;
  return Math.round(n);
}

/** The land alone. */
export const landPrice = (size: LotSizeName): number => LOT_SIZES[size].price;

/** A plot's price empty (template null) or with a ready-made design on it. */
export function readyMadePrice(size: LotSizeName, template: string | null): number {
  const lot = template ? templateLot(template) : null;
  return landPrice(size) + (lot ? lotValue(lot) : 0);
}
