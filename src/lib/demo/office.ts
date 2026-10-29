// The web demo's ghosts (workers let go, kept apart from the live roster as in the core) and
// Build/Buy budget: same rules as crates/codly-core/src/orchestrator/careers.rs.

import type { Agent } from '../gen/Agent';
import type { Shop } from '../gen/Shop';
import { CATALOG, emptyCareer } from '../util/career';
import type { Sim } from './sim';

let ghosts: Agent[] | null = null;
let owned: string[] = [];

function seeded(sim: Sim): Agent[] {
  if (ghosts) return ghosts;
  const day = 86_400_000;
  const ghost = (name: string, color: number, look: string, xp: number, merged: number, traits: string[], ago: number): Agent => {
    const a = sim.newAgent({ name, provider: 'claude', model: null, role: 'worker', color, billing: 'subscription', status: 'offline' });
    const at = sim.now() - ago * day;
    return {
      ...a,
      look_seed: look,
      title: 'Backend dev',
      consultant: true,
      archived: true,
      updated_at: at,
      career: {
        ...emptyCareer(),
        xp,
        rank: xp >= 250 ? 'senior' : xp >= 60 ? 'junior' : 'intern',
        stats: { ...emptyCareer().stats, commits: merged * 2, checks_passed: merged * 2, approved: merged, merged },
        skills: { TypeScript: merged * 3, Testing: merged },
        milestones: [{ at: at - day, text: 'First merge' }, ...(xp >= 250 ? [{ at, text: 'Promoted to Senior' }] : [])],
        traits,
      },
    };
  };
  ghosts = [ghost('Rosa', 5, 'dusk', 420, 6, ['perfectionist', 'nightowl'], 3), ghost('Theo', 2, 'moss', 180, 3, ['social', 'clumsy'], 9)];
  return ghosts;
}

export const demoGhosts = (sim: Sim) => seeded(sim);

export function demoBringBack(sim: Sim, id: string): Agent {
  const g = seeded(sim).find((x) => x.id === id);
  if (!g) throw new Error('no such ghost in this repo');
  if ([...sim.agents.values()].some((a) => a.name.toLowerCase() === g.name.toLowerCase())) throw new Error(`someone called ${g.name} already works here`);
  ghosts = seeded(sim).filter((x) => x !== g);
  const back: Agent = { ...g, archived: false, consultant: false, status: 'idle', task_id: null, updated_at: sim.now(), career: { ...g.career, milestones: [...g.career.milestones, { at: sim.now(), text: 'Came back' }] } };
  sim.putAgent(back);
  return back;
}

export function demoShop(sim: Sim): Shop {
  const earned = [...sim.agents.values(), ...seeded(sim)].reduce((n, a) => n + a.career.xp, 0);
  const items = CATALOG.map(({ id, label, price }) => ({ id, label, price }));
  const spent = items.filter((i) => owned.includes(i.id)).reduce((n, i) => n + i.price, 0);
  return { earned, spent, items, owned: [...owned] };
}

export function demoTrade(sim: Sim, item: string, buy: boolean): Shop {
  const shop = demoShop(sim);
  const it = shop.items.find((i) => i.id === item);
  if (!it) throw new Error('no such item');
  const has = owned.includes(item);
  if (buy && has) throw new Error('already in the office');
  if (buy && shop.earned - shop.spent < it.price) throw new Error(`not enough XP yet (${shop.earned - shop.spent} of ${it.price})`);
  if (!buy && !has) throw new Error('not in the office');
  owned = buy ? [...owned, item] : owned.filter((o) => o !== item);
  return demoShop(sim);
}
