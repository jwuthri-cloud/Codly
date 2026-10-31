// The floor plan, made from a lot (the core's `Lot`: walls with doors and windows, floor tiles,
// furniture): rooms (found by flood fill of the areas walls close), furniture placements, the
// stations characters walk to (with use-slots, from the furniture's job), the desks, and the
// walkable grid for A*. Pure data: no three.js, so it is testable in Node and the renderer
// just reads it.
//
// Coordinates: x grows east (right), z grows south (towards the default camera); one unit is
// about 60 cm at chibi scale (a character is ~1.15 units tall). The Studio is 28 × 24:
//
//      x→ 0        7          15                          28
//   z 0  ┌─────────┬──────────┬─────────────────────────────┐
//        │ LIBRARY │ MEETING  │  LEADS' CORNER OFFICE        │  (glass front)
//        │ shelves │ whiteb.  │  one desk per active Lead    │
//     7  ├──door───┴────door──┴──glass────door───┬──────────┤
//        │B                                      │ SERVER   │
//        │O   desk pods (3 × 2 pods of 4)        │ racks,   │
//        │A                                      │ test mon.│
//        │R                                      glass      │
//   16.5 ├─────open──┬──door──┬────(open)────────┼──────────┤
//        │ LOUNGE    │ MAIL   │  LOBBY           │ ARCHIVE  │
//        │ kitchen   │ outbox │       ▼ entrance │ cabinets │
//        │ foosball  │        │                  │          │
//        │ TV ▸ sofa │        │ pool    (door)   │          │
//   24   └───────────┴────────┴───────door───────┴──────────┘
//
// The Garage (a new repo's first office) is one 10 × 8 room: the Lead's desk, two desks, a
// server machine, a bookshelf and a mailbox, and a roll-up door. The Small office (16 × 12) is a
// work floor (the Lead and six desks, an empty kitchen corner) and a back room (the rack and the
// test monitor, bookshelves, the mailbox and an archive cabinet).

import type { DoorKind } from '../lib/gen/DoorKind';
import type { Lot } from '../lib/gen/Lot';
import type { Placed } from '../lib/gen/Placed';
import type { WallKind } from '../lib/gen/WallKind';
import RAW from './furniture.json';
import GARAGE from './lots/garage.json';
import SMALL_OFFICE from './lots/small-office.json';
import STUDIO from './lots/studio.json';

export type { WallKind };

/** The ready-made lots (the core reads the same files). */
export const TEMPLATES = { studio: STUDIO as unknown as Lot, garage: GARAGE as unknown as Lot, 'small-office': SMALL_OFFICE as unknown as Lot };

export const CELL = 0.25;
/** Floor tiles and rooms: walls run on this grid. */
export const TILE = 0.5;

export const WALL_H = 2.3;
export const INNER_H = 0.95;
export const LOW_H = 0.32;
export const GLASS_H = 2.0;
export const DESK_H = 0.56;
export const SEAT_H = 0.33;

export interface Rect {
  x: number;
  z: number;
  w: number;
  d: number;
}

/** A room's kind, named after what's in it (several rooms of a kind: `studio-2`…). */
export type RoomId = string;
export type FloorKind = 'oak' | 'walnut' | 'walnut-planks' | 'carpet-sage' | 'carpet-blue' | 'tile-cool' | 'tile-warm' | 'terrazzo' | 'concrete';

export interface Room {
  id: RoomId;
  name: string;
  /** Bounding box (a room can be any shape: `roomOf` has its tiles). */
  rect: Rect;
  /** Where the room's floor label sits. */
  label: [number, number];
}

/** Connected tiles of one floor material, not crossing walls. */
export interface FloorRegion {
  m: FloorKind;
  rect: Rect;
  /** Tile indices (row-major, `tiles.cols` wide). */
  tiles: number[];
}

export interface Wall {
  id: string;
  /** Axis-aligned segment from (x0, z0) to (x1, z1), x0 ≤ x1 and z0 ≤ z1. */
  x0: number;
  z0: number;
  x1: number;
  z1: number;
  kind: WallKind;
  /** Openings along the segment (its doors), as [from, to] distances from (x0, z0). */
  gaps: [number, number][];
  /** Window openings (exterior walls), as [from, to] distances. */
  windows?: [number, number][];
}

export type ItemType =
  | 'desk'
  | 'chair'
  | 'monitor'
  | 'lamp'
  | 'divider'
  | 'leadDesk'
  | 'shelf'
  | 'armchair'
  | 'floorLamp'
  | 'rug'
  | 'readingTable'
  | 'meetingTable'
  | 'whiteboard'
  | 'taskBoard'
  | 'rack'
  | 'serverMachine'
  | 'testMonitor'
  | 'pigeonholes'
  | 'mailbox'
  | 'sortingTable'
  | 'cabinet'
  | 'catalog'
  | 'counter'
  | 'fridge'
  | 'coffee'
  | 'roundTable'
  | 'stool'
  | 'sofa'
  | 'plant'
  | 'tallPlant'
  | 'coatRack'
  | 'doormat'
  | 'bench'
  | 'cooler'
  | 'printer'
  | 'couch'
  | 'sideTable'
  | 'poster'
  | 'sconce'
  | 'clock'
  | 'credenza'
  | 'tvStand'
  | 'foosball'
  | 'pingpong'
  | 'poolTable';

/** What a piece of furniture does for the work (furniture.json `job`). */
export type Job = 'desk' | 'lead' | 'server' | 'tests' | 'books' | 'mail' | 'memory' | 'board' | 'whiteboard' | 'meeting';

export interface Item {
  type: ItemType;
  x: number;
  z: number;
  /** Height above the floor (wall lights, the clock). */
  y?: number;
  /** Yaw: the direction the item's front faces, (sin rot, cos rot) on (x, z). */
  rot: number;
  /** Footprint (unrotated w along x, d along z) that blocks walking; null = walkable. */
  w: number;
  d: number;
  block: boolean;
  variant?: number;
  room: RoomId;
  /** Desk index for desk-bound items (monitor, lamp, chair). */
  desk?: number;
  /** The lot piece it is (pieces a desk or table brings along have their desk's or table's). */
  uid?: string;
  job?: Job;
}

