// The behaviour layer (design-world §9.4): from an agent's activity state (the reducer's
// model) to what its character should be doing — where (desk / a station / the door), in
// which pose, holding which prop, with which emote — and when that answer next changes by
// time alone (an excursion threshold, a fading check mark, long idle).
//
// Honesty contract:
//  1. ≤ 250 ms to truth: emotes, icons and the desk screen follow the model immediately.
//  2. Minimum dwell of 600 ms per pose (Dwell below): rapid events coalesce.
//  3. Excursions: walk to a station only if the activity is still live after its threshold.
//  4. No phantom work: idle agents sit still (or sleep after 10 minutes without events).
//  5. Errands only for real instant events (a commit, a memory, a card changing column).

import type { ActivityKind } from '../lib/gen/ActivityKind';
import type { StationId } from './layout';
import type { AgentState, ErrandKind, OfficeModel, ToolCall } from './model';
import { EXCURSION_MS } from './reducer';

export type Pose =
  | 'sit'
  | 'type'
  | 'read'
  | 'think'
  | 'phone'
  | 'sleep'
  | 'slump'
  | 'stand'
  | 'browse'
  | 'write'
  | 'watch'
  | 'raise'
  | 'celebrate'
  | 'drop'
  | 'talk'
  | 'review'
  | 'wave';

export type Prop = 'paper' | 'book' | 'card' | 'envelope' | 'folder' | 'phone' | 'marker' | 'note' | 'mug' | null;
export type Emote = 'alert' | 'question' | 'rain' | 'zz' | 'think' | 'check' | null;
export type Screen = 'off' | 'idle' | 'code' | 'doc' | 'terminal' | 'chat' | 'alert' | 'error' | 'think' | 'search';

export type Place = { kind: 'desk'; stand?: boolean } | { kind: 'station'; id: StationId } | { kind: 'lead'; leadId: string } | { kind: 'door' };

export interface Intent {
  place: Place;
  /** Pose on arrival (while walking the character walks, holding `prop`). */
  pose: Pose;
  prop: Prop;
  emote: Emote;
  screen: Screen;
  /** Overlay icon (the activity, or a state like approval / sleep). */
  icon: string;
  /** Running errand, if this intent is one. */
  errand: ErrandKind | null;
}

export const LONG_IDLE_MS = 10 * 60_000;
export const DONE_CHECK_MS = 2000;
export const CHEER_MS = 1700;
export const ACT_MS = 1500;
export const DWELL_MS = 600;
export const THINK_HOLD_MS = 1200;
const ERRAND_QUEUE_MS = 9000;
const ERRAND_MAX_MS = 22_000;

export function placeKey(p: Place): string {
  switch (p.kind) {
    case 'desk':
      return p.stand ? 'desk:stand' : 'desk';
    case 'station':
      return 'st:' + p.id;
    case 'lead':
      return 'lead:' + p.leadId;
    case 'door':
      return 'door';
  }
}

export function intentKey(i: Intent): string {
  return `${placeKey(i.place)}|${i.pose}|${i.prop ?? ''}`;
}

/** Stuck: a fatal error / failed turn, or a blocked status with nothing running. */
export function isBlocked(a: AgentState): boolean {
  return !!a.blocked || a.status === 'failed' || (a.status === 'blocked' && !a.tools.size);
}

/** Activity priority (§9.4): the highest live one wins the pose. */
const PRIORITY: Partial<Record<ActivityKind, number>> = {
  await_approval: 100,
  test: 60,
  execute: 58,
  edit: 50,
  create: 50,
  delete: 50,
  search: 40,
  read: 35,
  fetch: 32,
  mcp: 30,
  message: 25,
  delegate: 24,
  note: 22,
  memory: 21,
  plan: 20,
  review: 20,
  commit: 20,
  merge: 20,
};

/** The live tool that wins the pose: highest priority, latest on ties. */
export function dominant(a: AgentState): ToolCall | null {
  let best: ToolCall | null = null;
  let bp = -1;
  for (const t of a.tools.values()) {
    const p = PRIORITY[t.kind] ?? 10;
    if (p >= bp) {
      best = t;
      bp = p;
    }
  }
  return best;
}

const isMemoryTool = (t: ToolCall) => /memory_search|kg_query|memory_read/i.test(t.name + ' ' + (t.tool ?? ''));

