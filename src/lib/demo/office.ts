// The web demo's ghosts (workers let go, kept apart from the live roster as in the core), its
// wallet and Build/Buy, the developer panel's cheats (same rules as
// crates/codly-core/src/orchestrator/careers.rs and wallet.rs), and its neighbourhood (same
// rules as orchestrator/lots.rs, priced by office/price.ts): the demo office is the Studio, on a
// large plot, next to its two other offices and plots for sale.

import type { Agent } from '../gen/Agent';
import type { ModelOffer } from '../gen/ModelOffer';
import type { Placed } from '../gen/Placed';
import type { Shop } from '../gen/Shop';
import type { Wallet } from '../gen/Wallet';
import type { Lot } from '../gen/Lot';
import type { LotSaved } from '../gen/LotSaved';
import type { LotSlot } from '../gen/LotSlot';
import type { LotSizeName } from '../gen/LotSizeName';
import { BY_ID } from '../../office/buy';
import { bareLot, diff, fit, isBare, keptGifts, landBack, landPrice, lotIds, plotSize, sellValue, TEMPLATES, value } from '../../office/price';
import { atLeast, emptyCareer, rankFor, rankLabel, staminaOf } from '../util/career';
import type { Rank } from '../gen/Rank';
import type { DayReport } from '../gen/DayReport';
import type { Digest } from '../gen/Digest';
import type { Office } from '../gen/Office';
import { transferBlock } from '../util/transfer';
import type { Sim } from './sim';

let ghosts: Agent[] | null = null;
let granted = 0;
/** The ledger's sum: paid for plots, building and furniture, less what selling gave back. */
let spent = 0;
let cheated = false;

// The neighbourhood: 4 × 3 plots, row by row (orchestrator/lots.rs's seed). `HERE` stands for
// the open repo, whatever its id.
const HERE = '@here';
type Row = { id: string; x: number; z: number; size: LotSizeName; owned: boolean; workspace: string | null; gift: boolean };
const SEED: LotSizeName[] = ['small', 'small', 'medium', 'large', 'small', 'medium', 'medium', 'large', 'small', 'small', 'medium', 'large'];
let rows: Row[] | null = null;
const lots = new Map<string, Lot>();

const allGift = (lot: Lot): Lot => ({ ...structuredClone(lot), gift: [...lotIds(lot)] });

function hood(): Row[] {
  if (rows) return rows;
  rows = SEED.map((size, i) => ({ id: `lot-${i + 1}`, x: i % 4, z: Math.floor(i / 4), size, owned: false, workspace: null, gift: false }));
  const home = (i: number, workspace: string, lot: Lot) => {
    Object.assign(rows![i], { owned: true, workspace, gift: true });
    const { w, d } = plotSize(rows![i].size);
    lots.set(rows![i].id, allGift(fit(lot, w, d)!));
  };
  home(3, HERE, TEMPLATES.studio);
  home(0, OFFICES[0].id, TEMPLATES.garage);
  home(2, OFFICES[1].id, TEMPLATES.garage);
  return rows;
}

const here = (sim: Sim, ws: string) => (ws === sim.workspace?.id || ws === HERE ? HERE : ws);
const rowOf = (id: string) => hood().find((r) => r.id === id) ?? fail('no such plot');
const homeOf = (ws: string) => hood().find((r) => r.workspace === ws) ?? fail('this repo has no plot');
const lotOn = (r: Row) => lots.get(r.id) ?? bareLot(r.size);
function fail(why: string): never {
  throw new Error(why);
}

function pay(sim: Sim, cost: number) {
  const left = demoWallet(sim).balance;
  if (cost > 0 && left < cost) throw new Error(`not enough coins yet (${left} of ${cost})`);
  spent += cost;
}

export function demoLot(): Lot {
  return structuredClone(lotOn(homeOf(HERE)));
}