export interface Slot {
  x: number;
  z: number;
  /** Facing yaw (see Item.rot). */
  face: number;
  pose: 'stand' | 'sit';
  /** Seat height when sitting. */
  seat?: number;
}

export type StationId =
  | 'library.shelf'
  | 'library.chair'
  | 'nap.armchair'
  | 'meeting.board'
  | 'meeting.table'
  | 'board'
  | 'server.rack'
  | 'server.monitor'
  | 'mail'
  | 'archive'
  | 'lead.visit'
  | 'kitchen'
  | 'games.console'
  | 'games.foosball'
  | 'games.pingpong'
  | 'games.pool'
  | 'door'
  | 'wait';

export const STATION_IDS: StationId[] = [
  'library.shelf',
  'library.chair',
  'nap.armchair',
  'meeting.board',
  'meeting.table',
  'board',
  'server.rack',
  'server.monitor',
  'mail',
  'archive',
  'lead.visit',
  'kitchen',
  'games.console',
  'games.foosball',
  'games.pingpong',
  'games.pool',
  'door',
  'wait',
];

/** A station's slots; empty when the lot has nothing for it (then agents stay at their desk). */
export interface Station {
  id: StationId;
  room: RoomId;
  slots: Slot[];
}

export interface Desk {
  index: number;
  lead: boolean;
  /** Desk center and the yaw its occupant faces (towards the monitor). */
  x: number;
  z: number;
  rot: number;
  seat: Slot;
  /** Where the occupant stands up (approval, celebration) — next to the chair. */
  stand: Slot;
  /** Where a helper (subagent "intern") stands, beside the desk. */
  intern: Slot;
  /** World position of the monitor screen center, and where desk props go. */
  screen: [number, number, number];
  /** Spot on the desk top for a framed artifact / task card. */
  propSpot: [number, number];
  room: RoomId;
}

/** An evening light: position, colour, intensity, reach. */
export type Light = [number, number, number, string, number, number];

export interface OfficeLayout {
  /** The lot it was made from. */
  lot: Lot;
  /** Size (units) and the walking grid's size (cells of CELL). */
  w: number;
  d: number;
  cols: number;
  rows: number;
  rooms: Room[];
  /** Tile → index in `rooms`, −1 outside every room. */
  roomOf: Int16Array;
  tiles: { cols: number; rows: number };
  floors: FloorRegion[];
  walls: Wall[];
  items: Item[];
  /** Model pieces that came with the lot (bought ones come from the shop: `setDecor`). */
  pieces: Placed[];
  stations: Record<StationId, Station>;
  desks: Desk[];
  /** Spawn/despawn: just inside the front door, and the hall point in front of it. `rot` faces
   *  out of the lot; (nx, nz) points in. */
  door: { outside: Slot; inside: Slot; x: number; z: number; rot: number; nx: number; nz: number; width: number; kind: DoorKind };
  /** Walkable grid (1 = blocked), cols × rows cells of CELL units. */
  blocked: Uint8Array;
  /** The freestanding task board, facing the camera (south). */
  board: { z: number; x0: number; x1: number; y0: number; y1: number } | null;
  /** The meeting room whiteboard, hung on a north wall. */
  whiteboard: { z: number; x0: number; x1: number; y0: number; y1: number } | null;
  /** The test monitor (screen center; the server machine's own screen in a garage). */
  testMonitor: { x: number; z: number; y: number; w: number; h: number; rot: number } | null;
  /** Server racks (LED anchors): `front` is how far their doors stand out, `wide` two columns. */
  racks: { x: number; z: number; rot: number; front: number; wide: boolean }[];
  /** Mail room: outbox position (flag) and pigeonhole grid origin. */
  mailbox: { x: number; z: number; rot: number } | null;
  pigeonholes: { x: number; z: number; rot: number; cols: number; rows: number } | null;
  /** Archive cabinets (drawer lights). */
  cabinets: { x: number; z: number; rot: number }[];
  catalog: { x: number; z: number; rot: number } | null;
  /** Library shelves (book glow on retrieval). */
  shelves: { x: number; z: number; rot: number; len: number }[];
  /** The wall clock, if there is one. */
  clock: { x: number; y: number; z: number; rot: number } | null;
  /** Where ghosts float (layout x, z, facing) and the point over the memory archive. */
  ghosts: [number, number, number][];
  archive: [number, number];
  /** Evening lights, one or two per room. */
  lights: Light[];
}

const UP = Math.PI; // facing north (away from the camera, towards z = 0)
const DOWN = 0; // facing south (towards the camera)
const RIGHT = Math.PI / 2; // facing east (+x)
const LEFT = -Math.PI / 2; // facing west (−x)

export const FACE = { north: UP, south: DOWN, east: RIGHT, west: LEFT };

/** Footprint of an item (w × d before rotation) as an axis-aligned rect. */
export function footprint(it: Pick<Item, 'x' | 'z' | 'w' | 'd' | 'rot'>): Rect {
  const quarter = Math.abs(Math.round(it.rot / (Math.PI / 2))) % 2 === 1;
  const w = quarter ? it.d : it.w;
  const d = quarter ? it.w : it.d;
  return { x: it.x - w / 2, z: it.z - d / 2, w, d };
}

/** A catalog piece the office builds itself (furniture.json entries with `builtin`). */
interface Builtin {
  id: string;
  builtin: ItemType;
  w: number;
  d: number;
  variant?: number;
  block?: boolean;
  job?: Job;
}
const BUILTIN = new Map((RAW as { id: string; builtin?: string }[]).filter((f): f is Builtin => !!f.builtin).map((f) => [f.id, f]));
const KNOWN = new Set((RAW as { id: string }[]).map((f) => f.id));

