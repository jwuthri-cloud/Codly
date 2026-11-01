// The Sims layer: each agent's needs, moodlets, skills and relationships, all from real data.
//   Energy  = Codly's own need, over a shift (PROGRESSION.md §5–7): real working time drains it
//             (~4 h from full to empty for a new hire, ~3× slower at stamina 100), idle time a
//             little, waiting on you not at all. Going home refills it in minutes; a nap on a
//             sofa at a third of that speed; a cup of coffee by 15% (and caffeine).
//   Focus   = context window left (its latest turn)
//   Morale  = how its recent work went (checks, merges, reviews), refilled by games and reading,
//             drifting back to normal
//   Hunger, Thirst = 1 fed / watered … 0 starving / parched: work drains them (thirst ~3 h, hunger
//             ~5 h of real work). They fill while the agent stays at a source (`FEED`), so a big
//             refill takes longer: a toaster snack or the fridge (never past 60% fed), the
//             microwave, the stove (fastest, and a good meal lifts morale); the water cooler, the
//             fridge, a coffee. Home fills them up. Under 25% morale slides, under 10% faster (Starving,
//             Parched); a hungry or thirsty agent's break goes to the kitchen.
// Energy and Morale also move with what the agent does (`setActivity`): a nap, a PS5 session…
// Moodlets are short-lived feelings from real events ("Tests passed", "Waiting on you");
// relationships are the reviews saved on its career (`career.friends`). Skills are on the
// career too (SimsHud reads them). Nothing here is invented: no event, no change.
//
// Home: under 30% an agent is Tired; at 0% Exhausted, and once its task is done it goes home by
// itself (or you send it). Too much coffee, or shifts in a row without going home, make it
// Sick: it goes home for a long rest. The Lead never goes home (it only looks tired). Going
// home never touches the agent's conversation: the core only stops the Lead giving it new work.
// Stamina (career.stamina, from the core) scales all of it.
//
// Pure and stateful: feed it events (`ingest`), ask for an agent's sheet (`stats`); `tick`
// sends agents home and back, and `drain` hands the office what to tell the core.

import type { Agent } from '../lib/gen/Agent';
import type { EventRow } from '../lib/gen/EventRow';
import type { Task } from '../lib/gen/Task';
import type { AgentState } from './model';

export type Tone = 'good' | 'bad' | 'neutral';

export interface Moodlet {
  key: string;
  label: string;
  /** One line: why. */
  why: string;
  tone: Tone;
  icon: string;
  /** Shown on its name tag in the office (only the feelings worth seeing from afar). */
  emoji?: string;
  at: number;
  /** null = while the condition lasts. */
  until: number | null;
}

export interface Relation {
  agentId: string;
  /** 0 … 100. */
  score: number;
  /** "Friend", "Best friend"… */
  label: string;
  /** "5 times together". */
  why: string;
}

/** Times together (reviews, plans shipped together, breaks: `career.friends`) as a bar and a word. */
export const friendScore = (n: number) => Math.min(100, Math.round(25 * Math.log2(1 + 2 * n)));
export const bondLabel = (n: number) => (n >= 12 ? 'Best friend' : n >= 6 ? 'Good friend' : n >= 3 ? 'Friend' : 'Acquaintance');
/** From this many times together they are friends: a break together lifts both more. */
export const FRIEND_AT = 3;

/** Mood at work: in a great mood its energy lasts longer, a tense one tires faster. */
export function moodDrain(morale: number): number {
  return morale > 0.75 ? 0.8 : morale < 0.35 ? 1.25 : 1;
}

/** What its mood does right now, in words (the Mood tab). */
export function moodEffects(morale: number): string[] {
  if (morale > 0.78) return ['Its energy lasts 25% longer', 'Wants a game with a friend on its breaks', 'Walks with a spring in its step'];
  if (morale > 0.75) return ['Its energy lasts 25% longer', 'Walks with a spring in its step'];
  if (morale < 0.35) return ['Tires 25% faster: it goes home sooner', 'Plays alone on the PS5 on its breaks'];
  if (morale < 0.4) return ['Plays alone on the PS5 on its breaks'];
  return [];
}

/** Together this long on a break counts as hanging out; once per pair per HANG_AGAIN_MS. */
const HANG_MS = 20_000;
const HANG_AGAIN_MS = 15 * 60_000;

export type PlumbobColor = 'green' | 'amber' | 'red' | 'blue' | 'grey';

/** Where it is with rest: home, coffee, sickness. */
export interface RestState {
  /** At home until then (ms); sick = the long rest after being sick. */
  home: { until: number; sick: boolean } | null;
  /** Cups of caffeine in it now (each wears off over an hour). */
  caffeine: number;
  jittery: boolean;
  sick: boolean;
  /** Out of energy: it goes home once its task is done (the Lead only looks it). */
  exhausted: boolean;
  /** The most energy it can have this shift (60% after being sick). */
  cap: number;
  /** Everyone but the Lead (you talk to it). */
  canGoHome: boolean;
}

