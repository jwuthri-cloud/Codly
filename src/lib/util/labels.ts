// The UI vocabulary: one place that turns contract enums into words, so every surface says
// the same thing (Munder critique §7.2: one five-state vocabulary, used identically everywhere).
import type { ActivityKind } from '../gen/ActivityKind';
import type { AgentStatus } from '../gen/AgentStatus';
import type { Billing } from '../gen/Billing';
import type { MemoryKind } from '../gen/MemoryKind';
import type { MemoryScope } from '../gen/MemoryScope';
import type { PlanStatus } from '../gen/PlanStatus';
import type { Provider } from '../gen/Provider';
import type { TaskStatus } from '../gen/TaskStatus';
import type { Target } from '../gen/Target';
import { basename, oneLine } from './sanitize';

// ───────────── five-state vocabulary ─────────────

export type State5 = 'working' | 'waiting' | 'blocked' | 'idle' | 'done';

export function agentState(status: AgentStatus): State5 {
  switch (status) {
    case 'starting':
    case 'thinking':
    case 'working':
      return 'working';
    case 'waiting_approval':
      return 'waiting';
    case 'blocked':
    case 'rate_limited':
    case 'failed':
      return 'blocked';
    case 'done':
      return 'done';
    default:
      return 'idle';
  }
}

export const STATE_LABEL: Record<State5, string> = {
  working: 'Working',
  waiting: 'Waiting on you',
  blocked: 'Blocked',
  idle: 'Idle',
  done: 'Done',
};

export const STATUS_DETAIL: Partial<Record<AgentStatus, string>> = {
  starting: 'Starting',
  thinking: 'Thinking',
  rate_limited: 'Rate limited',
  failed: 'Failed',
  offline: 'Offline',
};

// ───────────── activities ─────────────

const VERB: Record<ActivityKind, string> = {
  idle: 'Idle',
  thinking: 'Thinking',
  replying: 'Writing a reply',
  read: 'Reading',
  search: 'Searching',
  edit: 'Editing',
  create: 'Creating',
  delete: 'Deleting',
  execute: 'Running',
  test: 'Running tests',
  fetch: 'Reading',
  mcp: 'Asking',
  plan: 'Updating the plan',
  delegate: 'Briefing a helper',
  message: 'Messaging',
  note: 'Posting a note',
  memory: 'Saving a memory',
  review: 'Reviewing',
  commit: 'Committing',
  merge: 'Merging',
  await_approval: 'Waiting for approval',
  blocked: 'Stuck',
  retrying: 'Retrying',
  rate_limited: 'Out of quota',
  compacting: 'Tidying context',
  done: 'Done',
};

const PAST: Record<ActivityKind, string> = {
  idle: 'Idle',
  thinking: 'Thought',
  replying: 'Replied',
  read: 'Read',
  search: 'Searched',
  edit: 'Edited',
  create: 'Created',
  delete: 'Deleted',
  execute: 'Ran',
  test: 'Ran tests',
  fetch: 'Fetched',
  mcp: 'Called',
  plan: 'Updated the plan',
  delegate: 'Briefed a helper',
  message: 'Messaged',
  note: 'Posted a note',
  memory: 'Saved a memory',
  review: 'Reviewed',
  commit: 'Committed',
  merge: 'Merged',
  await_approval: 'Asked for approval',
  blocked: 'Got stuck',
  retrying: 'Retried',
  rate_limited: 'Hit the rate limit',
  compacting: 'Compacted context',
  done: 'Finished',
};

export const verb = (a: ActivityKind) => VERB[a] ?? a;
export const pastVerb = (a: ActivityKind) => PAST[a] ?? a;