let cached: { lot: Lot; L: OfficeLayout } | null = null;

/** The office made from a lot (the Studio by default); the last one is kept. */
export function officeLayout(lot: Lot = TEMPLATES.studio): OfficeLayout {
  if (cached?.lot !== lot) cached = { lot, L: build(lot) };
  return cached.L;
}

// ───────────── walls and rooms ─────────────

const near = (a: number, b: number) => Math.abs(a - b) < 1e-6;

function walls(lot: Lot): Wall[] {
  return lot.walls.map((lw) => {
    const flip = lw.x1 < lw.x0 || lw.z1 < lw.z0;
    const [x0, z0, x1, z1] = flip ? [lw.x1, lw.z1, lw.x0, lw.z0] : [lw.x0, lw.z0, lw.x1, lw.z1];
    const len = x1 - x0 + (z1 - z0);
    const span = (at: number, width: number): [number, number] => {
      const c = flip ? len - at : at;
      return [c - width / 2, c + width / 2];
    };
    const gaps = lot.doors.filter((d) => d.wall === lw.id).map((d) => span(d.at, d.width));
    const windows = lot.windows.filter((w) => w.wall === lw.id).map((w) => span(w.at, w.width));
    return { id: lw.id, x0, z0, x1, z1, kind: lw.kind, gaps, windows: windows.length ? windows : undefined };
  });
}

/** Wall barriers between tiles: `h[r * cols + c]` closes the north edge of tile (c, r), `v` its west edge. */
function barriers(L: { walls: Wall[] }, cols: number, rows: number) {
  const h = new Uint8Array(cols * (rows + 1));
  const v = new Uint8Array((cols + 1) * rows);
  for (const w of L.walls) {
    if (near(w.z0, w.z1)) {
      const r = Math.round(w.z0 / TILE);
      for (let c = Math.round(w.x0 / TILE); c < Math.round(w.x1 / TILE); c++) if (c >= 0 && c < cols && r >= 0 && r <= rows) h[r * cols + c] = 1;
    } else {
      const c = Math.round(w.x0 / TILE);
      for (let r = Math.round(w.z0 / TILE); r < Math.round(w.z1 / TILE); r++) if (c >= 0 && c <= cols && r >= 0 && r < rows) v[r * (cols + 1) + c] = 1;
    }
  }
  return { h, v };
}

/** Flood fill: regions of tiles reachable without crossing a wall (`same` limits a region
 *  further). Returns tile → region and, per region, whether it leaked off the lot's open edge. */
function fill(cols: number, rows: number, bar: { h: Uint8Array; v: Uint8Array }, same: (a: number, b: number) => boolean = () => true) {
  const of = new Int16Array(cols * rows).fill(-1);
  const open: boolean[] = [];
  for (let start = 0; start < of.length; start++) {
    if (of[start] >= 0) continue;
    const id = open.length;
    open.push(false);
    of[start] = id;
    const queue = [start];
    for (let i = 0; i < queue.length; i++) {
      const t = queue[i];
      const c = t % cols;
      const r = (t - c) / cols;
      const steps: [number, boolean][] = [
        [r > 0 ? t - cols : -1, !!bar.h[r * cols + c]],
        [r < rows - 1 ? t + cols : -1, !!bar.h[(r + 1) * cols + c]],
        [c > 0 ? t - 1 : -1, !!bar.v[r * (cols + 1) + c]],
        [c < cols - 1 ? t + 1 : -1, !!bar.v[r * (cols + 1) + c + 1]],
      ];
      for (const [n, wall] of steps) {
        if (wall) continue;
        if (n < 0) {
          open[id] = true;
          continue;
        }
        if (of[n] >= 0 || !same(t, n)) continue;
        of[n] = id;
        queue.push(n);
      }
    }
  }
  return { of, open };
}

/** A room's name and id by what's inside, first rule that matches. */
const ROOM_RULES: { id: string; name: string; has: (inside: Item[], frontDoor: boolean, rollup: boolean) => boolean }[] = [
  { id: 'garage', name: 'Garage', has: (_, front, rollup) => front && rollup },
  { id: 'studio', name: 'Studio', has: (its) => its.some((i) => i.job === 'desk') },
  { id: 'lead', name: 'Leads’ office', has: (its) => its.some((i) => i.job === 'lead') },
  { id: 'server', name: 'Server room', has: (its) => its.some((i) => i.job === 'server') },
  { id: 'library', name: 'Library', has: (its) => its.some((i) => i.job === 'books') },
  { id: 'archive', name: 'Memory archive', has: (its) => its.some((i) => i.job === 'memory') },
  { id: 'mail', name: 'Mail room', has: (its) => its.some((i) => i.job === 'mail') },
  { id: 'meeting', name: 'Meeting room', has: (its) => its.some((i) => i.job === 'meeting' || i.job === 'whiteboard') },
  { id: 'lounge', name: 'Lounge', has: (its) => its.some((i) => i.type === 'counter' || i.type === 'coffee' || i.type === 'fridge') },
  { id: 'lobby', name: 'Lobby', has: (_, front) => front },
  { id: 'room', name: 'Room', has: () => true },
];

// ───────────── the layout ─────────────

