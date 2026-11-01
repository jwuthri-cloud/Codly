// Build/Buy: the catalog matches its model files, and placement keeps the office walkable.
import { describe, expect, it } from 'vitest';
import { applyDecor, blocker, BY_ID, crowded, decorOf, FOR_SALE, FURNITURE, kitchens, locked, snap, surfaceAt, withBought, type Piece } from '../buy';
import { officeLayout, passableFn, cellOf, TEMPLATES } from '../layout';

const piece = (item: string, x: number, z: number, w = 1, d = 0.5, extra: Partial<Piece> = {}): Piece => ({ uid: `${item}@${x},${z}`, item, x, y: 0, z, rot: 0, dims: { w, d, h: 0.5 }, ...extra });

describe('Build/Buy', () => {
  it('has a model file (the office’s own pieces: a footprint) and a price for every piece', () => {
    const files = new Set(Object.keys(import.meta.glob('/public/furniture/*/*.{glb,gltf}')));
    for (const f of FURNITURE) {
      if (f.builtin) expect(f.w! > 0 && f.d! > 0, f.id).toBe(true);
      else expect(files.has(`/public/furniture/${f.src}`), f.src).toBe(true);
      expect(f.price, f.id).toBeGreaterThan(0);
    }
    expect(new Set(FURNITURE.map((f) => f.id)).size).toBe(FURNITURE.length);
  });

  it('sells the games corner, and keeps the work stations the office’s own', () => {
    const sold = FOR_SALE.filter((f) => f.builtin).map((f) => f.id);
    expect(sold.sort()).toEqual(['office.foosball', 'office.pingpong', 'office.poolTable', 'office.sofa', 'office.tvStand']);
    for (const id of ['office.desk', 'office.rack', 'office.counter', 'office.coffee']) expect(FOR_SALE.some((f) => f.id === id), id).toBe(false);
    // Comforts were already for sale as models.
    for (const id of ['kenney.loungeSofa', 'kenney.televisionModern', 'polypizza.espresso', 'polypizza.fridge', 'polypizza.waterCooler']) expect(FOR_SALE.some((f) => f.id === id), id).toBe(true);
  });

  it('locks what is not earned yet: a rank on the crew, or a kitchen for coffee', () => {
    const f = (id: string) => BY_ID.get(id)!;
    expect(locked(f('office.tvStand'), ['intern', 'junior'], true)).toBe('Someone on the crew must be a Senior');
    expect(locked(f('office.tvStand'), ['intern', 'staff'], false)).toBeNull();
    expect(locked(f('office.pingpong'), [], false)).toBe('Someone on the crew must be a Junior');
    expect(locked(f('office.foosball'), [], false)).toBeNull();
    expect(locked(f('polypizza.espresso'), ['staff'], false)).toMatch(/kitchen/);
    expect(locked(f('polypizza.espresso'), [], true)).toBeNull();
    // The Studio's lounge has a counter and a fridge; the garage nothing until one is bought.
    const studio = officeLayout(TEMPLATES.studio);
    expect([...kitchens(studio, [])]).toEqual(['lounge']);
    const G = officeLayout(TEMPLATES.garage);
    expect(kitchens(G, []).size).toBe(0);
    const fridge = piece('kenney.kitchenFridge', 8.5, 6.5, 0.7, 0.7);
    expect([...kitchens(G, [fridge])]).toEqual(['garage']);
    // Coffee goes in a kitchen.
    const cup = piece('polypizza.espresso', 7, 4, 0.3, 0.3);
    expect(blocker(G, [], cup)).toBe('Put it in the kitchen');
    expect(blocker(G, [fridge], cup)).toBeNull();
  });

  it('puts the games bought in the lot (they play there), the rest with the models', () => {
    const lot = TEMPLATES.garage;
    const foos = { uid: 'foos', item: 'office.foosball', x: 7, y: 0, z: 5.5, rot: 0 };
    const chair = { uid: 'chair', item: 'kenney.chair', x: 2, y: 0, z: 6, rot: 0 };
    expect(withBought(lot, [chair])).toBe(lot);
    const with1 = withBought(lot, [chair, foos]);
    expect(with1.furniture).toHaveLength(lot.furniture.length + 1);
    expect(officeLayout(with1).stations['games.foosball'].slots).toHaveLength(2);
    expect(decorOf([chair, foos])).toEqual([chair]);
    // The core's copy of a bought piece in the lot is the shop's: sold, it goes.
    const mirrored = { ...lot, furniture: [...lot.furniture, { ...foos, x: 3 }] };
    expect(withBought(mirrored, [foos]).furniture.filter((p) => p.uid === 'foos')).toEqual([foos]);
    expect(withBought(mirrored, []).furniture.some((p) => p.uid === 'foos')).toBe(false);
    // The Studio's own games are gifts and stay.
    expect(withBought(TEMPLATES.studio, [])).toBe(TEMPLATES.studio);
  });

  it('snaps footprints to whole cells and turns in quarter turns', () => {
    expect(snap(5.06, 20.02, 0.4, { w: 1, d: 0.5, h: 1 })).toEqual({ x: 5, z: 20, rot: 0 });
    expect(snap(5.06, 20.02, 1.4, { w: 0.75, d: 0.5, h: 1 })).toEqual({ x: 5, z: 20.125, rot: Math.PI / 2 });
  });

  it('puts pieces on free floor only, and never cuts a room off', () => {
    const L = officeLayout();
    expect(blocker(L, [], piece('kenney.loungeSofa', 19.5, 18.75))).toBeNull(); // the lobby
    const desk = L.desks[0];
    expect(blocker(L, [], piece('kenney.loungeSofa', desk.seat.x, desk.seat.z))).not.toBeNull();
    expect(blocker(L, [], piece('kenney.loungeSofa', L.door.inside.x, L.door.inside.z))).toBe('In the way of a seat or the door');
    expect(blocker(L, [], piece('kenney.loungeSofa', 50, 5))).toBe('Outside the office');
    const sofa = piece('kenney.loungeSofa', 19.5, 18.75);
    expect(blocker(L, [sofa], piece('kenney.desk', 19.75, 18.75))).toBe('Something is already there');
    // The library's door (its gap in the wall at z 7 is free from x 4.5 to 5.5).
    expect(blocker(L, [], piece('kenney.bench', 5, 7, 1, 0.5))).toBe('It would block the way');
    // Rugs never block.
    expect(blocker(L, [sofa], piece('kenney.rugRectangle', 19.5, 18.75, 2, 2))).toBeNull();
  });

  it('sets small things on a surface, which then do not block', () => {
    const L = officeLayout();
    const table = piece('kenney.table', 19.5, 18.75);
    expect(surfaceAt([table], 19.7, 18.85)).toBe(table);
    expect(surfaceAt([table], 21, 19)).toBeNull();
    const lamp = piece('kenney.lampRoundTable', 19.7, 18.85, 0.25, 0.25, { y: 0.5 });
    expect(blocker(L, [table], lamp)).toBeNull();
  });

  it('blocks the walking grid where pieces stand', () => {
    const L = officeLayout();
    const ok = passableFn(L);
    const [cx, cz] = cellOf(L, 19.5, 18.75);
    applyDecor(L, [piece('kenney.loungeSofa', 19.5, 18.75)]);
    expect(ok(cx, cz)).toBe(false);
    applyDecor(L, []);
    expect(ok(cx, cz)).toBe(true);
  });

  it('a small thing sits on the highest top under it, never inside another small thing', () => {
    const counter = piece('kenney.kitchenCabinetDrawer', 3, 3, 2, 0.7, { uid: 'item:4:0' });
    expect(surfaceAt([counter], 3.5, 3)).toBe(counter);
    expect(surfaceAt([counter], 5, 3)).toBeNull();
    const espresso = piece('polypizza.espresso', 2.5, 3, 0.3, 0.3, { y: 0.5 });
    const micro = (x: number) => piece('kenney.kitchenMicrowave', x, 3, 0.5, 0.35, { y: 0.5 });
    expect(crowded([counter, espresso], micro(2.7))).toBe(true);
    expect(crowded([counter, espresso], micro(3.5))).toBe(false);
    expect(crowded([espresso], { ...micro(2.5), y: 0 })).toBe(false); // on the floor: blocker's job
  });
});
