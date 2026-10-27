// The UI store: a projection of (snapshot + ordered event batches) from the transport.
//
// Reactivity rules (keep them, the panels rely on them):
// - Entity lists (agents, tasks, plans, approvals, chats, memories, triples) are `$state.raw`
//   arrays, REPLACED on change (never mutated in place).
// - Log-derived structures (segments, files, todo lists, messages, decisions…) are plain JS,
//   versioned by `rev` (persisted events) and `liveRev` (ephemeral deltas). Consumers read
//   the counter inside `$derived.by` and copy what they render. Objects stored in them are
//   never mutated after being exposed: they are replaced (so keyed `#each` sees a new ref).
// - The event log is a plain capped array versioned by `rev`.
import { api, getTransport, type UiBatch } from '../api';
import type { Agent } from '../gen/Agent';
import type { WorkspaceTeam } from '../gen/WorkspaceTeam';
import type { Approval } from '../gen/Approval';
import type { Artifact } from '../gen/Artifact';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Citation } from '../gen/Citation';
import type { CliInfo } from '../gen/CliInfo';
import type { Confidence } from '../gen/Confidence';
import type { EventRow } from '../gen/EventRow';
import type { ActivityKind } from '../gen/ActivityKind';
import type { Memory } from '../gen/Memory';
import type { MessagePhase } from '../gen/MessagePhase';
import type { Plan } from '../gen/Plan';
import type { PlanStep } from '../gen/PlanStep';
import type { Settings } from '../gen/Settings';
import type { Snapshot } from '../gen/Snapshot';
import type { Target } from '../gen/Target';
import type { Task } from '../gen/Task';
import type { Triple } from '../gen/Triple';
import type { Workspace } from '../gen/Workspace';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import type { Provider } from '../gen/Provider';
import { targetLabel, type Provenance } from '../util/labels';
import { accounts } from './accounts.svelte';
import { controls } from './control.svelte';

export const LOG_CAP = 5000;
const SEG_CAP = 1500;
const MSG_CAP = 300;
const LIVE_OUTPUT_CAP = 24_000;

export const DEFAULT_SETTINGS: Settings = {
  lead_provider: null,
  lead_model: null,
  worker_providers: ['claude', 'codex', 'gemini', 'opencode'],
  max_parallel: 3,
  max_per_provider: 2,
  accounts: [],
  account_strategy: 'spread',
  lead_account: null,
  approval_mode: 'balanced',
  auto_run_plans: false,
  review_gate: false,
  lead_followups: true,
  world_ambient: false,
  theme: 'system',
  team: [],
};

// ───────────── derived record types ─────────────

export type Seg = {
  callId: string;
  agentId: string;
  taskId: string | null;
  activity: ActivityKind;
  name: string;
  label: string;
  target: Target;
  input: JsonValue;
  confidence: string;
  start: number;
  end: number | null;
  ok: boolean | null;
  exitCode: number | null;
  testsPassed: number | null;
  testsFailed: number | null;
  eventId: number;
};

export type FileReason = { prov: Provenance; text: string; eventId: number; ts: number };
export type FileTouch = {
  path: string;
  read: boolean;
  modified: boolean;
  created: boolean;
  deleted: boolean;
  provided: boolean;
  reasons: FileReason[];
  lastTs: number;
};

export type AgentMsg = { itemId: string; agentId: string; text: string; phase: MessagePhase; ts: number; eventId: number };
export type Thought = { itemId: string; text: string; ts: number; eventId: number };
export type Retrieval = { query: string; confidence: Confidence; citations: Citation[]; ts: number; eventId: number };
export type Decision = {
  id: string;
  type: string;
  method: string;
  summary: string;
  detail: JsonValue;
  ts: number;
  agentId: string | null;
  taskId: string | null;
  eventId: number;
};
export type TaskMark = {
  kind: 'committed' | 'verified' | 'reviewed' | 'merged';
  ts: number;
  eventId: number;
  agentId: string | null;
  text: string;
  ok: boolean;
  sha?: string;
  files?: string[];
};
export type RateWindow = {
  provider: Provider | null;
  /** The reporting agent's account id (its provider when accounts are unknown). */
  account: string | null;
  window: string;
  usedFraction: number | null;
  resetsAt: number | null;
  status: string | null;
  ts: number;
};

