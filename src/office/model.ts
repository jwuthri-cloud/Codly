// The office's state: a pure projection of the event stream (see reducer.ts). The renderer
// and the behaviour layer read it; nothing in here knows about three.js.

import type { ActivityKind } from '../lib/gen/ActivityKind';
import type { AgentStatus } from '../lib/gen/AgentStatus';
import type { Artifact } from '../lib/gen/Artifact';
import type { Provider } from '../lib/gen/Provider';
import type { Role } from '../lib/gen/Role';
import type { TaskStatus } from '../lib/gen/TaskStatus';

export type WorldSelection = { agentId?: string | null; taskId?: string | null; path?: string | null };

export type Tone = 'normal' | 'muted' | 'warn' | 'danger' | 'success';

export interface ToolCall {
  callId: string;
  kind: ActivityKind;
  name: string;
  path: string | null;
  paths: string[];
  command: string | null;
  query: string | null;
  url: string | null;
  tool: string | null;
  server: string | null;
  since: number;
}

export interface SubAgent {
  callId: string;
  description: string;
  since: number;
}

export interface AgentState {
  id: string;
  name: string;
  provider: Provider;
  role: Role;
  /** Team title ("Backend dev") or "Consultant"; null for the rest. */
  badge: string | null;
  color: number;
  seed: string;
  /** Lead agents: the request thread they serve. */
  thread: string | null;
  status: AgentStatus;
  /** When the status last changed from an event (a stale roster must not undo it). */
  statusAt: number;
  taskId: string | null;
  present: boolean;
  /** Open tool calls in start order. */
  tools: Map<string, ToolCall>;
  /** The most recent open call. */
  focus: ToolCall | null;
  thinkingSince: number | null;
  replyingSince: number | null;
  compactingSince: number | null;
  retrying: { attempt: number; since: number } | null;
  approval: { id: string; text: string; since: number } | null;
  blocked: { reason: string; since: number } | null;
  /** The last fatal error, kept for the "Stuck: …" line while the status says blocked. */
  lastError: string | null;
  rateLimited: { since: number; resetsAt: number | null } | null;
  subagents: Map<string, SubAgent>;
  todo: { text: string; done: boolean; active: boolean }[];
  todoAt: number;
  /** A short-lived result line ("Tests pass (33)"), shown for a few seconds. */
  outcome: { text: string; code: string | null; tone: Tone; at: number } | null;
  /** Reads in a burst (for "Reading 12 files…"). */
  readBurst: { count: number; at: number };
  doneAt: number | null;
  lastEventAt: number;
  /** Recent place-bound tool kinds by time (to dedupe errands after a live tool). */
  lastToolAt: Partial<Record<ActivityKind, number>>;
  filesTouched: number;
  /** The last finished tool call (its pose holds through a short think between calls). */
  lastTool: ToolCall | null;
  lastToolEnd: number;
  /** The end of a test run (tool or verify): celebrate or slump where it ran. */
  cheer: { ok: boolean; at: number; atMonitor: boolean } | null;
}

export type Column = 'todo' | 'doing' | 'review' | 'done';

export interface TaskCard {
  id: string;
  key: string;
  title: string;
  status: TaskStatus;
  column: Column;
  assignee: string | null;
  planId: string | null;
  attempt: number;
  updatedAt: number;
  createdAt: number;
}

export interface Note {
  id: string;
  text: string;
  author: string;
  at: number;
}

export interface OfficeModel {
  agents: Map<string, AgentState>;
  tasks: Map<string, TaskCard>;
  /** plan id → request thread ("lead" | "lead:<id>"). */
  plans: Map<string, { thread: string; request: string; status: string; at: number }>;
  notes: Note[];
  memories: { id: string; author: string; at: number }[];
  triples: number;
  commits: { sha: string; agentId: string | null; taskId: string | null; at: number }[];
  merges: { taskId: string | null; agentId: string | null; at: number; branch: string }[];
  tests: { ok: boolean; passed: number | null; failed: number | null; at: number; agentId: string | null; command: string } | null;
  /** Outcomes of recent test runs (oldest first). */
  testHistory: boolean[];
  artifacts: Map<string, Artifact>;
  approvals: Map<string, { agentId: string; text: string; since: number }>;
  resolved: Set<string>;
  lastEventId: number;
}

export function emptyModel(): OfficeModel {
  return {
    agents: new Map(),
    tasks: new Map(),
    plans: new Map(),
    notes: [],
    memories: [],
    triples: 0,
    commits: [],
    merges: [],
    tests: null,
    testHistory: [],
    artifacts: new Map(),
    approvals: new Map(),
    resolved: new Set(),
    lastEventId: 0,
  };
}

export function columnOf(s: TaskStatus): Column {
  switch (s) {
    case 'pending':
    case 'ready':
      return 'todo';
    case 'running':
    case 'changes_requested':
    case 'blocked':
    case 'failed':
      return 'doing';
    case 'verifying':
    case 'reviewing':
    case 'merging':
      return 'review';
    default:
      return 'done';
  }
}

/** Errands: short trips that tell an instant event (a commit, a memory, a card move). */
export type ErrandKind = 'commit' | 'merge' | 'memory' | 'note' | 'card-take' | 'card-return' | 'ask';

/** One-shot effects requested by events (dropped during replay, except tickers). */
export type Fx =
  | { t: 'errand'; agentId: string; kind: ErrandKind; taskId: string | null; at: number }
  | { t: 'card'; taskId: string; from: Column | null; to: Column; at: number }
  | { t: 'plane'; agentId: string; to: string | null; at: number }
  | { t: 'sticky'; noteId: string; agentId: string | null; at: number }
  | { t: 'drawer'; agentId: string | null; at: number }
  | { t: 'catalog'; at: number }
  | { t: 'mail'; agentId: string | null; at: number }
  | { t: 'merge'; agentId: string | null; taskId: string | null; at: number }
  | { t: 'tests'; agentId: string | null; ok: boolean; at: number }
  | { t: 'shelf'; at: number }
  | { t: 'sparkle'; agentId: string; at: number }
  | { t: 'trash'; agentId: string; at: number }
  | { t: 'artifact'; artifactId: string; at: number }
  | { t: 'hello'; agentId: string; at: number }
  | { t: 'ticker'; agentId: string | null; text: string; code: string | null; tone: Tone; at: number; eventId: number };
