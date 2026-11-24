// In-memory backend for the browser demo: entity state, the ordered event log, a 30 Hz batch
// bus (like the real UI bus), and "actors" that play agents through the same EventKinds the
// adapters emit. The story (story.ts) is written against these helpers.
import type { UiBatch } from '../api';
import type { ActivityKind } from '../gen/ActivityKind';
import type { Agent } from '../gen/Agent';
import type { AgentStatus } from '../gen/AgentStatus';
import type { Approval } from '../gen/Approval';
import type { Artifact } from '../gen/Artifact';
import type { ArtifactKind } from '../gen/ArtifactKind';
import type { Billing } from '../gen/Billing';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Citation } from '../gen/Citation';
import type { CliInfo } from '../gen/CliInfo';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { McpServerStatus } from '../gen/McpServerStatus';
import type { Memory } from '../gen/Memory';
import type { MemoryKind } from '../gen/MemoryKind';
import type { MemoryScope } from '../gen/MemoryScope';
import type { Plan } from '../gen/Plan';
import type { Provider } from '../gen/Provider';
import type { RecentWorkspace } from '../gen/RecentWorkspace';
import type { Role } from '../gen/Role';
import type { Settings } from '../gen/Settings';
import type { Snapshot } from '../gen/Snapshot';
import type { Target } from '../gen/Target';
import type { Task } from '../gen/Task';
import type { Triple } from '../gen/Triple';
import type { Usage } from '../gen/Usage';
import type { Workspace } from '../gen/Workspace';
import type { WorkspaceTeam } from '../gen/WorkspaceTeam';
import type { RequestPrefs } from '../gen/RequestPrefs';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import { DemoAccounts } from './accounts';
import { VirtualClock } from './clock';
import { emptyCareer } from '../util/career';

export class Cancelled extends Error {
  constructor() {
    super('cancelled');
  }
}

/** Thrown in a turn's body: the turn ends failed, with `reason` as the CLI's adapter says it (`Actor.turn`). */
export class TurnFailed extends Error {
  constructor(readonly reason: string) {
    super(reason);
  }
}

export const HOME = '/Users/demo';
export const REPO = `${HOME}/Code/acme-web`;
export const WORKTREES = `${HOME}/.codly/worktrees/acme-web`;

let uidSeq = 0;
export function uid(prefix = ''): string {
  // UUIDv7-ish: time-sortable, unique enough for a demo.
  const t = Date.now().toString(16).padStart(12, '0');
  const r = Math.random().toString(16).slice(2, 10) + (++uidSeq).toString(16).padStart(4, '0');
  return `${prefix}${t.slice(0, 8)}-${t.slice(8, 12)}-7${r.slice(0, 3)}-${r.slice(3, 7)}-${r.slice(7)}${Math.random().toString(16).slice(2, 6)}`;
}

/** The core's notice for a Claude run that started without the folder's own configuration (orchestrator/runs.rs, with `provider.label()`). */
export const WITHOUT_FOLDER_CONFIG =
  "Started without this folder's Claude Code configuration (its settings, hooks and MCP servers): you have not trusted these files. See Settings → Safety.";

export const sha = () => Array.from({ length: 40 }, () => '0123456789abcdef'[Math.floor(Math.random() * 16)]).join('');

const PRICES: Record<string, { in: number; out: number; cacheRead: number }> = {
  'claude-opus-5-5': { in: 15, out: 75, cacheRead: 1.5 },
  'claude-sonnet-5-5': { in: 3, out: 15, cacheRead: 0.3 },
  'claude-haiku-5-5': { in: 1, out: 5, cacheRead: 0.1 },
};
const WINDOWS: Record<string, number> = {
  'claude-opus-5-5': 1_000_000,
  'claude-sonnet-5-5': 1_000_000,
  'gpt-6.1-sol': 400_000,
  'gemini-3.8-flash': 1_048_576,
};

export type SimOptions = { speed: number; skipMs: number; autoplay: boolean };

type Waiter = { test: () => boolean; resolve: () => void };

