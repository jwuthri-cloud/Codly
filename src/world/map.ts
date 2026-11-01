// The neighbourhood, as data: where each plot sits on the ground, what its sign says, what a
// click on it offers, and what buying it costs. Pure (no three.js), so it is testable in Node.
//
// Plots sit on a grid of equal cells, each big enough for the largest lot; a plot is centred in
// its cell and the whole grid is centred on the origin. A building smaller than its plot sits
// centred against the plot's south edge (the street side), as the core places it.

import type { Lot } from '../lib/gen/Lot';
import type { LotSlot } from '../lib/gen/LotSlot';
import { coins } from '../lib/util/format';
import { plotSize, readyMadePrice, TEMPLATE_NAMES } from '../office/price';
import { templateLabel } from './price';

/** Metres between plot centres, along x and along z: the large lot (28 × 24) plus a street with
 *  its sidewalks (town.ts) and a little verge on each side. */
export const PITCH_X = 44;
export const PITCH_Z = 40;

/** Yours with a repo's office · yours with a building and no repo · yours and empty · for sale. */
export type SlotKind = 'office' | 'building' | 'empty' | 'sale';

export function slotKind(s: LotSlot): SlotKind {
  if (!s.owned) return 'sale';
  if (s.workspace_id) return 'office';
  return s.template ? 'building' : 'empty';
}

export interface Plot {
  id: string;
  /** Centre on the ground. */
  cx: number;
  cz: number;
  w: number;
  d: number;
  x0: number;
  z0: number;
  x1: number;
  z1: number;
}

/** The grid's centre (in cells), so the map is centred on the origin. */
export function gridCentre(slots: Pick<LotSlot, 'x' | 'z'>[]): { x: number; z: number } {
  if (!slots.length) return { x: 0, z: 0 };
  let x0 = Infinity;
  let x1 = -Infinity;
  let z0 = Infinity;
  let z1 = -Infinity;
  for (const s of slots) {
    x0 = Math.min(x0, s.x);
    x1 = Math.max(x1, s.x);
    z0 = Math.min(z0, s.z);
    z1 = Math.max(z1, s.z);
  }
  return { x: (x0 + x1) / 2, z: (z0 + z1) / 2 };
}

/** Where a slot's plot lies on the ground. */
export function plotOf(s: Pick<LotSlot, 'id' | 'x' | 'z' | 'size'>, centre: { x: number; z: number }): Plot {
  const { w, d } = plotSize(s.size);
  const cx = (s.x - centre.x) * PITCH_X;
  const cz = (s.z - centre.z) * PITCH_Z;
  return { id: s.id, cx, cz, w, d, x0: cx - w / 2, z0: cz - d / 2, x1: cx + w / 2, z1: cz + d / 2 };
}

export function plotsOf(slots: LotSlot[]): Plot[] {
  const c = gridCentre(slots);
  return slots.map((s) => plotOf(s, c));
}

/** The plot under a ground point (a little margin around each, so the outline counts). */
export function plotAt(plots: Plot[], x: number, z: number, margin = 0.6): string | null {
  for (const p of plots) if (x >= p.x0 - margin && x <= p.x1 + margin && z >= p.z0 - margin && z <= p.z1 + margin) return p.id;
  return null;
}

/** Where a lot's own (0, 0) lands in the world: centred on x, against the plot's south edge. */
export function buildingOrigin(p: Plot, lot: Pick<Lot, 'size'>): { x: number; z: number } {
  return { x: p.cx - lot.size.w / 2, z: p.z1 - lot.size.d };
}

/** The ground area the grid covers (for the camera's bounds). */
export function extent(plots: Plot[]): { x0: number; z0: number; x1: number; z1: number } {
  if (!plots.length) return { x0: -20, z0: -20, x1: 20, z1: 20 };
  return {
    x0: Math.min(...plots.map((p) => p.x0)),
    z0: Math.min(...plots.map((p) => p.z0)),
    x1: Math.max(...plots.map((p) => p.x1)),
    z1: Math.max(...plots.map((p) => p.z1)),
  };
}

// ───────────── signs ─────────────

export const agentsLabel = (n: number): string => `${n} agent${n === 1 ? '' : 's'}`;

