// The office's own furniture drawn with the Build/Buy models (CC0, public/furniture) instead of
// code-built boxes: each piece is fitted to its spot on the floor plan, so seats, use-slots and
// the walking grid stay as layout.ts has them. Pieces that show live data (desk monitors, server
// racks, the boards, the mail room, foosball, pool, the TV picture) stay code-built.

import { breakIntent, stationBreak, type Intent } from '../behavior';
import type { Item, ItemType, OfficeLayout, StationId } from '../layout';
import { DESK_H } from '../layout';
import { BY_ID } from '../buy';
import { model, type Shown } from './decor';

interface Spec {
  id: string;
  /** 'uniform' keeps proportions (fits the footprint, or `h`); 'stretch' fills the footprint. */
  fit?: 'uniform' | 'stretch';
  /** Target height (stretch: the top; uniform: scales the whole piece to it). */
  h?: number;
  /** Raised (a coffee machine on the counter). */
  y?: number;
  /** Fill the length with copies; `mid` replaces the middle one (the sink in the counter). */
  tile?: boolean;
  mid?: string;
  /** Fabric color (Kenney models are salmon by default). */
  tint?: string;
}

const BY_TYPE: Partial<Record<ItemType, (it: Item) => Spec>> = {
  desk: () => ({ id: 'kenney.desk', fit: 'stretch', h: DESK_H }),
  leadDesk: () => ({ id: 'kenney.desk', fit: 'stretch', h: DESK_H + 0.02 }),
  chair: (it) => ({ id: it.variant === 2 ? 'kenney.chairCushion' : 'kenney.chairDesk', tint: ['#3f8077', '#3a3c43', '#8c8a4e'][it.variant ?? 0] }),
  shelf: () => ({ id: 'kaykit.shelf_B_large_decorated', tile: true, fit: 'stretch', h: 1.9 }),
  armchair: (it) => ({ id: it.variant ? 'kaykit.armchair_pillows' : 'kenney.loungeChair', tint: '#c8682f' }),
  sideTable: () => ({ id: 'kenney.sideTable' }),
  floorLamp: () => ({ id: 'kenney.lampRoundFloor', h: 1.45 }),
  readingTable: () => ({ id: 'kenney.table', fit: 'stretch', h: 0.5 }),
  meetingTable: () => ({ id: 'kaykit.table_medium_long', fit: 'stretch', h: 0.5 }),
  counter: () => ({ id: 'kenney.kitchenCabinetDrawer', tile: true, mid: 'kenney.kitchenSink', fit: 'stretch', h: 0.62 }),
  fridge: () => ({ id: 'kenney.kitchenFridgeLarge', h: 1.56 }),
  coffee: () => ({ id: 'kenney.kitchenCoffeeMachine', y: 0.62 }),
  roundTable: () => ({ id: 'kenney.tableRound', fit: 'stretch', h: 0.5 }),
  stool: () => ({ id: 'kaykit.chair_stool' }),
  sofa: () => ({ id: 'kenney.loungeSofa', fit: 'stretch', h: 0.75, tint: '#d6a23c' }),
  couch: () => ({ id: 'kenney.loungeDesignSofa', fit: 'stretch', h: 0.75, tint: '#7b4a2d' }),
  plant: (it) => ({ id: it.variant === 1 ? 'kaykit.cactus_medium_A' : 'kenney.pottedPlant', h: 0.9 }),
  tallPlant: () => ({ id: 'kenney.pottedPlant', h: 1.55 }),
  coatRack: () => ({ id: 'kenney.coatRackStanding', h: 1.6 }),
  doormat: () => ({ id: 'kenney.rugDoormat', fit: 'stretch' }),
  bench: () => ({ id: 'kenney.benchCushion', fit: 'stretch', h: 0.5, tint: '#3f8077' }),
  credenza: () => ({ id: 'kaykit.cabinet_medium_decorated', tile: true, fit: 'stretch', h: 0.62 }),
  tvStand: () => ({ id: 'kenney.cabinetTelevision', fit: 'stretch', h: 0.43 }),
};