/** Short label of what a tool call targets (basename for files, first tokens for commands). */
export function targetLabel(t: Target | null | undefined, a?: ActivityKind): string {
  if (!t) return '';
  const fileish = a === 'read' || a === 'edit' || a === 'create' || a === 'delete';
  if (fileish && t.path) return basename(t.path);
  if (t.command) return a === 'test' ? oneLine(t.command, 34) : oneLine(t.command, 48);
  if (t.query) return `“${oneLine(t.query, 32)}”`;
  if (t.path) return basename(t.path);
  if (t.paths?.length) return t.paths.length === 1 ? basename(t.paths[0]) : `${t.paths.length} files`;
  if (t.url) return oneLine(t.url.replace(/^https?:\/\//, ''), 36);
  if (t.server || t.tool) return [t.server, t.tool].filter(Boolean).join('.');
  return '';
}

/** "Editing app.ts", "Running npm test", "Searching “useAuth”". */
export function phrase(a: ActivityKind, label?: string | null): string {
  if (a === 'await_approval') return 'Waiting on you';
  const l = label ? oneLine(label, 60) : '';
  if (a === 'test' && l) return `Running ${l}`;
  return l ? `${verb(a)} ${l}` : verb(a);
}

export type ActivityClass = 'write' | 'exec' | 'read' | 'think' | 'wait';

export function activityClass(a: ActivityKind): ActivityClass {
  switch (a) {
    case 'edit':
    case 'create':
    case 'delete':
    case 'commit':
    case 'merge':
      return 'write';
    case 'execute':
    case 'test':
      return 'exec';
    case 'read':
    case 'search':
    case 'fetch':
    case 'mcp':
      return 'read';
    case 'await_approval':
    case 'blocked':
    case 'rate_limited':
    case 'retrying':
      return 'wait';
    default:
      return 'think';
  }
}

// ───────────── tasks & plans ─────────────

export type Column = 'backlog' | 'ready' | 'running' | 'review' | 'done' | 'blocked';

export function taskColumn(s: TaskStatus): Column {
  switch (s) {
    case 'pending':
      return 'backlog';
    case 'ready':
      return 'ready';
    case 'running':
      return 'running';
    case 'verifying':
    case 'reviewing':
    case 'changes_requested':
    case 'merging':
      return 'review';
    case 'done':
      return 'done';
    default:
      return 'blocked';
  }
}

export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  pending: 'Backlog',
  ready: 'Ready',
  running: 'Running',
  verifying: 'Verifying',
  reviewing: 'In review',
  changes_requested: 'Changes requested',
  merging: 'Merging',
  done: 'Done',
  blocked: 'Blocked',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

export function taskState(s: TaskStatus): State5 {
  switch (s) {
    case 'running':
    case 'verifying':
    case 'reviewing':
    case 'changes_requested':
    case 'merging':
      return 'working';
    case 'done':
      return 'done';
    case 'blocked':
    case 'failed':
      return 'blocked';
    default:
      return 'idle';
  }
}

export const PLAN_STATUS_LABEL: Record<PlanStatus, string> = {
  planning: 'Planning',
  awaiting_approval: 'Awaiting your go',
  running: 'Running',
  integrating: 'Integrating',
  done: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

export function planState(s: PlanStatus): State5 {
  switch (s) {
    case 'awaiting_approval':
      return 'waiting';
    case 'running':
    case 'integrating':
    case 'planning':
      return 'working';
    case 'done':
      return 'done';
    case 'failed':
      return 'blocked';
    default:
      return 'idle';
  }
}

// ───────────── providers, billing, models ─────────────

export const PROVIDER_LABEL: Record<Provider, string> = {
  claude: 'Claude Code',
  codex: 'Codex',
  gemini: 'Gemini CLI',
  opencode: 'OpenCode',
  cursor: 'Cursor',
  copilot: 'Copilot',
};

/** The company behind each product. OpenCode is open source and routes to any model provider. */
export const COMPANY_LABEL: Record<Provider, string> = {
  claude: 'Anthropic',
  codex: 'OpenAI',
  gemini: 'Google',
  opencode: 'Open source',
  cursor: 'xAI',
  copilot: 'GitHub',
};

export const BILLING_LABEL: Record<Billing, string> = {
  subscription: 'Subscription',
  api_key: 'API key',
  third_party: 'Third party',
  unknown: 'Unknown billing',
};

export const BILLING_TIP: Record<Billing, string> = {
  subscription: 'Runs on the plan you already pay for. Costs shown are list-price estimates, included in your plan.',
  api_key: 'Billed per token to an API key found in this CLI’s configuration.',
  third_party: 'Billed by the third-party provider configured in this CLI.',
  unknown: 'Codly could not tell how this CLI is billed.',
};

const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** "claude-sonnet-5-5" → "Sonnet 5.5", "gpt-6.1-sol" → "GPT-6.1 Sol", "gemini-3.8-flash" → "Gemini 3.8 Flash", "opus[1m]" → "Opus (1M)". */
export function prettyModel(model: string | null | undefined): string {
  if (!model) return '';
  let m = /^claude-([a-z]+)-(\d+)-(\d+)/.exec(model);
  if (m) return `${cap(m[1])} ${m[2]}.${m[3]}`;
  m = /^gpt-([\d.]+)(?:-(.+))?$/.exec(model);
  if (m) return `GPT-${m[1]}${m[2] ? ' ' + m[2].split('-').map(cap).join(' ') : ''}`;
  // Claude Code's aliases: "opus[1m]" → "Opus (1M)".
  m = /^(opus|sonnet|haiku)(\[1m\])?$/.exec(model);
  if (m) return `${cap(m[1])}${m[2] ? ' (1M)' : ''}`;
  m = /^gemini-([\d.]+)-(.+)$/.exec(model);
  if (m) return `Gemini ${m[1]} ${m[2].split('-').map(cap).join(' ')}`;
  return model;
}

// ───────────── identity ─────────────

/** Agent identity hue: `color` indexes the 8 tokens `--id-1`…`--id-8`. */
export const idVar = (color: number | bigint | null | undefined) =>
  `var(--id-${(((Number(color ?? 0) % 8) + 8) % 8) + 1})`;

// ───────────── memory ─────────────

export const SCOPE_LABEL: Record<MemoryScope, string> = {
  global: 'Global',
  project: 'Shared',
  agent: 'Private',
  task: 'Task',
};

export const SCOPE_TIP: Record<MemoryScope, string> = {
  global: 'Shared by every agent in every project.',
  project: 'Shared by every agent in this workspace — the team’s memory.',
  agent: 'Private to one agent.',
  task: 'Scoped to one task; passed to the tasks that depend on it.',
};

export const KIND_LABEL: Record<MemoryKind, string> = {
  fact: 'Fact',
  decision: 'Decision',
  lesson: 'Lesson',
  preference: 'Preference',
  note: 'Note',
  summary: 'Summary',
};

// ───────────── provenance ─────────────

export type Provenance = 'observed' | 'inferred' | 'provided' | 'model';

export const PROVENANCE_LABEL: Record<Provenance, string> = {
  observed: 'Observed',
  inferred: 'Inferred',
  provided: 'Provided',
  model: 'Model-stated',
};

export const PROVENANCE_TIP: Record<Provenance, string> = {
  observed: 'Observed: came verbatim from an event the CLI emitted.',
  inferred: 'Inferred: a Codly heuristic. Usually right, sometimes wrong.',
  provided: 'Provided: Codly put this in the agent’s context (retrieval, memory, notes).',
  model: 'Model-stated: the model’s own words. Plausible, not verified.',
};

/** Decision.method → badge. */
export function methodProvenance(method: string): Provenance {
  if (method === 'model') return 'model';
  if (method === 'human') return 'observed';
  return 'inferred';
}

// ───────────── glossary (educational tooltips) ─────────────

export const GLOSSARY = {
  worktree: 'A separate checkout of your repo on its own branch, so agents never step on each other or on you.',
  context: 'How much of the model’s working memory the latest turn used. A snapshot, not a running total.',
  cache: 'Tokens re-read from the provider’s prompt cache: cheaper and faster than fresh input.',
  tool: 'One action the agent took through its CLI: reading a file, running a command, editing code…',
  integration: 'Every task merges into this branch first. Your branch is only touched when you click Merge.',
  critical: 'The longest chain of unfinished, dependent tasks. It bounds when the plan can finish.',
} as const;