export type SubAgent = { callId: string; description: string; status: string; startedAt: number; ts: number };

export type AgentLive = {
  segments: Seg[];
  open: Record<string, number>; // callId → index into segments
  todo: PlanStep[];
  todoTs: number;
  files: Record<string, FileTouch>;
  messages: AgentMsg[];
  reasoning: Thought[];
  retrievals: Retrieval[];
  searches: { query: string; output: string }[];
  session: { provider: Provider; model: string | null; cwd: string; cliVersion: string | null; tools: string[]; ts: number } | null;
  turn: { startedAt: number; input: string | null; endedAt: number | null; status: string | null } | null;
  /** Turns in order; `input` is the prompt Codly sent (it contains the user's words verbatim). */
  turns: { start: number; end: number | null; input: string | null }[];
  statusSince: number;
  lastOutput: { callId: string; text: string; ts: number; ok: boolean | null } | null;
  errors: { message: string; fatal: boolean; ts: number }[];
  subagents: SubAgent[];
  firstTs: number;
  lastTs: number;
};

export type Stream = { itemId: string; agentId: string; kind: 'text' | 'reasoning'; text: string; ts: number };
/** A message you sent that the core has not echoed yet (optimistic), or that failed to send. */
export type Outgoing = { id: string; thread: string; text: string; ts: number; error: string | null };
export type LiveOut = { callId: string; agentId: string; text: string; ts: number };

const emptyLive = (ts: number): AgentLive => ({
  segments: [],
  open: {},
  todo: [],
  todoTs: 0,
  files: {},
  messages: [],
  reasoning: [],
  retrievals: [],
  searches: [],
  session: null,
  turn: null,
  turns: [],
  statusSince: ts,
  lastOutput: null,
  errors: [],
  subagents: [],
  firstTs: ts,
  lastTs: ts,
});

const upsert = <T extends { id: string }>(list: T[], item: T): T[] => {
  const i = list.findIndex((x) => x.id === item.id);
  if (i < 0) return [...list, item];
  const next = list.slice();
  next[i] = item;
  return next;
};

const pathsOf = (t: Target): string[] => {
  const out = [...(t.paths ?? [])];
  if (t.path && !out.includes(t.path)) out.unshift(t.path);
  return out;
};

class CodlyStore {
  // connection
  /** The first snapshot arrived. Later resyncs keep the current screen until theirs lands. */
  ready = $state(false);
  error = $state<string | null>(null);
  private gen = 0;
  /** Waiting for a snapshot: live events are buffered until it is applied. */
  private syncing = false;
  private lastId = 0;
  private buffered: EventRow[] = [];

  // entities
  clis = $state.raw<CliInfo[]>([]);
  workspace = $state.raw<Workspace | null>(null);
  settings = $state.raw<Settings>(DEFAULT_SETTINGS);
  /** The open repo's changes to the global team. */
  workspaceTeam = $state.raw<WorkspaceTeam>({ off: [], members: [] });
  agents = $state.raw<Agent[]>([]);
  plans = $state.raw<Plan[]>([]);
  tasks = $state.raw<Task[]>([]);
  approvals = $state.raw<Approval[]>([]);
  chats = $state.raw<ChatMessage[]>([]);
  memories = $state.raw<Memory[]>([]);
  triples = $state.raw<Triple[]>([]);
  artifacts = $state.raw<Artifact[]>([]);
  rateLimits = $state.raw<RateWindow[]>([]);
  /** Optimistic user messages, until their `chat` event arrives. */
  outbox = $state.raw<Outgoing[]>([]);

  // versions
  rev = $state(0);
  liveRev = $state(0);
  /** Bumped when state is replaced wholesale (snapshot): imperative consumers re-derive. */
  epoch = $state(0);

  // log-derived (plain; see rules above)
  events: EventRow[] = [];
  live: Record<string, AgentLive> = {};
  streams = new Map<string, Stream>();
  liveOut = new Map<string, LiveOut>();
  decisions: Decision[] = [];
  taskMarks: Record<string, TaskMark[]> = {};
  /** Chat id → the key its optimistic copy had (so the bubble never remounts). */
  chatKeys = new Map<string, string>();
  /** Plans merged into the user's branch (decision `merge_plan`, or a successful merge command). */
  planMerges: Record<string, { sha: string; ts: number }> = {};
  sessionStart = 0;

