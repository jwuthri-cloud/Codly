// The UI store: a projection of (snapshot + ordered event batches) from the transport.
//
// Reactivity rules (keep them, the panels rely on them):
// - Entity lists (agents, tasks, plans, approvals, chats, memories, triples) are `$state.raw`
//   arrays, REPLACED on change (never mutated in place).
// - Log-derived structures (segments, files, todo lists, messages, decisions…) are plain JS,
//   versioned by `rev` (persisted events) and `liveRev` (ephemeral deltas). Consumers read
//   the counter inside `$derived.by` and copy what they render. Objects stored in them are
//   never mutated after being exposed: they are replaced (so keyed `#each` sees a new ref).
//   While one batch is applied, a list or file map is copied once and then changed in place.
// - What belongs to one agent (its `live` record, its streams, its live output, its events in
//   the log) is also versioned per agent: `revBy` and `liveRevBy`. A panel about one agent
//   reads these, so the stream of another agent costs it nothing. `rev` and `liveRev` are for
//   what several agents share (`decisions`, `planMerges`, the whole log).
// - The event log is a plain capped array versioned by `rev`.
// - History never gets shorter: a snapshot holds the newest events only, so a new snapshot of
//   the open office is merged into what is held (`mergeSnapshot`), and a thread loads what is
//   older on request (`loadEarlier`).
import { SvelteMap } from 'svelte/reactivity';
import { api, getTransport, LinkLost, type UiBatch } from '../api';
import type { Agent } from '../gen/Agent';
import type { WorkspaceTeam } from '../gen/WorkspaceTeam';
import type { RequestPrefs } from '../gen/RequestPrefs';
import type { Approval } from '../gen/Approval';
import type { Artifact } from '../gen/Artifact';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Citation } from '../gen/Citation';
import type { CliInfo } from '../gen/CliInfo';
import type { Confidence } from '../gen/Confidence';
import type { EventKind } from '../gen/EventKind';
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
import { transferredOut } from '../util/transfer';
import { accounts } from './accounts.svelte';
import { recordSaid } from './answers';
import { controls } from './control.svelte';
import { isDown, Reconnect, type Link } from './connection';
import { withBackgroundTasks, withKnownStarts, withMcpServer, type BackgroundWork, type McpServer } from './session';

export const LOG_CAP = 5000;
const SEG_CAP = 1500;
const MSG_CAP = 300;
const LIVE_OUTPUT_CAP = 24_000;
/** Files one agent's record holds: all it changed, and of the rest the ones it touched last. */
export const FILE_CAP = 2000;
/** What is kept of a search's output, to tell which file was found by which search. */
const SEARCH_OUTPUT_CAP = 16_000;
/** Events asked for at once when a thread loads earlier history. */
const EARLIER_PAGE = 500;
/** Event kinds whose derivation is no part of one agent's own state (`AgentLive`). */
const SHARED_KINDS: ReadonlySet<EventKind['type']> = new Set([
  'control_changed', 'text_delta', 'reasoning_delta', 'tool_output', 'rate_limit', 'decision',
  'approval_requested', 'approval_said', 'approval_resolved', 'committed', 'verified', 'reviewed', 'merged',
]);

