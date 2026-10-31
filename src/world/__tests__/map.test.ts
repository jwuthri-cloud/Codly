import { describe, expect, it } from 'vitest';
import type { LotSlot } from '../../lib/gen/LotSlot';
import { TEMPLATES } from '../../office/layout';
import { exteriorOf, STOREY } from '../exterior';
import { buildingOrigin, buyOptions, gridCentre, lightsOf, PITCH_X, PITCH_Z, plotAt, plotOf, plotsOf, signOf, slotActions, slotKind } from '../map';
import { fits, landPrice, lotValue, readyMadePrice } from '../price';

const slot = (extra: Partial<LotSlot> = {}): LotSlot => ({
  id: 'a', x: 0, z: 0, size: 'small', owned: false, workspace_id: null, name: null, crew: 0, price: 300, template: null, gift: false, sells_for: 0, ...extra,
});
const office = slot({ owned: true, workspace_id: 'ws-1', name: 'codly', crew: 5, template: 'studio', size: 'large', gift: true });
const empty = slot({ owned: true, sells_for: 240 });
const building = slot({ owned: true, template: 'garage', sells_for: 900 });
const sale = slot({ size: 'medium', price: 1200 });

describe('grid to world', () => {
  it('centres the grid on the origin', () => {
    expect(gridCentre([{ x: 0, z: 0 }, { x: 3, z: 1 }])).toEqual({ x: 1.5, z: 0.5 });
    expect(gridCentre([])).toEqual({ x: 0, z: 0 });
  });

  it('places each plot in its cell, at its size', () => {
    const p = plotOf({ id: 'a', x: 2, z: 1, size: 'large' }, { x: 1, z: 0 });
    expect([p.cx, p.cz]).toEqual([PITCH_X, PITCH_Z]);
    expect([p.w, p.d]).toEqual([28, 24]);
    expect([p.x0, p.x1, p.z0, p.z1]).toEqual([PITCH_X - 14, PITCH_X + 14, PITCH_Z - 12, PITCH_Z + 12]);
  });

  it('never lets two neighbouring plots overlap, whatever their sizes', () => {
    const plots = plotsOf([slot({ id: 'a', x: 0, z: 0, size: 'large' }), slot({ id: 'b', x: 1, z: 0, size: 'large' }), slot({ id: 'c', x: 0, z: 1, size: 'large' })]);
    expect(plots[0].x1).toBeLessThan(plots[1].x0);
    expect(plots[0].z1).toBeLessThan(plots[2].z0);
  });

  it('finds the plot under a ground point, and none between plots', () => {
    const plots = plotsOf([slot({ id: 'a', x: 0 }), slot({ id: 'b', x: 1 })]);
    expect(plotAt(plots, plots[0].cx, plots[0].cz)).toBe('a');
    expect(plotAt(plots, plots[1].x0 + 0.1, plots[1].cz)).toBe('b');
    expect(plotAt(plots, (plots[0].x1 + plots[1].x0) / 2, 0)).toBeNull();
  });

  it('puts a smaller building against the plot’s south edge, centred', () => {
    const p = plotOf({ id: 'a', x: 0, z: 0, size: 'medium' }, { x: 0, z: 0 });
    const o = buildingOrigin(p, TEMPLATES.garage);
    expect(o.x + TEMPLATES.garage.size.w / 2).toBe(p.cx);
    expect(o.z + TEMPLATES.garage.size.d).toBe(p.z1);
  });
});

describe('signs', () => {
  it('names the repo and its crew; only the open one is open', () => {
    expect(signOf(office, true, 0)).toEqual({ title: 'codly · 5 agents', sub: 'Open', state: 'open' });
    expect(signOf(office, true, 2).sub).toBe('Open · 2 working');
    expect(signOf(office, false, 2)).toEqual({ title: 'codly · 5 agents', sub: 'Closed', state: 'closed' });
    expect(signOf({ ...office, crew: 1 }).title).toBe('codly · 1 agent');
  });

  it('says what an empty, a vacant and a for-sale lot are', () => {
    expect(signOf(empty).title).toBe('Empty lot · Build here');
    expect(signOf(building).title).toBe('Garage · No office yet');
    expect(signOf(sale).title).toBe('For sale · 🪙 1,200');
    expect(signOf(sale).sub).toBe('Medium · 16 × 12 m');
  });

  it('lights only the open repo, brighter while it works', () => {
    expect(lightsOf(office, false, 3)).toBe('off');
    expect(lightsOf(office, true, 0)).toBe('on');
    expect(lightsOf(office, true, 3)).toBe('busy');
    expect(lightsOf(building, true, 3)).toBe('off');
  });
});