export interface SimSheet {
  needs: { energy: number; focus: number | null; morale: number; hunger: number; thirst: number };
  moodlets: Moodlet[];
  relations: Relation[];
  plumbob: PlumbobColor;
  /** One word for the overall mood. */
  mood: string;
  /** What that mood does now (`moodEffects`). */
  effects: string[];
  rest: RestState;
}

const BASE_MORALE = 0.65;
/** Morale drifts back to normal with this time constant (ms). */
const MORALE_TAU = 10 * 60_000;
const MOODLET_MS = 6 * 60_000;
/** Under this it is Tired: morale slides and it walks slower. */
export const TIRED = 0.3;
/** A cup of coffee: energy right away. */
const CUP = 0.15;
/** Caffeine wears off over an hour. */
const CAFFEINE_MS = 60 * 60_000;
/** Sick: the long rest at home, and the energy cap for the shift after it. */
const SICK_MS = 30 * 60_000;
const SICK_CAP = 0.6;
/** Energy drained since it last went home that counts as a shift worked (stamina). */
const SHIFT_DONE = 0.5;
const OVERTIME_MORALE = -0.15;
/** Hunger and thirst a working hour drains (an idle one less, by LOAD). */
const HUNGER_PER_HOUR = 0.2;
const THIRST_PER_HOUR = 0.33;
/** Under this it is Hungry / Thirsty: morale slides; under STARVING, faster. */
export const PECKISH = 0.25;
const STARVING = 0.1;
/** What a minute at a source gives: [per minute, up to] for hunger and thirst. Bigger meals fill
 *  faster per minute, so cooking beats snacking on repeat; a snack never makes a meal. */
export const FEED: Partial<Record<RestActivity, { hunger?: [number, number]; thirst?: [number, number] }>> = {
  coffee: { thirst: [0.2, 0.5] },
  water: { thirst: [1, 1] },
  snack: { hunger: [0.3, 0.6], thirst: [0.6, 0.8] }, // the fridge: a snack and a soda
  toast: { hunger: [0.3, 0.6] },
  heat: { hunger: [0.45, 1] },
  cook: { hunger: [0.6, 1] },
};
/** How long a stay at `activity` takes to fill what it can (ms), for the break room. */
export function feedMs(activity: RestActivity, needs: { hunger: number; thirst: number }): number {
  const f = FEED[activity];
  if (!f) return 0;
  const ms = (k: 'hunger' | 'thirst') => {
    const r = f[k];
    return r && needs[k] < r[1] ? ((r[1] - needs[k]) / r[0]) * 60_000 : 0;
  };
  return Math.max(ms('hunger'), ms('thirst'));
}
/** Fed at least this much at the stove: a Good meal (morale). */
const GOOD_MEAL = 0.3;

/** Skill level from XP: 1, 4, 9, 16 … XP per level, up to 10 (like a Sims skill bar). */
export function levelOf(xp: number): { level: number; progress: number } {
  const level = Math.min(10, Math.floor(Math.sqrt(xp)));
  if (level >= 10) return { level, progress: 1 };
  const lo = level * level;
  const hi = (level + 1) * (level + 1);
  return { level, progress: (xp - lo) / (hi - lo) };
}

/** A shift by stamina (0 … 1 = `career.stamina` / 100): PROGRESSION.md §7's table. */
export function shiftOf(stamina: number) {
  const s = Math.min(1, Math.max(0, stamina));
  const lerp = (a: number, b: number) => a + (b - a) * s;
  return {
    /** Energy a working hour drains: 25% (a ~4 h shift) … 8% (~12 h). */
    drainPerHour: lerp(0.25, 0.08),
    /** Minutes at home from empty to full: ~10 … ~3. */
    homeMinutes: lerp(10, 3),
    /** Cups of caffeine at once that make it jittery: 3 … 6. */
    jitteryAt: Math.round(lerp(3, 6)),
    /** Cups since it last went home that make it sick: 5 … 8. */
    sickCups: Math.round(lerp(5, 8)),
    /** Shifts in a row without going home that make it sick: 2 … 4. */
    sickShifts: Math.round(lerp(2, 4)),
  };
}

/** What an agent is doing right now, for its needs. `wait` = waiting on you (no drain). */
export type RestActivity = 'work' | 'idle' | 'wait' | 'nap' | 'coffee' | 'water' | 'snack' | 'toast' | 'heat' | 'cook' | 'console' | 'table' | 'library';

