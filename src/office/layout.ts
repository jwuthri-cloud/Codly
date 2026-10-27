// The floor plan of the studio: rooms, walls (with door gaps), furniture placements, the
// stations characters walk to (with use-slots), the desks, and the walkable grid for A*.
// Pure data: no three.js, so it is testable in Node and the renderer just reads it.
//
// Coordinates: x grows east (right), z grows south (towards the default camera); one unit is
// about 60 cm at chibi scale (a character is ~1.15 units tall). The plan is 28 × 24.
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

export const W = 28;
export const D = 24;
export const CELL = 0.25;
export const COLS = Math.round(W / CELL);
export const ROWS = Math.round(D / CELL);

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

export type RoomId = 'library' | 'meeting' | 'lead' | 'studio' | 'server' | 'lounge' | 'mail' | 'lobby' | 'archive';
export type FloorKind = 'oak' | 'walnut' | 'walnut-planks' | 'carpet-sage' | 'carpet-blue' | 'tile-cool' | 'tile-warm' | 'terrazzo';

export interface Room {
  id: RoomId;
  name: string;
  rect: Rect;
  floor: FloorKind;
  /** Where the room's floor label sits. */
  label: [number, number];
}

export type WallKind = 'exterior' | 'interior' | 'glass' | 'low';

