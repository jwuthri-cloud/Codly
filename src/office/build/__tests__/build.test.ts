// Build mode's pure parts, on the Garage (10 × 8: the Lead's desk and two desks, a server by the
// east wall, a bookshelf by the west wall, a mailbox in the south-east corner, a roll-up door in
// the south wall from x 5.2 to 7.8, and a window in the west wall from z 4.8 to 6.4).
import { describe, expect, it } from 'vitest';
import type { Lot } from '../../../lib/gen/Lot';
import type { Placed } from '../../../lib/gen/Placed';
import { officeLayout, TEMPLATES } from '../../layout';
import { checklist, guard } from '../checklist';
import { costOf } from '../cost';
import { edgeMap, isH, lenOf, tidy, type BLot } from '../grid';
import { History } from '../history';
import { forgetRoom, saveRoom, savedRooms, type Shelf } from '../library';
import {
  blueprint,
  drawRoom,
  drawWall,
  failed,
  fromStorage,
  knockDown,
  moveRoom,
  paintFloor,
  paintRoom,
  paintSide,
  placeBlueprint,
  placeOpening,
  pushPull,
  removeOpening,
  removeRoom,
  roomBox,
  snapRect,
  snapWall,
  floorGrid,
  type Edit,
  type Outcome,
} from '../tools';

const garage = () => tidy(TEMPLATES.garage);
const ok = (o: Outcome): Edit => {
  if (failed(o)) throw new Error(o.error);
  return o;
};
const why = (o: Outcome) => (failed(o) ? o.error : null);
/** The room whose tiles include (x, z). */
const roomAt = (lot: Lot, x: number, z: number) => {
  const L = officeLayout(lot);
  return { L, k: L.roomOf[Math.floor(z / 0.5) * L.tiles.cols + Math.floor(x / 0.5)] };
};
const wallOn = (lot: Lot, x: number, z: number, o: 'h' | 'v') =>
  lot.walls.find((w) => (o === 'h' ? isH(w) && w.z0 === z && x >= w.x0 && x <= w.x1 : !isH(w) && w.x0 === x && z >= w.z0 && z <= w.z1))!;
/** A small room in the south-west corner: 3 × 1.5 m against the west and south walls. */
const corner = (lot: BLot = garage(), bought: Placed[] = []) => ok(drawRoom(lot, bought, 0, 6.5, 3, 8, 'solid', 'oak'));