/** Kinds whose live tool takes the agent to a station (when past its threshold). */
export function stationFor(t: ToolCall, a: AgentState, m: OfficeModel): Place | null {
  switch (t.kind) {
    case 'test':
      return { kind: 'station', id: 'server.monitor' };
    case 'execute':
      return { kind: 'station', id: 'server.rack' };
    case 'search':
      return { kind: 'station', id: isMemoryTool(t) ? 'archive' : 'library.shelf' };
    case 'fetch':
      return { kind: 'station', id: 'library.chair' };
    case 'plan':
    case 'note':
      return { kind: 'station', id: 'meeting.board' };
    case 'review':
      return { kind: 'station', id: 'meeting.table' };
    case 'commit':
    case 'merge':
      return { kind: 'station', id: 'mail' };
    case 'memory':
      return { kind: 'station', id: 'archive' };
    case 'message': {
      if (a.role === 'lead') return null;
      const lead = leadId(m, a);
      return lead ? { kind: 'lead', leadId: lead } : null;
    }
    default:
      return null;
  }
}

function leadId(m: OfficeModel, a: AgentState): string | null {
  const task = a.taskId ? m.tasks.get(a.taskId) : null;
  const thread = task?.planId ? m.plans.get(task.planId)?.thread : null;
  let first: string | null = null;
  for (const l of m.agents.values()) {
    if (l.role !== 'lead' || !l.present || l.id === a.id) continue;
    if (thread && l.thread === thread) return l.id;
    first ??= l.id;
  }
  return first;
}

/** Is a reviewer currently reviewing (its task is in review)? Then it works at the meeting table. */
function reviewing(a: AgentState, m: OfficeModel): boolean {
  if (a.role !== 'reviewer') return false;
  if ([...a.tools.values()].some((t) => t.kind === 'review')) return true;
  if (a.status === 'idle' || a.status === 'done' || a.status === 'offline') return false;
  const t = a.taskId ? m.tasks.get(a.taskId) : null;
  return t?.status === 'reviewing';
}

function make(place: Place, pose: Pose, icon: string, extra: Partial<Intent> = {}): Intent {
  return { place, pose, prop: null, emote: null, screen: 'idle', icon, errand: null, ...extra };
}

const DESK: Place = { kind: 'desk' };

/** The in-place (desk) version of a tool's activity. */
function atDesk(t: ToolCall, a: AgentState): Intent {
  switch (t.kind) {
    case 'read':
    case 'compacting':
      return make(DESK, 'read', t.kind, { prop: 'paper', screen: 'doc' });
    case 'search':
      return make(DESK, 'read', 'search', { prop: 'paper', screen: 'search' });
    case 'fetch':
      return make(DESK, 'type', 'fetch', { screen: 'doc' });
    case 'edit':
    case 'create':
    case 'delete':
      return make(DESK, 'type', t.kind, { screen: 'code' });
    case 'execute':
    case 'test':
      return make(DESK, 'type', t.kind, { screen: 'terminal' });
    case 'mcp':
      return make(DESK, 'phone', 'mcp', { prop: 'phone', screen: 'idle' });
    case 'delegate':
      return make(DESK, 'talk', 'delegate', { screen: 'chat' });
    case 'message':
    case 'note':
    case 'memory':
    case 'plan':
    case 'review':
    case 'commit':
    case 'merge':
      return make(DESK, 'type', t.kind, { screen: a.role === 'lead' ? 'chat' : 'code' });
    default:
      return make(DESK, 'type', t.kind, { screen: 'code' });
  }
}

/** The at-station version (after the excursion threshold). */
function atStation(t: ToolCall, place: Place): Intent {
  switch (t.kind) {
    case 'test':
      return make(place, 'watch', 'test', { screen: 'terminal' });
    case 'execute':
      return make(place, 'watch', 'execute', { screen: 'terminal' });
    case 'search':
      return make(place, 'browse', 'search', { screen: 'search' });
    case 'fetch':
      return make(place, 'read', 'fetch', { prop: 'book', screen: 'doc' });
    case 'plan':
      return make(place, 'write', 'plan', { prop: 'marker' });
    case 'note':
      return make(place, 'write', 'note', { prop: 'note' });
    case 'review':
      return make(place, 'review', 'review', { prop: 'paper' });
    case 'commit':
      return make(place, 'drop', 'commit', { prop: 'envelope' });
    case 'merge':
      return make(place, 'celebrate', 'merge');
    case 'memory':
      return make(place, 'browse', 'memory', { prop: 'folder' });
    case 'message':
      return make(place, 'talk', 'message', { prop: 'paper' });
    default:
      return make(place, 'stand', t.kind);
  }
}

