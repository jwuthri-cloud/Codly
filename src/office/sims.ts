// The Sims layer: each agent's needs, moodlets, skills and relationships, all from real data.
//   Energy  = Codly's own need, over a shift (PROGRESSION.md §5–7): real working time drains it
//             (~4 h from full to empty for a new hire, ~3× slower at stamina 100), idle time a
//             little, waiting on you not at all. Going home refills it in minutes; a nap on a
//             sofa at a third of that speed; a cup of coffee by 15% (and caffeine).
//   Focus   = context window left (its latest turn)
//   Morale  = how its recent work went (checks, merges, reviews), refilled by games and reading,
//             drifting back to normal
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
  /** "3 reviews between them". */
  why: string;
}

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
  needs: { energy: number; focus: number | null; morale: number };
  moodlets: Moodlet[];
  relations: Relation[];
  plumbob: PlumbobColor;
  /** One word for the overall mood. */
  mood: string;
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
export type RestActivity = 'work' | 'idle' | 'wait' | 'nap' | 'coffee' | 'console' | 'table' | 'library';

/** How hard each activity draws on its energy, as a share of working (the rest restores). */
const LOAD: Record<RestActivity, number> = { work: 1, idle: 0.2, wait: 0, nap: 0, coffee: 0.2, console: 0.5, table: 0.6, library: 0.2 };
/** Morale per minute while doing it (0 … 1). */
const MORALE: Record<RestActivity, number> = { work: 0, idle: 0, wait: 0, nap: 0.02, coffee: 0.01, console: 0.02, table: 0.02, library: 0.01 };

/** The activity for what the office is showing: a station id (or null at the desk) and a pose. */
export function activityFor(stationId: string | null, pose: string): RestActivity {
  if (pose === 'sleep' || stationId === 'nap.armchair') return 'nap';
  switch (stationId) {
    case 'kitchen':
      return 'coffee';
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
  /** The away_until it came back from (a stale agent row must not send it home again). */
  cleared: number | null;
}

/** What the office tells the core: home and back (`api.setAgentHome`), shifts and sicknesses (`api.noteRest`). */
export interface SimsOutbox {
  home: { id: string; until: number | null }[];
  rest: { id: string; sick: boolean }[];
}

/** Failures in a row that make it Frustrated. */
const FRUSTRATED_AT = 2;

const clamp = (x: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, x));

export class SimsTracker {
  private minds = new Map<string, Mind>();
  private agents = new Map<string, Agent>();
  private tasks = new Map<string, Task>();
  private out: SimsOutbox = { home: [], rest: [] };

  setAgents(list: Agent[], now = Date.now()) {
    this.agents = new Map(list.map((a) => [a.id, a]));
    for (const a of list) this.adopt(a, now);
  }

  setTasks(list: Task[]) {
    this.tasks = new Map(list.map((t) => [t.id, t]));
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
          cleared: null,
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
      if (m.home) m.energy = Math.min(m.cap, m.energy + min / sh.homeMinutes);
      else {
        const a = m.activity;
        // A nap rests at a third of home speed; the Lead rests at its desk when nothing runs.
        const rests = a === 'nap' || (lead && (a === 'idle' || a === 'wait'));
        const d = rests ? min / (sh.homeMinutes * 3) : (-LOAD[a] * sh.drainPerHour * min) / 60;
        if (d < 0) m.worked += Math.min(m.energy, -d);
        m.energy = clamp(m.energy + d, 0, m.cap);
        if (m.energy < 1e-9) m.energy = 0; // empty is empty (no float dust)
        mood = MORALE[a];
        if (m.energy < TIRED) mood -= 0.01;
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

  /** Call it in early: Overtime, morale −0.15, and it keeps half the energy it banked at home. */
  callIn(id: string, now: number): boolean {
    const m = this.mind(id);
    if (!m.home) return false;
    this.advance(id, m, now);
    const h = m.home;
    m.energy = h.from + Math.max(0, m.energy - h.from) / 2;
    this.leaveHome(id, m);
    this.feel(id, now, OVERTIME_MORALE, { key: 'rest', label: 'Overtime', why: 'Called in early from home', tone: 'bad', icon: 'battery', emoji: '😩', ms: 30 * 60_000 });
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
   *  and call in one that started working at home (you gave it work). The office calls this
   *  every couple of seconds. `busy`: working on something now; `present`: in the office at all. */
  tick(now: number, busy: (id: string) => boolean, present: (id: string) => boolean = () => true) {
    for (const a of this.agents.values()) {
      if (a.archived || a.role === 'lead') continue;
      const m = this.mind(a.id);
      if (m.home) {
        if (busy(a.id)) this.callIn(a.id, now);
        else if (now >= m.home.until) this.back(a.id, m, now);
        continue;
      }
      // Not in the office (offline): nothing drains it meanwhile.
      if (!present(a.id)) {
        this.setActivity(a.id, now, 'wait');
        continue;
      }
      this.advance(a.id, m, now);
      if ((m.energy <= 0 || m.sick) && !busy(a.id)) this.sendHome(a.id, now);
    }
  }

  /** What to tell the core since the last call. */
  drain(): SimsOutbox {
    const o = this.out;
    this.out = { home: [], rest: [] };
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
      const { energy, restAt, cap, cups, cupsSinceHome, worked, sick, home } = m;
      minds[id] = { energy, restAt, cap, cups, cupsSinceHome, worked, sick, home };
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
    if (focus != null && focus < 0.2) live.push({ key: 'foggy', label: 'Foggy', why: 'Its context window is nearly full', tone: 'bad', icon: 'cloud', emoji: '🌫️', at: now, until: null });
    m.moodlets = m.moodlets.filter((x) => x.until == null || x.until > now);
    const keys = new Set(live.map((x) => x.key));
    const moodlets = [...live, ...m.moodlets.filter((x) => !keys.has(x.key))].sort((a, b) => b.at - a.at);
    // Relationships: the reviews saved on its career (they survive restarts).
    const relations = Object.entries(agent?.career?.friends ?? {})
      .filter(([, n]) => n > 0)
      .map(([other, n]) => ({ agentId: other, score: Math.min(100, Math.round(25 * Math.log2(1 + 2 * n))), why: `${n} review${n === 1 ? '' : 's'} between them` }))
      .sort((a, b) => b.score - a.score);
    const rest: RestState = { home: m.home && { until: m.home.until, sick: m.home.sick }, caffeine, jittery, sick: m.sick, exhausted, cap: m.cap, canGoHome: !lead };
    return { needs: { energy, focus, morale }, moodlets, relations, plumbob: plumbobOf(state), mood: moodWord(morale, moodlets), rest };
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