/** How hard each activity draws on its energy, as a share of working (the rest restores). */
const LOAD: Record<RestActivity, number> = { work: 1, idle: 0.2, wait: 0, nap: 0, coffee: 0.2, water: 0.2, snack: 0.2, toast: 0.2, heat: 0.2, cook: 0.3, console: 0.5, table: 0.6, library: 0.2 };
/** Morale per minute while doing it (0 … 1). */
const MORALE: Record<RestActivity, number> = { work: 0, idle: 0, wait: 0, nap: 0.02, coffee: 0.01, water: 0.01, snack: 0.01, toast: 0.01, heat: 0.01, cook: 0.02, console: 0.02, table: 0.02, library: 0.01 };

/** The activity for what the office is showing: a station id (or null at the desk) and a pose. */
export function activityFor(stationId: string | null, pose: string): RestActivity {
  if (pose === 'sleep' || stationId === 'nap.armchair') return 'nap';
  switch (stationId) {
    case 'kitchen':
      return 'coffee';
    case 'water':
      return 'water';
    case 'fridge':
      return 'snack';
    case 'toaster':
      return 'toast';
    case 'microwave':
      return 'heat';
    case 'stove':
      return 'cook';
    case 'games.console':
      return 'console';
    case 'games.pool':
    case 'games.foosball':
    case 'games.pingpong':
      return 'table';
    case 'library.chair':
      return 'library';
  }
  return ['type', 'read', 'think', 'phone', 'review'].includes(pose) ? 'work' : 'idle';
}

interface Home {
  since: number;
  until: number;
  /** Energy it left with (a call-in keeps half of what it banked since). */
  from: number;
  sick: boolean;
}

interface Mind {
  morale: number;
  moraleAt: number;
  energy: number;
  /** 1 fed / watered … 0. */
  hunger: number;
  thirst: number;
  /** When energy and morale were last brought up to date (null = never). */
  restAt: number | null;
  activity: RestActivity;
  moodlets: Moodlet[];
  /** Checks failed in a row (a pass resets it). */
  fails: number;
  /** The most energy it can have this shift. */
  cap: number;
  /** When it had each cup still in its blood. */
  cups: number[];
  /** Cups since it last went home. */
  cupsSinceHome: number;
  /** Energy drained since it last went home (1 = a whole bar: a shift). */
  worked: number;
  /** Sick: it must go home for a long rest. */
  sick: boolean;
  home: Home | null;
  /** Hunger when it started cooking (a Good meal once it is done). */
  cookFrom: number | null;
  /** The away_until it came back from (a stale agent row must not send it home again). */
  cleared: number | null;
  /** Tired or rested, as last told to the core (null: the core's row already agrees). */
  told: boolean | null;
}

/** What the office tells the core: home and back (`api.setAgentHome`), shifts and sicknesses
 *  (`api.noteRest`), and tired or rested again (`api.setAgentTired`: the Lead prefers the rested). */
export interface SimsOutbox {
  home: { id: string; until: number | null }[];
  rest: { id: string; sick: boolean }[];
  tired: { id: string; tired: boolean }[];
  /** Two agents who spent a break together (`api.noteHangout`: friends). */
  hangouts: { a: string; b: string }[];
}

/** What keeps an agent from going home, for `SimsTracker.tick`: false, true (you talk to it), or
 *  'task': a queued task started while it was at home, which the core does only when nobody
 *  else on the team was at work (scheduler.rs `CALLED_IN`). */
export type Busy = boolean | 'task';
/** The Overtime moodlet's why when a task called it in. */
export const CALLED_IN = 'Called in: nobody else was free';

/** Failures in a row that make it Frustrated. */
const FRUSTRATED_AT = 2;

const clamp = (x: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, x));

/** The open office's tracker, for Settings → Developer (null while no office is open). */
export const live: { sims: SimsTracker | null } = { sims: null };

export class SimsTracker {
  private minds = new Map<string, Mind>();
  private agents = new Map<string, Agent>();
  private tasks = new Map<string, Task>();
  /** Pairs on a break together now, since when; and when each pair last counted as a hangout. */
  private with = new Map<string, number>();
  private hung = new Map<string, number>();
  private out: SimsOutbox = { home: [], rest: [], tired: [], hangouts: [] };

  setAgents(list: Agent[], now = Date.now()) {
    const before = this.agents;
    this.agents = new Map(list.map((a) => [a.id, a]));
    for (const a of list) {
      this.adopt(a, now);
      this.wanted(before.get(a.id), a);
    }
  }

  /** Its want for the day just fulfilled (the core's career says so, with the bonus XP): a lift. */
  private wanted(before: Agent | undefined, a: Agent) {
    const w = a.career?.want;
    if (!before || !w?.done_at || before.career?.want?.done_at === w.done_at) return;
    this.feel(a.id, w.done_at, 0.2, { key: 'want', label: 'Want fulfilled', why: w.label, tone: 'good', icon: 'check', emoji: '🌟', ms: 30 * 60_000 });
  }

  setTasks(list: Task[]) {
    this.tasks = new Map(list.map((t) => [t.id, t]));
  }