export class Sim {
  clock: VirtualClock;
  opts: SimOptions;
  private nextId = 1;
  log: EventRow[] = [];
  private queue: EventRow[] = [];
  /** The last `mcp_server` event of each agent's server, and each agent's last list of background work: the log is cut, and a snapshot still says where they stand. */
  private session = new Map<string, EventRow>();
  sink: ((b: UiBatch) => void) | null = null;

  clis: CliInfo[] = [];
  recent: RecentWorkspace[] = [];
  settings: Settings;
  workspaceTeam: WorkspaceTeam = { off: [], members: [] };
  requestPrefs: Record<string, RequestPrefs> = {};
  /** The demo machine's sign-ins (two Claude accounts, a signed-out Codex one…). */
  signIns: DemoAccounts;
  workspace: Workspace | null = null;
  agents = new Map<string, Agent>();
  plans = new Map<string, Plan>();
  tasks = new Map<string, Task>();
  approvals = new Map<string, Approval>();
  chats: ChatMessage[] = [];
  memories = new Map<string, Memory>();
  triples: Triple[] = [];
  artifacts = new Map<string, Artifact>();
  artifactBytes = new Map<string, ArrayBuffer>();
  /** Task keys whose work Codly committed (their files exist in the repo map). */
  committedKeys = new Set<string>();
  cancelled = new Set<string>();
  /** task key → paths written so far (diffs grow as agents edit). */
  touched = new Map<string, Set<string>>();
  /** The agents that changed a file so far (an agent that changed nothing has no checkpoint). */
  edited = new Set<string>();
  /** Runs after each tool call of an agent's turn, while the turn goes on (the story sets it). */
  afterStep: ((act: Actor) => Promise<void>) | null = null;
  /** Runs when an agent's turn was stopped, after its `turn_completed` (the story sets it). */
  turnStopped: ((act: Actor) => void) | null = null;
  /** Runs when an agent's process starts again, for a turn (the story sets it: the CLI's session line). */
  runStarts: ((act: Actor) => void) | null = null;
  /** Whether a Claude run that starts now leaves out the folder's own CLI configuration: you opened the repo without it (mock.ts sets it). */
  withoutFolderConfig: () => boolean = () => false;
  /** The offices of this machine that the demo plays, this one included (`sameMachineAs`). */
  machine: Sim[] = [this];
  private waiters = new Set<Waiter>();

  constructor(opts: SimOptions, settings: Settings) {
    this.opts = opts;
    this.clock = new VirtualClock({ speed: opts.speed, skipMs: opts.skipMs });
    this.signIns = new DemoAccounts(HOME, this.clock.now());
    this.settings = this.signIns.normalize(settings);
    setInterval(() => this.flush(), 33);
  }

  /**
   * Another office of the same machine: the settings, the installed CLIs, the sign-ins and the
   * recent repositories are the first office's, whichever of the two reads or changes them.
   */
  sameMachineAs(first: Sim) {
    const share = <K extends 'settings' | 'clis' | 'recent' | 'signIns'>(key: K) =>
      Object.defineProperty(this, key, { get: () => first[key], set: (v: Sim[K]) => void (first[key] = v), enumerable: true, configurable: true });
    share('settings');
    share('clis');
    share('recent');
    share('signIns');
    first.machine.push(this);
    this.machine = first.machine;
  }

  now() {
    return this.clock.now();
  }

  // ───────────── bus ─────────────

  emit(kind: EventKind, meta: { agent?: string | null; task?: string | null; run?: string | null } = {}): EventRow {
    const ephemeral = kind.type === 'text_delta' || kind.type === 'reasoning_delta' || kind.type === 'tool_output';
    const row: EventRow = {
      id: ephemeral ? 0 : this.nextId++,
      ts: this.now(),
      agent_id: meta.agent ?? null,
      task_id: meta.task ?? null,
      run_id: meta.run ?? null,
      kind,
    };
    if (!ephemeral) {
      this.log.push(row);
      if (this.log.length > 8000) this.log = this.log.slice(-6000);
      if (kind.type === 'mcp_server') this.session.set(`${row.agent_id}:mcp:${kind.name}`, row);
      if (kind.type === 'background_tasks') this.session.set(`${row.agent_id}:background`, row);
    }
    this.queue.push(row);
    this.check();
    return row;
  }