function build(lot: Lot): OfficeLayout {
  const W = lot.size.w;
  const D = lot.size.d;
  const tc = Math.round(W / TILE);
  const tr = Math.round(D / TILE);
  const tileAt = (x: number, z: number) => Math.min(tr - 1, Math.max(0, Math.floor(z / TILE))) * tc + Math.min(tc - 1, Math.max(0, Math.floor(x / TILE)));
  const wallList = walls(lot);
  const bar = barriers({ walls: wallList }, tc, tr);

  // Rooms: closed areas (one that leaks off an edge without a wall is outside).
  const regions = fill(tc, tr, bar);
  const roomOf = new Int16Array(tc * tr).fill(-1);
  const roomIdx = new Map<number, number>();
  regions.of.forEach((g, t) => {
    if (regions.open[g]) return;
    if (!roomIdx.has(g)) roomIdx.set(g, roomIdx.size);
    roomOf[t] = roomIdx.get(g)!;
  });
  const nRooms = roomIdx.size;
  const roomAtXZ = (x: number, z: number) => (x < 0 || z < 0 || x > W || z > D ? -1 : roomOf[tileAt(x, z)]);

  // The front door: the first door in a wall on the lot's edge.
  const onEdge = (w: Wall) => (near(w.z0, w.z1) && (near(w.z0, 0) || near(w.z0, D))) || (near(w.x0, w.x1) && (near(w.x0, 0) || near(w.x0, W)));
  const fd = lot.doors.find((d) => wallList.some((w) => w.id === d.wall && onEdge(w))) ?? lot.doors[0];
  const fw = fd && wallList.find((w) => w.id === fd.wall);
  let door: OfficeLayout['door'];
  {
    const horiz = !fw || near(fw.z0, fw.z1);
    const len = fw ? fw.x1 - fw.x0 + (fw.z1 - fw.z0) : W;
    const gap = fw?.gaps[lot.doors.filter((d) => d.wall === fd.wall).indexOf(fd)];
    const at = gap ? (gap[0] + gap[1]) / 2 : len / 2;
    const x = fw ? (horiz ? fw.x0 + at : fw.x0) : W / 2;
    const z = fw ? (horiz ? fw.z0 : fw.z0 + at) : D;
    // In: towards the lot's middle.
    const [nx, nz] = horiz ? [0, z > D / 2 ? -1 : 1] : [x > W / 2 ? -1 : 1, 0];
    const face = Math.atan2(nx, nz);
    const s = (k: number): Slot => ({ x: x + nx * k, z: z + nz * k, face, pose: 'stand' });
    door = { outside: s(0.3), inside: s(1.8), x, z, rot: Math.atan2(-nx, -nz), nx, nz, width: fd?.width ?? 1.6, kind: fd?.kind ?? 'single' };
  }

  // Furniture: the office's own pieces become items; model pieces that came with the lot are
  // drawn like bought ones.
  const items: Item[] = [];
  const pieces: Placed[] = [];
  const gift = new Set(lot.gift);
  const roomIndexOf = new Map<Item, number>();
  const put = (it: Omit<Item, 'room'>) => {
    const item = { ...it, room: '' };
    roomIndexOf.set(item, roomAtXZ(it.x, it.z));
    items.push(item);
    return item;
  };
  const placed: Item[] = [];
  for (const p of lot.furniture) {
    const b = BUILTIN.get(p.item);
    if (!b) {
      if (KNOWN.has(p.item) && gift.has(p.uid)) pieces.push(p);
      continue;
    }
    placed.push(put({ type: b.builtin, x: p.x, z: p.z, y: p.y || undefined, rot: p.rot, w: b.w, d: b.d, block: b.block ?? true, variant: b.variant, uid: p.uid, job: b.job }));
  }

  // Rooms named by what's inside.
  const inside: Item[][] = Array.from({ length: nRooms }, () => []);
  for (const it of placed) {
    const k = roomIndexOf.get(it)!;
    if (k >= 0) inside[k].push(it);
  }
  const doorRoom = roomAtXZ(door.inside.x, door.inside.z);
  const box: { x0: number; z0: number; x1: number; z1: number }[] = Array.from({ length: nRooms }, () => ({ x0: Infinity, z0: Infinity, x1: -Infinity, z1: -Infinity }));
  roomOf.forEach((k, t) => {
    if (k < 0) return;
    const c = t % tc;
    const r = (t - c) / tc;
    const b = box[k];
    b.x0 = Math.min(b.x0, c * TILE);
    b.z0 = Math.min(b.z0, r * TILE);
    b.x1 = Math.max(b.x1, (c + 1) * TILE);
    b.z1 = Math.max(b.z1, (r + 1) * TILE);
  });
  const taken = new Map<string, number>();
  const rooms: Room[] = box.map((b, k) => {
    const rule = ROOM_RULES.find((r) => r.has(inside[k], k === doorRoom, door.kind === 'rollup'))!;
    const n = (taken.get(rule.id) ?? 0) + 1;
    taken.set(rule.id, n);
    const tag = lot.labels.find((l) => roomAtXZ(l.x, l.z) === k);
    let label: [number, number] = tag ? [tag.x, tag.z] : [(b.x0 + b.x1) / 2, b.z1 - 0.6];
    if (!tag && roomAtXZ(...label) !== k) {
      // An odd-shaped room: the label goes on its own tile nearest the middle.
      let best = Infinity;
      roomOf.forEach((kk, t) => {
        if (kk !== k) return;
        const c = t % tc;
        const p: [number, number] = [(c + 0.5) * TILE, ((t - c) / tc + 0.5) * TILE];
        const d = Math.hypot(p[0] - label[0], p[1] - label[1]);
        if (d < best) (best = d), (label = p);
      });
    }
    return { id: n > 1 ? `${rule.id}-${n}` : rule.id, name: tag?.name || rule.name, rect: { x: b.x0, z: b.z0, w: b.x1 - b.x0, d: b.z1 - b.z0 }, label };
  });
  const roomName = (k: number) => (k >= 0 ? rooms[k].id : '');
  for (const it of items) it.room = roomName(roomIndexOf.get(it) ?? -1);

  // ── desks (and the chair, monitor and lamp each brings) ──
  const desks: Desk[] = [];
  const deskItems = placed.filter((it) => it.job === 'desk' || it.job === 'lead');
  for (const it of deskItems) {
    const lead = it.job === 'lead';
    const index = desks.length;
    const { x, z, rot, room } = it;
    const fx = Math.sin(rot);
    const fz = Math.cos(rot);
    // The occupant's right hand (facing f = (fx, fz), y up): r = (−fz, fx).
    const rx = -fz;
    const rz = fx;
    const depth = it.d;
    const width = it.w;
    // Helpers stand on the outside of a pod: away from the desk next to it (`internSide` is the
    // occupant's right, +1, or left, −1); a desk on its own, away from the middle of its room
    // (its left on a tie).
    const side = (s: 1 | -1) =>
      deskItems.some(
        (o) =>
          o !== it &&
          Math.abs(o.rot - rot) < 0.01 &&
          Math.abs((o.x - x) * fx + (o.z - z) * fz) < 0.1 &&
          Math.abs(((o.x - x) * rx + (o.z - z) * rz) * s - (width + o.w) / 2) < 0.02,
      );
    const r = rooms[roomIndexOf.get(it) ?? -1]?.rect;
    const mid = r ? (r.x + r.w / 2 - x) * rx + (r.z + r.d / 2 - z) * rz : 0;
    const internSide: 1 | -1 = side(1) ? -1 : side(-1) ? 1 : mid < 0 ? 1 : -1;
    const seatX = x - fx * (depth / 2 + 0.36);
    const seatZ = z - fz * (depth / 2 + 0.36);
    const standX = seatX + rx * internSide * 0.62 - fx * 0.15;
    const standZ = seatZ + rz * internSide * 0.62 - fz * 0.15;
    const internX = x + rx * internSide * (width / 2 + 0.42) - fx * 0.12;
    const internZ = z + rz * internSide * (width / 2 + 0.42) - fz * 0.12;
    desks.push({
      index,
      lead,
      x,
      z,
      rot,
      seat: { x: seatX, z: seatZ, face: rot, pose: 'sit', seat: SEAT_H },
      stand: { x: standX, z: standZ, face: rot, pose: 'stand' },
      intern: { x: internX, z: internZ, face: Math.atan2(seatX - internX, seatZ - internZ), pose: 'stand' },
      screen: [x + fx * (depth * 0.18), DESK_H + 0.27, z + fz * (depth * 0.18)],
      propSpot: [x - rx * internSide * (lead ? 0.6 : 0.42) + fx * 0.06, z - rz * internSide * (lead ? 0.6 : 0.42) + fz * 0.06],
      room,
    });
    it.desk = index;
    const own = (type: ItemType, px: number, pz: number, prot: number, w: number, d: number, variant?: number) => {
      const c = put({ type, x: px, z: pz, rot: prot, w, d, block: false, variant, desk: index, uid: it.uid });
      c.room = room;
    };
    own('chair', seatX, seatZ, rot, 0.5, 0.5, lead ? 1 : 0);
    own('monitor', x + fx * (depth * 0.18), z + fz * (depth * 0.18), rot + Math.PI, 0.6, 0.1, lead ? 1 : 0);
    own('lamp', x - rx * internSide * (lead ? 0.66 : 0.5) + fx * 0.18, z - rz * internSide * (lead ? 0.66 : 0.5) + fz * 0.18, rot, 0.2, 0.2);
  }

  // ── stations: use-slots in each piece's own frame (lx to its right, lz out of its front) ──
  const stations = Object.fromEntries(STATION_IDS.map((id) => [id, { id, room: '', slots: [] as Slot[] }])) as Record<StationId, Station>;
  const at = (it: Item, lx: number, lz: number, face: number, pose: 'stand' | 'sit' = 'stand', seat?: number): Slot => ({
    x: it.x + Math.cos(it.rot) * lx + Math.sin(it.rot) * lz,
    z: it.z - Math.sin(it.rot) * lx + Math.cos(it.rot) * lz,
    face: Math.atan2(Math.sin(face), Math.cos(face)),
    pose,
    seat,
  });
  const add = (id: StationId, it: Item | null, ...slots: Slot[]) => {
    const st = stations[id];
    if (!st.slots.length && it) st.room = it.room;
    st.slots.push(...slots);
  };
  const back = (it: Item) => it.rot + Math.PI;
  const of = (type: ItemType) => placed.filter((i) => i.type === type);
  const hasTv = placed.some((i) => i.type === 'tvStand');
  for (const it of placed) {
    switch (it.job) {
      case 'books':
        add('library.shelf', it, at(it, 0, it.d / 2 + 0.48, back(it)));
        break;
      case 'board':
        add('board', it, ...[-1.9, 0, 1.9].map((dx) => at(it, dx, 0.95, back(it))));
        break;
      case 'whiteboard':
        add('meeting.board', it, at(it, -1.0, 0.92, back(it)), at(it, 1.0, 0.92, back(it)));
        break;
      case 'server':
        if (it.type === 'serverMachine') {
          add('server.rack', it, at(it, -0.45, 0.9, back(it)));
          add('server.monitor', it, at(it, 0.3, 0.9, back(it)));
        } else add('server.rack', it, at(it, 0, it.d / 2 + 0.775, back(it)));
        break;
      case 'tests':
        add('server.monitor', it, at(it, -0.6, 1.2, back(it)), at(it, 0.6, 1.2, back(it)));
        break;
      case 'memory':
        add('archive', it, at(it, 0, 1.0, back(it)));
        break;
      case 'meeting': {
        // Its chairs come with it: three a side.
        for (const dx of [-1.15, 0, 1.15]) {
          for (const s of [at(it, dx, -1.05, it.rot, 'sit', SEAT_H), at(it, dx, 1.05, it.rot + Math.PI, 'sit', SEAT_H)]) {
            add('meeting.table', it, s);
            const c = put({ type: 'chair', x: s.x, z: s.z, rot: s.face, w: 0.5, d: 0.5, block: false, variant: 2, uid: it.uid });
            c.room = it.room;
          }
        }
        break;
      }
    }
    switch (it.type) {
      case 'armchair':
        if (it.room.startsWith('library')) add('library.chair', it, at(it, 0, 0, it.rot, 'sit', 0.36));
        else if (it.room.startsWith('archive')) add('nap.armchair', it, at(it, 0, 0, it.rot, 'sit', 0.36));
        break;
      case 'sofa':
        if (hasTv) add('games.console', it, at(it, -0.5, 0.15, it.rot, 'sit', 0.38), at(it, 0.5, 0.15, it.rot, 'sit', 0.38));
        break;
      case 'foosball':
        add('games.foosball', it, at(it, -0.85, 0, it.rot + Math.PI / 2), at(it, 0.85, 0, it.rot - Math.PI / 2));
        break;
      case 'pingpong':
        add('games.pingpong', it, at(it, 1.35, 0, it.rot - Math.PI / 2), at(it, -1.35, 0, it.rot + Math.PI / 2));
        break;
      case 'poolTable':
        add('games.pool', it, at(it, -0.5, -1.15, it.rot), at(it, 0.5, 1.1, it.rot + Math.PI));
        break;
    }
  }
  // The mailbox before the pigeonholes; the espresso machine first, then the fridge.
  for (const it of of('mailbox')) add('mail', it, at(it, 0, 0.9, back(it)));
  for (const it of of('pigeonholes')) add('mail', it, at(it, 0, 0.97, back(it)));
  for (const it of of('coffee')) add('kitchen', it, at(it, 0, 0.65, back(it)));
  for (const it of of('fridge')) add('kitchen', it, at(it, 0, 0.9, back(it)));
  for (const d of desks.filter((d) => d.lead)) {
    const it = deskItems[d.index];
    add('lead.visit', it, at(it, -0.45, 1.25, back(it)), at(it, 0.45, 1.25, back(it)));
  }
  stations.door = { id: 'door', room: roomName(doorRoom), slots: [{ x: door.x + door.nx * 0.45, z: door.z + door.nz * 0.45, face: door.outside.face, pose: 'stand' }] };
  // No test monitor: they watch the racks. No archive cabinet: memory lives on the bookshelf.
  if (!stations['server.monitor'].slots.length) stations['server.monitor'] = { ...stations['server.rack'], id: 'server.monitor' };
  if (!stations.archive.slots.length) stations.archive = { ...stations['library.shelf'], id: 'archive' };

  // ── what the props hang on ──
  const first = (type: ItemType) => placed.find((i) => i.type === type) ?? null;
  const tb = first('taskBoard');
  const wb = first('whiteboard');
  const tm = first('testMonitor');
  const sm = first('serverMachine');
  const fwd = (it: Item, k: number, lx = 0): [number, number] => [it.x + Math.sin(it.rot) * k + Math.cos(it.rot) * lx, it.z + Math.cos(it.rot) * k - Math.sin(it.rot) * lx];
  const mb = first('mailbox');
  const ph = first('pigeonholes');
  const cat = first('catalog');
  const ck = first('clock');
  const memory = placed.find((i) => i.job === 'memory') ?? cat ?? placed.find((i) => i.job === 'books');
  const ghostRoom = rooms[memory ? (roomIndexOf.get(memory) ?? -1) : doorRoom] ?? rooms[doorRoom];

  const layout: OfficeLayout = {
    lot,
    w: W,
    d: D,
    cols: Math.round(W / CELL),
    rows: Math.round(D / CELL),
    rooms,
    roomOf,
    tiles: { cols: tc, rows: tr },
    floors: floorRegions(lot, tc, tr, bar),
    walls: wallList,
    items,
    pieces,
    stations,
    desks,
    door,
    blocked: new Uint8Array(Math.round(W / CELL) * Math.round(D / CELL)),
    board: tb && { z: tb.z + 0.07, x0: tb.x - 3.0, x1: tb.x + 3.0, y0: 0.42, y1: 1.72 },
    whiteboard: wb && { z: wb.z + 0.06, x0: wb.x - 2.45, x1: wb.x + 2.45, y0: 0.6, y1: 1.95 },
    testMonitor: tm
      ? { x: fwd(tm, 0.07)[0], z: fwd(tm, 0.07)[1], y: 1.45, w: 2.1, h: 1.05, rot: tm.rot }
      : sm
        ? { x: fwd(sm, -0.03, 0.3)[0], z: fwd(sm, -0.03, 0.3)[1], y: 1.02, w: 0.56, h: 0.32, rot: sm.rot }
        : null,
    racks: placed.flatMap((i): OfficeLayout['racks'] =>
      i.type === 'rack' ? [{ x: i.x, z: i.z, rot: i.rot, front: 0.395, wide: true }] : i.type === 'serverMachine' ? [{ ...xz(fwd(i, 0, -0.45)), rot: i.rot, front: 0.27, wide: false }] : [],
    ),
    mailbox: mb && { x: mb.x, z: mb.z, rot: mb.rot },
    pigeonholes: ph && { x: ph.x, z: ph.z, rot: ph.rot, cols: 7, rows: 4 },
    cabinets: of('cabinet').map((i) => ({ x: i.x, z: i.z, rot: i.rot })),
    catalog: cat && { x: cat.x, z: cat.z, rot: cat.rot },
    shelves: placed.filter((i) => i.job === 'books').map((i) => ({ x: i.x, z: i.z, rot: i.rot, len: i.w })),
    clock: ck && { x: ck.x, y: ck.y ?? 1.55, z: ck.z, rot: ck.rot },
    ghosts: [],
    archive: ghostRoom ? [ghostRoom.rect.x + ghostRoom.rect.w / 2, ghostRoom.rect.z + Math.min(2.1, ghostRoom.rect.d / 2)] : [W / 2, D / 2],
    lights: lights(rooms),
  };
  rasterize(layout);
  stations.wait = { id: 'wait', room: roomName(doorRoom), slots: waitSpots(layout, rooms[doorRoom]) };
  if (ghostRoom) {
    const r = ghostRoom.rect;
    const [cx, cz] = [r.x + r.w / 2, r.z + r.d / 2];
    layout.ghosts = (
      [
        [cx + r.w * 0.18, cz + r.d * 0.3, 0],
        [cx - r.w * 0.3, cz - r.d * 0.08, 0.5],
        [cx + r.w * 0.25, cz - r.d * 0.07, -0.4],
      ] as [number, number, number][]
    ).map(([x, z, f]) => [...freeNear(layout, x, z), f]);
  }
  return layout;
}

