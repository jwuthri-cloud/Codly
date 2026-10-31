// Build/Buy: the catalog matches its model files, and placement keeps the office walkable.
import { describe, expect, it } from 'vitest';
import { applyDecor, blocker, FURNITURE, snap, surfaceAt, type Piece } from '../buy';
import { officeLayout, passableFn, cellOf } from '../layout';

const piece = (item: string, x: number, z: number, w = 1, d = 0.5, extra: Partial<Piece> = {}): Piece => ({ uid: `${item}@${x},${z}`, item, x, y: 0, z, rot: 0, dims: { w, d, h: 0.5 }, ...extra });

describe('Build/Buy', () => {
  it('has a model file (the office’s own pieces: a footprint) and a price for every piece', () => {
    const files = new Set(Object.keys(import.meta.glob('/public/furniture/*/*.{glb,gltf}')));
    for (const f of FURNITURE) {
      if (f.builtin) expect(f.hidden && f.w! > 0 && f.d! > 0, f.id).toBe(true);
      else expect(files.has(`/public/furniture/${f.src}`), f.src).toBe(true);
      expect(f.price, f.id).toBeGreaterThan(0);
    }
    expect(new Set(FURNITURE.map((f) => f.id)).size).toBe(FURNITURE.length);
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
});