  flush() {
    if (!this.queue.length || !this.sink) return;
    const events = this.queue;
    this.queue = [];
    this.sink({ events });
  }

  get lastId() {
    return this.nextId - 1;
  }

  /**
   * The agent's process starts: a run of its own, as in the core. Like Claude Code at the start
   * of a session, it says which session it is, then where each of its MCP servers stands (here:
   * as it last stood).
   */
  startRun(agentId: string) {
    const a = this.patchAgent(agentId, { run_id: uid('r-') });
    this.runStarts?.(this.actor(agentId));
    for (const [key, row] of [...this.session]) if (key.startsWith(`${agentId}:mcp:`)) this.emit(row.kind, { agent: a.id, task: a.task_id, run: a.run_id });
  }

  /** Where an agent's MCP server stands, as it last reported (null: it reported none by that name). */
  mcpStatus(agentId: string, name: string): McpServerStatus | null {
    const k = this.session.get(`${agentId}:mcp:${name}`)?.kind;
    return k?.type === 'mcp_server' ? k.status : null;
  }

  snapshot(): Snapshot {
    this.flush();
    const events = this.log.slice(-2000);
    const from = events[0]?.id ?? 0;
    return {
      as_of: this.lastId,
      clis: this.clis,
      accounts: this.signIns.list(this.settings, this.clis),
      workspace: this.workspace,
      settings: this.settings,
      workspace_team: this.workspace ? this.workspaceTeam : null,
      request_prefs: this.requestPrefs,
      agents: [...this.agents.values()],
      plans: [...this.plans.values()],
      tasks: [...this.tasks.values()],
      approvals: [...this.approvals.values()],
      chats: this.chats,
      memories: [...this.memories.values()],
      triples: this.triples,
      artifacts: [...this.artifacts.values()],
      events,
      // Like the core: where each MCP server stands and each agent's last list of background work, when that is older than `events`.
      session_events: [...this.session.values()].filter((e) => e.id < from).sort((a, b) => a.id - b.id),
    };
  }

  // ───────────── waiting ─────────────

  waitFor(test: () => boolean): Promise<void> {
    if (test()) return Promise.resolve();
    return new Promise((resolve) => this.waiters.add({ test, resolve }));
  }

  private check() {
    for (const w of this.waiters)
      if (w.test()) {
        this.waiters.delete(w);
        w.resolve();
      }
  }

  sleep(ms: number) {
    return this.clock.sleep(ms);
  }

  // ───────────── entities ─────────────

  putAgent(a: Agent) {
    const before = this.agents.get(a.id)?.run_id ?? null;
    this.agents.set(a.id, a);
    this.emit({ type: 'agent_changed', agent: a }, { agent: a.id, task: a.task_id, run: a.run_id });
    // Like the core (orchestrator/runs.rs), once per run: a Claude run that leaves out the folder's own configuration says so.
    if (a.run_id && a.run_id !== before && a.provider === 'claude' && this.withoutFolderConfig()) {
      this.emit({ type: 'notice', text: WITHOUT_FOLDER_CONFIG }, { agent: a.id, task: a.task_id, run: a.run_id });
    }
    return a;
  }

  patchAgent(id: string, patch: Partial<Agent>) {
    const prev = this.agents.get(id);
    if (!prev) throw new Error(`no agent ${id}`);
    return this.putAgent({ ...prev, ...patch, updated_at: this.now() });
  }

  putTask(t: Task) {
    this.tasks.set(t.id, t);
    this.emit({ type: 'task_changed', task: t }, { agent: t.assignee, task: t.id });
    return t;
  }

  patchTask(id: string, patch: Partial<Task>) {
    const prev = this.tasks.get(id);
    if (!prev) throw new Error(`no task ${id}`);
    return this.putTask({ ...prev, ...patch, updated_at: this.now() });
  }

  putPlan(p: Plan) {
    this.plans.set(p.id, p);
    this.emit({ type: 'plan_changed', plan: p });
    return p;
  }