/** A priced change to the open office's lot, as the core's `lots::change`. */
function change(sim: Sim, f: (lot: Lot) => void): LotSaved {
  const r = homeOf(HERE);
  const old = lotOn(r);
  const next = structuredClone(old);
  f(next);
  if (next.v !== 1) throw new Error('unknown lot version');
  const { w, d } = plotSize(r.size);
  if (next.size.w !== w || next.size.d !== d) throw new Error(`this plot is ${w} × ${d} m: a lot keeps its size`);
  for (const p of next.furniture) if (!BY_ID.has(p.item)) throw new Error(`no such piece: ${p.item}`);
  next.template = old.template;
  next.gift = keptGifts(old, next);
  const cost = diff(old, next);
  pay(sim, cost);
  lots.set(r.id, next);
  return { lot: structuredClone(next), wallet: demoWallet(sim), cost };
}

export const demoSaveLot = (sim: Sim, next: Lot): LotSaved =>
  change(sim, (lot) => {
    Object.assign(lot, structuredClone(next));
  });

export function demoLots(sim: Sim): LotSlot[] {
  return hood().map((r) => {
    const lot = r.owned ? lotOn(r) : null;
    const other = OFFICES.find((o) => o.id === r.workspace);
    const crew = r.workspace === HERE ? [...sim.agents.values()].filter((a) => a.role !== 'lead' && !a.archived).length : (other?.crew.length ?? 0);
    return {
      id: r.id,
      x: r.x,
      z: r.z,
      size: r.size,
      owned: r.owned,
      workspace_id: r.workspace === HERE ? (sim.workspace?.id ?? null) : r.workspace,
      name: r.workspace === HERE ? (sim.workspace?.name ?? 'codly') : (other?.name ?? null),
      crew,
      price: landPrice(r.size),
      template: lot?.template || null,
      gift: r.gift,
      sells_for: lot ? (r.gift ? 0 : landBack(r.size)) + sellValue(lot) : 0,
    };
  });
}

export const demoLotOf = (slotId: string): Lot => {
  const r = rowOf(slotId);
  return structuredClone(r.owned ? lotOn(r) : bareLot(r.size));
};

export function demoBuyLot(sim: Sim, slotId: string, template: string | null): Wallet {
  const r = rowOf(slotId);
  const { w, d } = plotSize(r.size);
  let made: Lot | null = null;
  if (template != null) {
    const t = TEMPLATES[template] ?? fail(`no such design: ${template}`);
    made = { ...structuredClone(fit(t, w, d) ?? fail(`the ${template} doesn't fit on a ${w} × ${d} m plot`)), gift: [] };
  }
  if (!r.owned) {
    const lot = made ?? bareLot(r.size);
    pay(sim, landPrice(r.size) + value(lot));
    Object.assign(r, { owned: true, gift: false });
    lots.set(r.id, lot);
    return demoWallet(sim);
  }
  if (!made) throw new Error('that plot is already yours');
  const old = lotOn(r);
  if (!isBare(old)) throw new Error('something already stands there: knock it down first');
  pay(sim, diff(old, made));
  lots.set(r.id, made);
  return demoWallet(sim);
}

export function demoSellLot(sim: Sim, slotId: string): Wallet {
  const r = rowOf(slotId);
  if (!r.owned) throw new Error("that plot isn't yours");
  if (r.workspace) throw new Error(`${demoLots(sim).find((s) => s.id === r.id)?.name ?? 'a repo'}'s office is there: move it out first`);
  pay(sim, -((r.gift ? 0 : landBack(r.size)) + sellValue(lotOn(r))));
  Object.assign(r, { owned: false, gift: false });
  lots.delete(r.id);
  return demoWallet(sim);
}

