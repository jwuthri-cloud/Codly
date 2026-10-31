import { describe, expect, it } from 'vitest';
import type { LotSlot } from '../../lib/gen/LotSlot';
import { plotsOf } from '../map';
import { townOf } from '../town';

const slot = (id: string, x: number, z: number, size: LotSlot['size']): LotSlot => ({
  id,
  x,
  z,
  size,
  owned: false,
  workspace_id: null,
  name: null,
  crew: 0,
  price: 0,
  template: null,
  gift: false,
  sells_for: 0,
});

describe('the town around the plots', () => {
  const plots = plotsOf([slot('a', 0, 0, 'large'), slot('b', 1, 0, 'small'), slot('c', 0, 1, 'medium')]);
  const t = townOf(plots);

  it('is the same every time', () => {
    expect(townOf(plots)).toEqual(t);
  });

  it('keeps the plots clear: no house, road or sidewalk on one', () => {
    const over = (r: { x0: number; z0: number; x1: number; z1: number }) => plots.some((p) => r.x0 < p.x1 && r.x1 > p.x0 && r.z0 < p.z1 && r.z1 > p.z0);
    expect(t.roads.some(over)).toBe(false);
    expect(t.walks.some(over)).toBe(false);
    expect(t.houses.some((h) => over({ x0: h.x - h.w / 2, z0: h.z - h.d / 2, x1: h.x + h.w / 2, z1: h.z + h.d / 2 }))).toBe(false);
  });

  it('fills the ring and the gap in the grid with other buildings, and covers it all', () => {
    expect(t.houses.length).toBeGreaterThan(8);
    for (const p of plots) expect(p.x0 > t.bounds.x0 && p.x1 < t.bounds.x1 && p.z0 > t.bounds.z0 && p.z1 < t.bounds.z1).toBe(true);
  });
});