const xz = ([x, z]: [number, number]) => ({ x, z });

const FLOOR_KINDS = new Set<string>(['oak', 'walnut', 'walnut-planks', 'carpet-sage', 'carpet-blue', 'tile-cool', 'tile-warm', 'terrazzo', 'concrete']);

/** Paint the tiles (default, then each rect in order) and group them: one region per material
 *  area between walls. */
function floorRegions(lot: Lot, tc: number, tr: number, bar: { h: Uint8Array; v: Uint8Array }): FloorRegion[] {
  const kinds: FloorKind[] = [];
  const idx = (m: string) => {
    const k = (FLOOR_KINDS.has(m) ? m : 'oak') as FloorKind;
    const i = kinds.indexOf(k);
    return i >= 0 ? i : kinds.push(k) - 1;
  };
  const mat = new Uint8Array(tc * tr).fill(idx(lot.floors.default));
  for (const r of lot.floors.rects) {
    const m = idx(r.m);
    for (let z = Math.round(r.z / TILE); z < Math.round((r.z + r.d) / TILE); z++)
      for (let x = Math.round(r.x / TILE); x < Math.round((r.x + r.w) / TILE); x++) if (x >= 0 && z >= 0 && x < tc && z < tr) mat[z * tc + x] = m;
  }
  const { of } = fill(tc, tr, bar, (a, b) => mat[a] === mat[b]);
  const out: FloorRegion[] = [];
  of.forEach((g, t) => {
    const c = t % tc;
    const r = (t - c) / tc;
    const reg = (out[g] ??= { m: kinds[mat[t]], rect: { x: c * TILE, z: r * TILE, w: 0, d: 0 }, tiles: [] });
    reg.tiles.push(t);
    const x1 = Math.max(reg.rect.x + reg.rect.w, (c + 1) * TILE);
    const z1 = Math.max(reg.rect.z + reg.rect.d, (r + 1) * TILE);
    reg.rect.x = Math.min(reg.rect.x, c * TILE);
    reg.rect.w = x1 - reg.rect.x;
    reg.rect.d = z1 - reg.rect.z;
  });
  return out;
}

