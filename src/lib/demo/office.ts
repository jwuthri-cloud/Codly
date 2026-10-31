// The web demo's ghosts (workers let go, kept apart from the live roster as in the core), its
// wallet and Build/Buy, and the developer panel's cheats: same rules as
// crates/codly-core/src/orchestrator/careers.rs and wallet.rs.

import type { Agent } from '../gen/Agent';
import type { Placed } from '../gen/Placed';
import type { Shop } from '../gen/Shop';
import type { Wallet } from '../gen/Wallet';
import { BY_ID } from '../../office/buy';
import { atLeast, emptyCareer, rankFor, rankLabel } from '../util/career';
import type { Rank } from '../gen/Rank';
import type { DayReport } from '../gen/DayReport';
import type { Digest } from '../gen/Digest';
import type { Sim } from './sim';

let ghosts: Agent[] | null = null;
let placed: Placed[] = [];
let granted = 0;
let cheated = false;

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
  const back: Agent = { ...g, archived: false, consultant: false, kept: true, status: 'idle', task_id: null, updated_at: sim.now(), career: { ...g.career, milestones: [...g.career.milestones, { at: sim.now(), text: 'Came back' }] } };
  sim.putAgent(back);
  return back;
}

export function demoLook(sim: Sim, id: string, lookSeed: string): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  const next: Agent = { ...a, updated_at: sim.now(), look_seed: lookSeed };
  sim.putAgent(next);
  return next;
}

export function demoTrust(sim: Sim, id: string, rank: Rank): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  const trust = atLeast(rank, a.career.rank) ? a.career.rank : rank;
  const milestones = atLeast(trust, 'junior') && !atLeast(a.career.trust, trust) ? [...a.career.milestones, { at: sim.now(), text: `Trusted as ${rankLabel(trust)}` }] : a.career.milestones;
  const next: Agent = { ...a, updated_at: sim.now(), career: { ...a.career, trust, milestones } };
  sim.putAgent(next);
  return next;
}

/** The digest from the demo's own event log (same counting as the core's careers). */
export function demoDigest(sim: Sim, since: number): Digest {
  const by = new Map<string, DayReport>();
  const rep = (id: string | null | undefined) => {
    if (!id || sim.agents.get(id)?.role === 'lead') return null;
    if (!by.has(id)) by.set(id, { agent_id: id, xp: 0, stats: { ...emptyCareer().stats } });
    return by.get(id)!;
  };
  const worker = (t: string | null) => (t ? (sim.tasks.get(t)?.assignee ?? null) : null);
  for (const e of sim.log) {
    if (e.ts < since) continue;
    const k = e.kind;
    if (k.type === 'committed') {
      const r = rep(e.agent_id);
      if (r) (r.stats.commits++, (r.xp += 15));
    } else if (k.type === 'verified') {
      const r = rep(e.agent_id ?? worker(e.task_id));
      if (r && k.ok) (r.stats.checks_passed++, (r.xp += 10));
      else if (r) r.stats.checks_failed++;
    } else if (k.type === 'merged') {
      const r = rep(worker(e.task_id) ?? e.agent_id);
      if (r) (r.stats.merged++, (r.xp += 30));
    } else if (k.type === 'reviewed') {
      const r = rep(e.agent_id);
      if (r) (r.stats.reviews_done++, (r.xp += 10));
    }
  }
  const all = [...sim.agents.values(), ...seeded(sim)];
  return {
    since,
    reports: [...by.values()].sort((a, b) => b.xp - a.xp),
    milestones: all.flatMap((a) => a.career.milestones.filter((m) => m.at >= since).map((m) => [a.id, m] as [string, typeof m])),
    done: [...sim.tasks.values()].filter((t) => t.status === 'done' && t.updated_at >= since).map((t) => t.title),
  };
}

/** The wallet (crates/codly-core/src/orchestrator/wallet.rs): the demo has one repo, so its
 *  agents' XP, plus grants, less its furniture. */
export function demoWallet(sim: Sim): Wallet {
  const earned = [...sim.agents.values(), ...seeded(sim)].reduce((n, a) => n + a.career.xp, 0);
  const spent = placed.reduce((n, p) => n + (BY_ID.get(p.item)?.price ?? 0), 0);
  return { earned, granted, spent, balance: earned + granted - spent, cheated };
}

export const demoShop = (sim: Sim): Shop => ({ wallet: demoWallet(sim), placed: [...placed] });

export function demoPlace(sim: Sim, piece: Placed): Shop {
  const old = placed.find((p) => p.uid === piece.uid);
  if (old && old.item !== piece.item) throw new Error(`that piece is a ${old.item}`);
  if (!old) {
    const price = BY_ID.get(piece.item)?.price;
    if (price === undefined) throw new Error('no such item');
    const left = demoWallet(sim).balance;
    if (left < price) throw new Error(`not enough coins yet (${left} of ${price})`);
  }
  placed = [...placed.filter((p) => p.uid !== piece.uid), piece];
  return demoShop(sim);
}

// The developer panel's cheats, refused outside Developer mode as in the core. Each one that
// adds coins flags the wallet for good.
function developer(sim: Sim) {
  if (!sim.settings.developer_mode) throw new Error('turn on Developer mode in Settings first');
}

export function demoGrant(sim: Sim, amount: number): Wallet {
  developer(sim);
  if (!(amount >= 1 && amount <= 1_000_000)) throw new Error('grant between 1 and 1,000,000 coins');
  cheated = true;
  granted += amount;
  sim.decision('developer', 'rule', `cheat: +${amount} coins`, { coins: amount });
  return demoWallet(sim);
}

export function demoGiveXp(sim: Sim, id: string, xp: number): Agent {
  developer(sim);
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  if (a.role === 'lead') throw new Error('the Lead has no career');
  cheated = true;
  const at = sim.now();
  const total = a.career.xp + xp;
  const earned = rankFor(total);
  const rank = atLeast(earned, a.career.rank) ? earned : a.career.rank;
  const milestones = [...a.career.milestones, { at, text: `Cheat: +${xp} XP` }, ...(rank !== a.career.rank ? [{ at, text: `Promoted to ${rankLabel(rank)}` }] : [])];
  const next: Agent = { ...a, updated_at: at, career: { ...a.career, xp: total, rank, milestones } };
  sim.putAgent(next);
  sim.decision('developer', 'rule', `cheat: +${xp} XP for ${a.name}`, { agent: a.id, xp });
  return next;
}

export function demoResetOffice(sim: Sim): Shop {
  developer(sim);
  placed = [];
  sim.decision('developer', 'rule', 'reset the office', {});
  return demoShop(sim);
}

export function demoSell(sim: Sim, uid: string): Shop {
  if (!placed.some((p) => p.uid === uid)) throw new Error('not in the office');
  placed = placed.filter((p) => p.uid !== uid);
  return demoShop(sim);
}