export const DEFAULT_SETTINGS: Settings = {
  lead_provider: null,
  lead_model: null,
  worker_providers: ['claude', 'codex', 'gemini', 'opencode'],
  max_parallel: 3,
  max_per_provider: 2,
  accounts: [],
  account_strategy: 'spread',
  lead_account: null,
  read_claude_quota: false,
  approval_mode: 'balanced',
  auto_run_plans: false,
  review_gate: false,
  lead_followups: true,
  use_my_claude_setup: true,
  codex_network_access: false,
  paused: false,
  paused_offices: [],
  game_rules: false,
  developer_mode: false,
  verify_timeout_secs: 600,
  watchdog: { quiet_secs: 600, quiet_tool_secs: 3600, idle_secs: 120 },
  notifications: { approval: true, question: true, plan_done: true, plan_failed: true, task_blocked: true, agent_stalled: true, budget: true, merge_ready: true, only_in_background: true },
  keep_awake: true,
  confirm_quit: true,
  claude_sandbox: false,
  claude_shell_auth: false,
  resume_on_launch: 'ask',
  world_ambient: false,
  theme: 'system',
  team: [],
  byok: { anthropic: { kind: 'off' }, openai: { kind: 'off' }, openrouter: { kind: 'off' } },
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
  /** A helper (subagent) made this call: the id of the agent's call that started that helper. */
  parent?: string;
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

/**
 * A helper (subagent) the agent started with the tool call `callId`. `status` is the CLI's own
 * word ("running", "completed", "failed"…) and `ts` when it last changed. The rest is what the
 * CLI said about the helper so far: its kind, what it does now, its tool calls and tokens, its report.
 */
export type SubAgent = {
  callId: string;
  description: string;
  status: string;
  startedAt: number;
  ts: number;
  name: string | null;
  step: string | null;
  toolUses: number | null;
  tokens: number | null;
  report: string | null;
};

export type AgentLive = {
  segments: Seg[];
  open: Record<string, number>; // callId → index into segments (the store's own index: changed in place)
  todo: PlanStep[];
  todoTs: number;
  files: Record<string, FileTouch>;
  messages: AgentMsg[];
  reasoning: Thought[];
  retrievals: Retrieval[];
  searches: { query: string; output: string }[];
  session: { provider: Provider; model: string | null; cwd: string; cliVersion: string | null; tools: string[]; ts: number } | null;
  turn: { startedAt: number; input: string | null; endedAt: number | null; status: string | null } | null;
  /** Turns in order; `input` is the prompt Codly sent (it contains the user's words verbatim), `steers` the prompts the turn took in while it ran. */
  turns: { start: number; end: number | null; input: string | null; steers?: { ts: number; input: string }[] }[];
  statusSince: number;
  lastOutput: { callId: string; text: string; ts: number; ok: boolean | null } | null;
  errors: { message: string; fatal: boolean; ts: number }[];
  subagents: SubAgent[];
  /** Its MCP servers and what it left at work in the background, of its last run that reported any (state/session.ts). */
  mcp: McpServer[];
  background: BackgroundWork[];
  firstTs: number;
  lastTs: number;
};

export type Stream = { itemId: string; agentId: string; kind: 'text' | 'reasoning'; text: string; ts: number };
/** A message you sent that the core has not echoed yet (optimistic), or that failed to send. `ws`: the office it was sent in. */
export type Outgoing = { id: string; thread: string; text: string; ts: number; ws: string | null; error: string | null };
export type LiveOut = { callId: string; agentId: string; text: string; ts: number };
/** A thread's history before what is shown: whether there is any, and whether a page is on its way. */
export type Earlier = { more: boolean; loading: boolean };

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
  mcp: [],
  background: [],
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

/** The finished step of a task an event stands for (a commit, a check, a review, a merge). */
const markOf = (ev: EventRow): TaskMark | null => {
  const k = ev.kind;
  const at = { ts: ev.ts, eventId: ev.id, agentId: ev.agent_id };
  switch (k.type) {
    case 'committed':
      return { ...at, kind: 'committed', text: k.message, ok: true, sha: k.sha, files: k.files };
    case 'verified':
      return { ...at, kind: 'verified', text: k.command, ok: k.ok };
    case 'reviewed':
      return { ...at, kind: 'reviewed', text: `${k.verdict}: ${k.summary}`, ok: !/reject|change/i.test(k.verdict) };
    case 'merged':
      return { ...at, kind: 'merged', text: k.branch, ok: true, sha: k.sha };
    default:
      return null;
  }
};

/** The file map within `FILE_CAP`: changed files stay, the others leave oldest first. Checked
 *  once per batch, and only past a margin, so a long run does not sort its files on every one. */
const capFiles = (files: Record<string, FileTouch>): Record<string, FileTouch> => {
  const all = Object.values(files);
  if (all.length <= FILE_CAP * 1.2) return files;
  const old = all.filter((f) => !f.modified).sort((a, b) => a.lastTs - b.lastTs);
  const drop = new Set(old.slice(0, all.length - FILE_CAP).map((f) => f.path));
  return Object.fromEntries(all.filter((f) => !drop.has(f.path)).map((f) => [f.path, f]));
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
  /** The office whose state is held (undefined: no snapshot yet). A new snapshot of the same
   *  office is merged into it; another office's replaces it. */
  private held: string | null | undefined = undefined;

  // entities
  clis = $state.raw<CliInfo[]>([]);
  workspace = $state.raw<Workspace | null>(null);
  settings = $state.raw<Settings>(DEFAULT_SETTINGS);
  /** The open repo's changes to the global team. */
  workspaceTeam = $state.raw<WorkspaceTeam>({ off: [], members: [] });
  /** Per request thread: its own mode, permissions and Lead model (unset = Settings). */
  requestPrefs = $state.raw<Record<string, RequestPrefs>>({});
  agents = $state.raw<Agent[]>([]);
  plans = $state.raw<Plan[]>([]);
  tasks = $state.raw<Task[]>([]);
  approvals = $state.raw<Approval[]>([]);
  chats = $state.raw<ChatMessage[]>([]);
  memories = $state.raw<Memory[]>([]);
  triples = $state.raw<Triple[]>([]);
  artifacts = $state.raw<Artifact[]>([]);
  rateLimits = $state.raw<RateWindow[]>([]);
  /** Optimistic user messages of every office, until their `chat` event arrives. */
  private sent = $state.raw<Outgoing[]>([]);
  /** The open office's: thread names repeat across offices, so another office's never show here. */
  outbox = $derived(this.sent.filter((o) => o.ws === (this.workspace?.id ?? null)));

  // versions
  rev = $state(0);
  liveRev = $state(0);
  /** Per agent: counts the batches that held a persisted event of that agent. */
  revBy = new SvelteMap<string, number>();
  /** Per agent: counts the batches that held an ephemeral event (a delta) of that agent. */
  liveRevBy = new SvelteMap<string, number>();
  /** Bumped when state is replaced wholesale (snapshot): imperative consumers re-derive. */
  epoch = $state(0);

  // log-derived (plain; see rules above)
  events: EventRow[] = [];
  live: Record<string, AgentLive> = {};
  streams = new Map<string, Stream>();
  liveOut = new Map<string, LiveOut>();
  decisions: Decision[] = [];
  /** Per task, its finished steps. Replaced on change like the entity lists (any agent's event
   *  can add one, so a per-agent version would miss it). */
  taskMarks = $state.raw<Record<string, TaskMark[]>>({});
  /** Chat id → the key its optimistic copy had (so the bubble never remounts). */
  chatKeys = new Map<string, string>();
  /** Plans merged into the user's branch (decision `merge_plan`, or a successful merge command). */
  planMerges: Record<string, { sha: string; ts: number }> = {};
  sessionStart = 0;

  // history before what a snapshot holds, loaded on request (per thread)
  /** The oldest event the held state was built from (the start of the last snapshot that took
   *  the place of everything). The log may have been trimmed past it since. */
  private origin = 0;
  /** The stored rows of the snapshot's `session_events`: the MCP servers and the last list of
   *  background tasks of the live runs, from before `origin`. An agent's state that is built
   *  again starts from them. */
  private beside: EventRow[] = [];
  /** Per thread that was asked: is there earlier history, is a page loading. */
  earlier = $state.raw<Record<string, Earlier>>({});
  /** Per thread loaded further back: how far (an event id), and for which owner. */
  private paged = new Map<string, { owner: string | null; before: number }>();
  /** Per agent whose thread was loaded further back: all its persisted events held, oldest
   *  first. Kept apart from the capped log, so its history is never trimmed or capped again. */
  private deep = new Map<string, EventRow[]>();
  /** Agents with transcript rows dropped by a cap: their threads can load them again. */
  private clipped = new Set<string>();
  /** The status a task had before the `task_changed` being applied (null: first seen), for listeners. */
  prevTaskStatus: Task['status'] | null = null;

  private listeners = new Set<(ev: EventRow) => void>();
  private elsewhere = new Set<(ev: EventRow) => void>();
  /** The lists and file maps copied for the batch being applied: only these change in place. */
  private owned = new Set<object>();
  /** The agents whose file map got an entry in the batch being applied. */
  private touched = new Set<AgentLive>();

  // lookups
  agentMap = $derived(new Map(this.agents.map((a) => [a.id, a])));
  taskMap = $derived(new Map(this.tasks.map((t) => [t.id, t])));
  planMap = $derived(new Map(this.plans.map((p) => [p.id, p])));
  artifactMap = $derived(new Map(this.artifacts.map((a) => [a.id, a])));
  /** The Leads, one per request thread (a Lead with no thread serves "lead"). */
  leads = $derived(this.agents.filter((a) => a.role === 'lead').sort((a, b) => a.created_at - b.created_at));
  pendingApprovals = $derived(this.approvals.filter((a) => a.status === 'pending'));

  // ───────────── connection ─────────────

  /** A browser page's link to `codly serve` (always "connected" in the app's window). */
  link = $state.raw<Link>({ kind: 'connected' });
  /** This page cannot reach Codly: the polls that call it skip. */
  get offline(): boolean {
    return isDown(this.link);
  }
  private reconnect = new Reconnect(() => this.connect(), (l) => (this.link = l));

  async connect(): Promise<void> {
    const gen = ++this.gen;
    this.syncing = true;
    this.buffered = [];
    try {
      const t = await getTransport();
      const snap = await t.subscribe((b: UiBatch) => this.onBatch(gen, b));
      if (gen !== this.gen) return;
      const missed = await this.missedSince(snap);
      if (gen !== this.gen) return;
      if (missed) this.mergeSnapshot(snap, missed);
      else this.applySnapshot(snap);
      const early = this.buffered;
      this.buffered = [];
      this.syncing = false;
      this.ready = true;
      this.error = null;
      this.reconnect.reached();
      if (early.length) this.applyEvents(early);
    } catch (e) {
      if (gen !== this.gen) return; // a newer resync owns the state
      this.syncing = false;
      this.ready = true;
      if (e instanceof LinkLost) this.reconnect.lost(e.signedOut);
      else this.reconnect.reached(); // Codly answered, with why it could not start
      // What is held stays on screen while the bar says the link is down.
      if (e instanceof LinkLost && this.held !== undefined) return;
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  /** "Try again" after a failed start. The app keeps a failed core start, so connecting again
   *  would only repeat its error: ask the app to start the core again first. */
  async retry(): Promise<void> {
    // In a browser that lost `codly serve`, its core is not the problem: open the stream again.
    if (isDown(this.link)) return this.reconnect.tryNow();
    this.ready = false; // the loading screen, while the core starts
    try {
      await api.retryCore();
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      this.ready = true;
      return;
    }
    await this.connect();
  }

  /** Fresh snapshot (workspace switch, long hidden period). Old-generation batches are ignored.
   *  For the office already open it is merged into what is held: nothing older is dropped. */
  resync = () => this.connect();

  /** "Try now" on the bar: one try at once (the waits go on if it fails). */
  tryNow = () => this.reconnect.tryNow();

  private onBatch(gen: number, b: UiBatch) {
    if (gen !== this.gen) return;
    if (b.resync) {
      void this.resync();
      return;
    }
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

  /** Events of your other offices (never applied here): for what waits for you there. */
  onElsewhere(cb: (ev: EventRow) => void): () => void {
    this.elsewhere.add(cb);
    return () => this.elsewhere.delete(cb);
  }

  private tell(to: Set<(ev: EventRow) => void>, ev: EventRow) {
    for (const l of to) {
      try {
        l(ev);
      } catch (e) {
        console.error('[codly] event listener failed', e);
      }
    }
  }

  /** The snapshot's own lists: all but approvals and messages (a merge keeps older ones of ours). */
  private adopt(s: Snapshot) {
    this.lastId = s.as_of;
    this.clis = s.clis;
    accounts.set(s.accounts ?? []);
    this.workspace = s.workspace;
    this.settings = s.settings ?? DEFAULT_SETTINGS;
    this.workspaceTeam = s.workspace_team ?? { off: [], members: [] };
    this.requestPrefs = (s.request_prefs ?? {}) as Record<string, RequestPrefs>;
    this.agents = s.agents;
    this.plans = s.plans;
    this.tasks = s.tasks;
    const ws = s.workspace?.id ?? null;
    this.held = ws;
    this.sent = this.sent.filter((o) => o.ws !== ws || !s.chats.some((c) => c.author === 'user' && c.thread === o.thread && c.text.trim() === o.text && c.ts >= o.ts - 10_000));
    this.memories = s.memories;
    this.triples = s.triples;
    this.artifacts = [...(s.artifacts ?? [])].sort((a, b) => a.created_at - b.created_at);
  }

  /** The events between what is held and a new snapshot of the same office, oldest first, up
   *  to the snapshot's cursor. The core reads its cursor first and its events last, so while
   *  agents work a snapshot also holds rows above the cursor: those come again as deltas, after
   *  the snapshot's lists are in, and are left to that (taken here, they would be applied twice).
   *  Null: the snapshot cannot be merged (another office, nothing held yet, or too far behind)
   *  and takes the place of what is held. */
  private async missedSince(s: Snapshot): Promise<EventRow[] | null> {
    if (this.held === undefined || this.held !== (s.workspace?.id ?? null) || s.as_of < this.lastId) return null;
    const rows = [...s.events].sort((a, b) => a.id - b.id);
    const missed = (e: EventRow) => e.id > this.lastId && e.id <= s.as_of;
    const fresh = rows.filter(missed);
    // The snapshot's events reach back to ours: nothing is missing in between.
    if (!rows.length || rows[0].id <= this.lastId) return fresh;
    try {
      const page = await api.events(null, rows[0].id, LOG_CAP); // newest first
      const between = page.filter((e) => e.id > this.lastId);
      // A full page and still no row of ours: too far behind to catch up row by row.
      if (between.length === page.length && page.length >= LOG_CAP) return null;
      return [...between.reverse().filter(missed), ...fresh];
    } catch (e) {
      console.error('[codly] could not read the events missed before a snapshot; loading it anew', e);
      return null;
    }
  }

  /** A fresh snapshot of the office already held (back from a hidden period, a lagging bus, a
   *  reconnect). The missed events go through like live ones, then the snapshot's lists take
   *  the place of ours. Nothing older is dropped, and nothing remounts (`epoch` stays). */
  private mergeSnapshot(s: Snapshot, missed: EventRow[]) {
    this.applyEvents(missed);
    // A server that stands where an earlier run recorded it has no row in any log (id 0), so
    // the missed rows do not hold it, and its live event may be lost with what was missed. It
    // is the newest word about that server, of the run that is live: it lands last.
    for (const ev of s.session_events ?? []) if (ev.id === 0) this.derive(ev, true);
    this.adopt(s);
    // Approvals: the snapshot has the ones that wait. Answered ones are in no snapshot: ours stay.
    const waiting = new Set(s.approvals.map((a) => a.id));
    const answered = this.approvals.filter((a) => !waiting.has(a.id) && a.status !== 'pending' && this.hasThread(a));
    this.approvals = [...answered, ...s.approvals].sort((a, b) => a.created_at - b.created_at);
    // Messages: the snapshot has the newest of each open thread. Older ones of ours stay. A
    // thread with none left was closed (its messages moved to its archive without events).
    const oldest = new Map<string, number>();
    for (const c of s.chats) oldest.set(c.thread, Math.min(oldest.get(c.thread) ?? Infinity, c.ts));
    const fresh = new Set(s.chats.map((c) => c.id));
    for (const c of this.chats) if (!oldest.has(c.thread)) this.dropEarlier(c.thread);
    const older = this.chats.filter((c) => !fresh.has(c.id) && c.ts <= (oldest.get(c.thread) ?? -Infinity));
    this.chats = [...older, ...s.chats].sort((a, b) => a.ts - b.ts);
    // Deltas were dropped while hidden: a half stream would never be whole.
    this.streams = new Map();
    this.liveOut = new Map();
    this.rev++;
    this.liveRev++;
    this.resetRevs();
  }

  private applySnapshot(s: Snapshot) {
    this.adopt(s);
    this.approvals = s.approvals;
    this.chats = [...s.chats].sort((a, b) => a.ts - b.ts);
    this.rateLimits = [];
    this.earlier = {};
    this.paged = new Map();
    this.deep = new Map();
    this.clipped = new Set();
    this.live = {};
    this.streams = new Map();
    this.liveOut = new Map();
    this.decisions = [];
    this.taskMarks = {};
    this.planMerges = {};
    // Up to the cursor: the rows above it come as deltas (see `missedSince`).
    const evs = s.events.filter((e) => e.id <= s.as_of).sort((a, b) => a.id - b.id);
    this.events = evs.slice(-LOG_CAP);
    this.origin = evs[0]?.id ?? s.as_of + 1;
    this.sessionStart = evs[0]?.ts ?? Date.now();
    // What runs beside the live runs and is not in the log: state, not rows of the log.
    const beside = s.session_events ?? [];
    this.beside = beside.filter((e) => e.id !== 0).sort((a, b) => a.id - b.id);
    // Replay the log into the derived structures only: entities are already current.
    this.owned.clear();
    for (const ev of this.beside) this.derive(ev, true);
    for (const ev of evs) this.derive(ev, true);
    // A server that stands where an earlier run recorded it has no row in any log (id 0). It
    // is the newest word about that server, of the run that is live: it lands last.
    for (const ev of beside) if (ev.id === 0) this.derive(ev, true);
    this.settle();
    this.rev++;
    this.liveRev++;
    this.resetRevs();
    this.epoch++;
  }

  private applyEvents(evs: EventRow[]) {
    let persisted = false;
    let live = false;
    const stored = new Set<string>();
    const lively = new Set<string>();
    this.owned.clear();
    for (const ev of evs) {
      // Your other offices keep working: their events are not this view's.
      if (ev.workspace_id && this.workspace && ev.workspace_id !== this.workspace.id) {
        this.tell(this.elsewhere, ev);
        continue;
      }
      if (ev.id !== 0) {
        if (ev.id <= this.lastId) continue;
        this.lastId = ev.id;
        this.events.push(ev);
        if (ev.agent_id) this.deep.get(ev.agent_id)?.push(ev);
        persisted = true;
      } else if (this.staleAgent(ev)) continue;
      else live = true;
      if (ev.agent_id) (ev.id !== 0 ? stored : lively).add(ev.agent_id);
      if (!this.sessionStart) this.sessionStart = ev.ts;
      this.applyEntity(ev);
      this.derive(ev, false);
      this.tell(this.listeners, ev);
    }
    this.settle();
    if (this.events.length > LOG_CAP * 1.2) {
      this.events = this.events.slice(-LOG_CAP);
      this.forgetEarlier(); // what left the log is earlier history now
    }
    if (persisted) this.rev++;
    if (live) this.liveRev++;
    this.bump(this.revBy, stored);
    this.bump(this.liveRevBy, lively);
  }

  private bump(by: SvelteMap<string, number>, agents: Iterable<string>) {
    for (const id of agents) by.set(id, (by.get(id) ?? 0) + 1);
  }

  /** An agent has an entry in `revBy` and `liveRevBy` from the moment it is known. A panel that
   *  reads an entry that is not there is told of every change of the map (that is how a
   *  `SvelteMap` lets it see the entry appear), so it would run again on every agent's batch. */
  private seedRevs(id: string) {
    for (const by of [this.revBy, this.liveRevBy]) if (!by.has(id)) by.set(id, 0);
  }

  /** A snapshot took the place of what the per-agent versions count: they start again, and
   *  every panel that read one runs again. */
  private resetRevs() {
    this.revBy.clear();
    this.liveRevBy.clear();
    for (const a of this.agents) this.seedRevs(a.id);
  }

  /** The agent's segments, to change in place: copied once per batch, so a list that a panel
   *  holds is never mutated. */
  private segsOf(l: AgentLive): Seg[] {
    if (!this.owned.has(l.segments)) this.owned.add((l.segments = l.segments.slice()));
    return l.segments;
  }

  /** The agent's messages, to change in place (as `segsOf`). */
  private msgsOf(l: AgentLive): AgentMsg[] {
    if (!this.owned.has(l.messages)) this.owned.add((l.messages = l.messages.slice()));
    return l.messages;
  }

  /** The agent's file map, to change in place (as `segsOf`). */
  private filesOf(l: AgentLive): Record<string, FileTouch> {
    if (!this.owned.has(l.files)) this.owned.add((l.files = { ...l.files }));
    this.touched.add(l);
    return l.files;
  }

  /** The end of a batch: the file maps that got an entry are held to their cap. */
  private settle() {
    for (const l of this.touched) l.files = capFiles(l.files);
    this.touched.clear();
  }

  // ───────────── entities (only from live events, never from snapshot replay) ─────────────

  /** An agent's live change (id 0: what it is doing now, its usage; the log keeps no row of it)
   *  that is older than the agent we hold. One that waited for a snapshot must not undo it, or
   *  bring back an agent that was let go meanwhile. */
  private staleAgent(ev: EventRow): boolean {
    if (ev.id !== 0 || ev.kind.type !== 'agent_changed') return false;
    const agent = ev.kind.agent;
    const known = this.agents.find((a) => a.id === agent.id);
    return !known || known.updated_at > agent.updated_at;
  }

  private patchAgent(id: string | null, patch: Partial<Agent>) {
    if (!id) return;
    const i = this.agents.findIndex((a) => a.id === id);
    if (i < 0) return;
    const next = this.agents.slice();
    next[i] = { ...next[i], ...patch };
    this.agents = next;
  }

  /** The agent's current run (or no run named). A start that failed can come up late: the core
   *  drops that dead run's events (`project_agent`), and so do we. */
  private ofRun(ev: EventRow): boolean {
    return ev.run_id == null || this.agents.find((a) => a.id === ev.agent_id)?.run_id === ev.run_id;
  }

  private applyEntity(ev: EventRow) {
    const k = ev.kind;
    switch (k.type) {
      case 'agent_changed':
        // A let-go consultant or a member who left the team leaves the board.
        this.agents = k.agent.archived ? this.agents.filter((a) => a.id !== k.agent.id) : upsert(this.agents, k.agent);
        this.seedRevs(k.agent.id);
        break;
      case 'decision': {
        // Transferred to another of your offices: it leaves this one's crew (and walks out).
        const gone = transferredOut(k, this.workspace?.id);
        if (gone) this.agents = this.agents.filter((a) => a.id !== gone);
        break;
      }
      case 'plan_changed':
        this.plans = upsert(this.plans, k.plan);
        break;
      case 'task_changed':
        this.prevTaskStatus = this.tasks.find((t) => t.id === k.task.id)?.status ?? null;
        this.tasks = upsert(this.tasks, k.task);
        break;
      case 'chat':
        if (!this.chats.some((c) => c.id === k.message.id)) {
          const m = k.message;
          if (m.author === 'user') {
            const o = this.outbox.find((x) => x.thread === m.thread && x.text === m.text.trim());
            if (o) {
              this.chatKeys.set(m.id, `o:${o.id}`);
              this.sent = this.sent.filter((x) => x !== o);
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
        if (this.ofRun(ev)) this.patchAgent(ev.agent_id, { status: k.status });
        break;
      case 'usage_updated':
        if (this.ofRun(ev)) this.patchAgent(ev.agent_id, { usage: k.usage });
        break;
    }
  }

  // ───────────── log-derived structures (live events AND snapshot replay) ─────────────

  liveFor(agentId: string): AgentLive {
    return (this.live[agentId] ??= emptyLive(Date.now()));
  }

  private setFile(l: AgentLive, path: string, ts: number, patch: Partial<FileTouch>, reason: FileReason | null) {
    const files = this.filesOf(l);
    const prev = files[path] ?? {
      path, read: false, modified: false, created: false, deleted: false, provided: false, reasons: [], lastTs: ts,
    };
    const reasons = reason ? [...prev.reasons, reason].slice(-8) : prev.reasons;
    files[path] = { ...prev, ...patch, reasons, lastTs: ts };
  }

  /** `shared`: also drop the live output of the calls that are closed (they were cut off, and
   *  their output is in no later event). `only`: the calls to close (all of them when left out). */
  private closeOpen(l: AgentLive, ts: number, shared: boolean, only?: (seg: Seg) => boolean) {
    const ids = Object.keys(l.open).filter((id) => {
      const seg = l.segments[l.open[id]];
      return !only || !seg || only(seg);
    });
    if (!ids.length) return;
    const segs = this.segsOf(l);
    const open = { ...l.open };
    for (const id of ids) {
      const i = open[id];
      if (segs[i] && segs[i].end == null) segs[i] = { ...segs[i], end: ts };
      if (shared) this.liveOut.delete(id);
      delete open[id];
    }
    l.open = open;
  }

  /** What the agent was writing (a reply, a thought) will not be finished: it leaves the screen. */
  private dropStreams(aid: string | null) {
    for (const [id, s] of this.streams) if (s.agentId === aid) this.streams.delete(id);
  }

  /** The newest `n` of an agent's rows. An agent whose history was loaded on request keeps all. */
  private tail<T>(aid: string, list: T[], n: number): T[] {
    if (list.length <= n || this.deep.has(aid)) return list;
    this.clip(aid);
    return list.slice(-n);
  }

  /** As `tail`, for a list of this batch: the oldest rows leave it in place. Says how many left. */
  private trim<T>(aid: string, list: T[], n: number): number {
    const over = list.length - n;
    if (over <= 0 || this.deep.has(aid)) return 0;
    this.clip(aid);
    list.splice(0, over);
    return over;
  }

  /** Rows of this agent left the screen to a cap: its threads can load them again. */
  private clip(aid: string) {
    if (this.clipped.has(aid)) return;
    this.clipped.add(aid);
    this.forgetEarlier();
  }

  /** A finished step of a task, once, in the order it happened (an older page adds earlier ones). */
  private addMark(tid: string, mark: TaskMark) {
    const marks = this.taskMarks[tid] ?? [];
    if (marks.some((m) => m.eventId === mark.eventId)) return;
    this.taskMarks = { ...this.taskMarks, [tid]: [...marks, mark].sort((a, b) => a.eventId - b.eventId) };
  }

  /** An answered approval still has a place to show: its task's request, or its agent's thread.
   *  (One of an agent that left, with no task, would land in the first request.) */
  private hasThread(a: Approval): boolean {
    return a.task_id != null || this.agents.some((x) => x.id === a.agent_id);
  }

  /** An approval read from history (a snapshot's events, an older page). One we hold is newer. */
  private pastApproval(a: Approval) {
    if (this.hasThread(a) && !this.approvals.some((x) => x.id === a.id)) this.approvals = [...this.approvals, a];
  }

  /** `into`: build that agent's state only (its history, replayed) and touch nothing else. */
  private derive(ev: EventRow, replay: boolean, into?: AgentLive) {
    const k = ev.kind;
    const aid = ev.agent_id;
    if (into && SHARED_KINDS.has(k.type)) return;
    const l = into ?? (aid ? this.liveFor(aid) : null);
    if (l) {
      if (!l.segments.length && !l.messages.length) l.firstTs = Math.min(l.firstTs, ev.ts);
      if (ev.id) l.lastTs = ev.ts;
    }

    switch (k.type) {
      case 'session_started':
        // Replay keeps every run's (the agent may be between runs); live, only its own run's.
        if (l && (replay || this.ofRun(ev))) l.session = { provider: k.provider, model: k.model, cwd: k.cwd, cliVersion: k.cli_version, tools: k.tools, ts: ev.ts };
        break;
      case 'turn_started':
        if (l) {
          l.turn = { startedAt: ev.ts, input: k.input, endedAt: null, status: null };
          l.turns = [...l.turns, { start: ev.ts, end: null, input: k.input }];
          if (aid) l.turns = this.tail(aid, l.turns, 200);
        }
        break;
      case 'prompt_steered':
        // The running turn took this prompt in: no turn of its own will carry it (state/queue.ts).
        if (l) {
          const t = l.turns;
          const open = t[t.length - 1];
          if (open && open.end == null) l.turns = [...t.slice(0, -1), { ...open, steers: [...(open.steers ?? []), { ts: ev.ts, input: k.input }] }];
        }
        break;
      case 'turn_completed':
        if (l) {
          if (l.turn) l.turn = { ...l.turn, endedAt: ev.ts, status: k.status };
          const t = l.turns;
          if (t.length && t[t.length - 1].end == null) l.turns = [...t.slice(0, -1), { ...t[t.length - 1], end: ev.ts }];
          // A turn that was interrupted or that failed takes its helpers with it: none of them
          // is at work any more. Not so for a Claude agent: a helper in the background works on
          // after such a turn, and its adapter says which helpers ended with the turn (a
          // `sub_agent` row "stopped" for each, right after this one), so its rows decide.
          const cli = (aid ? this.agentMap.get(aid)?.provider : undefined) ?? l.session?.provider;
          if (cli !== 'claude' && (k.status === 'interrupted' || k.status === 'failed') && l.subagents.some((s) => s.status === 'running'))
            l.subagents = l.subagents.map((s) => (s.status === 'running' ? { ...s, status: 'stopped', ts: ev.ts } : s));
          // A helper that is still at work when a turn ends normally goes on after it: a call of its
          // own stays open until its result comes, or until the helper is over (`sub_agent`, `exited`).
          const atWork = new Set(l.subagents.filter((s) => s.status === 'running').map((s) => s.callId));
          this.closeOpen(l, ev.ts, !into, (seg) => seg.parent == null || !atWork.has(seg.parent));
          if (!into) this.dropStreams(aid);
        }
        break;
      case 'exited':
        if (l) {
          this.closeOpen(l, ev.ts, !into);
          // Its helpers ran inside the process that is gone.
          if (l.subagents.some((s) => s.status === 'running')) l.subagents = l.subagents.map((s) => (s.status === 'running' ? { ...s, status: 'stopped', ts: ev.ts } : s));
          const t = l.turns;
          if (t.length && t[t.length - 1].end == null) l.turns = [...t.slice(0, -1), { ...t[t.length - 1], end: ev.ts }];
        }
        // The process is gone, so what it was writing stays unfinished. (A turn ends before its
        // process does, so nothing is left here unless that end was lost. An old run's late
        // exit must not take what the agent's new run is writing.)
        if (!into && aid && this.ofRun(ev)) {
          this.dropStreams(aid);
          for (const [id, o] of this.liveOut) if (o.agentId === aid) this.liveOut.delete(id);
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
        // A delta that was on its way while a snapshot was taken: its item is already whole in
        // the snapshot, and a stream opened now would never close (two rows with one key).
        if ((kind === 'text' ? l?.messages : l?.reasoning)?.some((x) => x.itemId === k.item_id)) break;
        const prev = this.streams.get(k.item_id);
        this.streams.set(k.item_id, {
          itemId: k.item_id, agentId: aid, kind, text: (prev?.text ?? '') + k.text, ts: prev?.ts ?? ev.ts,
        });
        break;
      }
      case 'message':
        if (!into) this.streams.delete(k.item_id);
        if (l && aid) {
          const m: AgentMsg = { itemId: k.item_id, agentId: aid, text: k.text, phase: k.phase, ts: ev.ts, eventId: ev.id };
          const list = this.msgsOf(l);
          const was = list.findIndex((x) => x.itemId === k.item_id);
          if (was >= 0) list.splice(was, 1);
          list.push(m);
          this.trim(aid, list, MSG_CAP);
        }
        break;
      case 'reasoning':
        if (!into) this.streams.delete(k.item_id);
        if (l && aid) l.reasoning = this.tail(aid, [...l.reasoning.filter((x) => x.itemId !== k.item_id), { itemId: k.item_id, text: k.text, ts: ev.ts, eventId: ev.id }], 60);
        break;
      case 'tool_started': {
        if (!l || !aid) break;
        const seg: Seg = {
          callId: k.call_id, agentId: aid, taskId: ev.task_id, activity: k.activity, name: k.name,
          label: targetLabel(k.target, k.activity), target: k.target, input: k.input, confidence: k.confidence,
          start: ev.ts, end: null, ok: null, exitCode: null, testsPassed: null, testsFailed: null, eventId: ev.id,
        };
        const segs = this.segsOf(l);
        const existing = l.open[k.call_id];
        if (existing != null) segs[existing] = { ...seg, start: segs[existing].start };
        else {
          segs.push(seg);
          const drop = this.trim(aid, segs, SEG_CAP);
          if (drop) {
            const open: Record<string, number> = {};
            for (const [id, i] of Object.entries(l.open)) if (i - drop >= 0) open[id] = i - drop;
            l.open = open;
          }
          l.open[k.call_id] = segs.length - 1;
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
          const segs = this.segsOf(l);
          segs[i] = {
            ...segs[i], end: ev.ts, ok: k.ok, exitCode: k.exit_code, testsPassed: k.tests_passed, testsFailed: k.tests_failed,
          };
          delete l.open[k.call_id];
          if (segs[i].activity === 'search' && k.output) {
            const q = segs[i].target.query ? `“${segs[i].target.query}”` : segs[i].label;
            l.searches = [{ query: q, output: k.output.slice(0, SEARCH_OUTPUT_CAP) }, ...l.searches.filter((s) => s.output)].slice(0, 6);
          }
        }
        const live = this.liveOut.get(k.call_id);
        const text = k.output ?? live?.text ?? '';
        if (text) l.lastOutput = { callId: k.call_id, text, ts: ev.ts, ok: k.ok };
        if (!into) this.liveOut.delete(k.call_id);
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
          const next: SubAgent = {
            name: null, step: null, toolUses: null, tokens: null, report: null, ...prev,
            callId: k.call_id, description: k.description, status: k.status, startedAt: prev?.startedAt ?? ev.ts, ts: ev.ts,
          };
          l.subagents = [...l.subagents.filter((x) => x.callId !== k.call_id), next].slice(-20);
          // The helper is over: a call of its own that is still open gets no result any more.
          if (k.status !== 'running') this.closeOpen(l, ev.ts, !into, (seg) => seg.parent === k.call_id);
        }
        break;
      case 'sub_agent_progress':
        if (l) {
          // A call of the helper's own (its `tool_started` came just before): it shows under the helper.
          const i = k.tool_call_id != null ? l.open[k.tool_call_id] : undefined;
          if (i != null && l.segments[i]) {
            const segs = this.segsOf(l);
            segs[i] = { ...segs[i], parent: k.call_id };
          }
          const prev = l.subagents.find((x) => x.callId === k.call_id);
          if (prev && (k.name ?? k.step ?? k.tool_uses ?? k.tokens ?? k.report) != null) {
            const next: SubAgent = {
              ...prev, name: k.name ?? prev.name, step: k.step ?? prev.step, toolUses: k.tool_uses ?? prev.toolUses, tokens: k.tokens ?? prev.tokens,
              report: k.report ?? prev.report,
            };
            l.subagents = l.subagents.map((x) => (x === prev ? next : x));
          }
        }
        break;
      case 'mcp_server':
        if (l) l.mcp = withMcpServer(l.mcp, k, ev.run_id, ev.ts);
        break;
      case 'background_tasks':
        if (l) l.background = withBackgroundTasks(l.background, k, ev.run_id, ev.ts, replay);
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
      case 'approval_requested':
        // Replay only (a live one is an entity): an approval that no longer waits shows as it
        // was asked, then as it was answered.
        if (replay) this.pastApproval(k.approval);
        break;
      case 'approval_said':
        // What you said with an answer: the closed card and the history show it from here.
        recordSaid(k);
        break;
      case 'approval_resolved': {
        const a = replay ? this.approvals.find((x) => x.id === k.approval_id) : undefined;
        if (a) this.approvals = upsert(this.approvals, { ...a, status: k.status, decided_by: k.by });
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
        const mark = markOf(ev);
        if (ev.task_id && mark) this.addMark(ev.task_id, mark);
        break;
      }
    }
  }

  // ───────────── history before what is held, loaded on request ─────────────

  /** Every event of this office from this id on is in the log. */
  private floor(): number {
    return this.events[0]?.id ?? this.lastId + 1;
  }

  private setEarlier(thread: string, e: Earlier) {
    this.earlier = { ...this.earlier, [thread]: e };
  }

  private dropEarlier(thread: string) {
    this.paged.delete(thread);
    if (!(thread in this.earlier)) return;
    const { [thread]: _, ...rest } = this.earlier;
    this.earlier = rest;
  }

  /** Rows left the log or the screen: a thread that was not loaded further back (for the agent
   *  it has now) is asked again whether it has earlier history. One that is asking right now
   *  settles that by itself. */
  private forgetEarlier() {
    for (const [thread, e] of Object.entries(this.earlier)) {
      const at = this.paged.get(thread);
      if (!e.loading && (!at || at.owner !== (this.threadAgent(thread)?.id ?? null))) this.dropEarlier(thread);
    }
  }

  /** Find out, once per thread, if it has history before what is held ("Load earlier" shows then). */
  async peekEarlier(thread: string): Promise<void> {
    if (this.earlier[thread]) return;
    const epoch = this.epoch;
    const floor = this.floor();
    const owner = this.threadAgent(thread)?.id ?? null;
    this.setEarlier(thread, { more: false, loading: true });
    let found: boolean;
    try {
      found = (await api.threadEvents(thread, owner, floor, 1)).length > 0;
    } catch (e) {
      // The control shows, so a click says what is wrong.
      console.error('[codly] could not look for earlier history', e);
      found = true;
    }
    if (epoch !== this.epoch) return; // another office's state now
    // Rows left the log meanwhile: the answer is for another floor, so the thread asks again.
    if (floor !== this.floor()) this.dropEarlier(thread);
    else this.setEarlier(thread, { more: found || (owner != null && this.clipped.has(owner)), loading: false });
  }

  /** Load a thread's history before what is shown: one page, or as many as it takes to reach
   *  back to where the held state starts (events the log trimmed since). The owner's state is
   *  built again from the events held and loaded, so they must cover all it was built from:
   *  then the thread, and what its agent shows (session, files, to-do), never get shorter. */
  async loadEarlier(thread: string): Promise<void> {
    if (this.earlier[thread]?.loading) return;
    const epoch = this.epoch;
    const owner = this.threadAgent(thread)?.id ?? null;
    // From now on the owner's events are kept apart from the capped log: what is loaded for
    // it is never trimmed away again.
    if (owner && !this.deep.has(owner)) this.deep.set(owner, this.events.filter((e) => e.agent_id === owner));
    const at = this.paged.get(thread);
    let before = at && at.owner === owner ? at.before : this.floor();
    let more = true;
    const rows: EventRow[] = [];
    this.setEarlier(thread, { more, loading: true });
    try {
      do {
        const page = await api.threadEvents(thread, owner, before, EARLIER_PAGE); // newest first
        if (epoch !== this.epoch) return; // another office's state now
        rows.push(...page);
        more = page.length >= EARLIER_PAGE;
        if (page.length) before = page[page.length - 1].id;
      } while (more && before > this.origin);
    } catch (e) {
      if (epoch === this.epoch) this.setEarlier(thread, { more: true, loading: false });
      throw e;
    }
    this.paged.set(thread, { owner, before });
    this.absorb(thread, owner, rows.reverse());
    this.setEarlier(thread, { more, loading: false });
  }

  /** Older rows of a thread, oldest first: its messages, its approvals and its tasks' finished
   *  steps join what is held; its owner's state is built again from all of the owner's events. */
  private absorb(thread: string, owner: string | null, rows: EventRow[]) {
    const known = new Set(this.chats.map((c) => c.id));
    const said: ChatMessage[] = [];
    for (const ev of rows) {
      const k = ev.kind;
      const mark = markOf(ev);
      if (k.type === 'chat' && k.message.thread === thread && !known.has(k.message.id)) said.push(k.message);
      else if (k.type === 'approval_requested') this.pastApproval(k.approval); // the core sends it as it ended
      else if (k.type === 'approval_said') recordSaid(k);
      else if (mark && ev.task_id) this.addMark(ev.task_id, mark);
    }
    if (said.length) this.chats = [...this.chats, ...said].sort((a, b) => a.ts - b.ts);
    if (owner) {
      const all = new Map<number, EventRow>();
      for (const ev of this.beside) if (ev.agent_id === owner) all.set(ev.id, ev);
      for (const ev of rows) if (ev.agent_id === owner) all.set(ev.id, ev);
      for (const ev of this.deep.get(owner) ?? []) all.set(ev.id, ev);
      const log = [...all.values()].sort((a, b) => a.id - b.id);
      this.deep.set(owner, log);
      const prev = this.live[owner];
      const l = emptyLive(prev?.statusSince ?? Date.now());
      this.owned.clear();
      for (const ev of log) this.derive(ev, true, l);
      this.settle();
      // Output that only came as live chunks is in no stored row.
      if (prev?.lastOutput && (!l.lastOutput || l.lastOutput.ts <= prev.lastOutput.ts)) l.lastOutput = prev.lastOutput;
      // Nor is a server that stands where an earlier run recorded it. The servers as held are
      // the newest there are: older rows add nothing to them.
      if (prev) l.mcp = prev.mcp;
      // A start of background work that the window saw is known, though the rows do not say it.
      if (prev) l.background = withKnownStarts(l.background, prev.background);
      this.live[owner] = l;
      this.bump(this.revBy, [owner]);
    }
    this.rev++;
  }

  // ───────────── queries used by several panels ─────────────

  /**
   * What the agent is doing right now: its own open tool call first, then the agent's own
   * activity. A call of one of its helpers (`seg.parent`) is never the agent's own: while a turn
   * of the agent runs and only such calls are open, it waits for its helper (`seg` null: the
   * call is not the agent's); between turns it rests, also while a helper's calls set its state
   * "working" or "thinking" (narrate.ts `turnRuns`: the same rule).
   */
  now(agentId: string): { activity: ActivityKind; label: string; since: number; seg: Seg | null } | null {
    const a = this.agentMap.get(agentId);
    const l = this.live[agentId];
    if (a?.status === 'waiting_approval') {
      let open: Seg | null = null;
      if (l) for (const i of Object.values(l.open)) if (l.segments[i] && (!open || l.segments[i].start > open.start)) open = l.segments[i];
      return { activity: 'await_approval', label: open?.label ?? '', since: l?.statusSince ?? a.updated_at, seg: open };
    }
    let helping: Seg | null = null;
    if (l) {
      let best: Seg | null = null;
      for (const i of Object.values(l.open)) {
        const s = l.segments[i];
        if (!s) continue;
        if (s.parent == null) {
          if (!best || s.start > best.start) best = s;
        } else if (!helping || s.start > helping.start) helping = s;
      }
      if (best) return { activity: best.activity, label: best.label, since: best.start, seg: best };
    }
    if (!a) return null;
    for (const s of this.streams.values())
      if (s.agentId === agentId) return { activity: s.kind === 'text' ? 'replying' : 'thinking', label: '', since: s.ts, seg: null };
    const turnRuns = l?.turn ? l.turn.endedAt == null : a.status === 'working' || a.status === 'thinking' || a.status === 'starting';
    if (helping && turnRuns) {
      const parent = helping.parent;
      return { activity: 'delegate', label: l?.subagents.find((h) => h.callId === parent)?.description ?? '', since: helping.start, seg: null };
    }
    if (!turnRuns && a.status !== 'starting') return null;
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

  /** Where an approval shows (the broker's rule): its task's request; a Lead's own, its request; an agent with no task, its 1:1. */
  approvalThread(ap: Approval): string {
    if (ap.task_id) return this.taskThread(ap.task_id);
    const a = this.agentMap.get(ap.agent_id);
    if (a?.role === 'lead') return a.thread ?? 'lead';
    if (a?.task_id) return this.taskThread(a.task_id);
    return a ? `agent:${a.id}` : 'lead';
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
  /** Still the office a call was made in. Check it after an await: you may have switched. */
  isOpen(ws: string | null | undefined): boolean {
    return (this.workspace?.id ?? null) === (ws ?? null);
  }
  setSettings(s: Settings) {
    this.settings = s;
  }
  setWorkspaceTeam(t: WorkspaceTeam) {
    this.workspaceTeam = t;
  }
  setRequestPrefs(thread: string, p: RequestPrefs) {
    this.requestPrefs = { ...this.requestPrefs, [thread]: p };
  }
  upsertTask(t: Task) {
    this.tasks = upsert(this.tasks, t);
  }
  upsertAgent(a: Agent) {
    this.agents = upsert(this.agents, a);
    this.seedRevs(a.id);
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
    this.sent = [...this.sent, { id, thread, text, ts, ws: this.workspace?.id ?? null, error: null }];
    return id;
  }
  failOutgoing(id: string, error: string) {
    this.sent = this.sent.map((o) => (o.id === id ? { ...o, error } : o));
  }
  dropOutgoing(id: string) {
    this.sent = this.sent.filter((o) => o.id !== id);
  }
  /** The plan's merge into your branch, if any: on record on the plan (`merged_sha`, so it
   *  survives a reload and a switch of office), else this session's merge command. */
  mergeOf(planId: string): { sha: string } | null {
    const sha = this.planMap.get(planId)?.merged_sha;
    return sha ? { sha } : (this.planMerges[planId] ?? null);
  }
  markPlanMerged(planId: string, sha: string) {
    this.planMerges = { ...this.planMerges, [planId]: { sha, ts: Date.now() } };
    this.rev++;
  }
  /** The plan's merge was undone: this session no longer counts it as merged. */
  unmarkPlanMerged(planId: string) {
    this.planMerges = Object.fromEntries(Object.entries(this.planMerges).filter(([id]) => id !== planId));
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
