import { describe, expect, it } from 'vitest';
import type { Lot } from '../../lib/gen/Lot';
import type { LotSizeName } from '../../lib/gen/LotSizeName';
import { diff, fit, landBack, landPrice, readyMadePrice, sellValue, sizeFor, TEMPLATE_NAMES, TEMPLATES, value } from '../price';
import FIXTURE from './price-fixture.json';

// The core writes this fixture from its own prices (price.rs, `the_office_prices_lots_the_same`):
// the office must agree to the coin.
const F = FIXTURE as unknown as {
  templates: Record<string, { value: number; sell_gift: number; sell_bought: number; ready_made: Record<LotSizeName, number | null> }>;
  diffs: { name: string; old: Lot; new: Lot; cost: number }[];
};

describe('build mode prices', () => {
  it('knows the same designs as the core', () => {
    expect(TEMPLATE_NAMES.sort()).toEqual(Object.keys(F.templates).sort());
  });

  it('prices every design as the core does', () => {
    for (const [name, want] of Object.entries(F.templates)) {
      const t = TEMPLATES[name];
      expect(value(t), name).toBe(want.value);
      expect(sellValue(t), name).toBe(want.sell_gift);
      expect(sellValue({ ...t, gift: [] }), name).toBe(want.sell_bought);
      for (const size of ['small', 'medium', 'large'] as LotSizeName[]) expect(readyMadePrice(size, name), `${name} on ${size}`).toBe(want.ready_made[size]);
    }
  });

  it('prices every change as the core does', () => {
    for (const c of F.diffs) expect(diff(c.old, c.new), c.name).toBe(c.cost);
  });

  it('never sells for more than it cost', () => {
    for (const name of TEMPLATE_NAMES) {
      const t = TEMPLATES[name];
      const size = sizeFor(t.size.w, t.size.d)!;
      expect(landBack(size) + sellValue({ ...t, gift: [] })).toBeLessThanOrEqual(readyMadePrice(size, name)!);
    }
    expect(readyMadePrice('medium', null)).toBe(landPrice('medium'));
    expect(readyMadePrice('small', 'studio')).toBeNull();
    const g = fit(TEMPLATES.garage, 16, 12)!;
    expect(g.walls.find((w) => w.id === 'w3')).toMatchObject({ x0: 3, z0: 12, x1: 13, z1: 12 });
  });
});