export interface Errand {
  kind: ErrandKind;
  taskId: string | null;
  at: number;
  /** Set by the engine when the character reaches the errand's place. */
  arrivedAt: number | null;
}

export function errandPlace(e: Errand, a: AgentState, m: OfficeModel): Place | null {
  switch (e.kind) {
    case 'commit':
    case 'merge':
      return { kind: 'station', id: 'mail' };
    case 'memory':
      return { kind: 'station', id: 'archive' };
    case 'note':
      return { kind: 'station', id: 'meeting.board' };
    case 'card-take':
    case 'card-return':
      return { kind: 'station', id: 'board' };
    case 'ask': {
      const l = leadId(m, a);
      return l ? { kind: 'lead', leadId: l } : null;
    }
  }
}

function errandIntent(e: Errand, place: Place): Intent {
  const acting = e.arrivedAt != null;
  switch (e.kind) {
    case 'commit':
      return make(place, 'drop', 'commit', { prop: acting ? null : 'envelope', errand: e.kind });
    case 'merge':
      return make(place, 'celebrate', 'merge', { errand: e.kind });
    case 'memory':
      return make(place, 'browse', 'memory', { prop: acting ? null : 'folder', errand: e.kind });
    case 'note':
      return make(place, 'write', 'note', { prop: 'note', errand: e.kind });
    case 'card-take':
      return make(place, 'browse', 'card', { prop: acting ? 'card' : null, errand: e.kind });
    case 'card-return':
      return make(place, 'browse', 'card', { prop: acting ? null : 'card', errand: e.kind });
    case 'ask':
      return make(place, 'talk', 'message', { prop: 'paper', errand: e.kind });
  }
}

/** Can an errand run now? Only when nothing more truthful needs the character. */
function errandAllowed(a: AgentState): boolean {
  if (!a.present || a.approval || isBlocked(a) || a.rateLimited || a.status === 'rate_limited') return false;
  for (const t of a.tools.values()) if (EXCURSION_MS[t.kind] != null || t.kind === 'await_approval') return false;
  return true;
}

export interface Derived {
  intent: Intent;
  /** When the intent may next change by time alone (Infinity: only on events). */
  nextAt: number;
}

/**
 * What the character should be doing now. Pure: (agent, model, now, errand) → intent.
 * `reducedMotion` drops errands (walks become teleports, so trips would only flicker).
 */
