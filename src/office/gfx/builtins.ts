// The office's own furniture drawn with the Build/Buy models (CC0, public/furniture) instead of
// code-built boxes: each piece is fitted to its spot on the floor plan, so seats, use-slots and
// the walking grid stay as layout.ts has them. Pieces that show live data (desk monitors, server
// racks, the boards, the mail room, foosball, pool, the TV picture) stay code-built.

import { Box3, Group, Mesh, MeshStandardMaterial, Vector3 } from 'three';
import { breakIntent, stationBreak, type Intent } from '../behavior';
import type { Item, ItemType, OfficeLayout, StationId } from '../layout';
import { DESK_H } from '../layout';
import { BY_ID, type Dims, type Furniture } from '../buy';
import { Builder } from './builder';
import { model, type Shown } from './decor';
import * as F from './furniture';

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
  desk: () => ({ id: 'polypizza.desk', fit: 'stretch', h: DESK_H }),
  leadDesk: () => ({ id: 'polypizza.desk', fit: 'stretch', h: DESK_H + 0.02 }),
  chair: (it) => ({ id: it.variant === 2 ? 'kenney.chairCushion' : 'kenney.chairDesk', tint: ['#3f8077', '#3a3c43', '#8c8a4e'][it.variant ?? 0] }),
  shelf: () => ({ id: 'polypizza.bookcase', tile: true, fit: 'stretch', h: 2.0 }),
  armchair: (it) => ({ id: it.variant ? 'kaykit.armchair_pillows' : 'kaykit.armchair' }),
  sideTable: () => ({ id: 'kenney.sideTable' }),
  floorLamp: () => ({ id: 'kenney.lampRoundFloor', h: 1.45 }),
  readingTable: () => ({ id: 'kenney.table', fit: 'stretch', h: 0.5 }),
  meetingTable: () => ({ id: 'kaykit.table_medium_long', fit: 'stretch', h: 0.5 }),
  counter: () => ({ id: 'kenney.kitchenCabinetDrawer', tile: true, mid: 'kenney.kitchenSink', fit: 'stretch', h: 0.62 }),
  fridge: () => ({ id: 'polypizza.fridge', h: 1.56 }),
  coffee: () => ({ id: 'polypizza.espresso', y: 0.62, h: 0.4 }),
  cooler: () => ({ id: 'polypizza.waterCooler', h: 1.35 }),
  roundTable: () => ({ id: 'polypizza.bistroTable', fit: 'stretch', h: 0.72 }),
  stool: () => ({ id: 'polypizza.cafeChair' }),
  sofa: () => ({ id: 'polypizza.sectionalSofa', fit: 'stretch', h: 0.75 }),
  couch: () => ({ id: 'polypizza.sectionalSofa', fit: 'stretch', h: 0.75 }),
  plant: (it) => ({ id: it.variant === 1 ? 'kaykit.cactus_medium_A' : 'kenney.pottedPlant', h: 0.9 }),
  tallPlant: () => ({ id: 'kenney.pottedPlant', h: 1.55 }),
  coatRack: () => ({ id: 'kenney.coatRackStanding', h: 1.6 }),
  doormat: () => ({ id: 'kenney.rugDoormat', fit: 'stretch' }),
  bench: () => ({ id: 'kenney.benchCushion', fit: 'stretch', h: 0.5, tint: '#3f8077' }),
  credenza: () => ({ id: 'kaykit.cabinet_medium_decorated', tile: true, fit: 'stretch', h: 0.62 }),
  tvStand: () => ({ id: 'kenney.cabinetTelevision', fit: 'stretch', h: 0.43 }),
  mailbox: () => ({ id: 'polypizza.postbox', h: 1.15 }),
  // The felt at 0.705 like the ball simulation (gfx/games.ts) expects.
  poolTable: () => ({ id: 'polypizza.poolTable', fit: 'stretch', h: 0.755 }),
  pingpong: () => ({ id: 'polypizza.pingPong', fit: 'stretch', h: 0.9 }),
};