/** Built-in pieces anyone may use on a break like bought ones (the rest belong to a station). */
export const freeToUse = (it: Item) => ['couch', 'bench', 'stool'].includes(it.type) || (it.type === 'armchair' && it.room === 'lobby');

/** What the click menu offers on a built-in piece that is part of a station (the PS5 sofa…). */
export function stationActions(L: OfficeLayout, it: Item): { label: string; intent: Intent }[] {
  const nearest = (id: StationId) => {
    const slots = L.stations[id].slots;
    let k = 0;
    slots.forEach((s, i) => Math.hypot(s.x - it.x, s.z - it.z) < Math.hypot(slots[k].x - it.x, slots[k].z - it.z) && (k = i));
    return k;
  };
  switch (it.type) {
    case 'sofa':
    case 'tvStand':
      return [{ label: 'Play on the PS5', intent: breakIntent({ game: 'console', slot: 0 }) }];
    case 'armchair':
      if (it.room === 'library') return [{ label: 'Read', intent: breakIntent({ game: 'library', slot: nearest('library.chair') }) }];
      if (it.room === 'archive') return [{ label: 'Nap', intent: breakIntent({ game: 'nap', slot: 0 }) }];
      return [];
    case 'shelf':
      return [{ label: 'Browse the shelves', intent: stationBreak('library.shelf', 'browse', 'book') }];
    case 'counter':
    case 'coffee':
    case 'fridge':
      return [{ label: 'Get a coffee', intent: breakIntent({ game: 'coffee', slot: 0 }) }];
    case 'foosball':
      return [{ label: 'Play foosball', intent: breakIntent({ game: 'foosball', slot: 0 }) }];
    case 'poolTable':
      return [{ label: 'Play pool', intent: breakIntent({ game: 'pool', slot: 0 }) }];
    default:
      return [];
  }
}

/** Click-menu names where the model's catalog name doesn't fit (and the code-built pieces). */
export const BUILT_LABEL: Partial<Record<ItemType, string>> = { foosball: 'Foosball table', poolTable: 'Pool table', counter: 'Kitchen counter', tvStand: 'TV', shelf: 'Bookshelf', coffee: 'Coffee machine' };

/** Whether the building leaves this piece to a model (the TV stand keeps its code-built TV). */
export const modeled = (t: ItemType) => t in BY_TYPE;

/** The built-in furniture as model pieces (uid `item:<index>`, `item:<index>:<copy>` for tiles). */
export async function builtinPieces(L: OfficeLayout): Promise<Shown[]> {
  const out: Shown[] = [];
  await Promise.all(
    L.items.map(async (it, i) => {
      const spec = BY_TYPE[it.type]?.(it);
      const f = spec && BY_ID.get(spec.id);
      if (!spec || !f) return;
      const { dims } = await model(f);
      const at = (dx: number, scale: [number, number, number], id = spec.id, uid = `item:${i}`) =>
        out.push({ uid, item: id, x: it.x + Math.cos(it.rot) * dx, y: spec.y ?? 0, z: it.z - Math.sin(it.rot) * dx, rot: it.rot, scale, tint: spec.tint });
      if (spec.tile) {
        const s = it.d / dims.d;
        const n = Math.max(1, Math.round(it.w / (dims.w * s)));
        const sy = spec.h ? spec.h / dims.h : s;
        for (let k = 0; k < n; k++) {
          const id = spec.mid && k === Math.floor(n / 2) ? spec.mid : spec.id;
          const d = id === spec.id ? dims : (await model(BY_ID.get(id)!)).dims;
          at(-it.w / 2 + ((k + 0.5) * it.w) / n, [it.w / n / d.w, spec.h ? spec.h / d.h : sy, it.d / d.d], id, `item:${i}:${k}`);
        }
      } else if (spec.fit === 'stretch') {
        const sx = it.w / dims.w;
        const sz = it.d / dims.d;
        at(0, [sx, spec.h ? spec.h / dims.h : (sx + sz) / 2, sz]);
      } else {
        const s = spec.h ? spec.h / dims.h : Math.min(it.w / dims.w, it.d / dims.d);
        at(0, [s, s, s]);
      }
    }),
  );
  return out;
}
