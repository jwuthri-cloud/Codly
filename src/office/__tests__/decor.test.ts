// Build/Buy: every piece bought for the office has its own spot, inside its room, clear of
// the furniture that is already there and of the spots agents use.
import { describe, expect, it } from 'vitest';
import { CATALOG } from '../../lib/util/career';
import { DECOR, footprint, officeLayout, type Rect } from '../layout';

const overlap = (a: Rect, b: Rect) => a.x < b.x + b.w && b.x < a.x + a.w && a.z < b.z + b.d && b.z < a.z + a.d;

describe('decor', () => {
  const L = officeLayout();
  it('every catalog item has a spot', () => {
    expect(CATALOG.map((c) => c.id).sort()).toEqual(Object.keys(DECOR).sort());
  });
  it('each spot is inside its room, off the furniture and the use-spots', () => {
    const slots = Object.values(L.stations).flatMap((s) => s.slots);
    for (const [id, it] of Object.entries(DECOR)) {
      const f = footprint(it);
      const room = L.rooms.find((r) => r.id === it.room)!.rect;
      expect(f.x >= room.x && f.z >= room.z && f.x + f.w <= room.x + room.w && f.z + f.d <= room.z + room.d, `${id} in ${it.room}`).toBe(true);
      for (const o of L.items) if (o.type !== 'rug' && o.type !== 'doormat') expect(overlap(f, footprint(o)), `${id} hits ${o.type} at ${o.x},${o.z}`).toBe(false);
      for (const s of slots) expect(overlap(f, { x: s.x - 0.3, z: s.z - 0.3, w: 0.6, d: 0.6 }), `${id} on a use-spot ${s.x},${s.z}`).toBe(false);
      for (const [other, o] of Object.entries(DECOR)) if (other !== id) expect(overlap(f, footprint(o)), `${id} hits ${other}`).toBe(false);
    }
  });
});