  patchPlan(id: string, patch: Partial<Plan>) {
    const prev = this.plans.get(id)!;
    return this.putPlan({ ...prev, ...patch, updated_at: this.now() });
  }

  /** `itemId`: the agent's message this row shows (as the core writes it on the row of a message). */
  chat(thread: string, author: string, text: string, attachment: JsonValue | null = null, itemId?: string) {
    const m: ChatMessage = { id: uid('m-'), thread, author, text, attachment, ts: this.now(), ...(itemId ? { item_id: itemId } : {}) };
    this.chats.push(m);
    const agent = author !== 'user' && author !== 'codly' ? author : null;
    this.emit({ type: 'chat', message: m }, { agent });
    return m;
  }

  decision(type: string, method: 'rule' | 'model' | 'human', summary: string, detail: JsonValue, meta: { agent?: string | null; task?: string | null } = {}) {
    this.emit({ type: 'decision', id: uid('d-'), decision_type: type, method, summary, detail }, meta);
  }

  memory(text: string, scope: MemoryScope, kind: MemoryKind, author: string, opts: { scopeId?: string | null; title?: string; refs?: string[]; importance?: number; pinned?: boolean; sourceEvent?: number | null; emit?: boolean } = {}) {
    const m: Memory = {
      id: uid('mem-'),
      workspace_id: scope === 'global' ? null : (this.workspace?.id ?? null),
      scope,
      scope_id: opts.scopeId ?? null,
      kind,
      text,
      title: opts.title ?? '',
      tags: [],
      refs: opts.refs ?? [],
      author,
      source_event: opts.sourceEvent ?? null,
      importance: opts.importance ?? 0.6,
      pinned: opts.pinned ?? false,
      superseded_by: null,
      created_at: this.now(),
      accessed_at: this.now(),
    };
    this.memories.set(m.id, m);
    if (opts.emit !== false) this.emit({ type: 'memory_added', memory: m }, { agent: author.startsWith('user') || author === 'codly' ? null : author });
    return m;
  }

  triple(subject: string, predicate: string, object: string, source: 'observed' | 'agent' | 'user', opts: { emit?: boolean; agent?: string | null; confidence?: number } = {}) {
    const t: Triple = {
      id: uid('kg-'),
      workspace_id: this.workspace?.id ?? null,
      subject,
      predicate,
      object,
      source,
      source_event: source === 'observed' ? this.lastId : null,
      confidence: opts.confidence ?? (source === 'observed' ? 1 : 0.8),
      valid_from: this.now(),
      valid_to: null,
    };
    this.triples.push(t);
    if (opts.emit !== false) this.emit({ type: 'triple_added', triple: t }, { agent: opts.agent ?? null });
    return t;
  }

  /** Record a file an agent produced as an artifact (emits `artifact_added`). */
  artifact(o: { kind: ArtifactKind; mime: string; path: string; title: string; bytes: ArrayBuffer; agent: Agent; caption?: string | null }) {
    const root = o.agent.worktree ?? this.workspace?.repo_path ?? '';
    const a: Artifact = {
      id: uid('art-'),
      kind: o.kind,
      mime: o.mime,
      path: o.path,
      abs_path: `${root}/${o.path}`,
      title: o.title,
      bytes: o.bytes.byteLength,
      agent_id: o.agent.id,
      task_id: o.agent.task_id,
      caption: o.caption ?? null,
      created_at: this.now(),
    };
    this.artifacts.set(a.id, a);
    this.artifactBytes.set(a.id, o.bytes);
    this.emit({ type: 'artifact_added', artifact: a }, { agent: o.agent.id, task: o.agent.task_id, run: o.agent.run_id });
    return a;
  }

  private actors = new Map<string, Actor>();
  /** One Actor per agent (it accumulates usage totals). */
  actor(id: string): Actor {
    let a = this.actors.get(id);
    if (!a) this.actors.set(id, (a = new Actor(this, id)));
    return a;
  }

  agentByName(name: string) {
    for (const a of this.agents.values()) if (a.name === name) return a;
    return null;
  }

