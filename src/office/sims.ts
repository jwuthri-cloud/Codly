// The Sims layer: each agent's needs, moodlets, skills and relationships, all from real data.
//   Energy  = quota left on the account it runs on (its usage windows)
//   Focus   = context window left (its latest turn)
//   Morale  = how its recent work went (checks, merges, reviews), drifting back to normal
// Moodlets are short-lived feelings from real events ("Tests passed", "Waiting on you");
// skills grow with the files it changes and the checks it passes; relationships with the
// agents it asks, reviews or is reviewed by. Nothing here is invented: no event, no change.
//
// Pure and stateful: feed it events (`ingest`), ask for an agent's sheet (`stats`).

import type { AccountInfo } from '../lib/gen/AccountInfo';
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
  at: number;
  /** null = while the condition lasts. */
  until: number | null;
}

export interface Skill {
  name: string;
  level: number;
  /** 0 … 1 towards the next level. */
  progress: number;
}

export interface Relation {
  agentId: string;
  /** 0 … 100. */
  score: number;
  /** "Reviewed 3 of their changes", "Asked them 2 questions". */
  why: string;
}

export type PlumbobColor = 'green' | 'amber' | 'red' | 'blue' | 'grey';

export interface SimSheet {
  needs: { energy: number | null; focus: number | null; morale: number };
  moodlets: Moodlet[];
  skills: Skill[];
  relations: Relation[];
  plumbob: PlumbobColor;
  /** One word for the overall mood. */
  mood: string;
}

const BASE_MORALE = 0.65;
/** Morale drifts back to normal with this time constant (ms). */
const MORALE_TAU = 10 * 60_000;
const MOODLET_MS = 6 * 60_000;

