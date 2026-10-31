// Lots as data: the Studio from its JSON is the office it always was, the Garage and the Small
// office have what the work needs within reach of their door, and rooms are found from walls.
import { describe, expect, it } from 'vitest';
import type { Lot } from '../../lib/gen/Lot';
import type { Placed } from '../../lib/gen/Placed';
import BEHAVIOR from '../behavior.ts?raw';
import { aStar } from '../astar';
import { cellOf, footprint, officeLayout, passableFn, roomAt, STATION_IDS, TEMPLATES, wallPieces, type OfficeLayout, type StationId } from '../layout';

const reachable = (L: OfficeLayout, x: number, z: number) => {
  const pass = passableFn(L);
  const [sx, sz] = cellOf(L, L.door.inside.x, L.door.inside.z);
  const [gx, gz] = cellOf(L, x, z);
  return aStar(L.cols, L.rows, sx, sz, gx, gz, pass) !== null;
};

/** Station ids the behaviour layer sends agents to. */
const USED = [...new Set([...BEHAVIOR.matchAll(/'((?:library|nap|meeting|server|games|lead)\.[a-z]+|board|mail|archive|kitchen)'/g)].map((m) => m[1]))] as StationId[];

describe('the Studio, from studio.json', () => {
  const L = officeLayout(TEMPLATES.studio);

  it('has the desks it always had: three Leads’ desks, then six pods of four', () => {
    expect(L.desks).toHaveLength(27);
    expect(L.desks.filter((d) => d.lead).map((d) => d.index)).toEqual([0, 1, 2]);
    const d0 = L.desks[0];
    expect([d0.x, d0.z, d0.seat.x, d0.seat.z, d0.room]).toEqual([18, 3, 18, 2.24, 'lead']);
    // Helpers stand on the outside of each pod, and away from the middle of the Leads' office.
    expect(L.desks[0].intern.x).toBeLessThan(18);
    expect(L.desks[2].intern.x).toBeGreaterThan(25);
    const [a, b] = [L.desks[3], L.desks[4]];
    expect(Math.abs(a.intern.x - b.intern.x)).toBeGreaterThan(Math.abs(a.x - b.x));
    expect(L.items.filter((i) => i.type === 'chair')).toHaveLength(27 + 6);
  });

  it('has every station the behaviour layer uses, with its slots', () => {
    expect(USED.length).toBeGreaterThan(10);
    for (const id of USED) expect(L.stations[id].slots.length, id).toBeGreaterThan(0);
    const n = Object.fromEntries(STATION_IDS.filter((id) => id !== 'wait').map((id) => [id, L.stations[id].slots.length]));
    expect(n).toEqual({
      'library.shelf': 4,
      'library.chair': 2,
      'nap.armchair': 1,
      'meeting.board': 2,
      'meeting.table': 6,
      board: 3,
      // One spot per rack and per cabinet now (were 3 and 3).
      'server.rack': 8,
      'server.monitor': 2,
      mail: 2,
      archive: 8,
      'lead.visit': 6,
      kitchen: 2,
      'games.console': 2,
      'games.foosball': 2,
      'games.pingpong': 2,
      'games.pool': 2,
      door: 1,
    });
    expect(L.stations['library.shelf'].room).toBe('library');
    expect(L.stations['server.rack'].room).toBe('server');
    // Where agents without a desk wait: open floor in the lobby.
    expect(L.stations.wait.slots.length).toBeGreaterThan(3);
    for (const s of L.stations.wait.slots) expect(roomAt(L, s.x, s.z)).toBe('lobby');
  });

  it('finds its nine rooms from the walls and names them by what is inside', () => {
    expect(L.rooms.map((r) => `${r.id}:${r.name}`)).toEqual([
      'library:Library',
      'meeting:Meeting room',
      'lead:Leads’ office',
      'studio:Studio',
      'server:Server room',
      'lounge:Lounge',
      'mail:Mail room',
      'lobby:Lobby',
      'archive:Memory archive',
    ]);
    expect(L.rooms.find((r) => r.id === 'studio')!.rect).toEqual({ x: 0, z: 7, w: 21.5, d: 9.5 });
    expect(roomAt(L, 18.2, 22)).toBe('lobby');
  });

  it('keeps what the props hang on where it was', () => {
    expect(L.door).toMatchObject({ x: 18.2, z: 24, nx: 0, nz: -1, kind: 'single' });
    for (const [k, v] of Object.entries({ z: 7.42, x0: 6.3, x1: 12.3, y0: 0.42, y1: 1.72 })) expect(L.board![k as 'z'], k).toBeCloseTo(v);
    expect(L.whiteboard).toEqual({ z: 0.14, x0: 8.55, x1: 13.45, y0: 0.6, y1: 1.95 });
    expect(L.testMonitor).toMatchObject({ x: 25.3, y: 1.45, w: 2.1, h: 1.05 });
    expect(L.testMonitor!.z).toBeCloseTo(7.32);
    expect(L.mailbox).toEqual({ x: 9, z: 22.6, rot: Math.PI / 2 });
    expect([L.racks.length, L.cabinets.length, L.shelves.length]).toEqual([8, 8, 4]);
    expect(L.clock).toEqual({ x: 0.1, y: 1.55, z: 11.65, rot: Math.PI / 2 });
    expect(L.floors.map((f) => f.m)).toEqual(['carpet-sage', 'carpet-blue', 'walnut', 'oak', 'tile-cool', 'tile-warm', 'oak', 'terrazzo', 'walnut-planks']);
  });
});