  taskByKey(key: string) {
    for (const t of this.tasks.values()) if (t.key === key) return t;
    return null;
  }

  /** By name: how many agents of that name this office made so far (`agentId`). */
  private named = new Map<string, number>();

  /**
   * A new agent's id: the same on every page load for the same office, name and count of that
   * name, as a real office keeps its agents. What the office keeps of an agent by its id (its
   * look, in the browser's storage) is then its own again after a reload.
   */
  private agentId(name: string): string {
    const n = (this.named.get(name) ?? 0) + 1;
    this.named.set(name, n);
    const key = `${this.workspace?.id ?? ''}\u0000${name}\u0000${n}`;
    // Two FNV-1a hashes of the key (the second of it read backwards): 64 bits, as 16 hex letters.
    const fnv = (s: string) => {
      let h = 2166136261;
      for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
      return (h >>> 0).toString(16).padStart(8, '0');
    };
    return `a-${fnv(key)}${fnv([...key].reverse().join(''))}`;
  }

  newAgent(o: { name: string; provider: Provider; model: string | null; role: Role; color: number; billing: Billing; taskId?: string | null; worktree?: string | null; status?: AgentStatus; thread?: string }): Agent {
    const now = this.now();
    return {
      id: this.agentId(o.name),
      thread: o.role === 'lead' ? (o.thread ?? 'lead') : null,
      account: this.signIns.pick(o.provider, o.role, this.settings, this.clis, [...this.agents.values()], now),
      name: o.name,
      provider: o.provider,
      model: o.model,
      role: o.role,
      status: o.status ?? 'starting',
      activity: null,
      activity_label: null,
      task_id: o.taskId ?? null,
      run_id: uid('r-'),
      session_id: uid(''),
      worktree: o.worktree ?? null,
      billing: o.billing,
      color: o.color,
      look_seed: o.name.toLowerCase(),
      member: null,
      title: null,
      consultant: false,
      kept: false,
      archived: false,
      usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: null, context_window: (o.model && WINDOWS[o.model]) || null, cost_usd: null },
      career: emptyCareer(),
      created_at: now,
      updated_at: now,
    };
  }
}

// ───────────── actors: one agent's behaviour, expressed as EventKinds ─────────────

export type ToolSpec = {
  name: string;
  activity: ActivityKind;
  target?: Partial<Target>;
  input?: JsonValue;
  ms: number;
  output?: string;
  /** Streamed as tool_output chunks over the duration (Codex/ACP style live output). */
  lines?: string[];
  ok?: boolean;
  exit?: number | null;
  passed?: number | null;
  failed?: number | null;
  /** Paths reported as changed when the call completes. */
  files?: string[];
  confidence?: 'observed' | 'inferred';
  /** Task key whose diff grows with these files. */
  taskKey?: string;
};

export class Actor {
  constructor(public sim: Sim, public id: string) {}

  get a(): Agent {
    return this.sim.agents.get(this.id)!;
  }

  meta() {
    const a = this.a;
    return { agent: a.id, task: a.task_id, run: a.run_id };
  }

  emit(kind: EventKind) {
    return this.sim.emit(kind, this.meta());
  }

  async sleep(ms: number) {
    await this.sim.sleep(ms);
    if (this.sim.cancelled.has(this.id)) throw new Cancelled();
  }

  set(patch: Partial<Agent>) {
    return this.sim.patchAgent(this.id, patch);
  }

  status(status: AgentStatus, activity: ActivityKind | null = null, label: string | null = null) {
    const a = this.a;
    if (a.status !== status) this.emit({ type: 'status_changed', status });
    if (a.status !== status || a.activity !== activity || a.activity_label !== label) this.set({ status, activity, activity_label: label });
  }

  /** True while a turn runs (a message sent now would queue). */
  inTurn = false;