  /** Times together, saved on its career (reviews, plans shipped together, breaks). */
  bond(a: string, b: string): number {
    return this.agents.get(a)?.career?.friends?.[b] ?? 0;
  }

  /** Who is on a break together now (breakroom.ts `together`): after a while they hang out,
   *  which lifts both (more with a friend) and tells the core (they get to know each other). */
  together(groups: string[][], now: number) {
    const seen = new Set<string>();
    for (const g of groups)
      for (let i = 0; i < g.length; i++)
        for (let j = i + 1; j < g.length; j++) {
          const [a, b] = [g[i], g[j]].sort();
          const k = `${a}|${b}`;
          seen.add(k);
          const since = this.with.get(k) ?? now;
          this.with.set(k, since);
          if (now - since < HANG_MS || now - (this.hung.get(k) ?? -Infinity) < HANG_AGAIN_MS) continue;
          this.hung.set(k, now);
          this.out.hangouts.push({ a, b });
          for (const [x, y] of [[a, b], [b, a]]) {
            const name = this.agents.get(y)?.name ?? 'someone';
            const friend = this.bond(x, y) >= FRIEND_AT;
            this.feel(x, now, friend ? 0.1 : 0.05, friend ? { key: 'company', label: 'Good company', why: `A break with its friend ${name}`, tone: 'good', icon: 'users', emoji: '😄', ms: 15 * 60_000 } : { key: 'company', label: 'Getting to know someone', why: `A break with ${name}`, tone: 'good', icon: 'users', ms: 10 * 60_000 });
          }
        }
    for (const k of [...this.with.keys()]) if (!seen.has(k)) this.with.delete(k);
  }

  private mind(id: string): Mind {
    let m = this.minds.get(id);
    if (!m)
      this.minds.set(
        id,
        (m = {
          morale: BASE_MORALE,
          moraleAt: 0,
          energy: 1,
          hunger: 1,
          thirst: 1,
          restAt: null,
          activity: 'idle',
          moodlets: [],
          fails: 0,
          cap: 1,
          cups: [],
          cupsSinceHome: 0,
          worked: 0,
          sick: false,
          home: null,
          cookFrom: null,
          cleared: null,
          told: null,
        }),
      );
    return m;
  }

  private isLead(id: string) {
    return this.agents.get(id)?.role === 'lead';
  }

  private shift(id: string) {
    return shiftOf((this.agents.get(id)?.career?.stamina ?? 0) / 100);
  }

  /** The core says it is at home (a restart, another window): it is, until then. */
  private adopt(a: Agent, now: number) {
    const until = a.away_until;
    if (until == null || until <= now || a.role === 'lead') return;
    const m = this.mind(a.id);
    if (m.home || m.cleared === until) return;
    this.advance(a.id, m, now);
    m.home = { since: now, until, from: m.energy, sick: false };
  }

  private moraleNow(m: Mind, now: number): number {
    if (!m.moraleAt) return m.morale;
    return BASE_MORALE + (m.morale - BASE_MORALE) * Math.exp(-(now - m.moraleAt) / MORALE_TAU);
  }

  private caffeine(m: Mind, now: number): number {
    m.cups = m.cups.filter((t) => now - t < CAFFEINE_MS);
    return m.cups.length;
  }

  /** Bring energy and morale up to `now` at the current activity's rates (a minute at a time,
   *  so a skip of hours crosses Tired and Exhausted where it should). */
  private advance(id: string, m: Mind, now: number) {
    if (m.restAt == null) {
      m.restAt = now;
      return;
    }
    if (now <= m.restAt) return;
    const sh = this.shift(id);
    const lead = this.isLead(id);
    let t = m.restAt;
    while (t < now) {
      const dt = Math.min(60_000, now - t);
      const min = dt / 60_000;
      t += dt;
      let mood = 0.01;
      if (m.home) {
        m.energy = Math.min(m.cap, m.energy + min / sh.homeMinutes);
        m.hunger = m.thirst = 1; // ponytail: dinner at home is instant
      } else {
        const a = m.activity;
        // A nap rests at a third of home speed; the Lead rests at its desk when nothing runs.
        const rests = a === 'nap' || (lead && (a === 'idle' || a === 'wait'));
        const d = rests ? min / (sh.homeMinutes * 3) : (-LOAD[a] * sh.drainPerHour * moodDrain(this.moraleNow(m, t)) * min) / 60;
        if (d < 0) m.worked += Math.min(m.energy, -d);
        // The Lead never goes home: it eats at its desk while it rests.
        if (rests && lead) m.hunger = m.thirst = 1;
        else {
          m.hunger = clamp(m.hunger - (LOAD[a] * HUNGER_PER_HOUR * min) / 60);
          m.thirst = clamp(m.thirst - (LOAD[a] * THIRST_PER_HOUR * min) / 60);
        }
        const f = FEED[a];
        if (f?.hunger && m.hunger < f.hunger[1]) m.hunger = Math.min(f.hunger[1], m.hunger + f.hunger[0] * min);
        if (f?.thirst && m.thirst < f.thirst[1]) m.thirst = Math.min(f.thirst[1], m.thirst + f.thirst[0] * min);
        m.energy = clamp(m.energy + d, 0, m.cap);
        if (m.energy < 1e-9) m.energy = 0; // empty is empty (no float dust)
        mood = MORALE[a];
        if (m.energy < TIRED) mood -= 0.01;
        if (m.hunger < PECKISH) mood -= m.hunger < STARVING ? 0.02 : 0.01;
        if (m.thirst < PECKISH) mood -= m.thirst < STARVING ? 0.02 : 0.01;
        if (this.caffeine(m, t) >= sh.jitteryAt) mood -= 0.015;
      }
      m.morale = clamp(this.moraleNow(m, t) + mood * min);
      m.moraleAt = t;
    }
    m.restAt = now;
    if (!lead && !m.home && m.worked >= sh.sickShifts - 1e-9) this.fallSick(id, m, now);
  }

