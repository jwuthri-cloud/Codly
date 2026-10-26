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
import type { JsonValue } from '../gen/serde_json/JsonValue';
import { VirtualClock } from './clock';

export class Cancelled extends Error {
  constructor() {
    super('cancelled');
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
  sink: ((b: UiBatch) => void) | null = null;

  clis: CliInfo[] = [];
  recent: RecentWorkspace[] = [];
  settings: Settings;
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
  private waiters = new Set<Waiter>();

  constructor(opts: SimOptions, settings: Settings) {
    this.opts = opts;
    this.clock = new VirtualClock({ speed: opts.speed, skipMs: opts.skipMs });
    this.settings = settings;
    setInterval(() => this.flush(), 33);
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

  snapshot(): Snapshot {
    this.flush();
    return {
      as_of: this.lastId,
      clis: this.clis,
      // One sign-in per CLI (the demo has no extra accounts).
      accounts: this.clis.map((c) => ({
        id: c.provider, provider: c.provider, home: null, label: 'default', enabled: !c.problem, primary: true, configured: false,
        logged_in: c.logged_in, billing: c.billing, plan: c.plan, identity: null, problem: c.problem, login_command: null, limits: [],
      })),
      workspace: this.workspace,
      settings: this.settings,
      agents: [...this.agents.values()],
      plans: [...this.plans.values()],
      tasks: [...this.tasks.values()],
      approvals: [...this.approvals.values()],
      chats: this.chats,
      memories: [...this.memories.values()],
      triples: this.triples,
      artifacts: [...this.artifacts.values()],
      events: this.log.slice(-2000),
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
    this.agents.set(a.id, a);
    this.emit({ type: 'agent_changed', agent: a }, { agent: a.id, task: a.task_id, run: a.run_id });
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

  chat(thread: string, author: string, text: string, attachment: JsonValue | null = null) {
    const m: ChatMessage = { id: uid('m-'), thread, author, text, attachment, ts: this.now() };
    this.chats.push(m);
    const agent = author !== 'user' && author !== 'codly' ? author : null;
    this.emit({ type: 'chat', message: m }, { agent });
    return m;
  }

  decision(type: string, method: 'rule' | 'model' | 'human', summary: string, detail: JsonValue, meta: { agent?: string | null; task?: string | null } = {}) {
    this.emit({ type: 'decision', id: uid('d-'), decision_type: type, method, summary, detail }, meta);
  }

  memory(text: string, scope: MemoryScope, kind: MemoryKind, author: string, opts: { scopeId?: string | null; importance?: number; pinned?: boolean; sourceEvent?: number | null; emit?: boolean } = {}) {
    const m: Memory = {
      id: uid('mem-'),
      workspace_id: scope === 'global' ? null : (this.workspace?.id ?? null),
      scope,
      scope_id: opts.scopeId ?? null,
      kind,
      text,
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

  newAgent(o: { name: string; provider: Provider; model: string | null; role: Role; color: number; billing: Billing; taskId?: string | null; worktree?: string | null; status?: AgentStatus; thread?: string }): Agent {
    const now = this.now();
    return {
      id: uid('a-'),
      thread: o.role === 'lead' ? (o.thread ?? 'lead') : null,
      account: null,
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
      usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: null, context_window: (o.model && WINDOWS[o.model]) || null, cost_usd: null },
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
      if (e instanceof Cancelled) {
        this.emit({ type: 'turn_completed', status: 'interrupted', result_text: null, duration_ms: this.sim.now() - started });
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
    if (opts.thread) this.sim.chat(opts.thread, this.id, text, opts.attachment ?? null);
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

  private rateBase = 0;
  rate() {
    const p = this.a.provider;
    this.rateBase += 0.006;
    const hour = 3_600_000;
    if (p === 'claude') {
      this.emit({ type: 'rate_limit', window: 'five_hour', used_fraction: Math.min(0.97, 0.58 + this.rateBase), resets_at: this.sim.now() + 2.2 * hour, status: 'allowed' });
      this.emit({ type: 'rate_limit', window: 'seven_day', used_fraction: 0.31 + this.rateBase / 4, resets_at: this.sim.now() + 86 * hour, status: 'allowed' });
    } else if (p === 'codex') {
      this.emit({ type: 'rate_limit', window: 'five_hour', used_fraction: 0.16 + this.rateBase, resets_at: this.sim.now() + 3.6 * hour, status: 'allowed' });
    }
  }
}