const LANG: [RegExp, string][] = [
  [/\.(test|spec)\.[a-z]+$|_test\.(go|py|rs)$|(^|\/)tests?\//, 'Testing'],
  [/\.(ts|tsx)$/, 'TypeScript'],
  [/\.(js|jsx|mjs|cjs)$/, 'JavaScript'],
  [/\.svelte$|\.vue$/, 'UI'],
  [/\.(css|scss|less)$/, 'Styling'],
  [/\.rs$/, 'Rust'],
  [/\.py$/, 'Python'],
  [/\.go$/, 'Go'],
  [/\.(md|mdx|txt)$/, 'Writing'],
  [/\.(sql|prisma)$|migrations?\//, 'Databases'],
];

export function skillFor(path: string): string | null {
  for (const [re, name] of LANG) if (re.test(path)) return name;
  return null;
}

/** Skill level from XP: 1, 4, 9, 16 … XP per level, up to 10 (like a Sims skill bar). */
export function levelOf(xp: number): { level: number; progress: number } {
  const level = Math.min(10, Math.floor(Math.sqrt(xp)));
  if (level >= 10) return { level, progress: 1 };
  const lo = level * level;
  const hi = (level + 1) * (level + 1);
  return { level, progress: (xp - lo) / (hi - lo) };
}

interface Mind {
  morale: number;
  moraleAt: number;
  moodlets: Moodlet[];
  xp: Map<string, number>;
  /** other agent → [interactions, why counts]. */
  rel: Map<string, { asks: number; reviews: number; reviewed: number; together: number }>;
}

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
    if (!m) this.minds.set(id, (m = { morale: BASE_MORALE, moraleAt: 0, moodlets: [], xp: new Map(), rel: new Map() }));
    return m;
  }

  private moraleNow(m: Mind, now: number): number {
    if (!m.moraleAt) return m.morale;
    return BASE_MORALE + (m.morale - BASE_MORALE) * Math.exp(-(now - m.moraleAt) / MORALE_TAU);
  }

  private feel(id: string, now: number, delta: number, moodlet?: Omit<Moodlet, 'at' | 'until'> & { ms?: number }) {
    const m = this.mind(id);
    m.morale = Math.min(1, Math.max(0, this.moraleNow(m, now) + delta));
    m.moraleAt = now;
    if (moodlet) {
      m.moodlets = m.moodlets.filter((x) => x.key !== moodlet.key);
      m.moodlets.push({ ...moodlet, at: now, until: now + (moodlet.ms ?? MOODLET_MS) });
    }
  }

  private xp(id: string, skill: string, n: number) {
    const m = this.mind(id);
    m.xp.set(skill, (m.xp.get(skill) ?? 0) + n);
  }

  private relate(a: string, b: string, kind: 'asks' | 'reviews' | 'reviewed' | 'together') {
    if (!a || !b || a === b) return;
    const r = this.mind(a).rel;
    const cur = r.get(b) ?? { asks: 0, reviews: 0, reviewed: 0, together: 0 };
    cur[kind]++;
    r.set(b, cur);
  }

  ingest(ev: EventRow) {
    const id = ev.agent_id;
    const now = ev.ts;
    const k = ev.kind;
    switch (k.type) {
      case 'files_changed':
        if (!id) return;
        for (const p of k.paths) {
          const s = skillFor(p);
          if (s) this.xp(id, s, 1);
        }
        return;
      case 'tool_completed': {
        if (!id) return;
        const pass = k.tests_passed ?? 0;
        const fail = k.tests_failed ?? 0;
        if (fail > 0) this.feel(id, now, -0.12, { key: 'tests', label: 'Tests failing', why: `${fail} failing`, tone: 'bad', icon: 'x-circle' });
        else if (pass > 0) {
          this.feel(id, now, 0.1, { key: 'tests', label: 'Tests passed', why: `${pass} passing`, tone: 'good', icon: 'check' });
          this.xp(id, 'Testing', Math.min(5, Math.ceil(pass / 5)));
        }
        return;
      }
      case 'task_changed': {
        const t = k.task;
        const prev = this.tasks.get(t.id);
        this.tasks.set(t.id, t);
        const who = t.assignee;
        if (!who || prev?.status === t.status) return;
        if (t.status === 'done') {
          this.feel(who, now, 0.2, { key: 'task', label: 'Task done', why: t.title, tone: 'good', icon: 'check', ms: 10 * 60_000 });
          this.xp(who, 'Shipping', 2);
        } else if (t.status === 'failed') this.feel(who, now, -0.25, { key: 'task', label: 'Task failed', why: t.title, tone: 'bad', icon: 'x-circle' });
        else if (t.status === 'running' && (t.attempt ?? 0) > 1 && /review|changes/i.test(t.status_reason ?? ''))
          this.feel(who, now, -0.12, { key: 'review', label: 'Changes requested', why: t.title, tone: 'bad', icon: 'message' });
        return;
      }
      case 'agent_changed':
        this.agents.set(k.agent.id, k.agent);
        return;
      case 'approval_resolved':
        return;
      case 'chat': {
        // A worker asking the Lead, the Lead answering: they get to know each other.
        const author = k.message.author;
        if (!this.agents.has(author)) return;
        const lead = [...this.agents.values()].find((a) => a.role === 'lead' && (a.thread ?? 'lead') === k.message.thread);
        if (lead && lead.id !== author) {
          this.relate(author, lead.id, 'asks');
          this.relate(lead.id, author, 'asks');
        }
        return;
      }
      default:
        break;
    }
    // A reviewer's events on someone's task: reviewer and author get to know each other.
    if (id && ev.task_id) {
      const a = this.agents.get(id);
      const t = this.tasks.get(ev.task_id);
      if (a?.role === 'reviewer' && t?.assignee && (k.type === 'turn_completed' || k.type === 'message')) {
        this.relate(id, t.assignee, 'reviews');
        this.relate(t.assignee, id, 'reviewed');
        if (k.type === 'turn_completed') this.xp(id, 'Reviewing', 2);
      }
    }
  }

  /** The sheet for one agent now. `state` = the office's live view of it (approvals, limits). */
  stats(agentId: string, now: number, state: AgentState | null, accounts: AccountInfo[] = []): SimSheet {
    const m = this.mind(agentId);
    const agent = this.agents.get(agentId) ?? null;
    // Needs.
    let energy: number | null = null;
    const acc = agent?.account ? accounts.find((a) => a.id === agent.account) : accounts.find((a) => a.provider === agent?.provider);
    const windows = (acc?.limits ?? []).filter((w) => w.resets_at == null || w.resets_at > now);
    if (windows.length) energy = 1 - Math.max(...windows.map((w) => (w.status === 'rejected' ? 1 : w.used_fraction)));
    const u = agent?.usage;
    const focus = u?.context_used != null && u.context_window ? Math.max(0, 1 - u.context_used / u.context_window) : null;
    const morale = this.moraleNow(m, now);
    // Moodlets: the live ones from its state, then the remembered ones still fresh.
    const live: Moodlet[] = [];
    if (state?.approval) live.push({ key: 'approval', label: 'Waiting on you', why: state.approval.text, tone: 'bad', icon: 'hand', at: state.approval.since, until: null });
    if (state?.rateLimited) live.push({ key: 'limit', label: 'Out of quota', why: 'Its account hit a usage limit', tone: 'bad', icon: 'moon', at: state.rateLimited.since, until: state.rateLimited.resetsAt });
    if (state?.blocked) live.push({ key: 'blocked', label: 'Stuck', why: state.blocked.reason, tone: 'bad', icon: 'alert', at: state.blocked.since, until: null });
    if (energy != null && energy < 0.2) live.push({ key: 'tired', label: 'Tired', why: `${Math.round(energy * 100)}% of its quota left`, tone: 'bad', icon: 'battery', at: now, until: null });
    if (focus != null && focus < 0.2) live.push({ key: 'foggy', label: 'Foggy', why: 'Its context window is nearly full', tone: 'bad', icon: 'cloud', at: now, until: null });
    m.moodlets = m.moodlets.filter((x) => x.until == null || x.until > now);
    const moodlets = [...live, ...m.moodlets].sort((a, b) => b.at - a.at);
    // Skills, best first.
    const skills = [...m.xp]
      .map(([name, xp]) => ({ name, ...levelOf(xp) }))
      .filter((s) => s.level > 0 || s.progress > 0)
      .sort((a, b) => b.level - a.level || b.progress - a.progress);
    // Relationships.
    const relations = [...m.rel]
      .map(([other, r]) => {
        const n = r.asks + r.reviews * 2 + r.reviewed * 2 + r.together;
        const parts = [r.reviews && `reviewed ${r.reviews} of their changes`, r.reviewed && `reviewed by them ${r.reviewed}×`, r.asks && `${r.asks} questions between them`].filter(Boolean);
        return { agentId: other, score: Math.min(100, Math.round(25 * Math.log2(1 + n))), why: parts.join(', ') || 'worked together' };
      })
      .sort((a, b) => b.score - a.score);
    return { needs: { energy, focus, morale }, moodlets, skills, relations, plumbob: plumbobOf(state), mood: moodWord(morale, moodlets) };
  }

  /** Everything learned, for persistence or an export of the agent (its "save"). */
  xpOf(agentId: string): Record<string, number> {
    return Object.fromEntries(this.minds.get(agentId)?.xp ?? []);
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