export function deriveIntent(a: AgentState, m: OfficeModel, now: number, errand: Errand | null): Derived {
  let nextAt = Infinity;
  const soon = (t: number) => {
    if (t > now) nextAt = Math.min(nextAt, t);
  };

  if (!a.present) return { intent: make({ kind: 'door' }, 'stand', 'offline', { screen: 'off' }), nextAt };

  // Needs you: stand up at the desk with a raised hand (amber "!" beacon).
  if (a.approval) {
    const q = a.approval.id.startsWith('q:');
    return { intent: make({ kind: 'desk', stand: true }, 'raise', q ? 'question' : 'approval', { emote: q ? 'question' : 'alert', screen: 'alert' }), nextAt };
  }
  if (a.rateLimited || a.status === 'rate_limited') return { intent: make(DESK, 'sleep', 'rate', { emote: 'zz', screen: 'off' }), nextAt };
  if (isBlocked(a)) return { intent: make(DESK, 'slump', 'blocked', { emote: 'rain', screen: 'error' }), nextAt };

  // Celebrate / slump where the test ran (a short beat, then back to work or the desk).
  if (a.cheer && now - a.cheer.at < CHEER_MS && !a.tools.size) {
    soon(a.cheer.at + CHEER_MS);
    const place: Place = a.cheer.atMonitor ? { kind: 'station', id: 'server.monitor' } : { kind: 'desk', stand: true };
    return { intent: make(place, a.cheer.ok ? 'celebrate' : 'slump', a.cheer.ok ? 'tests-pass' : 'tests-fail', { screen: a.cheer.ok ? 'terminal' : 'error' }), nextAt };
  }

  if (errand && errandAllowed(a)) {
    const place = errandPlace(errand, a, m);
    if (place) {
      if (errand.arrivedAt != null) soon(errand.arrivedAt + ACT_MS);
      const it = errandIntent(errand, place);
      // A desk activity running meanwhile keeps its truthful screen and icon.
      const t = dominant(a);
      if (t) {
        const d = atDesk(t, a);
        it.screen = d.screen;
        it.icon = d.icon;
        if (!it.prop && (t.kind === 'read' || t.kind === 'search')) it.prop = 'paper';
      }
      return { intent: it, nextAt };
    }
  }

  const review = reviewing(a, m);
  const t = dominant(a);
  if (t) {
    if (t.kind === 'await_approval') return { intent: make({ kind: 'desk', stand: true }, 'raise', 'question', { emote: 'question', screen: 'alert' }), nextAt };
    const threshold = EXCURSION_MS[t.kind];
    const place = threshold != null ? stationFor(t, a, m) : null;
    if (place && now - t.since >= threshold!) return { intent: atStation(t, place), nextAt };
    if (place) soon(t.since + threshold!);
    if (review && (t.kind === 'read' || t.kind === 'search')) return { intent: make({ kind: 'station', id: 'meeting.table' }, 'review', 'review', { prop: 'paper', screen: 'doc' }), nextAt };
    return { intent: atDesk(t, a), nextAt };
  }
  if (review) return { intent: make({ kind: 'station', id: 'meeting.table' }, 'review', 'review', { prop: 'paper', screen: 'doc' }), nextAt };
  if (a.retrying) return { intent: make(DESK, 'sit', 'retry', { screen: 'idle' }), nextAt };
  if (a.compactingSince != null) return { intent: make(DESK, 'read', 'compacting', { prop: 'paper', screen: 'doc' }), nextAt };
  if (a.replyingSince != null) return { intent: make(DESK, 'type', 'replying', { screen: 'chat' }), nextAt };
  if (a.thinkingSince != null || a.status === 'thinking') {
    // A short think between two calls: the last call's pose holds (no flicker), icon says thinking.
    const since = a.thinkingSince ?? a.statusAt;
    if (a.lastTool && now - since < THINK_HOLD_MS && Math.abs(since - a.lastToolEnd) < 50 && EXCURSION_MS[a.lastTool.kind] == null) {
      soon(since + THINK_HOLD_MS);
      return { intent: { ...atDesk(a.lastTool, a), icon: 'thinking' }, nextAt };
    }
    return { intent: make(DESK, 'think', 'thinking', { emote: 'think', screen: 'think' }), nextAt };
  }
  // Helpers run in the background: the lead sits and waits rather than gesturing on a loop.
  if (a.subagents.size) return { intent: make(DESK, 'sit', 'delegate', { screen: 'chat' }), nextAt };

  if (a.status === 'done' && a.doneAt != null && now - a.doneAt < DONE_CHECK_MS) {
    soon(a.doneAt + DONE_CHECK_MS);
    return { intent: make(DESK, 'sit', 'done', { emote: 'check', screen: 'idle' }), nextAt };
  }
  const quiet = a.status === 'idle' || a.status === 'done' || a.status === 'starting';
  if (quiet && now - a.lastEventAt >= LONG_IDLE_MS) return { intent: make(DESK, 'sleep', 'sleep', { emote: 'zz', screen: 'off' }), nextAt };
  if (quiet) soon(a.lastEventAt + LONG_IDLE_MS);
  if (a.status === 'working') return { intent: make(DESK, 'type', 'working', { screen: 'code' }), nextAt };
  return { intent: make(DESK, 'sit', a.status === 'done' ? 'done' : 'idle', { screen: a.status === 'starting' ? 'idle' : 'idle' }), nextAt };
}

// ───────────── errands ─────────────

/**
 * Per-agent errand queue. The engine pushes errands from live effects, tells it when a
 * character arrives somewhere, and asks for the current errand; finished, stale or
 * pre-empted errands drop out.
 */
export class Errands {
  private q = new Map<string, Errand[]>();
  /** Called when an errand's action happens (on arrival) and when it completes. */
  constructor(private on?: (agentId: string, e: Errand, phase: 'act' | 'done' | 'drop') => void) {}