describe('the Garage', () => {
  const L = officeLayout(TEMPLATES.garage);

  it('is one room behind a roll-up door', () => {
    expect([L.w, L.d]).toEqual([10, 8]);
    expect(L.rooms.map((r) => r.name)).toEqual(['Garage']);
    expect(L.door).toMatchObject({ kind: 'rollup', z: 8, nz: -1 });
    expect(L.floors).toHaveLength(1);
    expect(L.floors[0].m).toBe('concrete');
    expect([L.board, L.whiteboard, L.pigeonholes, L.catalog, L.clock]).toEqual([null, null, null, null, null]);
  });

  it('has everything the work needs, each reachable from the door', () => {
    expect(L.desks.filter((d) => d.lead)).toHaveLength(1);
    expect(L.desks.filter((d) => !d.lead)).toHaveLength(2);
    const need: [string, { x: number; z: number }[]][] = [
      ['front door', [L.door.inside, ...L.stations.door.slots]],
      ['lead desk', L.desks.filter((d) => d.lead).flatMap((d) => [d.seat, d.stand])],
      ['desks', L.desks.filter((d) => !d.lead).flatMap((d) => [d.seat, d.stand, d.intern])],
      ['server', [...L.stations['server.rack'].slots, ...L.stations['server.monitor'].slots]],
      ['bookshelf', L.stations['library.shelf'].slots],
      ['mailbox', L.stations.mail.slots],
      ['memory (on the bookshelf)', L.stations.archive.slots],
      ['the Lead’s visitors', L.stations['lead.visit'].slots],
      ['agents without a desk', L.stations.wait.slots],
    ];
    for (const [what, spots] of need) {
      expect(spots.length, what).toBeGreaterThan(0);
      for (const s of spots) expect(reachable(L, s.x, s.z), `${what} at ${s.x.toFixed(2)},${s.z.toFixed(2)}`).toBe(true);
    }
    expect(L.testMonitor).not.toBeNull();
    expect(L.racks).toHaveLength(1);
  });

  it('has no games, so breaks are spent at the desk', () => {
    for (const id of ['games.console', 'games.foosball', 'games.pingpong', 'games.pool', 'kitchen', 'meeting.table'] as const) expect(L.stations[id].slots, id).toHaveLength(0);
  });
});

describe('the Small office', () => {
  const lot = TEMPLATES['small-office'];
  const L = officeLayout(lot);

  it('is a work floor and a back room on a medium lot, behind a front door', () => {
    expect([L.w, L.d]).toEqual([16, 12]);
    expect(L.rooms.map((r) => r.name)).toEqual(['Studio', 'Server room']);
    expect(roomAt(L, 13.5, 3)).toBe('server');
    expect(L.door).toMatchObject({ kind: 'single', x: 5.5, z: 12, nz: -1 });
    // Windows on the outside walls only; everything came with it.
    const outside = new Set(lot.walls.filter((w) => w.kind === 'exterior').map((w) => w.id));
    expect(lot.windows.length).toBeGreaterThan(3);
    for (const w of lot.windows) expect(outside.has(w.wall), w.id).toBe(true);
    const ids = [...lot.walls, ...lot.doors, ...lot.windows].map((x) => x.id).concat(lot.furniture.map((p) => p.uid));
    expect([...lot.gift].sort()).toEqual([...ids].sort());
  });

  it('has everything the work needs, each reachable from the door', () => {
    expect(L.desks.filter((d) => d.lead)).toHaveLength(1);
    expect(L.desks.filter((d) => !d.lead)).toHaveLength(6);
    const need: [string, { x: number; z: number }[]][] = [
      ['front door', [L.door.inside, ...L.stations.door.slots]],
      ['lead desk', L.desks.filter((d) => d.lead).flatMap((d) => [d.seat, d.stand])],
      ['desks', L.desks.filter((d) => !d.lead).flatMap((d) => [d.seat, d.stand, d.intern])],
      ['server rack', L.stations['server.rack'].slots],
      ['test monitor', L.stations['server.monitor'].slots],
      ['bookshelves', L.stations['library.shelf'].slots],
      ['mailbox', L.stations.mail.slots],
      ['archive cabinet', L.stations.archive.slots],
      ['the Lead’s visitors', L.stations['lead.visit'].slots],
      ['agents without a desk', L.stations.wait.slots],
    ];
    for (const [what, spots] of need) {
      expect(spots.length, what).toBeGreaterThan(0);
      for (const s of spots) expect(reachable(L, s.x, s.z), `${what} at ${s.x.toFixed(2)},${s.z.toFixed(2)}`).toBe(true);
    }
    expect([L.racks.length, L.shelves.length, L.cabinets.length]).toEqual([1, 2, 1]);
    expect(L.testMonitor).not.toBeNull();
    for (const id of ['server.rack', 'server.monitor', 'library.shelf', 'mail', 'archive'] as const) expect(L.stations[id].room, id).toBe('server');
  });

  it('leaves the kitchen corner empty and has no games: those are bought', () => {
    expect(L.items.filter((i) => ['counter', 'fridge', 'coffee', 'cooler'].includes(i.type))).toEqual([]);
    for (const id of ['kitchen', 'games.console', 'games.foosball', 'games.pingpong', 'games.pool'] as const) expect(L.stations[id].slots, id).toHaveLength(0);
    // The corner (tiles, by the east window) is open floor.
    expect(L.floors.find((f) => f.m === 'tile-warm')!.rect).toEqual({ x: 11, z: 7, w: 5, d: 5 });
    for (const [x, z] of [[12, 8], [15, 11], [13.5, 9.5]]) expect(reachable(L, x, z), `${x},${z}`).toBe(true);
  });

  it('has no furniture over other furniture or a wall', () => {
    const solid = L.items.filter((i) => i.block).map((i) => ({ i, r: footprint(i) }));
    const over = (a: { x: number; z: number; w: number; d: number }, b: typeof a) => a.x < b.x + b.w - 1e-6 && b.x < a.x + a.w - 1e-6 && a.z < b.z + b.d - 1e-6 && b.z < a.z + a.d - 1e-6;
    for (const [k, a] of solid.entries()) {
      for (const b of solid.slice(k + 1)) expect(over(a.r, b.r), `${a.i.type} ${a.i.uid} and ${b.i.type} ${b.i.uid}`).toBe(false);
      for (const w of L.walls) for (const p of wallPieces(w)) expect(over(a.r, p), `${a.i.type} ${a.i.uid} and wall ${w.id}`).toBe(false);
    }
  });
});