describe('tool geometry', () => {
  it('snaps a wall drag to the grid, along the axis it moves most', () => {
    expect(snapWall([1.2, 2.1], [4.7, 2.4], 1)).toEqual([1, 2, 5, 2]);
    expect(snapWall([1.2, 2.1], [4.7, 2.4], 0.5)).toEqual([1, 2, 4.5, 2]);
    expect(snapWall([3.1, 6], [2.8, 1.3], 1)).toEqual([3, 1, 3, 6]);
    expect(snapWall([3.1, 6], [3.2, 6.2], 1)).toBeNull();
    expect(snapRect([2.6, 7.9], [0.2, 6.4])).toEqual([0, 6.5, 2.5, 8]);
  });

  it('a room rectangle adds only the walls that are missing, and its floor', () => {
    const before = garage();
    const { lot } = corner(before);
    // The west and south sides are the garage's own walls: two new walls, 3 m and 1.5 m.
    const added = lot.walls.filter((w) => !before.walls.some((b) => b.id === w.id));
    expect(added.map(lenOf).sort()).toEqual([1.5, 3]);
    expect(added.every((w) => w.kind === 'solid')).toBe(true);
    expect(lot.floors.rects).toEqual([{ x: 0, z: 6.5, w: 3, d: 1.5, m: 'oak' }]);
    const L = officeLayout(lot);
    expect(L.rooms).toHaveLength(2);
    // Again: nothing new to build.
    expect(why(drawRoom(lot, [], 0, 6.5, 3, 8, 'solid', 'oak'))).toBe('That room is there already');
  });

  it('refuses walls into a window or a doorway, through furniture, or off the lot', () => {
    // From the west wall at z 5.5: inside the window (4.8–6.4).
    expect(why(drawRoom(garage(), [], 0, 5.5, 3, 8, 'solid', null))).toBe('That wall would run into a door or window');
    // Down to the south wall at x 6: inside the roll-up door.
    expect(why(drawWall(garage(), [], 6, 6, 6, 8, 'solid'))).toBe('That wall would run into a door or window');
    // Through a desk (x 2.7–4, z 4.4–5.06).
    expect(why(drawWall(garage(), [], 3, 3, 3, 6, 'glass'))).toBe('There’s furniture in the way');
    expect(why(drawRoom(garage(), [], 8, 6, 11, 8, 'solid', null))).toBe('Keep it on the lot');
    expect(why(drawRoom(garage(), [], 1, 1, 1.5, 3, 'solid', null))).toBe('A room is at least 1 × 1 m');
  });

  it('a wall continuing one of the same kind lengthens it', () => {
    const a = ok(drawWall(garage(), [], 1, 6.5, 2, 6.5, 'half')).lot;
    const b = ok(drawWall(a, [], 2, 6.5, 3, 6.5, 'half')).lot;
    expect(b.walls).toHaveLength(a.walls.length);
    expect(wallOn(b, 1.5, 6.5, 'h')).toMatchObject({ x0: 1, x1: 3 });
  });

  it('places doors and windows where they fit', () => {
    const { lot } = corner();
    const north = wallOn(lot, 1.5, 6.5, 'h');
    const d = ok(placeOpening(lot, [], north.id, 1.6, 6.5, { door: 'single' })).lot;
    expect(d.doors.find((x) => x.wall === north.id)).toMatchObject({ at: 1.5, width: 1, kind: 'single' });
    // Clicking the door again with another kind changes it.
    const dd = ok(placeOpening(d, [], north.id, 1.4, 6.5, { door: 'double' })).lot;
    expect(dd.doors.filter((x) => x.wall === north.id)).toEqual([expect.objectContaining({ kind: 'double', width: 1.8 })]);
    // Windows: outside walls only.
    expect(why(placeOpening(lot, [], north.id, 1.5, 6.5, { window: 1 }))).toBe('Windows go in outside walls');
    expect(ok(placeOpening(lot, [], 'w1', 2, 0, { window: 1.6 })).lot.windows).toHaveLength(2);
    // Not over the roll-up door, not where a wall meets this one (x 3 on the south wall).
    expect(why(placeOpening(lot, [], 'w3', 5, 8, { door: 'single' }))).toBe('There’s a door or window there');
    expect(why(placeOpening(lot, [], 'w3', 3, 8, { door: 'single' }))).toBe('Another wall meets this one there');
    // Too wide for a short wall: it slides to fit, but not past the ends.
    const east = wallOn(lot, 3, 7, 'v');
    expect(ok(placeOpening(lot, [], east.id, 3, 6.6, { door: 'single' })).lot.doors.find((x) => x.wall === east.id)!.at).toBe(0.5);
    expect(why(placeOpening(lot, [], east.id, 3, 7, { door: 'rollup' }))).toBe('It doesn’t fit in this wall');
  });
});

describe('sledgehammer', () => {
  it('knocks down the stretch of wall between the walls that meet it', () => {
    const { lot } = corner();
    // The garage's south wall, west of the room's east wall: 0 to 3 goes, the rest stays (with its door).
    const out = ok(knockDown(lot, [], 'w3', 1, 8)).lot;
    const south = out.walls.filter((w) => isH(w) && w.z0 === 8);
    expect(south).toEqual([expect.objectContaining({ id: 'w3', x0: 3, x1: 10 })]);
    expect(out.doors.find((d) => d.id === 'd1')).toMatchObject({ wall: 'w3', at: 3.5 });
  });

  it('removes a room inside another by merging it back, and doors and windows on their own', () => {
    const { lot } = corner();
    const { L, k } = roomAt(lot, 1, 7);
    const out = ok(removeRoom(lot, [], L, k));
    expect(out.lot.walls.map((w) => w.id).sort()).toEqual(['w1', 'w2', 'w3', 'w4']);
    expect(officeLayout(out.lot).rooms).toHaveLength(1);
    expect(ok(removeOpening(lot, [], 'win1')).lot.windows).toHaveLength(0);
  });
});