  private listeners = new Set<(ev: EventRow) => void>();

  // lookups
  agentMap = $derived(new Map(this.agents.map((a) => [a.id, a])));
  taskMap = $derived(new Map(this.tasks.map((t) => [t.id, t])));
  planMap = $derived(new Map(this.plans.map((p) => [p.id, p])));
  artifactMap = $derived(new Map(this.artifacts.map((a) => [a.id, a])));
  /** The Leads, one per request thread (a Lead with no thread serves "lead"). */
  leads = $derived(this.agents.filter((a) => a.role === 'lead').sort((a, b) => a.created_at - b.created_at));
  pendingApprovals = $derived(this.approvals.filter((a) => a.status === 'pending'));

  // ───────────── connection ─────────────

  async connect(): Promise<void> {
    const gen = ++this.gen;
    this.syncing = true;
    this.buffered = [];
    try {
      const t = await getTransport();
      const snap = await t.subscribe((b: UiBatch) => this.onBatch(gen, b));
      if (gen !== this.gen) return;
      this.applySnapshot(snap);
      const early = this.buffered;
      this.buffered = [];
      this.syncing = false;
      this.ready = true;
      this.error = null;
      if (early.length) this.applyEvents(early);
    } catch (e) {
      if (gen !== this.gen) return; // a newer resync owns the state
      this.error = e instanceof Error ? e.message : String(e);
      this.syncing = false;
      this.ready = true;
    }
  }

  /** Fresh snapshot (workspace switch, long hidden period). Old-generation batches are ignored. */
  resync = () => this.connect();

  private onBatch(gen: number, b: UiBatch) {
    if (gen !== this.gen) return;
    if (this.syncing) {
      this.buffered.push(...b.events);
      return;
    }
    this.applyEvents(b.events);
  }