export const SIZE_LABEL: Record<LotSlot['size'], string> = { small: 'Small', medium: 'Medium', large: 'Large' };
export const sizeLabel = (s: LotSlot['size']): string => `${SIZE_LABEL[s]} · ${plotSize(s).w} × ${plotSize(s).d} m`;

export interface Sign {
  title: string;
  /** A second, quieter line. */
  sub: string | null;
  /** Open (the live repo, or another one at work), closed (another repo, idle), or not an office. */
  state: 'open' | 'closed' | null;
}

/** What a plot's sign says. `live`: this is the open repo; `working`: how many of its agents work now
 *  (another office keeps working while it has tasks in flight). */
/** `open`: the office has its tab (it works whenever it has work); `paused`: it starts nothing new. */
export function signOf(s: LotSlot, open = false, working = 0, paused = false): Sign {
  switch (slotKind(s)) {
    case 'office':
      return {
        title: `${s.name ?? 'Office'} · ${agentsLabel(s.crew)}`,
        sub: paused ? (working ? `Paused · ${working} finishing` : 'Paused') : open ? (working ? `Open · ${working} working` : 'Open') : working ? `Working · ${working}` : 'Closed',
        state: (open && !paused) || working ? 'open' : 'closed',
      };
    case 'building':
      return { title: `${templateLabel(s.template!)} · No office yet`, sub: 'Move an office here', state: null };
    case 'empty':
      return { title: 'Empty lot · Build here', sub: SIZE_LABEL[s.size], state: null };
    case 'sale':
      return { title: `For sale · ${coins(s.price)}`, sub: sizeLabel(s.size), state: null };
  }
}

/** Lights in the windows: every open office is lit, and any office whose agents work, brightest then. */
export function lightsOf(s: LotSlot, open: boolean, working: number): 'off' | 'on' | 'busy' {
  if (slotKind(s) !== 'office' || (!open && !working)) return 'off';
  return working > 0 ? 'busy' : 'on';
}

// ───────────── what a click offers ─────────────

export type ActionId = 'enter' | 'move-here' | 'move-away' | 'sell';

export interface SlotAction {
  id: ActionId;
  label: string;
  /** Why it can't be done now (greyed out), or null. */
  disabled: string | null;
  /** A price or a refund to show beside it. */
  note?: string;
}

export interface ActionContext {
  /** Repos of yours that could move onto this lot (not already on it). */
  movable: number;
  /** Other lots of yours with no repo on them (where this lot's office could go). */
  freeLots: number;
}

/** What an owned plot offers (a for-sale plot offers `buyOptions` instead). */
export function slotActions(s: LotSlot, ctx: ActionContext): SlotAction[] {
  const kind = slotKind(s);
  if (kind === 'sale') return [];
  const sell: SlotAction = {
    id: 'sell',
    label: 'Sell',
    disabled: kind === 'office' ? 'A repo lives here: move its office first' : null,
    note: s.gift ? 'a gift: sells for 🪙 0' : `refund ${coins(s.sells_for)}`,
  };
  if (kind === 'office') {
    return [
      { id: 'enter', label: 'Enter', disabled: null },
      { id: 'move-away', label: 'Move office', disabled: ctx.freeLots ? null : 'No free lot of yours to move to', note: ctx.freeLots ? `${ctx.freeLots} free lot${ctx.freeLots === 1 ? '' : 's'}` : undefined },
      sell,
    ];
  }
  return [{ id: 'move-here', label: 'Move an office here', disabled: ctx.movable ? null : 'No other office to move', note: undefined }, sell];
}

export interface BuyOption {
  template: string | null;
  label: string;
  price: number;
  /** Greyed out: the wallet can't pay. */
  short: boolean;
}

/** A for-sale plot's offers: empty, or with each ready-made design that fits it (smallest first). */
export function buyOptions(s: Pick<LotSlot, 'size'>, balance: number, templates: string[] = TEMPLATE_NAMES): BuyOption[] {
  const opts: BuyOption[] = [];
  for (const template of [null, ...templates]) {
    const price = readyMadePrice(s.size, template);
    if (price != null) opts.push({ template, label: template ? templateLabel(template) : 'Empty lot', price, short: price > balance });
  }
  return [opts[0], ...opts.slice(1).sort((a, b) => a.price - b.price)];
}