export interface Wall {
  /** Axis-aligned segment from (x0, z0) to (x1, z1). */
  x0: number;
  z0: number;
  x1: number;
  z1: number;
  kind: WallKind;
  /** Openings along the segment, as [from, to] distances from (x0, z0). */
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
  | 'door'
  | 'poster'
  | 'credenza'
  | 'tvStand'
  | 'foosball'
  | 'poolTable';

export interface Item {
  type: ItemType;
  x: number;
  z: number;
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
  | 'games.pool'
  | 'door';

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

export interface OfficeLayout {
  rooms: Room[];
  walls: Wall[];
  items: Item[];
  stations: Record<StationId, Station>;
  desks: Desk[];
  /** Spawn/despawn: just inside the front door, and the hall point in front of it. */
  door: { outside: Slot; inside: Slot; x: number; z: number };
  /** Walkable grid (1 = blocked), COLS × ROWS cells of CELL units. */
  blocked: Uint8Array;
  /** The freestanding task board along the studio's north walkway, facing the camera (south). */
  board: { z: number; x0: number; x1: number; y0: number; y1: number };
  /** Meeting room whiteboard on the north wall. */
  whiteboard: { z: number; x0: number; x1: number; y0: number; y1: number };
  /** Server room test monitor (front center) and the racks (LED anchors). */
  testMonitor: { x: number; z: number; y: number; w: number; h: number; rot: number };
  racks: { x: number; z: number; rot: number }[];
  /** Mail room: outbox position (flag) and pigeonhole grid origin. */
  mailbox: { x: number; z: number; rot: number };
  pigeonholes: { x: number; z: number; rot: number; cols: number; rows: number };
  /** Archive cabinets (drawer lights). */
  cabinets: { x: number; z: number; rot: number }[];
  catalog: { x: number; z: number; rot: number };
  /** Library shelves (book glow on retrieval). */
  shelves: { x: number; z: number; rot: number; len: number }[];
}

export const ROOM_NAMES: Record<RoomId, string> = {
  library: 'Library',
  meeting: 'Meeting room',
  lead: 'Leads’ office',
  studio: 'Studio',
  server: 'Server room',
  lounge: 'Lounge',
  mail: 'Mail room',
  lobby: 'Lobby',
  archive: 'Memory archive',
};

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

let cached: OfficeLayout | null = null;

export function officeLayout(): OfficeLayout {
  return (cached ??= build());
}

function build(): OfficeLayout {
  const rooms: Room[] = [
    { id: 'library', name: ROOM_NAMES.library, rect: { x: 0, z: 0, w: 7, d: 7 }, floor: 'carpet-sage', label: [3.5, 6.4] },
    { id: 'meeting', name: ROOM_NAMES.meeting, rect: { x: 7, z: 0, w: 8, d: 7 }, floor: 'carpet-blue', label: [11, 6.4] },
    { id: 'lead', name: ROOM_NAMES.lead, rect: { x: 15, z: 0, w: 13, d: 7 }, floor: 'walnut', label: [21.5, 6.4] },
    { id: 'studio', name: ROOM_NAMES.studio, rect: { x: 0, z: 7, w: 21.5, d: 9.5 }, floor: 'oak', label: [19.4, 15.9] },
    { id: 'server', name: ROOM_NAMES.server, rect: { x: 21.5, z: 7, w: 6.5, d: 9.5 }, floor: 'tile-cool', label: [24.75, 15.9] },
    { id: 'lounge', name: ROOM_NAMES.lounge, rect: { x: 0, z: 16.5, w: 8, d: 7.5 }, floor: 'tile-warm', label: [4, 23.4] },
    { id: 'mail', name: ROOM_NAMES.mail, rect: { x: 8, z: 16.5, w: 5, d: 7.5 }, floor: 'oak', label: [10.5, 23.4] },
    { id: 'lobby', name: ROOM_NAMES.lobby, rect: { x: 13, z: 16.5, w: 8.5, d: 7.5 }, floor: 'terrazzo', label: [15.2, 23.4] },
    { id: 'archive', name: ROOM_NAMES.archive, rect: { x: 21.5, z: 16.5, w: 6.5, d: 7.5 }, floor: 'walnut-planks', label: [24.75, 23.4] },
  ];

  const DOOR_X = 18.2;
  const walls: Wall[] = [
    // Exterior back walls: full height, with windows.
    { x0: 0, z0: 0, x1: W, z1: 0, kind: 'exterior', gaps: [], windows: [[16.6, 19.4], [20.1, 22.9], [23.6, 26.4]] },
    { x0: 0, z0: 0, x1: 0, z1: D, kind: 'exterior', gaps: [], windows: [[3.7, 6.3], [18.3, 21.8]] },
    // Exterior front walls: cut away (low), the entrance door in the south wall.
    { x0: 0, z0: D, x1: W, z1: D, kind: 'low', gaps: [[DOOR_X - 0.8, DOOR_X + 0.8]] },
    { x0: W, z0: 0, x1: W, z1: D, kind: 'low', gaps: [] },
    // Back rooms.
    { x0: 7, z0: 0, x1: 7, z1: 7, kind: 'interior', gaps: [] },
    { x0: 15, z0: 0, x1: 15, z1: 7, kind: 'interior', gaps: [] },
    { x0: 0, z0: 7, x1: 7, z1: 7, kind: 'interior', gaps: [[4.4, 5.8]] },
    { x0: 7, z0: 7, x1: 15, z1: 7, kind: 'interior', gaps: [[5.8, 7.2]] },
    { x0: 15, z0: 7, x1: 28, z1: 7, kind: 'glass', gaps: [[1.2, 2.6]] },
    // Server room (glass towards the studio).
    { x0: 21.5, z0: 7, x1: 21.5, z1: 16.5, kind: 'glass', gaps: [[6.6, 8.0]] },
    // Front rooms.
    { x0: 0, z0: 16.5, x1: 8, z1: 16.5, kind: 'interior', gaps: [[4.6, 6.6]] },
    { x0: 8, z0: 16.5, x1: 13, z1: 16.5, kind: 'interior', gaps: [[3.6, 4.8]] },
    { x0: 21.5, z0: 16.5, x1: 28, z1: 16.5, kind: 'interior', gaps: [] },
    { x0: 8, z0: 16.5, x1: 8, z1: 24, kind: 'interior', gaps: [] },
    { x0: 13, z0: 16.5, x1: 13, z1: 24, kind: 'interior', gaps: [[3.4, 4.8]] },
    { x0: 21.5, z0: 16.5, x1: 21.5, z1: 24, kind: 'interior', gaps: [[1.4, 2.8]] },
  ];

  const items: Item[] = [];
  const put = (type: ItemType, room: RoomId, x: number, z: number, rot: number, w: number, d: number, block = true, variant?: number, desk?: number) =>
    items.push({ type, room, x, z, rot, w, d, block, variant, desk });

  // ── desks: 6 pods of 4 in the studio ──
  const desks: Desk[] = [];
  const DESK_W = 1.3;
  const DESK_D = 0.66;
  const addDesk = (x: number, z: number, rot: number, room: RoomId, lead: boolean, internSide: 1 | -1) => {
    const index = desks.length;
    const fx = Math.sin(rot);
    const fz = Math.cos(rot);
    // The occupant's right hand (facing f = (fx, fz), y up): r = (−fz, fx).
    const rx = -fz;
    const rz = fx;
    const depth = lead ? 0.8 : DESK_D;
    const seatX = x - fx * (depth / 2 + 0.36);
    const seatZ = z - fz * (depth / 2 + 0.36);
    const standX = seatX + rx * internSide * 0.62 - fx * 0.15;
    const standZ = seatZ + rz * internSide * 0.62 - fz * 0.15;
    const internX = x + rx * internSide * ((lead ? 1.7 : DESK_W) / 2 + 0.42) - fx * 0.12;
    const internZ = z + rz * internSide * ((lead ? 1.7 : DESK_W) / 2 + 0.42) - fz * 0.12;
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
    put(lead ? 'leadDesk' : 'desk', room, x, z, rot, lead ? 1.7 : DESK_W, depth, true, undefined, index);
    put('chair', room, seatX, seatZ, rot, 0.5, 0.5, false, lead ? 1 : 0, index);
    put('monitor', room, x + fx * (depth * 0.18), z + fz * (depth * 0.18), rot + Math.PI, 0.6, 0.1, false, lead ? 1 : 0, index);
    put('lamp', room, x - rx * internSide * (lead ? 0.66 : 0.5) + fx * 0.18, z - rz * internSide * (lead ? 0.66 : 0.5) + fz * 0.18, rot, 0.2, 0.2, false, undefined, index);
  };

  // Leads first (indices 0..2): the corner office, facing the studio. `internSide` is the
  // occupant's right (+1) or left (−1): helpers stand on the outside of each pod.
  for (const x of [18.0, 21.5, 25.0]) addDesk(x, 3.0, DOWN, 'lead', true, x > 21 ? -1 : 1);

  const podXs = [5.0, 9.9, 14.8];
  const podZs = [10.35, 14.1];
  for (const pz of podZs) {
    for (const px of podXs) {
      // Two desks face north (occupants' backs to the camera, screens visible), two face south.
      addDesk(px - DESK_W / 2, pz + DESK_D / 2 + 0.03, UP, 'studio', false, -1);
      addDesk(px + DESK_W / 2, pz + DESK_D / 2 + 0.03, UP, 'studio', false, 1);
      addDesk(px - DESK_W / 2, pz - DESK_D / 2 - 0.03, DOWN, 'studio', false, 1);
      addDesk(px + DESK_W / 2, pz - DESK_D / 2 - 0.03, DOWN, 'studio', false, -1);
      put('divider', 'studio', px, pz, 0, DESK_W * 2, 0.06, true);
    }
  }

  // ── library ──
  const shelves: OfficeLayout['shelves'] = [];
  for (const [x, len] of [
    [1.5, 1.9],
    [3.6, 1.9],
    [5.7, 1.9],
  ] as const) {
    put('shelf', 'library', x, 0.32, DOWN, len, 0.5, true, 0);
    shelves.push({ x, z: 0.32, rot: DOWN, len });
  }
  for (const [z, len] of [[2.6, 1.9]] as const) {
    put('shelf', 'library', 0.32, z, RIGHT, len, 0.5, true, 1);
    shelves.push({ x: 0.32, z, rot: RIGHT, len });
  }
  put('rug', 'library', 3.9, 3.9, 0, 3.6, 2.6, false, 0);
  put('armchair', 'library', 2.7, 4.6, 2.2, 0.85, 0.8, true, 0);
  put('armchair', 'library', 5.0, 4.7, -2.3, 0.85, 0.8, true, 1);
  put('sideTable', 'library', 3.85, 5.2, 0, 0.5, 0.5, true);
  put('floorLamp', 'library', 1.25, 5.2, 0, 0.35, 0.35, true);
  put('tallPlant', 'library', 6.4, 6.2, 0, 0.5, 0.5, true, 1);
  put('readingTable', 'library', 0.75, 4.3, RIGHT, 1.2, 0.6, true);

  // ── meeting room ──
  put('whiteboard', 'meeting', 11, 0.08, DOWN, 5.2, 0.08, true);
  put('meetingTable', 'meeting', 11, 4.3, 0, 3.6, 1.3, true);
  const tableSeats: Slot[] = [];
  for (const dx of [-1.15, 0, 1.15]) {
    tableSeats.push({ x: 11 + dx, z: 4.3 - 1.05, face: DOWN, pose: 'sit', seat: SEAT_H });
    tableSeats.push({ x: 11 + dx, z: 4.3 + 1.05, face: UP, pose: 'sit', seat: SEAT_H });
  }
  for (const s of tableSeats) put('chair', 'meeting', s.x, s.z, s.face, 0.5, 0.5, false, 2);
  put('tallPlant', 'meeting', 7.6, 6.2, 0, 0.5, 0.5, true, 0);
  put('plant', 'meeting', 14.4, 0.6, 0, 0.45, 0.45, true, 2);
  put('floorLamp', 'meeting', 14.4, 6.4, 0, 0.35, 0.35, true);

  // ── leads' corner office ──
  put('rug', 'lead', 21.5, 3.6, 0, 9.5, 3.4, false, 1);
  put('couch', 'lead', 27.1, 3.6, LEFT, 2.4, 0.9, true);
  put('tallPlant', 'lead', 15.7, 0.7, 0, 0.5, 0.5, true, 2);
  put('tallPlant', 'lead', 27.3, 0.7, 0, 0.5, 0.5, true, 0);
  put('plant', 'lead', 27.3, 6.3, 0, 0.45, 0.45, true, 1);
  put('sideTable', 'lead', 15.8, 6.2, 0, 0.5, 0.5, true);
  put('floorLamp', 'lead', 27.3, 1.6, 0, 0.35, 0.35, true);

  // ── studio extras: task board, plants, cooler, printer ──
  put('taskBoard', 'studio', 9.3, 7.35, DOWN, 6.2, 0.2, true);
  put('poster', 'studio', 0.1, 10.4, RIGHT, 1.1, 0.04, false, 0);
  put('poster', 'studio', 0.1, 12.9, RIGHT, 1.1, 0.04, false, 1);
  put('credenza', 'studio', 0.35, 11.65, RIGHT, 3.2, 0.5, true);
  put('tallPlant', 'studio', 0.6, 7.7, 0, 0.5, 0.5, true, 1);
  put('tallPlant', 'studio', 0.6, 15.8, 0, 0.5, 0.5, true, 2);
  put('cooler', 'studio', 20.7, 7.7, LEFT, 0.45, 0.45, true);
  put('printer', 'studio', 20.6, 15.6, LEFT, 0.8, 0.6, true);
  put('plant', 'studio', 18.6, 11.75, 0, 0.45, 0.45, true, 0);
  put('bench', 'studio', 18.6, 9.8, LEFT, 1.6, 0.5, true);

  // ── server room ──
  const racks: OfficeLayout['racks'] = [];
  for (const z of [10.0, 11.3, 12.6, 13.9]) {
    put('rack', 'server', 23.2, z, RIGHT, 1.2, 0.75, true);
    racks.push({ x: 23.2, z, rot: RIGHT });
    put('rack', 'server', 27.35, z, LEFT, 1.2, 0.75, true);
    racks.push({ x: 27.35, z, rot: LEFT });
  }
  put('testMonitor', 'server', 25.3, 7.25, DOWN, 2.4, 0.3, true);
  put('plant', 'server', 27.4, 15.9, 0, 0.45, 0.45, true, 1);

  // ── lounge ──
  put('counter', 'lounge', 0.4, 20.2, RIGHT, 4.4, 0.7, true);
  put('fridge', 'lounge', 0.45, 23.2, RIGHT, 0.8, 0.75, true);
  put('coffee', 'lounge', 0.35, 18.6, RIGHT, 0.4, 0.4, false);
  put('roundTable', 'lounge', 2.9, 18.4, 0, 1.1, 1.1, true);
  for (const [dx, dz] of [
    [-0.85, 0],
    [0.85, 0],
    [0, -0.85],
    [0, 0.85],
  ])
    put('stool', 'lounge', 2.9 + dx, 18.4 + dz, 0, 0.35, 0.35, false);
  // Games corner: the TV faces the sofa; foosball by the studio door (the pool table is in the lobby).
  put('sofa', 'lounge', 6.3, 21.4, LEFT, 2.2, 0.9, true);
  put('tvStand', 'lounge', 3.2, 21.4, RIGHT, 1.6, 0.45, true);
  put('rug', 'lounge', 4.75, 21.4, 0, 3.0, 2.6, false, 2);
  put('foosball', 'lounge', 6.7, 18.7, 0, 1.2, 0.7, true);
  put('tallPlant', 'lounge', 0.55, 17.0, 0, 0.5, 0.5, true, 0);
  put('floorLamp', 'lounge', 7.45, 23.3, 0, 0.35, 0.35, true);

  // ── mail room ──
  put('pigeonholes', 'mail', 9.9, 16.78, DOWN, 3.0, 0.45, true);
  put('mailbox', 'mail', 9.0, 22.6, RIGHT, 0.6, 0.6, true);
  put('sortingTable', 'mail', 11.4, 20.6, 0, 1.6, 0.8, true);
  put('plant', 'mail', 12.4, 23.3, 0, 0.45, 0.45, true, 2);

  // ── lobby ──
  put('doormat', 'lobby', DOOR_X, 23.2, 0, 1.6, 0.9, false);
  put('coatRack', 'lobby', 21.1, 23.4, 0, 0.45, 0.45, true);
  put('bench', 'lobby', 21.1, 21.6, LEFT, 1.8, 0.5, true);
  put('tallPlant', 'lobby', 13.6, 23.3, 0, 0.5, 0.5, true, 1);
  put('rug', 'lobby', 16.2, 19.6, 0, 3.4, 2.2, false, 0);
  put('armchair', 'lobby', 15.0, 19.6, Math.PI / 2, 0.85, 0.8, true, 1);
  put('armchair', 'lobby', 17.4, 19.6, -Math.PI / 2, 0.85, 0.8, true, 0);
  put('sideTable', 'lobby', 16.2, 19.1, 0, 0.5, 0.5, true);
  put('floorLamp', 'lobby', 14.1, 18.4, 0, 0.35, 0.35, true);
  put('plant', 'lobby', 20.9, 17.1, 0, 0.45, 0.45, true, 0);
  put('poolTable', 'lobby', 15.6, 22.4, 0, 2.0, 1.1, true);
  put('door', 'lobby', DOOR_X, D, DOWN, 1.5, 0.1, false);

  // ── archive ──
  const cabinets: OfficeLayout['cabinets'] = [];
  for (const x of [22.7, 23.7, 24.7, 25.7, 26.7]) {
    put('cabinet', 'archive', x, 16.95, DOWN, 0.9, 0.6, true);
    cabinets.push({ x, z: 16.95, rot: DOWN });
  }
  for (const z of [19.2, 20.2, 21.2]) {
    put('cabinet', 'archive', 27.55, z, LEFT, 0.9, 0.6, true);
    cabinets.push({ x: 27.55, z, rot: LEFT });
  }
  put('catalog', 'archive', 24.4, 21.0, 0, 1.4, 0.7, true);
  put('rug', 'archive', 24.4, 21.6, 0, 3.0, 2.4, false, 2);
  put('armchair', 'archive', 23.0, 22.9, 2.6, 0.85, 0.8, true, 0);
  put('floorLamp', 'archive', 22.15, 23.4, 0, 0.35, 0.35, true);
  put('tallPlant', 'archive', 27.4, 23.3, 0, 0.5, 0.5, true, 2);

  // ── stations (use-slots) ──
  const slot = (x: number, z: number, face: number, pose: 'stand' | 'sit' = 'stand', seat?: number): Slot => ({ x, z, face, pose, seat });
  const leadDesks = desks.filter((d) => d.lead);
  const stations: Record<StationId, Station> = {
    'library.shelf': { id: 'library.shelf', room: 'library', slots: [slot(1.5, 1.05, UP), slot(3.6, 1.05, UP), slot(5.7, 1.05, UP), slot(1.05, 2.6, LEFT)] },
    'library.chair': { id: 'library.chair', room: 'library', slots: [slot(2.7, 4.6, 2.2, 'sit', 0.36), slot(5.0, 4.7, -2.3, 'sit', 0.36)] },
    'meeting.board': { id: 'meeting.board', room: 'meeting', slots: [slot(10.0, 1.0, UP), slot(12.0, 1.0, UP)] },
    'meeting.table': { id: 'meeting.table', room: 'meeting', slots: tableSeats },
    board: { id: 'board', room: 'studio', slots: [slot(7.4, 8.3, UP), slot(9.3, 8.3, UP), slot(11.2, 8.3, UP)] },
    'server.rack': { id: 'server.rack', room: 'server', slots: [slot(24.35, 10.6, LEFT), slot(26.2, 12.0, RIGHT), slot(24.35, 13.3, LEFT)] },
    'server.monitor': { id: 'server.monitor', room: 'server', slots: [slot(24.7, 8.45, UP), slot(25.9, 8.45, UP)] },
    mail: { id: 'mail', room: 'mail', slots: [slot(9.9, 22.6, LEFT), slot(9.9, 17.75, UP)] },
    archive: { id: 'archive', room: 'archive', slots: [slot(23.7, 17.95, UP), slot(25.7, 17.95, UP), slot(26.55, 20.2, RIGHT)] },
    'lead.visit': {
      id: 'lead.visit',
      room: 'lead',
      slots: leadDesks.flatMap((d) => [slot(d.x - 0.45, d.z + 1.25, UP), slot(d.x + 0.45, d.z + 1.25, UP)]),
    },
    kitchen: { id: 'kitchen', room: 'lounge', slots: [slot(1.0, 18.6, LEFT), slot(1.15, 21.0, LEFT)] },
    'games.console': { id: 'games.console', room: 'lounge', slots: [slot(6.15, 20.9, LEFT, 'sit', 0.38), slot(6.15, 21.9, LEFT, 'sit', 0.38)] },
    'games.foosball': { id: 'games.foosball', room: 'lounge', slots: [slot(5.85, 18.7, RIGHT), slot(7.55, 18.7, LEFT)] },
    'games.pool': { id: 'games.pool', room: 'lobby', slots: [slot(15.1, 21.25, DOWN), slot(16.1, 23.5, UP)] },
    door: { id: 'door', room: 'lobby', slots: [slot(DOOR_X, 23.55, UP)] },
  };

  const layout: OfficeLayout = {
    rooms,
    walls,
    items,
    stations,
    desks,
    door: { outside: slot(DOOR_X, 23.7, UP), inside: slot(DOOR_X, 22.2, UP), x: DOOR_X, z: D },
    blocked: new Uint8Array(COLS * ROWS),
    board: { z: 7.42, x0: 6.3, x1: 12.3, y0: 0.42, y1: 1.72 },
    whiteboard: { z: 0.14, x0: 8.55, x1: 13.45, y0: 0.6, y1: 1.95 },
    testMonitor: { x: 25.3, z: 7.32, y: 1.45, w: 2.1, h: 1.05, rot: DOWN },
    racks,
    mailbox: { x: 9.0, z: 22.6, rot: RIGHT },
    pigeonholes: { x: 9.9, z: 16.78, rot: DOWN, cols: 7, rows: 4 },
    cabinets,
    catalog: { x: 24.4, z: 21.0, rot: 0 },
    shelves,
  };
  rasterize(layout);
  return layout;
}

// ───────────── walkable grid ─────────────

const INFLATE = 0.14;
const WALL_T = 0.16;

function markRect(b: Uint8Array, r: Rect, inflate: number) {
  const x0 = Math.max(0, Math.floor((r.x - inflate) / CELL + 1e-6));
  const z0 = Math.max(0, Math.floor((r.z - inflate) / CELL + 1e-6));
  const x1 = Math.min(COLS - 1, Math.ceil((r.x + r.w + inflate) / CELL - 1e-6) - 1);
  const z1 = Math.min(ROWS - 1, Math.ceil((r.z + r.d + inflate) / CELL - 1e-6) - 1);
  for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) b[z * COLS + x] = 1;
}

function clearRect(b: Uint8Array, r: Rect) {
  const x0 = Math.max(0, Math.floor(r.x / CELL + 1e-6));
  const z0 = Math.max(0, Math.floor(r.z / CELL + 1e-6));
  const x1 = Math.min(COLS - 1, Math.ceil((r.x + r.w) / CELL - 1e-6) - 1);
  const z1 = Math.min(ROWS - 1, Math.ceil((r.z + r.d) / CELL - 1e-6) - 1);
  for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) b[z * COLS + x] = 0;
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
  const b = L.blocked;
  // The outer border is never walkable.
  for (let x = 0; x < COLS; x++) {
    b[x] = 1;
    b[(ROWS - 1) * COLS + x] = 1;
  }
  for (let z = 0; z < ROWS; z++) {
    b[z * COLS] = 1;
    b[z * COLS + COLS - 1] = 1;
  }
  for (const w of L.walls) for (const r of wallPieces(w)) markRect(b, r, INFLATE * 0.6);
  for (const it of L.items) if (it.block) markRect(b, footprint(it), INFLATE);
  // Use-slots, seats and the door are always reachable cells.
  const keep = (x: number, z: number) => clearRect(b, { x: x - 0.12, z: z - 0.12, w: 0.24, d: 0.24 });
  for (const st of Object.values(L.stations))
    for (const s of st.slots) {
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
  // The doorway itself (the front wall's gap) down to the threshold.
  clearRect(b, { x: L.door.x - 0.5, z: D - 1.2, w: 1.0, d: 1.2 - CELL });
}

export function cellOf(x: number, z: number): [number, number] {
  return [Math.min(COLS - 1, Math.max(0, Math.floor(x / CELL))), Math.min(ROWS - 1, Math.max(0, Math.floor(z / CELL)))];
}

export function cellCenter(cx: number, cz: number): [number, number] {
  return [(cx + 0.5) * CELL, (cz + 0.5) * CELL];
}

export function passableFn(L: OfficeLayout): (x: number, y: number) => boolean {
  const b = L.blocked;
  return (x, y) => x >= 0 && y >= 0 && x < COLS && y < ROWS && b[y * COLS + x] === 0;
}

export function roomAt(L: OfficeLayout, x: number, z: number): RoomId | null {
  for (const r of L.rooms) if (x >= r.rect.x && x <= r.rect.x + r.rect.w && z >= r.rect.z && z <= r.rect.z + r.rect.d) return r.id;
  return null;
}