  /** Imperative consumers (World, terminal, toasts): every event incl. ephemeral, in order. */
  onEvent(cb: (ev: EventRow) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private applySnapshot(s: Snapshot) {
    this.lastId = s.as_of;
    this.clis = s.clis;
    accounts.set(s.accounts ?? []);
    this.workspace = s.workspace;
    this.settings = s.settings ?? DEFAULT_SETTINGS;
    this.workspaceTeam = s.workspace_team ?? { off: [], members: [] };
    this.agents = s.agents;
    this.plans = s.plans;
    this.tasks = s.tasks;
    this.approvals = s.approvals;
    this.chats = [...s.chats].sort((a, b) => a.ts - b.ts);
    this.outbox = this.outbox.filter((o) => !s.chats.some((c) => c.author === 'user' && c.thread === o.thread && c.text.trim() === o.text && c.ts >= o.ts - 10_000));
    this.memories = s.memories;
    this.triples = s.triples;
    this.artifacts = [...(s.artifacts ?? [])].sort((a, b) => a.created_at - b.created_at);
    this.rateLimits = [];
    this.live = {};
    this.streams = new Map();
    this.liveOut = new Map();
    this.decisions = [];
    this.taskMarks = {};
    this.planMerges = {};
    const evs = [...s.events].sort((a, b) => a.id - b.id);
    this.events = evs.slice(-LOG_CAP);
    this.sessionStart = evs[0]?.ts ?? Date.now();
    // Replay the log into the derived structures only: entities are already current.
    for (const ev of evs) this.derive(ev, true);
    this.rev++;
    this.liveRev++;
    this.epoch++;
  }

  private applyEvents(evs: EventRow[]) {
    let persisted = false;
    let live = false;
    for (const ev of evs) {
      if (ev.id !== 0) {
        if (ev.id <= this.lastId) continue;
        this.lastId = ev.id;
        this.events.push(ev);
        persisted = true;
      } else live = true;
      if (!this.sessionStart) this.sessionStart = ev.ts;
      this.applyEntity(ev);
      this.derive(ev, false);
      for (const l of this.listeners) {
        try {
          l(ev);
        } catch (e) {
          console.error('[codly] event listener failed', e);
        }
      }
    }
    if (this.events.length > LOG_CAP * 1.2) this.events = this.events.slice(-LOG_CAP);
    if (persisted) this.rev++;
    if (live) this.liveRev++;
  }

  // ───────────── entities (only from live events, never from snapshot replay) ─────────────

  private patchAgent(id: string | null, patch: Partial<Agent>) {
    if (!id) return;
    const i = this.agents.findIndex((a) => a.id === id);
    if (i < 0) return;
    const next = this.agents.slice();
    next[i] = { ...next[i], ...patch };
    this.agents = next;
  }

  private applyEntity(ev: EventRow) {
    const k = ev.kind;
    switch (k.type) {
      case 'agent_changed':
        // A let-go consultant or a member who left the team leaves the board.
        this.agents = k.agent.archived ? this.agents.filter((a) => a.id !== k.agent.id) : upsert(this.agents, k.agent);
        break;
      case 'plan_changed':
        this.plans = upsert(this.plans, k.plan);
        break;
      case 'task_changed':
        this.tasks = upsert(this.tasks, k.task);
        break;
      case 'chat':
        if (!this.chats.some((c) => c.id === k.message.id)) {
          const m = k.message;
          if (m.author === 'user') {
            const o = this.outbox.find((x) => x.thread === m.thread && x.text === m.text.trim());
            if (o) {
              this.chatKeys.set(m.id, `o:${o.id}`);
              this.outbox = this.outbox.filter((x) => x !== o);
            }
          }
          this.chats = [...this.chats, m];
        }
        break;
      case 'approval_requested':
        this.approvals = upsert(this.approvals, k.approval);
        break;
      case 'approval_resolved': {
        const a = this.approvals.find((x) => x.id === k.approval_id);
        if (a) this.approvals = upsert(this.approvals, { ...a, status: k.status, decided_by: k.by });
        break;
      }
      case 'memory_added':
        this.memories = upsert(this.memories, k.memory);
        break;
      case 'triple_added':
        if (!this.triples.some((t) => t.id === k.triple.id)) this.triples = [...this.triples, k.triple];
        break;
      case 'artifact_added':
        this.artifacts = upsert(this.artifacts, k.artifact);
        break;
      case 'status_changed':
        this.patchAgent(ev.agent_id, { status: k.status });
        break;
      case 'usage_updated':
        this.patchAgent(ev.agent_id, { usage: k.usage });
        break;
    }
  }

  // ───────────── log-derived structures (live events AND snapshot replay) ─────────────

  liveFor(agentId: string): AgentLive {
    return (this.live[agentId] ??= emptyLive(Date.now()));
  }

  private setFile(l: AgentLive, path: string, ts: number, patch: Partial<FileTouch>, reason: FileReason | null) {
    const prev = l.files[path] ?? {
      path, read: false, modified: false, created: false, deleted: false, provided: false, reasons: [], lastTs: ts,
    };
    const reasons = reason ? [...prev.reasons, reason].slice(-8) : prev.reasons;
    l.files = { ...l.files, [path]: { ...prev, ...patch, reasons, lastTs: ts } };
  }

  private closeOpen(l: AgentLive, ts: number) {
    const ids = Object.keys(l.open);
    if (!ids.length) return;
    const segs = l.segments.slice();
    for (const id of ids) {
      const i = l.open[id];
      if (segs[i] && segs[i].end == null) segs[i] = { ...segs[i], end: ts };
    }
    l.segments = segs;
    l.open = {};
  }

  private derive(ev: EventRow, replay: boolean) {
    const k = ev.kind;
    const aid = ev.agent_id;
    const l = aid ? this.liveFor(aid) : null;
    if (l) {
      if (!l.segments.length && !l.messages.length) l.firstTs = Math.min(l.firstTs, ev.ts);
      if (ev.id) l.lastTs = ev.ts;
    }

    switch (k.type) {
      case 'session_started':
        if (l) l.session = { provider: k.provider, model: k.model, cwd: k.cwd, cliVersion: k.cli_version, tools: k.tools, ts: ev.ts };
        break;
      case 'turn_started':
        if (l) {
          l.turn = { startedAt: ev.ts, input: k.input, endedAt: null, status: null };
          l.turns = [...l.turns, { start: ev.ts, end: null, input: k.input }].slice(-200);
        }
        break;
      case 'turn_completed':
        if (l) {
          if (l.turn) l.turn = { ...l.turn, endedAt: ev.ts, status: k.status };
          const t = l.turns;
          if (t.length && t[t.length - 1].end == null) l.turns = [...t.slice(0, -1), { ...t[t.length - 1], end: ev.ts }];
          this.closeOpen(l, ev.ts);
          for (const [id, s] of this.streams) if (s.agentId === aid) this.streams.delete(id);
        }
        break;
      case 'exited':
        if (l) {
          this.closeOpen(l, ev.ts);
          const t = l.turns;
          if (t.length && t[t.length - 1].end == null) l.turns = [...t.slice(0, -1), { ...t[t.length - 1], end: ev.ts }];
        }
        break;
      case 'status_changed':
        if (l) l.statusSince = ev.ts;
        break;
      case 'control_changed':
        if (aid) controls.set(aid, k.control);
        break;
      case 'text_delta':
      case 'reasoning_delta': {
        if (!aid) break;
        const kind = k.type === 'text_delta' ? 'text' : 'reasoning';
        const prev = this.streams.get(k.item_id);
        this.streams.set(k.item_id, {
          itemId: k.item_id, agentId: aid, kind, text: (prev?.text ?? '') + k.text, ts: prev?.ts ?? ev.ts,
        });
        break;
      }
      case 'message':
        this.streams.delete(k.item_id);
        if (l && aid) {
          const m: AgentMsg = { itemId: k.item_id, agentId: aid, text: k.text, phase: k.phase, ts: ev.ts, eventId: ev.id };
          l.messages = [...l.messages.filter((x) => x.itemId !== k.item_id), m].slice(-MSG_CAP);
        }
        break;
      case 'reasoning':
        this.streams.delete(k.item_id);
        if (l) l.reasoning = [...l.reasoning.filter((x) => x.itemId !== k.item_id), { itemId: k.item_id, text: k.text, ts: ev.ts, eventId: ev.id }].slice(-60);
        break;
      case 'tool_started': {
        if (!l || !aid) break;
        const seg: Seg = {
          callId: k.call_id, agentId: aid, taskId: ev.task_id, activity: k.activity, name: k.name,
          label: targetLabel(k.target, k.activity), target: k.target, input: k.input, confidence: k.confidence,
          start: ev.ts, end: null, ok: null, exitCode: null, testsPassed: null, testsFailed: null, eventId: ev.id,
        };
        const existing = l.open[k.call_id];
        if (existing != null) {
          const segs = l.segments.slice();
          segs[existing] = { ...seg, start: segs[existing].start };
          l.segments = segs;
        } else {
          l.segments = [...l.segments, seg];
          if (l.segments.length > SEG_CAP) {
            const drop = l.segments.length - SEG_CAP;
            l.segments = l.segments.slice(drop);
            const open: Record<string, number> = {};
            for (const [id, i] of Object.entries(l.open)) if (i - drop >= 0) open[id] = i - drop;
            l.open = open;
          }
          l.open = { ...l.open, [k.call_id]: l.segments.length - 1 };
        }
        // files
        const paths = pathsOf(k.target);
        const a = k.activity;
        const prov: Provenance = k.confidence === 'observed' ? 'observed' : 'inferred';
        for (const p of paths) {
          if (a === 'read') {
            const via = l.searches.find((s) => s.output.includes(p));
            this.setFile(l, p, ev.ts, { read: true }, { prov, text: `Read with ${k.name}`, eventId: ev.id, ts: ev.ts });
            if (via) this.setFile(l, p, ev.ts, {}, { prov: 'inferred', text: `Found via search ${via.query}`, eventId: ev.id, ts: ev.ts });
          } else if (a === 'edit') this.setFile(l, p, ev.ts, { modified: true }, { prov, text: `Edited with ${k.name}`, eventId: ev.id, ts: ev.ts });
          else if (a === 'create') this.setFile(l, p, ev.ts, { created: true, modified: true }, { prov, text: `Created with ${k.name}`, eventId: ev.id, ts: ev.ts });
          else if (a === 'delete') this.setFile(l, p, ev.ts, { deleted: true, modified: true }, { prov, text: `Deleted with ${k.name}`, eventId: ev.id, ts: ev.ts });
        }
        if (a === 'search' && k.target.query) l.searches = [{ query: `“${k.target.query}”`, output: '' }, ...l.searches].slice(0, 6);
        break;
      }
      case 'tool_output': {
        if (!aid) break;
        const prev = this.liveOut.get(k.call_id);
        let text = (prev?.text ?? '') + k.chunk;
        if (text.length > LIVE_OUTPUT_CAP) text = text.slice(-LIVE_OUTPUT_CAP);
        this.liveOut.set(k.call_id, { callId: k.call_id, agentId: aid, text, ts: prev?.ts ?? ev.ts });
        break;
      }
      case 'tool_completed': {
        if (!l) break;
        const i = l.open[k.call_id];
        if (i != null && l.segments[i]) {
          const segs = l.segments.slice();
          segs[i] = {
            ...segs[i], end: ev.ts, ok: k.ok, exitCode: k.exit_code, testsPassed: k.tests_passed, testsFailed: k.tests_failed,
          };
          l.segments = segs;
          const { [k.call_id]: _, ...rest } = l.open;
          l.open = rest;
          if (segs[i].activity === 'search' && k.output) {
            const q = segs[i].target.query ? `“${segs[i].target.query}”` : segs[i].label;
            l.searches = [{ query: q, output: k.output }, ...l.searches.filter((s) => s.output)].slice(0, 6);
          }
        }
        const live = this.liveOut.get(k.call_id);
        const text = k.output ?? live?.text ?? '';
        if (text) l.lastOutput = { callId: k.call_id, text, ts: ev.ts, ok: k.ok };
        this.liveOut.delete(k.call_id);
        break;
      }
      case 'files_changed':
        if (l)
          for (const p of k.paths)
            this.setFile(l, p, ev.ts, { modified: true }, {
              prov: 'observed', text: k.source === 'git' ? 'Changed in the worktree (git)' : 'Reported changed by the CLI', eventId: ev.id, ts: ev.ts,
            });
        break;
      case 'plan_updated':
        if (l) {
          l.todo = k.steps;
          l.todoTs = ev.ts;
        }
        break;
      case 'retrieval':
        if (l) {
          l.retrievals = [...l.retrievals, { query: k.query, confidence: k.confidence, citations: k.citations, ts: ev.ts, eventId: ev.id }].slice(-30);
          for (const c of k.citations)
            this.setFile(l, c.path, ev.ts, { provided: true }, {
              prov: 'provided',
              text: `Included by Codly (${c.id}, lines ${c.start_line}–${c.end_line})${c.why.length ? ': ' + c.why.join(', ') : ''}`,
              eventId: ev.id, ts: ev.ts,
            });
        }
        break;
      case 'sub_agent':
        if (l) {
          const prev = l.subagents.find((x) => x.callId === k.call_id);
          const next: SubAgent = { callId: k.call_id, description: k.description, status: k.status, startedAt: prev?.startedAt ?? ev.ts, ts: ev.ts };
          l.subagents = [...l.subagents.filter((x) => x.callId !== k.call_id), next].slice(-20);
        }
        break;
      case 'error':
        if (l) l.errors = [...l.errors, { message: k.message, fatal: k.fatal, ts: ev.ts }].slice(-20);
        break;
      case 'rate_limit': {
        const agent = aid ? this.agentMap.get(aid) : undefined;
        const provider = agent?.provider ?? null;
        const account = agent ? accounts.idOf(agent) : null;
        const w: RateWindow = { provider, account, window: k.window, usedFraction: k.used_fraction, resetsAt: k.resets_at, status: k.status, ts: ev.ts };
        this.rateLimits = [...this.rateLimits.filter((r) => !(r.provider === provider && r.account === account && r.window === k.window)), w];
        if (agent) accounts.observe(agent, k, ev.ts);
        break;
      }
      case 'decision':
        if (k.decision_type === 'merge_plan' && k.detail && typeof k.detail === 'object' && !Array.isArray(k.detail)) {
          const d = k.detail as Record<string, unknown>;
          if (typeof d.plan_id === 'string') this.planMerges = { ...this.planMerges, [d.plan_id]: { sha: String(d.sha ?? ''), ts: ev.ts } };
        }
        this.decisions = [
          ...this.decisions,
          { id: k.id, type: k.decision_type, method: k.method, summary: k.summary, detail: k.detail, ts: ev.ts, agentId: aid, taskId: ev.task_id, eventId: ev.id },
        ].slice(-400);
        break;
      case 'committed':
      case 'verified':
      case 'reviewed':
      case 'merged': {
        const tid = ev.task_id;
        if (!tid) break;
        let mark: TaskMark;
        if (k.type === 'committed')
          mark = { kind: 'committed', ts: ev.ts, eventId: ev.id, agentId: aid, text: k.message, ok: true, sha: k.sha, files: k.files };
        else if (k.type === 'verified')
          mark = { kind: 'verified', ts: ev.ts, eventId: ev.id, agentId: aid, text: k.command, ok: k.ok };
        else if (k.type === 'reviewed')
          mark = { kind: 'reviewed', ts: ev.ts, eventId: ev.id, agentId: aid, text: `${k.verdict}: ${k.summary}`, ok: !/reject|change/i.test(k.verdict) };
        else mark = { kind: 'merged', ts: ev.ts, eventId: ev.id, agentId: aid, text: k.branch, ok: true, sha: k.sha };
        this.taskMarks = { ...this.taskMarks, [tid]: [...(this.taskMarks[tid] ?? []), mark] };
        break;
      }
    }
    void replay;
  }

  // ───────────── queries used by several panels ─────────────

  /** What the agent is doing right now (open tool call first, then the agent's own activity). */
  now(agentId: string): { activity: ActivityKind; label: string; since: number; seg: Seg | null } | null {
    const a = this.agentMap.get(agentId);
    const l = this.live[agentId];
    if (a?.status === 'waiting_approval') {
      let open: Seg | null = null;
      if (l) for (const i of Object.values(l.open)) if (l.segments[i] && (!open || l.segments[i].start > open.start)) open = l.segments[i];
      return { activity: 'await_approval', label: open?.label ?? '', since: l?.statusSince ?? a.updated_at, seg: open };
    }
    if (l) {
      let best: Seg | null = null;
      for (const i of Object.values(l.open)) {
        const s = l.segments[i];
        if (s && (!best || s.start > best.start)) best = s;
      }
      if (best) return { activity: best.activity, label: best.label, since: best.start, seg: best };
    }
    if (!a) return null;
    for (const s of this.streams.values())
      if (s.agentId === agentId) return { activity: s.kind === 'text' ? 'replying' : 'thinking', label: '', since: s.ts, seg: null };
    if (a.activity && a.activity !== 'idle' && a.status !== 'idle' && a.status !== 'done' && a.status !== 'offline')
      return { activity: a.activity, label: a.activity_label ?? '', since: l?.statusSince ?? a.updated_at, seg: null };
    if (a.status === 'thinking' || a.status === 'starting') return { activity: 'thinking', label: '', since: l?.statusSince ?? a.updated_at, seg: null };
    return null;
  }

  /** Thread id for an agent's own conversation (a Lead's request thread, else its 1:1). */
  threadOf(agentId: string): string {
    const a = this.agentMap.get(agentId);
    return a?.role === 'lead' ? (a.thread ?? 'lead') : `agent:${agentId}`;
  }

  /** The Lead serving a request thread (null until the core starts it). */
  leadOf(thread: string): Agent | null {
    let best: Agent | null = null;
    for (const a of this.leads) if ((a.thread ?? 'lead') === thread && (!best || best.status === 'offline')) best = a;
    return best;
  }

  /** Agent owning a thread: the request's Lead, or the 1:1 agent. */
  threadAgent(thread: string): Agent | null {
    if (thread === 'lead' || thread.startsWith('lead:')) return this.leadOf(thread);
    return this.agentMap.get(thread.replace(/^agent:/, '')) ?? null;
  }

  tasksOfPlan(planId: string): Task[] {
    return this.tasks.filter((t) => t.plan_id === planId);
  }

  /** The request thread a task belongs to: its plan's; ad-hoc tasks belong to "lead". */
  taskThread(taskId: string | null | undefined): string {
    const t = taskId ? this.taskMap.get(taskId) : null;
    const p = t?.plan_id ? this.planMap.get(t.plan_id) : null;
    return p?.thread || 'lead';
  }

  /** Where an approval shows: its task's request; a Lead's own, its request; otherwise "lead". */
  approvalThread(ap: Approval): string {
    if (ap.task_id) return this.taskThread(ap.task_id);
    const a = this.agentMap.get(ap.agent_id);
    if (a?.role === 'lead') return a.thread ?? 'lead';
    if (a?.task_id) return this.taskThread(a.task_id);
    return 'lead';
  }

  /** The request's current plan: its newest one that was not cancelled (else the newest). */
  planOf(thread: string): Plan | null {
    let best: Plan | null = null;
    let any: Plan | null = null;
    for (const p of this.plans) {
      if ((p.thread || 'lead') !== thread) continue;
      if (!any || p.created_at > any.created_at) any = p;
      if (p.status !== 'cancelled' && (!best || p.created_at > best.created_at)) best = p;
    }
    return best ?? any;
  }

  /** The newest plan that is not cancelled, across requests. */
  activePlan = $derived.by(() => {
    const live = this.plans.filter((p) => p.status !== 'cancelled');
    return live.sort((a, b) => b.created_at - a.created_at)[0] ?? null;
  });

  rateMeter = $derived.by(() => {
    let worst: RateWindow | null = null;
    for (const r of this.rateLimits) if (r.usedFraction != null && (!worst || r.usedFraction > (worst.usedFraction ?? 0))) worst = r;
    return worst;
  });

  // ───────────── local mutations from command results ─────────────

  setWorkspace(ws: Workspace | null) {
    this.workspace = ws;
  }
  setSettings(s: Settings) {
    this.settings = s;
  }
  setWorkspaceTeam(t: WorkspaceTeam) {
    this.workspaceTeam = t;
  }
  upsertTask(t: Task) {
    this.tasks = upsert(this.tasks, t);
  }
  upsertAgent(a: Agent) {
    this.agents = upsert(this.agents, a);
  }
  upsertMemory(m: Memory) {
    this.memories = upsert(this.memories, m);
  }
  removeMemory(id: string) {
    this.memories = this.memories.filter((m) => m.id !== id);
  }
  setMemories(m: Memory[]) {
    this.memories = m;
  }
  setTriples(t: Triple[]) {
    this.triples = t;
  }
  setClis(c: CliInfo[]) {
    this.clis = c;
  }
  /** Show a message right away; it leaves the outbox when the core echoes it as a `chat`. */
  queueOutgoing(thread: string, text: string, ts: number): string {
    const id = Math.random().toString(36).slice(2, 10);
    this.outbox = [...this.outbox, { id, thread, text, ts, error: null }];
    return id;
  }
  failOutgoing(id: string, error: string) {
    this.outbox = this.outbox.map((o) => (o.id === id ? { ...o, error } : o));
  }
  dropOutgoing(id: string) {
    this.outbox = this.outbox.filter((o) => o.id !== id);
  }
  markPlanMerged(planId: string, sha: string) {
    this.planMerges = { ...this.planMerges, [planId]: { sha, ts: Date.now() } };
    this.rev++;
  }
}

export const store = new CodlyStore();

/** Visibility gating: tell the core when the window is hidden (it may coalesce harder). */
export function installVisibility() {
  let hiddenAt = 0;
  const onVis = () => {
    const visible = document.visibilityState === 'visible';
    api.setVisible(visible).catch(() => {});
    if (!visible) hiddenAt = Date.now();
    // ponytail: after a long hidden period (backgroundThrottling may suspend the webview),
    // take a fresh snapshot instead of trusting that every batch arrived.
    else if (hiddenAt && Date.now() - hiddenAt > 30_000) store.resync();
  };
  document.addEventListener('visibilitychange', onVis);
  return () => document.removeEventListener('visibilitychange', onVis);
}