  async turn(input: string | null, body: () => Promise<void>) {
    // A turn needs a process: an agent that has none (you stopped it, or it never started) starts one.
    if (!this.a.run_id) this.sim.startRun(this.id);
    const started = this.sim.now();
    this.inTurn = true;
    this.emit({ type: 'turn_started', input });
    this.status('thinking', 'thinking');
    this.rate();
    try {
      await body();
      this.inTurn = false;
      this.emit({ type: 'turn_completed', status: 'completed', result_text: null, duration_ms: this.sim.now() - started });
    } catch (e) {
      this.inTurn = false;
      if (e instanceof TurnFailed) {
        // As the adapters say a failed turn: why, then the end with the same reason. The agent waits for you again.
        this.emit({ type: 'error', message: e.reason, fatal: false });
        this.emit({ type: 'turn_completed', status: 'failed', result_text: e.reason, duration_ms: this.sim.now() - started });
        return;
      }
      if (e instanceof Cancelled) {
        this.emit({ type: 'turn_completed', status: 'interrupted', result_text: null, duration_ms: this.sim.now() - started });
        this.sim.turnStopped?.(this);
      }
      throw e;
    }
  }

  private chunks(text: string): string[] {
    const words = text.match(/\S+\s*/g) ?? [text];
    const out: string[] = [];
    for (let i = 0; i < words.length; ) {
      const n = 1 + Math.floor(Math.random() * 3);
      out.push(words.slice(i, i + n).join(''));
      i += n;
    }
    return out;
  }

  async think(text: string, opts: { pace?: number } = {}) {
    const itemId = uid('rs-');
    this.status('thinking', 'thinking');
    for (const c of this.chunks(text)) {
      this.emit({ type: 'reasoning_delta', item_id: itemId, text: c });
      await this.sleep((opts.pace ?? 55) + Math.random() * 40);
    }
    this.emit({ type: 'reasoning', item_id: itemId, text });
    this.usage(Math.round(text.length * 1.2), Math.round(text.length / 3.6));
  }

  async say(text: string, opts: { phase?: 'final' | 'commentary'; thread?: string; attachment?: JsonValue | null; pace?: number } = {}) {
    const itemId = uid('msg-');
    this.status(this.a.status === 'waiting_approval' ? 'waiting_approval' : 'working', 'replying');
    for (const c of this.chunks(text)) {
      this.emit({ type: 'text_delta', item_id: itemId, text: c });
      await this.sleep((opts.pace ?? 38) + Math.random() * 34);
    }
    this.emit({ type: 'message', item_id: itemId, text, phase: opts.phase ?? 'final' });
    if (opts.thread) this.sim.chat(opts.thread, this.id, text, opts.attachment ?? null, itemId);
    this.usage(600 + Math.round(Math.random() * 900), Math.round(text.length / 3.4));
  }

  plan(steps: [string, 'pending' | 'in_progress' | 'completed'][]) {
    this.emit({ type: 'plan_updated', steps: steps.map(([text, status]) => ({ text, status })) });
  }

  retrieval(query: string, citations: Citation[], confidence: 'exact' | 'high' | 'medium' | 'low' = 'high') {
    this.emit({ type: 'retrieval', query, confidence, citations });
  }

  async tool(s: ToolSpec) {
    const callId = uid('call-');
    const target: Target = { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...(s.target ?? {}) };
    const fileish = ['read', 'edit', 'create', 'delete'].includes(s.activity);
    const label =
      fileish && target.path ? target.path.split('/').pop()! :
      target.command ? target.command.split(/\s+/).slice(0, 3).join(' ') :
      target.query ? target.query :
      target.path ? target.path.split('/').pop()! :
      target.tool ?? '';
    this.emit({
      type: 'tool_started', call_id: callId, name: s.name, activity: s.activity, target,
      input: s.input ?? (target.command ? { command: target.command } : target.path ? { file_path: target.path } : target.query ? { pattern: target.query } : {}),
      confidence: s.confidence ?? 'observed',
    });
    this.status('working', s.activity, label);
    const started = this.sim.now();
    if (s.lines?.length) {
      const per = s.ms / s.lines.length;
      for (const line of s.lines) {
        await this.sleep(per * (0.5 + Math.random()));
        this.emit({ type: 'tool_output', call_id: callId, chunk: line + '\n' });
      }
    } else await this.sleep(s.ms * (0.8 + Math.random() * 0.4));
    if (s.files?.length) {
      this.emit({ type: 'files_changed', call_id: callId, paths: s.files, source: 'tool_input' });
      this.sim.edited.add(this.id);
      if (s.taskKey) {
        const set = this.sim.touched.get(s.taskKey) ?? new Set<string>();
        for (const f of s.files) set.add(f);
        this.sim.touched.set(s.taskKey, set);
      }
    }
    const ok = s.ok ?? true;
    this.emit({
      type: 'tool_completed', call_id: callId, ok,
      output: s.output ?? (s.lines ? s.lines.join('\n') : null),
      exit_code: s.exit !== undefined ? s.exit : target.command ? (ok ? 0 : 1) : null,
      duration_ms: this.sim.now() - started,
      tests_passed: s.passed ?? null,
      tests_failed: s.failed ?? null,
    });
    this.usage(900 + Math.round(Math.random() * 2400), 40 + Math.round(Math.random() * 160), true);
    await this.sim.afterStep?.(this);
    return callId;
  }

