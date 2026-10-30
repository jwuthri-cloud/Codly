// The Sims layer: each agent's needs, moodlets, skills and relationships, all from real data.
//   Energy  = Codly's own need: drains while it works, refilled by a nap or coffee
//   Focus   = context window left (its latest turn)
//   Morale  = how its recent work went (checks, merges, reviews), refilled by games and reading,
//             drifting back to normal
// Energy and Morale also move with what the agent does (`setActivity`): a nap, a PS5 session…
// Moodlets are short-lived feelings from real events ("Tests passed", "Waiting on you");
// relationships are the reviews saved on its career (`career.friends`). Skills are on the
// career too (SimsHud reads them). Nothing here is invented: no event, no change.
//
// Pure and stateful: feed it events (`ingest`), ask for an agent's sheet (`stats`).

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

export interface SimSheet {
  needs: { energy: number; focus: number | null; morale: number };
  moodlets: Moodlet[];
  relations: Relation[];
  plumbob: PlumbobColor;
  /** One word for the overall mood. */
  mood: string;
}

const BASE_MORALE = 0.65;
/** Morale drifts back to normal with this time constant (ms). */
const MORALE_TAU = 10 * 60_000;
const MOODLET_MS = 6 * 60_000;

/** Skill level from XP: 1, 4, 9, 16 … XP per level, up to 10 (like a Sims skill bar). */
export function levelOf(xp: number): { level: number; progress: number } {
  const level = Math.min(10, Math.floor(Math.sqrt(xp)));
  if (level >= 10) return { level, progress: 1 };
  const lo = level * level;
  const hi = (level + 1) * (level + 1);
  return { level, progress: (xp - lo) / (hi - lo) };
}

/** What an agent is doing right now, for its needs. */
export type RestActivity = 'work' | 'idle' | 'nap' | 'coffee' | 'console' | 'table' | 'library';

/** Per minute, while doing it. Energy and morale are 0 … 1. */
const RATES: Record<RestActivity, { energy: number; morale: number }> = {
  work: { energy: -0.02, morale: 0 },
  idle: { energy: -0.004, morale: 0 },
  nap: { energy: 0.08, morale: 0.02 },
  coffee: { energy: 0.03, morale: 0.01 },
  console: { energy: -0.01, morale: 0.02 },
  table: { energy: -0.012, morale: 0.02 },
  library: { energy: -0.004, morale: 0.01 },
};

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
      return 'table';
    case 'library.chair':
      return 'library';
  }
  return ['type', 'read', 'think', 'phone', 'review'].includes(pose) ? 'work' : 'idle';
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
}

/** Failures in a row that make it Frustrated. */
const FRUSTRATED_AT = 2;

export class SimsTracker {
  private minds = new Map<string, Mind>();
  private agents = new Map<string, Agent>();
  private tasks = new Map<string, Task>();

  setAgents(list: Agent[]) {
    this.agents = new Map(list.map((a) => [a.id, a]));
  }

  setTasks(list: Task[]) {
    this.tasks = new Map(list.map((t) => [t.id, t]));
  }

  private mind(id: string): Mind {
    let m = this.minds.get(id);
    if (!m)
      this.minds.set(
        id,
        (m = { morale: BASE_MORALE, moraleAt: 0, energy: 1, restAt: null, activity: 'idle', moodlets: [], fails: 0 }),
      );
    return m;
  }

  private moraleNow(m: Mind, now: number): number {
    if (!m.moraleAt) return m.morale;
    return BASE_MORALE + (m.morale - BASE_MORALE) * Math.exp(-(now - m.moraleAt) / MORALE_TAU);
  }

  /** Bring energy and morale up to `now` at the current activity's rates. */
  private advance(m: Mind, now: number) {
    if (m.restAt == null) {
      m.restAt = now;
      return;
    }
    const minutes = (now - m.restAt) / 60_000;
    if (minutes <= 0) return;
    const r = RATES[m.activity];
    m.energy = Math.min(1, Math.max(0, m.energy + r.energy * minutes));
    m.morale = Math.min(1, Math.max(0, this.moraleNow(m, now) + r.morale * minutes));
    m.moraleAt = now;
    m.restAt = now;
  }

  /** The agent now does `activity` (the office calls this on each sync; a change is what counts). */
  setActivity(id: string, now: number, activity: RestActivity) {
    const m = this.mind(id);
    this.advance(m, now);
    m.activity = activity;
  }

  private feel(id: string, now: number, delta: number, moodlet?: Omit<Moodlet, 'at' | 'until'> & { ms?: number }) {
    const m = this.mind(id);
    this.advance(m, now);
    m.morale = Math.min(1, Math.max(0, this.moraleNow(m, now) + delta));
    m.moraleAt = now;
    if (moodlet) {
      m.moodlets = m.moodlets.filter((x) => x.key !== moodlet.key);
      m.moodlets.push({ ...moodlet, at: now, until: now + (moodlet.ms ?? MOODLET_MS) });
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
    // Needs.
    this.advance(m, now);
    const energy = m.energy;
    const u = agent?.usage;
    const focus = u?.context_used != null && u.context_window ? Math.max(0, 1 - u.context_used / u.context_window) : null;
    const morale = this.moraleNow(m, now);
    // Moodlets: the live ones from its state, then the remembered ones still fresh.
    const live: Moodlet[] = [];
    if (state?.approval) live.push({ key: 'approval', label: 'Waiting on you', why: state.approval.text, tone: 'bad', icon: 'hand', at: state.approval.since, until: null });
    if (state?.rateLimited) live.push({ key: 'limit', label: 'Out of quota', why: 'Its account hit a usage limit', tone: 'bad', icon: 'moon', at: state.rateLimited.since, until: state.rateLimited.resetsAt });
    if (state?.blocked) live.push({ key: 'blocked', label: 'Stuck', why: state.blocked.reason, tone: 'bad', icon: 'alert', at: state.blocked.since, until: null });
    if (energy < 0.2) live.push({ key: 'tired', label: 'Tired', why: `${Math.round(energy * 100)}% energy left`, tone: 'bad', icon: 'battery', emoji: '😴', at: now, until: null });
    if (focus != null && focus < 0.2) live.push({ key: 'foggy', label: 'Foggy', why: 'Its context window is nearly full', tone: 'bad', icon: 'cloud', emoji: '🌫️', at: now, until: null });
    m.moodlets = m.moodlets.filter((x) => x.until == null || x.until > now);
    const moodlets = [...live, ...m.moodlets].sort((a, b) => b.at - a.at);
    // Relationships: the reviews saved on its career (they survive restarts).
    const relations = Object.entries(agent?.career?.friends ?? {})
      .filter(([, n]) => n > 0)
      .map(([other, n]) => ({ agentId: other, score: Math.min(100, Math.round(25 * Math.log2(1 + 2 * n))), why: `${n} review${n === 1 ? '' : 's'} between them` }))
      .sort((a, b) => b.score - a.score);
    return { needs: { energy, focus, morale }, moodlets, relations, plumbob: plumbobOf(state), mood: moodWord(morale, moodlets) };
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