  push(agentId: string, kind: ErrandKind, taskId: string | null, at: number) {
    const list = this.q.get(agentId) ?? [];
    // One of each kind at a time (two commits in a row are one trip).
    if (list.some((e) => e.kind === kind && e.taskId === taskId)) return;
    list.push({ kind, taskId, at, arrivedAt: null });
    this.q.set(agentId, list);
  }

  /** The errand to run now for an agent (head of its queue), dropping stale ones. */
  current(agentId: string, now: number, allowed: boolean): Errand | null {
    const list = this.q.get(agentId);
    if (!list?.length) return null;
    while (list.length) {
      const e = list[0];
      const stale = e.arrivedAt == null ? now - e.at > (allowed ? ERRAND_MAX_MS : ERRAND_QUEUE_MS) : now - e.arrivedAt >= ACT_MS;
      if (!stale) break;
      list.shift();
      this.on?.(agentId, e, e.arrivedAt == null ? 'drop' : 'done');
    }
    if (!list.length) {
      this.q.delete(agentId);
      return null;
    }
    return allowed ? list[0] : null;
  }

  /** The character reached `key`: if that's its errand's place, the action starts. */
  arrived(agentId: string, key: string, now: number, placeOf: (e: Errand) => string | null) {
    const e = this.q.get(agentId)?.[0];
    if (!e || e.arrivedAt != null) return;
    if (placeOf(e) === key) {
      e.arrivedAt = now;
      this.on?.(agentId, e, 'act');
    }
  }

  /** Drop everything for an agent (left the office, approval, blocked…). */
  clear(agentId: string) {
    const list = this.q.get(agentId);
    if (!list) return;
    for (const e of list) this.on?.(agentId, e, 'drop');
    this.q.delete(agentId);
  }

  has(agentId: string): boolean {
    return !!this.q.get(agentId)?.length;
  }

  /** Earliest time an errand expires or finishes acting. */
  nextAt(): number {
    let t = Infinity;
    for (const list of this.q.values()) {
      const e = list[0];
      if (!e) continue;
      t = Math.min(t, e.arrivedAt != null ? e.arrivedAt + ACT_MS : e.at + ERRAND_MAX_MS);
    }
    return t;
  }

  get size() {
    let n = 0;
    for (const l of this.q.values()) n += l.length;
    return n;
  }
}

export function errandAllowedFor(a: AgentState): boolean {
  return errandAllowed(a);
}

// ───────────── dwell ─────────────

/**
 * Minimum dwell per agent so rapid events don't jitter the pose: a new value shows
 * immediately after a quiet period, but at most once per `ms`; the latest request wins
 * when the dwell ends.
 */
export class Dwell<T> {
  private shown = new Map<string, { value: T; key: string; at: number }>();
  private pending = new Map<string, { value: T; key: string }>();
  constructor(
    readonly ms: number,
    private keyOf: (v: T) => string,
  ) {}

  /** Request `value` for `id` at time `now`; returns what to show now. */
  request(id: string, value: T, now: number, force = false): T {
    const key = this.keyOf(value);
    const cur = this.shown.get(id);
    if (!cur || force) {
      this.shown.set(id, { value, key, at: now });
      this.pending.delete(id);
      return value;
    }
    if (cur.key === key) {
      this.pending.delete(id);
      cur.value = value;
      return cur.value;
    }
    if (now - cur.at >= this.ms) {
      this.shown.set(id, { value, key, at: now });
      this.pending.delete(id);
      return value;
    }
    this.pending.set(id, { value, key });
    return cur.value;
  }

  /** Apply pending requests whose dwell ended. Returns ids that changed. */
  flush(now: number): string[] {
    const changed: string[] = [];
    for (const [id, p] of this.pending) {
      const cur = this.shown.get(id);
      if (!cur || now - cur.at >= this.ms) {
        this.shown.set(id, { value: p.value, key: p.key, at: now });
        this.pending.delete(id);
        changed.push(id);
      }
    }
    return changed;
  }

  /** Earliest time a pending request becomes visible (Infinity when none). */
  nextAt(): number {
    let t = Infinity;
    for (const id of this.pending.keys()) {
      const cur = this.shown.get(id);
      if (cur) t = Math.min(t, cur.at + this.ms);
    }
    return t;
  }

  current(id: string): T | undefined {
    return this.shown.get(id)?.value;
  }

  delete(id: string) {
    this.shown.delete(id);
    this.pending.delete(id);
  }
}