  private totals: Usage = { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: null, context_window: null, cost_usd: null };

  usage(input: number, output: number, cached = false) {
    const a = this.a;
    const t = this.totals;
    t.input_tokens += input;
    t.output_tokens += output;
    if (cached) t.cache_read_tokens += input * 6;
    else t.cache_write_tokens += Math.round(input * 0.4);
    t.context_window = a.usage.context_window;
    t.context_used = Math.min((t.context_window ?? 400_000) * 0.92, 18_000 + t.input_tokens * 2.1 + t.cache_read_tokens * 0.12);
    t.context_used = Math.round(t.context_used);
    const price = a.model ? PRICES[a.model] : undefined;
    t.cost_usd = price
      ? (t.input_tokens * price.in + t.output_tokens * price.out + t.cache_read_tokens * price.cacheRead) / 1_000_000
      : null;
    // Keep the agent record in sync so later agent_changed events carry the same usage.
    this.sim.agents.set(this.id, { ...this.a, usage: { ...t } });
    this.emit({ type: 'usage_updated', usage: { ...t } });
  }

  /** Each turn spends a little of its sign-in's quota; the CLI reports its windows. */
  rate() {
    const a = this.a;
    for (const w of this.sim.signIns.bump(a.account ?? a.provider, this.sim.now()))
      this.emit({ type: 'rate_limit', window: w.window, used_fraction: w.used_fraction, resets_at: w.resets_at, status: w.status });
  }
}

/** One turn of an agent that changed files (`editedTurns`). */
export type EditedTurn = {
  /** The id of its `turn_started` row: a later turn has a higher one. */
  id: number;
  /** The task the agent was on (its id), null: its own work. */
  task: string | null;
  ts: number;
  /** As the core names a turn's checkpoint (orchestrator/checkpoints.rs `turn_label`): the prompt on one line, without its tag, cut at 120. */
  label: string;
  /** The files it changed, in the order it first changed them. */
  files: string[];
};

/**
 * The turns of agent `agentId` that changed files, oldest first, as the log tells them: the
 * demo's checkpoints follow them (demo/checkpoints.ts). The log is cut: its oldest turns go.
 */
export function editedTurns(sim: Pick<Sim, 'log'>, agentId: string): EditedTurn[] {
  const turns: EditedTurn[] = [];
  let at: EditedTurn | undefined;
  for (const row of sim.log) {
    if (row.agent_id !== agentId) continue;
    const k = row.kind;
    if (k.type === 'turn_started') {
      const line = (k.input ?? '').trim().replace(/^\[(User|Codly|Lead)\]/, '').split(/\s+/).filter(Boolean).join(' ');
      at = { id: row.id, task: row.task_id, ts: row.ts, label: !line ? 'a turn' : line.length > 120 ? `${line.slice(0, 119)}…` : line, files: [] };
      turns.push(at);
    } else if (k.type === 'files_changed' && at) {
      for (const p of k.paths) if (!at.files.includes(p)) at.files.push(p);
    }
  }
  return turns.filter((t) => t.files.length);
}