  /** The agent now does `activity` (the office calls this on each sync; a change is what counts). */
  setActivity(id: string, now: number, activity: RestActivity) {
    const m = this.mind(id);
    this.advance(id, m, now);
    if (activity === 'coffee' && m.activity !== 'coffee' && !m.home) this.drink(id, m, now);
    if (activity === 'cook' && m.activity !== 'cook') m.cookFrom = m.hunger;
    if (activity !== 'cook' && m.activity === 'cook' && m.cookFrom != null) {
      if (m.hunger - m.cookFrom >= GOOD_MEAL) this.feel(id, now, 0.1, { key: 'meal', label: 'Good meal', why: 'Cooked a real lunch at the stove', tone: 'good', icon: 'coffee', emoji: '🍝', ms: 20 * 60_000 });
      m.cookFrom = null;
    }
    m.activity = activity;
  }

  /** A cup: +15% now, a cup of caffeine for an hour; too many make it jittery, then sick. */
  private drink(id: string, m: Mind, now: number) {
    if (m.cups.length && now - m.cups[m.cups.length - 1] < 60_000) return; // the same cup
    const sh = this.shift(id);
    m.energy = Math.min(m.cap, m.energy + CUP);
    m.cups.push(now);
    m.cupsSinceHome++;
    if (this.caffeine(m, now) === sh.jitteryAt) this.feel(id, now, -0.08);
    if (!this.isLead(id) && m.cupsSinceHome >= sh.sickCups) this.fallSick(id, m, now);
  }

  private fallSick(id: string, m: Mind, now: number) {
    if (m.sick) return;
    m.sick = true;
    this.out.rest.push({ id, sick: true });
    this.feel(id, now, -0.15);
  }

  private feel(id: string, now: number, delta: number, moodlet?: Omit<Moodlet, 'at' | 'until'> & { ms?: number }) {
    const m = this.mind(id);
    // An event from the replayed log doesn't run the energy clock from back then.
    if (m.restAt != null) this.advance(id, m, now);
    m.morale = clamp(this.moraleNow(m, now) + delta);
    m.moraleAt = now;
    if (moodlet) {
      m.moodlets = m.moodlets.filter((x) => x.key !== moodlet.key);
      m.moodlets.push({ ...moodlet, at: now, until: now + (moodlet.ms ?? MOODLET_MS) });
    }
  }

  // ───────────── home and back ─────────────

  /** Send it home (you, or itself when exhausted or sick). False for the Lead, or already home. */
  sendHome(id: string, now: number): boolean {
    const m = this.mind(id);
    if (this.isLead(id) || m.home) return false;
    this.advance(id, m, now);
    const sh = this.shift(id);
    const sick = m.sick;
    if (!sick && m.worked >= SHIFT_DONE) this.out.rest.push({ id, sick: false });
    m.cap = sick ? SICK_CAP : 1;
    const ms = sick ? SICK_MS : Math.max(60_000, (m.cap - m.energy) * sh.homeMinutes * 60_000);
    m.home = { since: now, until: now + Math.round(ms), from: m.energy, sick };
    m.worked = 0;
    m.cupsSinceHome = 0;
    this.out.home.push({ id, until: m.home.until });
    return true;
  }

  /** Call it in early: Overtime, morale −0.15, and it keeps half the energy it banked at home.
   *  `why`: you clicked Call in, or (CALLED_IN) a task needed it. */
  callIn(id: string, now: number, why = 'Called in early from home'): boolean {
    const m = this.mind(id);
    if (!m.home) return false;
    this.advance(id, m, now);
    const h = m.home;
    m.energy = h.from + Math.max(0, m.energy - h.from) / 2;
    this.leaveHome(id, m);
    this.feel(id, now, OVERTIME_MORALE, { key: 'rest', label: 'Overtime', why, tone: 'bad', icon: 'battery', emoji: '😩', ms: 30 * 60_000 });
    return true;
  }