describe('what a lot offers', () => {
  const ctx = { movable: 2, freeLots: 1 };

  it('knows each kind', () => {
    expect([office, building, empty, sale].map(slotKind)).toEqual(['office', 'building', 'empty', 'sale']);
  });

  it('a lot with a repo: enter, move, and no selling while it lives there', () => {
    const a = slotActions(office, ctx);
    expect(a.map((x) => x.id)).toEqual(['enter', 'move-away', 'sell']);
    expect(a[0].disabled).toBeNull();
    expect(a[2].disabled).toMatch(/repo lives here/);
    expect(slotActions(office, { movable: 0, freeLots: 0 })[1].disabled).toMatch(/No free lot/);
  });

  it('an empty or vacant lot: move an office here, or sell it for its refund', () => {
    for (const s of [empty, building]) {
      const a = slotActions(s, ctx);
      expect(a.map((x) => x.id)).toEqual(['move-here', 'sell']);
      expect(a[1].disabled).toBeNull();
      expect(a[1].note).toBe(`refund 🪙 ${s.sells_for.toLocaleString('en-US')}`);
    }
    expect(slotActions(empty, { movable: 0, freeLots: 0 })[0].disabled).toMatch(/No other office/);
    expect(slotActions({ ...empty, gift: true }, ctx)[1].note).toMatch(/gift/);
  });

  it('a for-sale lot offers buying instead', () => {
    expect(slotActions(sale, ctx)).toEqual([]);
  });
});

describe('prices', () => {
  it('a ready-made lot costs the land plus everything on it', () => {
    expect(readyMadePrice('small', null)).toBe(landPrice('small'));
    expect(readyMadePrice('small', 'garage')).toBe(landPrice('small') + lotValue(TEMPLATES.garage));
    expect(lotValue(TEMPLATES.garage)).toBeGreaterThan(0);
  });

  it('offers empty, then every design that fits, greyed out past the wallet', () => {
    const small = buyOptions({ size: 'small' }, 500, ['garage', 'studio']);
    expect(small.map((o) => o.template)).toEqual([null, 'garage']);
    expect(small[0]).toMatchObject({ label: 'Empty lot', price: 300, short: false });
    expect(small[1]).toMatchObject({ label: 'Garage', short: true });
    const large = buyOptions({ size: 'large' }, 1e9, ['garage', 'studio']);
    expect(large.map((o) => o.template)).toEqual([null, 'garage', 'studio']);
    expect(large.every((o) => !o.short)).toBe(true);
    expect(fits('studio', 'medium')).toBe(false);
    expect(fits('nope', 'large')).toBe(false);
  });
});

describe('the building from outside', () => {
  it('raises only the outer walls, with the doors and windows cut in and a roof', () => {
    const ex = exteriorOf(TEMPLATES.studio)!;
    expect([ex.x0, ex.z0, ex.x1, ex.z1]).toEqual([0, 0, 28, 24]);
    expect(ex.walls.every((b) => b[4] <= STOREY)).toBe(true);
    // Inner walls (the Studio's rooms) don't show from outside.
    expect(ex.walls.every((b) => b[0] <= 0.2 || b[3] >= 27.8 || b[2] <= 0.2 || b[5] >= 23.8)).toBe(true);
    // Each window, and a skylight per 7 m of roof.
    expect(ex.glass.length).toBe(TEMPLATES.studio.windows.length + 4);
    expect(ex.doors.length).toBe(1);
    expect(ex.roof[0][1]).toBe(STOREY);
  });

  it('cuts the garage’s roll-up door where it is', () => {
    const ex = exteriorOf(TEMPLATES.garage)!;
    const d = ex.doors[0];
    expect(d.kind).toBe('rollup');
    expect((d.box[0] + d.box[3]) / 2).toBeCloseTo(6.5);
    expect(exteriorOf({ walls: [], doors: [], windows: [] })).toBeNull();
  });
});
