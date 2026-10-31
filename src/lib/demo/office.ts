// The web demo's ghosts (workers let go, kept apart from the live roster as in the core), its
// wallet and Build/Buy, the developer panel's cheats (same rules as
// crates/codly-core/src/orchestrator/careers.rs and wallet.rs) and lot: the demo office is the Studio.

import type { Agent } from '../gen/Agent';
import type { ModelOffer } from '../gen/ModelOffer';
import type { Placed } from '../gen/Placed';
import type { Shop } from '../gen/Shop';
import type { Wallet } from '../gen/Wallet';
import type { Lot } from '../gen/Lot';
import { BY_ID } from '../../office/buy';
import { TEMPLATES } from '../../office/layout';
import { atLeast, emptyCareer, rankFor, rankLabel, staminaOf } from '../util/career';
import type { Rank } from '../gen/Rank';
import type { DayReport } from '../gen/DayReport';
import type { Digest } from '../gen/Digest';
import type { Office } from '../gen/Office';
import { transferBlock } from '../util/transfer';
import type { Sim } from './sim';

let ghosts: Agent[] | null = null;
let placed: Placed[] = [];
let granted = 0;
let cheated = false;

let lot: Lot | null = null;

export function demoLot(): Lot {
  return (lot ??= structuredClone(TEMPLATES.studio));
}

export function demoSaveLot(next: Lot): Lot {
  if (next.v !== 1) throw new Error('unknown lot version');
  return (lot = structuredClone(next));
}

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

// Two other offices for the Transfer picker (the demo has one repo), with who works there.
const OFFICES = [
  { id: 'demo-office-docs', name: 'docs-site', repo_path: '~/code/docs-site', crew: ['Mira', 'Juno'] },
  { id: 'demo-office-billing', name: 'billing-api', repo_path: '~/code/billing-api', crew: ['Otto', 'Pia', 'Rex'] },
];

/** Agents moved to those offices: still yours, so their XP stays in the wallet. */
const transferred: Agent[] = [];

export const demoOffices = (): Office[] => OFFICES.map((o) => ({ id: o.id, name: o.name, repo_path: o.repo_path, crew: o.crew.length }));

/** Transfer an agent to one of the demo's other offices: same rules as the core's; it leaves this office's crew. */
export function demoTransfer(sim: Sim, id: string, to: string): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  const block = transferBlock(a, [...sim.tasks.values()]);
  if (block) throw new Error(`${a.name}: ${block}`);
  const o = OFFICES.find((x) => x.id === to);
  if (!o) throw new Error('no such office');
  if (o.crew.some((n) => n.toLowerCase() === a.name.toLowerCase())) throw new Error(`someone called ${a.name} already works in ${o.name}`);
  o.crew.push(a.name);
  const career = { ...a.career, trust: 'intern' as Rank, friends: {}, mentor: null, milestones: [...a.career.milestones, { at: sim.now(), text: `Moved to ${o.name}` }] };
  const moved: Agent = { ...a, career, session_id: null, worktree: null, task_id: null, status: 'idle', consultant: false, kept: true, updated_at: sim.now() };
  transferred.push(moved);
  sim.agents.delete(id);
  sim.decision('transfer', 'rule', `${a.name} moved to ${o.name}`, { agent: id, name: a.name, from: sim.workspace?.id ?? null, to: o.id, to_name: o.name });
  return moved;
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

/** Home and back (the office's energy): the demo keeps it on the agent, as the core does. */
export function demoHome(sim: Sim, id: string, until: number | null): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  if (until != null && a.role === 'lead') throw new Error('the Lead never goes home');
  const next: Agent = { ...a, updated_at: sim.now(), away_until: until ?? undefined, tired_since: until != null ? undefined : a.tired_since };
  sim.putAgent(next);
  return next;
}

/** Tired or rested again (a hint for the Lead), kept on the agent as the core does. */
export function demoTired(sim: Sim, id: string, tired: boolean): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  if ((a.tired_since != null) === tired) return a;
  const next: Agent = { ...a, updated_at: sim.now(), tired_since: tired ? sim.now() : undefined };
  sim.putAgent(next);
  return next;
}

/** A new Legend's offer: the demo knows Claude's aliases (`env::model_windows`); 1M beats 200k. */
export function demoBiggerModel(sim: Sim, id: string): ModelOffer | null {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  if (a.provider !== 'claude' || /\[1m\]/.test(a.model ?? '') || (a.usage.context_window ?? 0) >= 1_000_000) return null;
  return { model: 'opus[1m]', context_window: 1_000_000 };
}

/** A shift worked and rested from, or a sickness: its stamina moves (same rule as the core). */
export function demoRest(sim: Sim, id: string, sick: boolean): Agent {
  const a = sim.agents.get(id);
  if (!a) throw new Error('no such agent');
  const stats = { ...a.career.stats, shifts: a.career.stats.shifts + (sick ? 0 : 1), sick: a.career.stats.sick + (sick ? 1 : 0) };
  const career = { ...a.career, stats };
  const next: Agent = { ...a, updated_at: sim.now(), career: { ...career, stamina: staminaOf(career, (sim.now() - a.created_at) / 86_400_000) } };
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
  const earned = [...sim.agents.values(), ...seeded(sim), ...transferred].reduce((n, a) => n + a.career.xp, 0);
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