  private back(id: string, m: Mind, now: number) {
    this.advance(id, m, now);
    this.leaveHome(id, m);
    this.feel(id, now, 0.05, { key: 'rest', label: 'Rested', why: 'Back from home', tone: 'good', icon: 'battery', ms: 10 * 60_000 });
  }

  private leaveHome(id: string, m: Mind) {
    m.cleared = m.home?.until ?? null;
    m.home = null;
    m.sick = false;
    this.out.home.push({ id, until: null });
  }

  /** Send the exhausted and the sick home once their task is done, bring them back when rested,
   *  call in one that started working at home (you talked to it, or its task needed it: see
   *  `Busy`), and tell the core who is tired. The office calls this every couple of seconds.
   *  `busy`: working on something now; `present`: in the office at all. */
  tick(now: number, busy: (id: string) => Busy, present: (id: string) => boolean = () => true) {
    for (const a of this.agents.values()) {
      if (a.archived || a.role === 'lead') continue;
      const m = this.mind(a.id);
      this.tellTired(a, m);
      if (m.home) {
        // Rested first: a task the core starts as it comes back is no call-in.
        const b = busy(a.id);
        if (now >= m.home.until) this.back(a.id, m, now);
        else if (b) this.callIn(a.id, now, b === 'task' ? CALLED_IN : undefined);
        continue;
      }
      // Not in the office (offline): nothing drains it meanwhile.
      if (!present(a.id)) {
        this.setActivity(a.id, now, 'wait');
        continue;
      }
      this.advance(a.id, m, now);
      if ((m.energy <= 0 || m.sick) && !busy(a.id)) this.sendHome(a.id, now);
      this.tellTired(a, m);
    }
  }

  /** Tired (under 30%, not at home) or rested again, when the core's row says otherwise: once
   *  per change (the core keeps it on the agent as `tired_since`). */
  private tellTired(a: Agent, m: Mind) {
    const tired = !m.home && m.energy < TIRED;
    if (tired === (a.tired_since != null)) m.told = null;
    else if (m.told !== tired) {
      m.told = tired;
      this.out.tired.push({ id: a.id, tired });
    }
  }

  /** What to tell the core since the last call. */
  drain(): SimsOutbox {
    const o = this.out;
    this.out = { home: [], rest: [], tired: [], hangouts: [] };
    return o;
  }

  // ───────────── dev hooks (Settings → Developer) ─────────────

  /** Set an agent's energy (0 … 1): fill it (1) or drain it (0). */
  setEnergy(id: string, now: number, energy: number) {
    const m = this.mind(id);
    this.advance(id, m, now);
    m.energy = clamp(energy, 0, m.cap);
  }

  /** Skip time by `minutes`: energy, caffeine, home and moodlet timers all move on. */
  skip(minutes: number, now: number) {
    const ms = Math.max(0, minutes) * 60_000;
    for (const [id, m] of this.minds) {
      if (m.restAt != null) m.restAt -= ms;
      if (m.moraleAt) m.moraleAt -= ms;
      m.cups = m.cups.map((t) => t - ms);
      for (const x of m.moodlets) {
        x.at -= ms;
        if (x.until != null) x.until -= ms;
      }
      if (m.home) {
        m.home.since -= ms;
        m.home.until -= ms;
        this.out.home.push({ id, until: m.home.until });
      }
      this.advance(id, m, now);
    }
  }

  // ───────────── saved between sessions ─────────────

  /** Energy, coffee and home, to keep across a reload (a reload is not a night's sleep). */
  save(): string {
    const minds: Record<string, unknown> = {};
    for (const [id, m] of this.minds) {
      const { energy, hunger, thirst, restAt, cap, cups, cupsSinceHome, worked, sick, home } = m;
      minds[id] = { energy, hunger, thirst, restAt, cap, cups, cupsSinceHome, worked, sick, home };
    }
    return JSON.stringify({ v: 1, minds });
  }

  /** Back from `save`. Time with the office closed counts only for agents at home. */
  load(json: string | null, now: number) {
    if (!json) return;
    let data: { v?: number; minds?: Record<string, Partial<Mind>> };
    try {
      data = JSON.parse(json);
    } catch {
      return;
    }
    if (data?.v !== 1 || !data.minds) return;
    for (const [id, s] of Object.entries(data.minds)) {
      if (!s || typeof s.energy !== 'number') continue;
      const m = this.mind(id);
      m.energy = clamp(s.energy);
      m.hunger = typeof s.hunger === 'number' ? clamp(s.hunger) : 1;
      m.thirst = typeof s.thirst === 'number' ? clamp(s.thirst) : 1;
      m.cap = typeof s.cap === 'number' ? clamp(s.cap) : 1;
      m.cups = Array.isArray(s.cups) ? s.cups.filter((t) => typeof t === 'number') : [];
      m.cupsSinceHome = Number(s.cupsSinceHome) || 0;
      m.worked = Number(s.worked) || 0;
      m.sick = !!s.sick;
      const h = s.home;
      m.home = h && typeof h.until === 'number' ? { since: Number(h.since) || now, until: h.until, from: Number(h.from) || 0, sick: !!h.sick } : null;
      m.restAt = m.home && typeof s.restAt === 'number' ? Math.min(now, s.restAt) : now;
    }
  }