/** Warm light over each room in the evening (a blue one in a server room), two in a long room. */
function lights(rooms: Room[]): Light[] {
  const out: Light[] = [];
  for (const r of rooms) {
    const long = r.rect.w > r.rect.d * 1.8 ? 'x' : r.rect.d > r.rect.w * 1.8 ? 'z' : null;
    const n = long ? 2 : 1;
    const area = (r.rect.w * r.rect.d) / n;
    const cool = r.id.startsWith('server');
    for (let i = 0; i < n; i++) {
      const t = (i + 0.5) / n;
      const x = long === 'x' ? r.rect.x + r.rect.w * t : r.rect.x + r.rect.w / 2;
      const z = long === 'z' ? r.rect.z + r.rect.d * t : r.rect.z + r.rect.d / 2;
      const inten = Math.min(5, Math.max(1.8, 1.2 + area * 0.04)) * (cool ? 0.75 : 1);
      out.push([x, cool ? 1.5 : 1.8, z, cool ? '#8fb8ff' : '#ffc58a', inten, Math.min(9, Math.max(5.5, Math.sqrt(area) * 0.95))]);
    }
  }
  return out;
}

// ───────────── walkable grid ─────────────

const INFLATE = 0.14;
const WALL_T = 0.16;

export function markRect(b: Uint8Array, r: Rect, inflate: number, cols: number, rows: number) {
  const x0 = Math.max(0, Math.floor((r.x - inflate) / CELL + 1e-6));
  const z0 = Math.max(0, Math.floor((r.z - inflate) / CELL + 1e-6));
  const x1 = Math.min(cols - 1, Math.ceil((r.x + r.w + inflate) / CELL - 1e-6) - 1);
  const z1 = Math.min(rows - 1, Math.ceil((r.z + r.d + inflate) / CELL - 1e-6) - 1);
  for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) b[z * cols + x] = 1;
}