describe('push / pull', () => {
  it('moves a side: the room grows, its door goes along, the side keeps its id', () => {
    const base = corner().lot;
    const east = wallOn(base, 3, 7, 'v');
    const lot = ok(placeOpening(base, [], east.id, 3, 7.25, { door: 'single' })).lot;
    const { L, k } = roomAt(lot, 1, 7);
    const out = ok(pushPull(lot, [], L, k, 'e', 2)).lot;
    const moved = out.walls.find((w) => w.id === east.id)!;
    expect(moved).toMatchObject({ x0: 4, x1: 4, z0: 6.5, z1: 8 });
    expect(out.doors.find((d) => d.wall === east.id)).toMatchObject({ at: 0.75, kind: 'single' });
    const after = roomAt(out, 1, 7);
    expect(roomBox(after.L, after.k)).toMatchObject({ c0: 0, c1: 8, r0: 13, r1: 16 });
    // It costs the metre the north wall grew.
    expect(costOf(lot, out).net).toBe(10);
  });

  it('what no longer fits goes to storage', () => {
    const bench: Placed = { uid: 'b1', item: 'office.bench', x: 2.2, y: 0, z: 7.5, rot: 0 };
    const lot = { ...corner().lot };
    lot.furniture = [...lot.furniture, bench];
    const { L, k } = roomAt(lot, 1, 7);
    const out = ok(pushPull(lot, [], L, k, 'e', -1));
    expect(out.lot.furniture.some((p) => p.uid === 'b1')).toBe(false);
    expect(out.lot.storage).toEqual([bench]);
    expect(out.note).toMatch(/storage/);
    // A piece bought through Buy is sold back instead.
    const shop = ok(pushPull(corner().lot, [bench], L, k, 'e', -1));
    expect(shop.bought).toEqual([]);
    // Out of storage again, somewhere it fits.
    const back = ok(fromStorage(out.lot, [], 'b1', 1.5, 3, 0)).lot;
    expect(back.storage).toEqual([]);
    expect(back.furniture.find((p) => p.uid === 'b1')).toMatchObject({ x: 1.5, z: 3 });
  });

  it('only rectangular rooms, at least a metre, on the lot', () => {
    const lot = corner().lot;
    const { L, k } = roomAt(lot, 1, 7);
    expect(why(pushPull(lot, [], L, k, 'n', -2))).toBe('A room is at least 1 m across');
    expect(why(pushPull(lot, [], L, k, 's', 1))).toBe('Keep it on the lot');
  });
});

describe('move room', () => {
  it('picks a room up with its door and floor and puts it down elsewhere', () => {
    const base = corner().lot;
    const north = wallOn(base, 1.5, 6.5, 'h');
    const lot = ok(placeOpening(base, [], north.id, 1.5, 6.5, { door: 'single' })).lot;
    const { L, k } = roomAt(lot, 1, 7);
    const out = ok(moveRoom(lot, [], L, k, 2, 0, 0)).lot;
    const after = roomAt(out, 2, 7);
    expect(roomBox(after.L, after.k)).toMatchObject({ c0: 2, c1: 8, r0: 13, r1: 16 });
    // The door is the same door, a metre east; the floor came along as the same rect.
    const door = out.doors.find((d) => d.kind === 'single')!;
    const w = out.walls.find((x) => x.id === door.wall)!;
    expect(w.x0 + door.at).toBe(2.5);
    expect(floorGrid(out)[Math.floor(7 / 0.5) * 20 + 2]).toBe('oak');
    expect(out.floors.rects).toEqual([{ x: 1, z: 6.5, w: 3, d: 1.5, m: 'oak' }]);
    // The walls kept their ids; it costs only the new west wall it needs now (1.5 m).
    expect(costOf(lot, out).net).toBe(15);
  });

  it('turns a room in place', () => {
    const lot = ok(drawRoom(garage(), [], 6, 2.5, 8, 3.5, 'solid', null)).lot;
    const { L, k } = roomAt(lot, 7, 3);
    const out = ok(moveRoom(lot, [], L, k, 0, 0, 1)).lot;
    const after = roomAt(out, 7, 3);
    const b = roomBox(after.L, after.k);
    expect([b.c1 - b.c0, b.r1 - b.r0]).toEqual([2, 4]);
  });

  it('won’t land on furniture or off the lot', () => {
    const lot = corner().lot;
    const { L, k } = roomAt(lot, 1, 7);
    expect(why(moveRoom(lot, [], L, k, 6, -3, 0))).toBe('Something is in the way');
    expect(why(moveRoom(lot, [], L, k, -2, 0, 0))).toBe('Keep it on the lot');
  });
});

describe('paint', () => {
  it('floors: a whole room or a tile; paint: one side or a room’s inside', () => {
    const lot = corner().lot;
    const { L, k } = roomAt(lot, 1, 7);
    const tile = ok(paintFloor(lot, [], [L.roomOf.indexOf(k)], 'terrazzo')).lot;
    expect(tile.floors.rects.at(-1)).toMatchObject({ w: 0.5, d: 0.5, m: 'terrazzo' });
    expect(why(paintFloor(tile, [], [L.roomOf.indexOf(k)], 'terrazzo'))).toBe('It’s that floor already');
    const side = ok(paintSide(lot, [], 'w1', 1, '#b9cdb0')).lot;
    expect(side.paint).toEqual([{ wall: 'w1', side: 1, paint: '#b9cdb0' }]);
    const inside = ok(paintRoom(lot, [], L, k, 'paper:stripe-sage')).lot;
    // Four walls, each painted on the side facing in: west +x, south −z, north +z, east −x.
    expect(inside.paint.map((p) => `${p.wall === 'w2' || p.wall === 'w3' ? p.wall : 'new'}:${p.side}`).sort()).toEqual(['new:-1', 'new:1', 'w2:1', 'w3:-1']);
  });
});