  ingest(ev: EventRow) {
    const id = ev.agent_id;
    const now = ev.ts;
    const k = ev.kind;
    switch (k.type) {
      case 'tool_completed': {
        if (!id) return;
        const pass = k.tests_passed ?? 0;
        const fail = k.tests_failed ?? 0;
        if (fail > 0) this.failed(id, now, { key: 'tests', label: 'Tests failing', why: `${fail} failing`, tone: 'bad', icon: 'x-circle' });
        else if (pass > 0) this.passed(id, now, { key: 'tests', label: 'Tests passed', why: `${pass} passing`, tone: 'good', icon: 'check' });
        return;
      }
      case 'verified': {
        const who = this.workerOf(ev);
        if (!who) return;
        if (k.ok) this.passed(who, now, { key: 'tests', label: 'Checks passed', why: k.command, tone: 'good', icon: 'check' });
        else this.failed(who, now, { key: 'tests', label: 'Checks failing', why: k.command, tone: 'bad', icon: 'x-circle' });
        return;
      }
      case 'reviewed': {
        const who = this.workerOf(ev);
        if (who && who !== id && k.verdict === 'approve')
          this.feel(who, now, 0.12, { key: 'review', label: 'Approved', why: 'Its work passed review', tone: 'good', icon: 'check', emoji: '👍' });
        return;
      }
      case 'merged': {
        const who = this.workerOf(ev);
        if (who) this.feel(who, now, 0.25, { key: 'proud', label: 'Proud', why: `Its work landed on ${k.branch}`, tone: 'good', icon: 'check', emoji: '🏆', ms: 15 * 60_000 });
        return;
      }
      case 'task_changed': {
        const t = k.task;
        const prev = this.tasks.get(t.id);
        this.tasks.set(t.id, t);
        const who = t.assignee;
        if (!who || prev?.status === t.status) return;
        if (t.status === 'done') this.feel(who, now, 0.2, { key: 'task', label: 'Task done', why: t.title, tone: 'good', icon: 'check', emoji: '🎉', ms: 10 * 60_000 });
        else if (t.status === 'failed') this.feel(who, now, -0.25, { key: 'task', label: 'Task failed', why: t.title, tone: 'bad', icon: 'x-circle', emoji: '😞' });
        else if (t.status === 'running' && (t.attempt ?? 0) > 1 && /review|changes/i.test(t.status_reason ?? ''))
          this.feel(who, now, -0.12, { key: 'review', label: 'Changes requested', why: t.title, tone: 'bad', icon: 'message' });
        return;
      }
      case 'agent_changed':
        this.wanted(this.agents.get(k.agent.id), k.agent);
        this.agents.set(k.agent.id, k.agent);
        return;
    }
  }

  /** Whose work an event is about: the task's assignee, else the agent itself. */
  private workerOf(ev: EventRow): string | null {
    return (ev.task_id ? this.tasks.get(ev.task_id)?.assignee : null) ?? ev.agent_id;
  }

  private passed(id: string, now: number, m: Omit<Moodlet, 'at' | 'until'>) {
    this.mind(id).fails = 0;
    this.feel(id, now, 0.1, m);
  }

  /** A failure; the second in a row makes it Frustrated (a pass resets the streak). */
  private failed(id: string, now: number, m: Omit<Moodlet, 'at' | 'until'>) {
    const n = ++this.mind(id).fails;
    if (n < FRUSTRATED_AT) return this.feel(id, now, -0.12, m);
    this.feel(id, now, -0.18, { key: 'tests', label: 'Frustrated', why: `${n} failures in a row`, tone: 'bad', icon: 'x-circle', emoji: '😤' });
  }