function clearRect(b: Uint8Array, r: Rect, cols: number, rows: number) {
  const x0 = Math.max(0, Math.floor(r.x / CELL + 1e-6));
  const z0 = Math.max(0, Math.floor(r.z / CELL + 1e-6));
  const x1 = Math.min(cols - 1, Math.ceil((r.x + r.w) / CELL - 1e-6) - 1);
  const z1 = Math.min(rows - 1, Math.ceil((r.z + r.d) / CELL - 1e-6) - 1);
  for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) b[z * cols + x] = 0;
}

/** The solid pieces of a wall (segment minus gaps), as rects. */
export function wallPieces(w: Wall): Rect[] {
  const horiz = w.z0 === w.z1;
  const len = horiz ? w.x1 - w.x0 : w.z1 - w.z0;
  const gaps = [...w.gaps].sort((a, b) => a[0] - b[0]);
  const out: Rect[] = [];
  let at = 0;
  for (const [a, b] of [...gaps, [len, len] as [number, number]]) {
    if (a > at + 1e-6) {
      out.push(horiz ? { x: w.x0 + at, z: w.z0 - WALL_T / 2, w: a - at, d: WALL_T } : { x: w.x0 - WALL_T / 2, z: w.z0 + at, w: WALL_T, d: a - at });
    }
    at = Math.max(at, b);
  }
  return out;
}