describe('the checklist', () => {
  it('the Garage has everything, within reach of its door', () => {
    const c = checklist(officeLayout(garage()));
    expect(c.every((x) => x.have && x.reached)).toBe(true);
  });

  it('refuses a wall that cuts the server and the mailbox off', () => {
    const lot = garage();
    const out = ok(drawWall(lot, [], 8, 0, 8, 8, 'solid')).lot;
    expect(guard(checklist(officeLayout(lot)), checklist(officeLayout(out)))).toBe('That would cut the server off from the front door.');
  });

  it('refuses taking away the last server, or the front door', () => {
    const lot = garage();
    const { L, k } = roomAt(lot, 5, 4);
    const out = ok(pushPull(lot, [], L, k, 'e', -2)).lot;
    expect(out.storage?.map((p) => p.item)).toContain('office.serverMachine');
    expect(guard(checklist(officeLayout(lot)), checklist(officeLayout(out)))).toMatch(/needs a server/);
    const shut = ok(removeOpening(lot, [], 'd1')).lot;
    expect(guard(checklist(officeLayout(lot)), checklist(officeLayout(shut)))).toBe('Every office needs a front door in an outside wall.');
    // What was missing already doesn't block: with no door yet, any edit goes.
    expect(guard(checklist(officeLayout(shut)), checklist(officeLayout(ok(paintSide(shut, [], 'w1', 1, '#e8d6b8')).lot)))).toBeNull();
  });
});

describe('undo / redo', () => {
  it('steps back and forward through committed edits', () => {
    const h = new History('a');
    h.push('b');
    h.push('c');
    expect([h.undo(), h.undo(), h.undo()]).toEqual(['b', 'a', null]);
    expect(h.redo()).toBe('b');
    h.push('d');
    expect([h.canRedo, h.now]).toEqual([false, 'd']);
    h.reset('x');
    expect([h.canUndo, h.canRedo, h.now]).toEqual([false, false, 'x']);
  });
});

describe('cost previews', () => {
  it('prices building at the shared table, and selling back at its rate (gifts for nothing)', () => {
    const lot = garage();
    const room = corner(lot).lot;
    // 4.5 m of solid wall (🪙 10/m), and 4.5 m² of oak (🪙 1/m²).
    expect(costOf(lot, room)).toEqual({ net: 50, lines: [{ what: 'walls', coins: 45 }, { what: 'floors', coins: 5 }] });
    const north = wallOn(room, 1.5, 6.5, 'h');
    const door = ok(placeOpening(room, [], north.id, 1.5, 6.5, { door: 'single' })).lot;
    expect(costOf(room, door).net).toBe(40);
    expect(costOf(door, ok(removeOpening(door, [], door.doors.at(-1)!.id)).lot).net).toBe(-20);
    // The garage came free: knocking its wall down gives nothing back.
    const gift = { ...lot, gift: TEMPLATES.garage.gift };
    expect(costOf(gift, ok(removeOpening(gift, [], 'win1')).lot).net).toBe(0);
    // Paint: wall area at 🪙 1/m² (the north wall: 10 m × 2.3 m).
    expect(costOf(lot, ok(paintSide(lot, [], 'w1', 1, '#e8d6b8')).lot).net).toBe(23);
  });
});

describe('the room library', () => {
  it('saves a room and puts it down again elsewhere, priced like building it', () => {
    const mem = new Map<string, string>();
    const shelf: Shelf = { getItem: (k) => mem.get(k) ?? null, setItem: (k, v) => void mem.set(k, v) };
    const lot = corner().lot;
    const { L, k } = roomAt(lot, 1, 7);
    saveRoom('Phone booth', blueprint(lot, [], L, k), shelf);
    const [saved] = savedRooms(shelf);
    expect(saved.name).toBe('Phone booth');
    // Free ground: a bigger lot, the garage on its west half.
    const big: BLot = { ...lot, size: { w: 16, d: 8 } };
    const out = ok(placeBlueprint(big, [], officeLayout(big), saved.bp, 24, 4, 0)).lot;
    const added = out.walls.filter((w) => !big.walls.some((b) => b.id === w.id));
    expect(added.reduce((s, w) => s + lenOf(w), 0)).toBe(9);
    expect(costOf(big, out).net).toBe(90 + 5);
    expect(new Set(edgeMap(out).keys()).size).toBe(out.walls.reduce((s, w) => s + lenOf(w) / 0.5, 0));
    forgetRoom(saved.id, shelf);
    expect(savedRooms(shelf)).toEqual([]);
  });
});
