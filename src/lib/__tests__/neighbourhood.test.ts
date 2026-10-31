import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import { demoBuyLot, demoGrant, demoLot, demoLots, demoMoveOffice, demoPlace, demoSaveLot, demoSellLot, demoShop, demoWallet } from '../demo/office';
import type { Sim } from '../demo/sim';
import { emptyCareer } from '../util/career';
import { readyMadePrice } from '../../office/price';

// Just enough of the demo's Sim for its wallet and neighbourhood.
const sim = {
  agents: new Map<string, Agent>(),
  workspace: { id: 'ws-demo', name: 'codly' },
  settings: { developer_mode: true },
  now: () => 0,
  decision: () => {},
  newAgent: (a: Partial<Agent>) => ({ ...a, id: String(Math.random()), career: emptyCareer() }),
} as unknown as Sim;

describe("the demo's neighbourhood", () => {
  it('buys, builds, moves and sells as the core does, never for a profit', () => {
    const start = demoWallet(sim).balance;
    const all = demoLots(sim);
    expect(all).toHaveLength(12);
    const home = all.find((s) => s.workspace_id === 'ws-demo')!;
    expect(home).toMatchObject({ size: 'large', owned: true, gift: true, template: 'studio', name: 'codly', sells_for: 0 });
    expect(all.filter((s) => s.owned)).toHaveLength(3);

    const plot = all.find((s) => !s.owned && s.size === 'medium')!;
    const price = readyMadePrice('medium', 'garage')!;
    expect(() => demoBuyLot(sim, plot.id, 'garage')).toThrow(/not enough coins/);
    demoGrant(sim, price + 100);
    expect(demoBuyLot(sim, plot.id, 'garage').balance).toBe(start + 100);
    expect(() => demoSellLot(sim, home.id)).toThrow(/codly's office is there/);

    // A chair bought in Build/Buy is on the lot, and moves with the office.
    demoPlace(sim, { uid: 'c1', item: 'kenney.chair', x: 2, y: 0, z: 9, rot: 0 });
    expect(demoShop(sim).placed.map((p) => p.uid)).toEqual(['c1']);
    const moved = demoMoveOffice(sim, 'ws-demo', plot.id);
    expect(moved.find((s) => s.id === plot.id)!.workspace_id).toBe('ws-demo');
    expect(demoLot().furniture.some((p) => p.uid === 'c1')).toBe(true);
    expect(demoLot().size).toEqual({ w: 16, d: 12 });

    // A save is priced; the old plot keeps its (gift) building and sells for nothing.
    const lot = demoLot();
    lot.walls.push({ id: 'n1', x0: 1, z0: 1, x1: 1, z1: 3, kind: 'solid' });
    expect(demoSaveLot(sim, lot).cost).toBe(20);
    demoSellLot(sim, home.id);
    const worth = demoLots(sim).reduce((n, s) => n + s.sells_for, 0);
    expect(demoWallet(sim).balance + worth).toBeLessThanOrEqual(start + price + 100);
  });
});