function rasterize(L: OfficeLayout) {
  const { blocked: b, cols, rows } = L;
  // The outer border is never walkable.
  for (let x = 0; x < cols; x++) {
    b[x] = 1;
    b[(rows - 1) * cols + x] = 1;
  }
  for (let z = 0; z < rows; z++) {
    b[z * cols] = 1;
    b[z * cols + cols - 1] = 1;
  }
  for (const w of L.walls) for (const r of wallPieces(w)) markRect(b, r, INFLATE * 0.6, cols, rows);
  for (const it of L.items) if (it.block) markRect(b, footprint(it), INFLATE, cols, rows);
  for (const r of keepRects(L)) clearRect(b, r, cols, rows);
}

/** Cells that are always walkable: use-slots, seats, the door (Build/Buy can't cover them). */
export function keepRects(L: OfficeLayout): Rect[] {
  const out: Rect[] = [];
  const keep = (x: number, z: number) => out.push({ x: x - 0.12, z: z - 0.12, w: 0.24, d: 0.24 });
  for (const st of Object.values(L.stations))
    for (const s of st.id === 'wait' ? [] : st.slots) {
      keep(s.x, s.z);
      // Seats on furniture (armchairs, sofas): open the way in from the front.
      if (s.pose === 'sit') for (let d = 0.25; d <= 0.75; d += 0.25) keep(s.x + Math.sin(s.face) * d, s.z + Math.cos(s.face) * d);
    }
  for (const d of L.desks) {
    keep(d.seat.x, d.seat.z);
    keep(d.stand.x, d.stand.z);
    keep(d.intern.x, d.intern.z);
  }
  keep(L.door.inside.x, L.door.inside.z);
  // The doorway itself (the front wall's gap), from a cell inside the wall to 1.2 in.
  const { x, z, nx, nz } = L.door;
  const [ax, az] = [x + nx * CELL, z + nz * CELL];
  const [bx, bz] = [x + nx * 1.2, z + nz * 1.2];
  const [hx, hz] = [Math.abs(nz) * 0.5, Math.abs(nx) * 0.5];
  out.push({ x: Math.min(ax, bx) - hx, z: Math.min(az, bz) - hz, w: Math.abs(bx - ax) + 2 * hx, d: Math.abs(bz - az) + 2 * hz });
  return out;
}

/** Where an agent without a desk (more agents than desks) waits: open floor in the room by
 *  the front door, nearest its middle first, a metre and more apart. */
function waitSpots(L: OfficeLayout, room: Room | undefined): Slot[] {
  if (!room) return [];
  const r = room.rect;
  const [cx, cz] = [r.x + r.w / 2, r.z + r.d / 2];
  const keep = keepRects(L);
  const pass = passableFn(L);
  const open = (x: number, z: number) => {
    const [gx, gz] = cellOf(L, x, z);
    for (let dz = -2; dz <= 2; dz++) for (let dx = -2; dx <= 2; dx++) if (!pass(gx + dx, gz + dz)) return false;
    return roomAt(L, x, z) === room.id && !keep.some((k) => x > k.x - 0.6 && x < k.x + k.w + 0.6 && z > k.z - 0.6 && z < k.z + k.d + 0.6);
  };
  const spots: [number, number][] = [];
  for (let z = r.z + 0.5; z < r.z + r.d; z += 0.5) for (let x = r.x + 0.5; x < r.x + r.w; x += 0.5) if (open(x, z)) spots.push([x, z]);
  spots.sort((a, b) => Math.hypot(a[0] - cx, a[1] - cz) - Math.hypot(b[0] - cx, b[1] - cz));
  const out: Slot[] = [];
  for (const [x, z] of spots) {
    if (out.length >= 8) break;
    if (out.some((s) => Math.hypot(s.x - x, s.z - z) < 1.2)) continue;
    out.push({ x, z, face: Math.atan2(cx - x, cz - z), pose: 'stand' });
  }
  return out;
}

/** The walkable cell centre nearest (x, z) (itself when it is free). */
export function freeNear(L: OfficeLayout, x: number, z: number): [number, number] {
  const [cx, cz] = cellOf(L, x, z);
  let best: [number, number] = [x, z];
  let bd = Infinity;
  for (let r = 0; r <= 12 && bd === Infinity; r++)
    for (let dz = -r; dz <= r; dz++)
      for (let dx = -r; dx <= r; dx++) {
        if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
        const [gx, gz] = [cx + dx, cz + dz];
        if (gx < 0 || gz < 0 || gx >= L.cols || gz >= L.rows || L.blocked[gz * L.cols + gx]) continue;
        const [px, pz] = cellCenter(gx, gz);
        const d = Math.hypot(px - x, pz - z);
        if (d < bd) (bd = d), (best = [px, pz]);
      }
  return best;
}

export function cellOf(L: Pick<OfficeLayout, 'cols' | 'rows'>, x: number, z: number): [number, number] {
  return [Math.min(L.cols - 1, Math.max(0, Math.floor(x / CELL))), Math.min(L.rows - 1, Math.max(0, Math.floor(z / CELL)))];
}

export function cellCenter(cx: number, cz: number): [number, number] {
  return [(cx + 0.5) * CELL, (cz + 0.5) * CELL];
}

export function passableFn(L: OfficeLayout): (x: number, y: number) => boolean {
  const { blocked: b, cols, rows } = L;
  return (x, y) => x >= 0 && y >= 0 && x < cols && y < rows && b[y * cols + x] === 0;
}

export function roomAt(L: OfficeLayout, x: number, z: number): RoomId | null {
  if (x < 0 || z < 0 || x > L.w || z > L.d) return null;
  const c = Math.min(L.tiles.cols - 1, Math.floor(x / TILE));
  const r = Math.min(L.tiles.rows - 1, Math.floor(z / TILE));
  const k = L.roomOf[r * L.tiles.cols + c];
  return k >= 0 ? L.rooms[k].id : null;
}