describe('rooms from walls', () => {
  const piece = (uid: string, item: string, x: number, z: number, rot = 0): Placed => ({ uid, item, x, y: 0, z, rot });
  // 12 × 6: west room (shelves) | wall with a door | east room (a rack), the east room split
  // again by a wall that stops short (so it is one L-shaped room with its nook)…
  const lot: Lot = {
    v: 1,
    template: '',
    size: { w: 12, d: 6 },
    walls: [
      { id: 'n', x0: 0, z0: 0, x1: 12, z1: 0, kind: 'exterior' },
      { id: 'w', x0: 0, z0: 0, x1: 0, z1: 6, kind: 'exterior' },
      { id: 's', x0: 0, z0: 6, x1: 12, z1: 6, kind: 'half' },
      { id: 'e', x0: 12, z0: 0, x1: 12, z1: 6, kind: 'half' },
      { id: 'mid', x0: 5, z0: 0, x1: 5, z1: 6, kind: 'solid' },
      { id: 'stub', x0: 9, z0: 0, x1: 9, z1: 4, kind: 'glass' },
    ],
    doors: [
      { id: 'front', wall: 's', at: 2.5, width: 1.6, kind: 'single' },
      { id: 'between', wall: 'mid', at: 3, width: 1.2, kind: 'open' },
    ],
    windows: [],
    floors: { tile: 0.5, default: 'oak', rects: [{ x: 5, z: 0, w: 7, d: 6, m: 'tile-cool' }, { x: 9, z: 0, w: 3, d: 2, m: 'concrete' }] },
    paint: [],
    labels: [{ x: 10.5, z: 1, name: 'Nook' }],
    furniture: [piece('a', 'office.shelf', 2.5, 0.3), piece('b', 'office.rack', 7, 1, 0), piece('c', 'office.leadDesk', 2.5, 3)],
    gift: [],
  };
  const L = officeLayout(lot);

  it('finds each closed area (a door closes it too), names it by what is in it', () => {
    // The shelves and the Lead's desk share the west room: the desk wins (Leads' office).
    expect(L.rooms.map((r) => [r.id, r.name, r.rect])).toEqual([
      ['lead', 'Leads’ office', { x: 0, z: 0, w: 5, d: 6 }],
      ['server', 'Nook', { x: 5, z: 0, w: 7, d: 6 }],
    ]);
    expect(L.rooms[1].label).toEqual([10.5, 1]);
    expect(roomAt(L, 10.5, 1)).toBe('server');
    expect(roomAt(L, 7, 5)).toBe('server');
    expect(L.items.find((i) => i.type === 'rack')!.room).toBe('server');
  });

  it('paints floors per tile, one region per material between walls', () => {
    expect(L.floors.map((f) => [f.m, f.tiles.length])).toEqual([
      ['oak', 10 * 12],
      ['tile-cool', 14 * 12 - 6 * 4],
      ['concrete', 6 * 4],
    ]);
  });

  it('leaves an area open to the outside out of every room', () => {
    const open = officeLayout({ ...lot, walls: lot.walls.filter((w) => w.id !== 'e') });
    expect(open.rooms.map((r) => r.id)).toEqual(['lead']);
    expect(roomAt(open, 10, 3)).toBeNull();
  });
});