/** Move an office to another plot of yours with none on it (orchestrator/lots.rs's rules). */
export function demoMoveOffice(sim: Sim, workspaceId: string, slotId: string): LotSlot[] {
  const from = homeOf(here(sim, workspaceId));
  const to = rowOf(slotId);
  if (from === to) throw new Error('the office is already there');
  if (!to.owned) throw new Error('buy that plot first');
  if (to.workspace) throw new Error('another office is there');
  const old = structuredClone(lotOn(from));
  const next = structuredClone(lotOn(to));
  const gift = new Set(old.gift);
  const taken = lotIds(next);
  next.storage ??= [];
  const moving = [...old.furniture.map((p) => [p, true] as const), ...(old.storage ?? []).map((p) => [p, false] as const)];
  for (const [piece, up] of moving) {
    const p = { ...piece, uid: taken.has(piece.uid) ? crypto.randomUUID() : piece.uid };
    taken.add(p.uid);
    if (gift.has(piece.uid)) next.gift.push(p.uid);
    const fits = up && p.x >= 0 && p.x <= next.size.w && p.z >= 0 && p.z <= next.size.d;
    (fits ? next.furniture : next.storage).push(p);
  }
  if (!next.storage.length) delete next.storage;
  old.furniture = [];
  delete old.storage;
  const left = lotIds(old);
  old.gift = old.gift.filter((id) => left.has(id));
  lots.set(from.id, old);
  lots.set(to.id, next);
  to.workspace = from.workspace;
  from.workspace = null;
  return demoLots(sim);
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

/** The wallet (crates/codly-core/src/orchestrator/wallet.rs): the demo's agents' XP (and those
 *  it let go or moved), plus grants, less what was paid for plots, building and furniture. */
export function demoWallet(sim: Sim): Wallet {
  const earned = [...sim.agents.values(), ...seeded(sim), ...transferred].reduce((n, a) => n + a.career.xp, 0);
  return { earned, granted, spent, balance: earned + granted - spent, cheated };
}

/** Build/Buy: the furniture bought for this office (what came with the lot and the stations
 *  are drawn from the lot). */
export function demoShop(sim: Sim): Shop {
  const lot = lotOn(homeOf(HERE));
  const gift = new Set(lot.gift);
  return { wallet: demoWallet(sim), placed: lot.furniture.filter((p) => !gift.has(p.uid) && !BY_ID.get(p.item)?.builtin).map((p) => ({ ...p })) };
}

export function demoPlace(sim: Sim, piece: Placed): Shop {
  change(sim, (lot) => {
    const old = lot.furniture.find((p) => p.uid === piece.uid);
    const stored = lot.storage?.find((p) => p.uid === piece.uid);
    if ((old ?? stored) && (old ?? stored)!.item !== piece.item) throw new Error(`that piece is a ${(old ?? stored)!.item}`);
    if (!old && !stored && !BY_ID.has(piece.item)) throw new Error('no such item');
    lot.storage = lot.storage?.filter((p) => p.uid !== piece.uid);
    if (!lot.storage?.length) delete lot.storage;
    lot.furniture = [...lot.furniture.filter((p) => p.uid !== piece.uid), piece];
  });
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

/** The lot back to a gift Garage; what stood there sells back at the usual rates. */
export function demoResetOffice(sim: Sim): Shop {
  developer(sim);
  const r = homeOf(HERE);
  const { w, d } = plotSize(r.size);
  pay(sim, -sellValue(lotOn(r)));
  lots.set(r.id, allGift(fit(TEMPLATES.garage, w, d)!));
  sim.decision('developer', 'rule', 'reset the office', {});
  return demoShop(sim);
}

export function demoSell(sim: Sim, uid: string): Shop {
  change(sim, (lot) => {
    const n = lot.furniture.length + (lot.storage?.length ?? 0);
    lot.furniture = lot.furniture.filter((p) => p.uid !== uid);
    lot.storage = lot.storage?.filter((p) => p.uid !== uid);
    if (lot.furniture.length + (lot.storage?.length ?? 0) === n) throw new Error('not in the office');
  });
  return demoShop(sim);
}
