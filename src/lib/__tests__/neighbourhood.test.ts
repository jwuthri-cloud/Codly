import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import { demoBuyLot, demoCloseOffice, demoGrant, demoLot, demoLots, demoMoveOffice, demoOffices, demoPlace, demoSaveLot, demoSellLot, demoShop, demoWallet } from '../demo/office';
import type { Sim } from '../demo/sim';
import type { Task } from '../gen/Task';
import { emptyCareer } from '../util/career';
import { readyMadePrice } from '../../office/price';

// Just enough of the demo's Sim for its wallet and neighbourhood.
const inTurn = new Set<string>();
const agents = new Map<string, Agent>();
const sim = {
  agents,
  tasks: new Map<string, Task>(),
  workspace: { id: 'ws-demo', name: 'codly', repo_path: '/code/codly' },
  recent: [{ repo_path: '/code/codly', name: 'codly', opened_at: 0 }],
  settings: { developer_mode: true },
  now: () => 0,
  decision: () => {},
  newAgent: (a: Partial<Agent>) => ({ ...a, id: String(Math.random()), career: emptyCareer() }),
  actor: (id: string) => ({ inTurn: inTurn.has(id) }),
  patchAgent(id: string, patch: Partial<Agent>) {
    const next = { ...agents.get(id)!, ...patch };
    agents.set(id, next);
    return next;
  },
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
    // The Studio filled its plot: off the Garage's, so the chair waits in storage there.
    expect(demoLot().storage?.some((p) => p.uid === 'c1')).toBe(true);
    expect(demoLot().size).toEqual({ w: 16, d: 12 });

    // A save is priced; the old plot keeps its (gift) building and sells for nothing.
    const lot = demoLot();
    lot.walls.push({ id: 'n1', x0: 1, z0: 1, x1: 1, z1: 3, kind: 'solid' });
    expect(demoSaveLot(sim, lot).cost).toBe(20);
    demoSellLot(sim, home.id);
    const worth = demoLots(sim).reduce((n, s) => n + s.sells_for, 0);
    expect(demoWallet(sim).balance + worth).toBeLessThanOrEqual(start + price + 100);
  });

  it('closes an office as the core does: the repo leaves, its plot stays yours and is free', () => {
    const plotOf = (ws: string) => demoLots(sim).find((s) => s.workspace_id === ws);
    expect(() => demoCloseOffice(sim, 'no-such-office')).toThrow('no such office');
    // Agents are at work in billing-api: it does not close.
    expect(() => demoCloseOffice(sim, 'demo-office-billing')).toThrow('Otto is in the middle of a turn in billing-api: wait for it, or stop it first');
    expect(demoOffices().map((o) => o.name)).toEqual(['docs-site', 'billing-api']);

    const docs = plotOf('demo-office-docs')!;
    const after = demoCloseOffice(sim, 'demo-office-docs').find((s) => s.id === docs.id)!;
    expect(after).toMatchObject({ owned: true, workspace_id: null, name: null, crew: 0, template: docs.template });
    expect(demoOffices().map((o) => o.name)).toEqual(['billing-api']);
    expect(() => demoCloseOffice(sim, 'demo-office-docs')).toThrow('no such office');

    // The open repo: not while a task is unfinished, not while an agent is in a turn.
    const home = plotOf('ws-demo')!;
    const ada = { id: 'ada', name: 'Ada', status: 'idle' } as Agent;
    sim.agents.set(ada.id, ada);
    sim.tasks.set('t1', { id: 't1', key: 'a1', title: 'Upgrade ESLint', status: 'pending' } as Task);
    expect(() => demoCloseOffice(sim, 'ws-demo')).toThrow('codly still has unfinished work (a1 "Upgrade ESLint"): finish or cancel it first');
    sim.tasks.set('t1', { ...sim.tasks.get('t1')!, status: 'cancelled' });
    inTurn.add(ada.id);
    expect(() => demoCloseOffice(sim, 'ws-demo')).toThrow('Ada is in the middle of a turn in codly: wait for it, or stop it first');
    expect(sim.workspace).not.toBeNull();
    inTurn.clear();
    const closed = demoCloseOffice(sim, 'ws-demo').find((s) => s.id === home.id)!;
    expect(closed).toMatchObject({ owned: true, workspace_id: null, name: null });
    expect(sim.workspace).toBeNull();
    expect(sim.recent).toEqual([]);
    expect(sim.agents.get(ada.id)!.status).toBe('offline');
  });
});