  /** The sheet for one agent now. `state` = the office's live view of it (approvals, limits). */
  stats(agentId: string, now: number, state: AgentState | null): SimSheet {
    const m = this.mind(agentId);
    const agent = this.agents.get(agentId) ?? null;
    const lead = agent?.role === 'lead';
    // Needs.
    this.advance(agentId, m, now);
    const energy = m.energy;
    const u = agent?.usage;
    const focus = u?.context_used != null && u.context_window ? Math.max(0, 1 - u.context_used / u.context_window) : null;
    const morale = this.moraleNow(m, now);
    const caffeine = this.caffeine(m, now);
    const jittery = caffeine >= this.shift(agentId).jitteryAt;
    const exhausted = energy <= 0 && !m.home;
    // Moodlets: the live ones from its state, then the remembered ones still fresh.
    const live: Moodlet[] = [];
    if (state?.approval) live.push({ key: 'approval', label: 'Waiting on you', why: state.approval.text, tone: 'bad', icon: 'hand', at: state.approval.since, until: null });
    if (state?.rateLimited) live.push({ key: 'limit', label: 'Out of quota', why: 'Its account hit a usage limit', tone: 'bad', icon: 'moon', at: state.rateLimited.since, until: state.rateLimited.resetsAt });
    if (state?.blocked) live.push({ key: 'blocked', label: 'Stuck', why: state.blocked.reason, tone: 'bad', icon: 'alert', at: state.blocked.since, until: null });
    if (m.home) {
      const min = Math.max(1, Math.ceil((m.home.until - now) / 60_000));
      live.push({ key: 'home', label: 'Home', why: `Back in ${min} min`, tone: 'neutral', icon: 'home', emoji: '🏠', at: now, until: m.home.until });
    }
    if (m.sick) live.push({ key: 'sick', label: 'Sick', why: 'Too much coffee, not enough sleep', tone: 'bad', icon: 'alert', emoji: '🤒', at: now, until: null });
    else if (exhausted) live.push({ key: 'tired', label: 'Exhausted', why: lead ? 'Running on empty' : 'Goes home once its task is done', tone: 'bad', icon: 'battery', emoji: '😵', at: now, until: null });
    else if (energy < TIRED && !m.home) live.push({ key: 'tired', label: 'Tired', why: `${Math.round(energy * 100)}% energy left`, tone: 'bad', icon: 'battery', emoji: '😴', at: now, until: null });
    if (jittery && !m.home) live.push({ key: 'jittery', label: 'Jittery', why: `${caffeine} cups of coffee in the last hour`, tone: 'bad', icon: 'coffee', emoji: '⚡', at: now, until: null });
    if (m.hunger < PECKISH && !m.home) live.push({ key: 'hungry', label: m.hunger < STARVING ? 'Starving' : 'Hungry', why: `${Math.round(m.hunger * 100)}% fed: a meal at the stove or the microwave`, tone: 'bad', icon: 'coffee', emoji: '🍔', at: now, until: null });
    if (m.thirst < PECKISH && !m.home) live.push({ key: 'thirsty', label: m.thirst < STARVING ? 'Parched' : 'Thirsty', why: `${Math.round(m.thirst * 100)}% watered: a glass at the cooler`, tone: 'bad', icon: 'coffee', emoji: '💧', at: now, until: null });
    if (focus != null && focus < 0.2) live.push({ key: 'foggy', label: 'Foggy', why: 'Its context window is nearly full', tone: 'bad', icon: 'cloud', emoji: '🌫️', at: now, until: null });
    m.moodlets = m.moodlets.filter((x) => x.until == null || x.until > now);
    const keys = new Set(live.map((x) => x.key));
    const moodlets = [...live, ...m.moodlets.filter((x) => !keys.has(x.key))].sort((a, b) => b.at - a.at);
    // Relationships: the times together saved on its career (they survive restarts).
    const relations = Object.entries(agent?.career?.friends ?? {})
      .filter(([, n]) => n > 0)
      .map(([other, n]) => ({ agentId: other, score: friendScore(n), label: bondLabel(n), why: `${n} time${n === 1 ? '' : 's'} together: reviews, plans shipped, breaks` }))
      .sort((a, b) => b.score - a.score);
    const rest: RestState = { home: m.home && { until: m.home.until, sick: m.home.sick }, caffeine, jittery, sick: m.sick, exhausted, cap: m.cap, canGoHome: !lead };
    return { needs: { energy, focus, morale, hunger: m.hunger, thirst: m.thirst }, moodlets, relations, plumbob: plumbobOf(state), mood: moodWord(morale, moodlets), effects: lead ? [] : moodEffects(morale), rest };
  }
}

export function plumbobOf(a: AgentState | null): PlumbobColor {
  if (!a) return 'grey';
  if (a.approval) return 'amber';
  if (a.blocked || a.lastError) return 'red';
  if (a.rateLimited) return 'blue';
  if (a.status === 'working' || a.status === 'thinking' || a.tools.size || a.thinkingSince != null || a.replyingSince != null) return 'green';
  return 'grey';
}

function moodWord(morale: number, moodlets: Moodlet[]): string {
  const bad = moodlets.filter((m) => m.tone === 'bad').length;
  if (bad >= 2 || morale < 0.35) return 'Tense';
  if (morale > 0.82) return 'Inspired';
  if (morale > 0.7) return 'Happy';
  if (bad) return 'Uneasy';
  return 'Fine';
}

/** The feeling its name tag shows: the newest one worth seeing from afar. */
export function headline(sheet: SimSheet): Moodlet | null {
  return sheet.moodlets.find((m) => m.emoji) ?? null;
}