/** Built-in pieces anyone may use on a break like bought ones (the rest belong to a station). */
export const freeToUse = (it: Item) => ['couch', 'bench', 'stool', 'cooler'].includes(it.type) || (it.type === 'armchair' && it.room === 'lobby');

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
    // The kitchen's first spot is at the espresso machine, the second at the fridge.
    case 'counter':
    case 'coffee':
      return [{ label: 'Make a coffee', intent: breakIntent({ game: 'coffee', slot: 0 }) }];
    case 'fridge':
      return [{ label: 'Grab a drink', intent: breakIntent({ game: 'coffee', slot: 1 }) }];
    case 'foosball':
      return [{ label: 'Play foosball', intent: breakIntent({ game: 'foosball', slot: 0 }) }];
    case 'poolTable':
      return [{ label: 'Play pool', intent: breakIntent({ game: 'pool', slot: 0 }) }];
    case 'pingpong':
      return [{ label: 'Play ping-pong', intent: breakIntent({ game: 'pingpong', slot: 0 }) }];
    case 'mailbox':
      return [{ label: 'Post a letter', intent: stationBreak('mail', 'stand', 'envelope') }];
    default:
      return [];
  }
}

/** Click-menu names where the model's catalog name doesn't fit (and the code-built pieces). */
export const BUILT_LABEL: Partial<Record<ItemType, string>> = { foosball: 'Foosball table', poolTable: 'Pool table', pingpong: 'Ping-pong table', counter: 'Kitchen counter', tvStand: 'TV', shelf: 'Bookshelf', coffee: 'Coffee machine', mailbox: 'Post box' };

/** Whether the building leaves this piece to a model (the TV stand keeps its code-built TV). */
export const modeled = (t: ItemType) => t in BY_TYPE;

/** Pieces that show live data, drawn in code even in Build/Buy's catalog. */
const CODE_BUILT: Partial<Record<ItemType, (b: Builder) => void>> = { foosball: F.foosball, tvStand: (b) => F.tvStand(b, b) };
const codeMat = new MeshStandardMaterial({ vertexColors: true, roughness: 0.6, metalness: 0.05 });

/**
 * One of the office's own pieces as a single model, as Build/Buy shows it (its catalog
 * thumbnail and the ghost being carried): its CC0 model fitted to its footprint, or the
 * code-built piece. Its size is its footprint, as `layout.ts` blocks it.
 */
export async function builtinModel(f: Furniture): Promise<{ root: Group; dims: Dims }> {
  const type = f.builtin!;
  const [w, d] = [f.w ?? 1, f.d ?? 1];
  const root = new Group();
  const code = CODE_BUILT[type];
  const spec = BY_TYPE[type]?.({ type, x: 0, z: 0, rot: 0, w, d, block: true, variant: f.variant, room: '' });
  const look = spec && BY_ID.get(spec.id);
  if (code || !look) {
    const b = new Builder();
    (code ?? ((b: Builder) => b.box(0, 0.4, 0, w, 0.8, d, [0.5, 0.45, 0.4])))(b);
    root.add(new Mesh(b.build(), codeMat));
  } else {
    const m = await model(look);
    const inner = m.root.clone(true);
    const sx = w / m.dims.w;
    const sz = d / m.dims.d;
    if (spec.fit === 'stretch') inner.scale.set(sx, spec.h ? spec.h / m.dims.h : (sx + sz) / 2, sz);
    else inner.scale.setScalar(spec.h ? spec.h / m.dims.h : Math.min(sx, sz));
    inner.position.y = spec.y ?? 0;
    root.add(inner);
  }
  root.traverse((o) => o instanceof Mesh && ((o.castShadow = true), (o.receiveShadow = true)));
  return { root, dims: { w, d, h: new Box3().setFromObject(root).getSize(new Vector3()).y } };
}

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
  // A keyboard and a mouse on every desk, in front of its occupant (uid `desk:<index>:…`).
  for (const d of L.desks) {
    const [fx, fz] = [Math.sin(d.rot), Math.cos(d.rot)];
    const [rx, rz] = [Math.cos(d.rot), -Math.sin(d.rot)];
    const y = d.lead ? DESK_H + 0.02 : DESK_H;
    const kx = d.x - fx * 0.1;
    const kz = d.z - fz * 0.1;
    out.push({ uid: `desk:${d.index}:keyboard`, item: 'kenney.computerKeyboard', x: kx, y, z: kz, rot: d.rot + Math.PI });
    out.push({ uid: `desk:${d.index}:mouse`, item: 'kenney.computerMouse', x: kx - rx * 0.32, y, z: kz - rz * 0.32, rot: d.rot + Math.PI });
  }
  return out;
}

/** The desk a piece belongs to (its desk, chair, keyboard or mouse), or null. */
export function deskOfPiece(L: OfficeLayout, uid: string): number | null {
  const [kind, n] = uid.split(':');
  if (kind === 'desk') return +n;
  return kind === 'item' ? (L.items[+n]?.desk ?? null) : null;
}
