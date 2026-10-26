// Narration: ONE plain-language sentence per agent, derived deterministically from its latest
// events (no LLM). Rule: verb + target (+ since for things that take a while); tests show counts;
// approvals say exactly what is asked. The crew strip, the office label and the agent sheet all
// render the same sentence, so an agent reads the same everywhere.
//
//   "Reading `auth.ts`" · "Running `pnpm test` · 0:41" · "Waiting for you: `rm -rf dist`"
//   "Done — +48 −6 in 2 files"
//
// `narrateFrom` is pure (unit-tested); `narrate(agentId)` binds it to the live store.
import type { Agent } from './gen/Agent';
import type { Approval } from './gen/Approval';
import type { Task } from './gen/Task';
import type { AgentLive, RateWindow, Seg } from './state/store.svelte';
import type { DiffStat } from './state/diffstats.svelte';
import { timer } from './util/format';
import { basename, oneLine } from './util/sanitize';

export type Part = { t: string; code?: boolean; timer?: boolean };
/** live = working · wait = needs you · done · idle · blocked · off (stopped). */
export type Tone = 'live' | 'wait' | 'done' | 'idle' | 'blocked' | 'off';
export type Narration = {
  parts: Part[];
  /** Full sentence, including the timer ("Running `npm test` · 0:41"), backticks kept. */
  text: string;
  /** Sentence without the ticking timer: stable, for aria-live announcements. */
  key: string;
  tone: Tone;
  since: number | null;
  /** Todo-list progress 0…1 (null = the agent keeps no list). */
  progress: number | null;
};

export type NarrationInput = {
  agent: Agent;
  now: number;
  /** Pending approvals for this agent. */
  approvals: Approval[];
  live: AgentLive | null | undefined;
  /** An open text/reasoning stream, if any. */
  streaming: 'text' | 'reasoning' | null;
  /** The agent's current task. */
  task: Task | null;
  /** The agent's most recent finished task and its diffstat (if known). */
  lastDone: { task: Task; stat: DiffStat | null; files: number } | null;
  /** The worst rate-limit window for its provider. */
  rate: RateWindow | null;
};

const code = (t: string): Part => ({ t, code: true });
const txt = (t: string): Part => ({ t });

const fileOf = (s: Seg) => {
  const p = s.target.path ?? s.target.paths?.[0] ?? null;
  return p ? basename(p) : s.label.replace(/^“|”$/g, '');
};
const cmdOf = (s: Seg) => oneLine(s.target.command ?? s.label ?? s.name, 44);

const CODLY_TOOL: Record<string, string> = {
  codly_memory_write: 'Saving a memory for the team',
  codly_memory_search: 'Searching the team’s memory',
  codly_post_note: 'Posting a note for the team',
  codly_submit_plan: 'Submitting the plan',
  codly_submit_report: 'Writing its report',
  codly_search: 'Searching the code index',
  codly_ask_lead: 'Asking the Lead',
  codly_kg_add: 'Recording what it learned',
  codly_kg_query: 'Looking up the knowledge graph',
};

function toolName(s: Seg): string {
  const raw = s.target.tool ?? s.name;
  return raw.replace(/^mcp__codly__/, '').replace(/^codly\./, '');
}

/** What an open tool call is doing, as parts. */
export function activityParts(s: Seg): Part[] {
  switch (s.activity) {
    case 'read':
      return [txt('Reading '), code(fileOf(s))];
    case 'fetch':
      return s.target.url ? [txt('Reading '), code(oneLine(s.target.url.replace(/^https?:\/\//, ''), 36))] : [txt('Fetching')];
    case 'search':
      return s.target.query ? [txt('Searching for '), txt(`“${oneLine(s.target.query, 36)}”`)] : [txt('Searching the code')];
    case 'edit':
      return [txt('Editing '), code(fileOf(s))];
    case 'create':
      return [txt('Creating '), code(fileOf(s))];
    case 'delete':
      return [txt('Deleting '), code(fileOf(s))];
    case 'execute':
    case 'test':
      return [txt('Running '), code(cmdOf(s))];
    case 'mcp':
    case 'note':
    case 'memory': {
      const name = toolName(s);
      if (CODLY_TOOL[name]) return [txt(CODLY_TOOL[name])];
      if (s.activity === 'note') return [txt('Posting a note for the team')];
      if (s.activity === 'memory') return [txt('Saving a memory for the team')];
      return [txt('Using '), code(oneLine(name, 32))];
    }
    case 'plan':
      return [txt(/submit_plan/.test(s.name) ? 'Submitting the plan' : 'Updating its plan')];
    case 'delegate':
      return [txt('Briefing a helper')];
    case 'message':
      return [txt('Messaging the team')];
    case 'review':
      return s.label ? [txt('Reviewing '), code(s.label)] : [txt('Reviewing')];
    case 'commit':
      return [txt('Committing')];
    case 'merge':
      return [txt('Merging')];
    case 'compacting':
      return [txt('Tidying its context')];
    case 'retrying':
      return [txt('Retrying')];
    default:
      return [txt('Working')];
  }
}

function approvalParts(ap: Approval): Part[] {
  const s = ap.subject;
  switch (s.kind) {
    case 'command':
      return [txt('Waiting for you: '), code(oneLine(s.command, 44))];
    case 'file_change':
      return s.paths.length === 1 ? [txt('Waiting for you: change '), code(basename(s.paths[0]))] : [txt(`Waiting for you: change ${s.paths.length} files`)];
    case 'tool':
      return [txt('Waiting for you: use '), code(oneLine(s.name, 32))];
    case 'question': {
      const raw = s.questions as unknown;
      const list = Array.isArray(raw) ? raw : raw && typeof raw === 'object' ? ((raw as { questions?: unknown[] }).questions ?? []) : [];
      const q = list[0] as { question?: string; header?: string } | undefined;
      const text = q?.question ?? q?.header ?? 'a question';
      return [txt(`Asking you: ${oneLine(text, 64)}`)];
    }
    default:
      return [txt('Waiting for you: permissions')];
  }
}

function testCounts(s: Seg): string | null {
  if (s.testsPassed == null && s.testsFailed == null) return null;
  const p = s.testsPassed ?? 0;
  const f = s.testsFailed ?? 0;
  if (f) return `${f} failed, ${p} passed`;
  return `${p} ${p === 1 ? 'test' : 'tests'} passed`;
}

const plainOf = (parts: Part[]) => parts.map((p) => (p.code ? `\`${p.t}\`` : p.t)).join('');

export function narrateFrom(i: NarrationInput): Narration {
  const { agent: a, now, live } = i;
  const todo = live?.todo ?? [];
  const progress = todo.length ? todo.filter((s) => s.status === 'completed').length / todo.length : null;
  const out = (parts: Part[], tone: Tone, since: number | null = null, showTimer = false): Narration => {
    const key = plainOf(parts);
    const all = showTimer && since != null ? [...parts, txt(' · '), { t: timer(now - since), timer: true }] : parts;
    return { parts: all, text: plainOf(all), key, tone, since, progress: tone === 'done' && progress != null ? 1 : progress };
  };

  if (a.status === 'offline') return out([txt('Stopped')], 'off');

  // 1 · It needs you: say exactly what is asked.
  const ap = [...i.approvals].sort((x, y) => y.created_at - x.created_at)[0];
  if (ap) return out(approvalParts(ap), 'wait', ap.created_at, true);
  if (a.status === 'waiting_approval') return out([txt('Waiting for you')], 'wait', live?.statusSince ?? null, true);

  // 2 · It can't move.
  if (a.status === 'rate_limited') {
    const r = i.rate?.resetsAt ? i.rate.resetsAt - now : null;
    const when = r != null && r > 0 ? (r < 3_600_000 ? ` · resets in ${Math.max(1, Math.round(r / 60_000))} min` : ` · resets in ${Math.floor(r / 3_600_000)} h ${Math.round((r % 3_600_000) / 60_000)} min`) : '';
    return out([txt(`Out of quota${when}`)], 'blocked');
  }
  if (a.status === 'failed') {
    const err = live?.errors[live.errors.length - 1];
    return out([txt(err ? `Failed: ${oneLine(err.message, 60)}` : 'Failed')], 'blocked');
  }
  if (a.status === 'blocked' || i.task?.status === 'blocked') {
    const why = i.task?.status_reason;
    return out([txt(why ? `Blocked: ${oneLine(why, 64)}` : 'Blocked')], 'blocked');
  }

  // 3 · An open tool call (the most recent one wins).
  let open: Seg | null = null;
  if (live) for (const idx of Object.values(live.open)) {
    const s = live.segments[idx];
    if (s && s.end == null && (!open || s.start >= open.start)) open = s;
  }
  if (open) {
    const slow = open.activity === 'execute' || open.activity === 'test';
    return out(activityParts(open), 'live', open.start, slow);
  }

  // 4 · Streaming.
  if (i.streaming === 'text') return out([txt('Writing a reply')], 'live');
  if (i.streaming === 'reasoning') return out([txt('Thinking')], 'live');

  const busy = a.status === 'working' || a.status === 'thinking' || a.status === 'starting';
  if (busy) {
    if (a.status === 'starting') return out([txt('Starting up')], 'live');
    const last = live?.segments[live.segments.length - 1];
    const counts = last && last.activity === 'test' && last.end != null ? testCounts(last) : null;
    return out([txt(counts ? `Thinking · last run: ${counts}` : 'Thinking')], 'live');
  }

  // 5 · Its turn ended; Codly carries the task through the gate.
  const t = i.task;
  if (t && (t.status === 'verifying' || t.status === 'merging' || t.status === 'reviewing' || t.status === 'changes_requested')) {
    const step = t.status === 'verifying' ? 'Codly is verifying' : t.status === 'merging' ? 'in the merge queue' : t.status === 'reviewing' ? 'in review' : 'changes requested';
    return out([txt(`Handed off · ${step}`)], t.status === 'changes_requested' ? 'wait' : 'done');
  }

  // 6 · Done (with the diff it produced).
  if (i.lastDone) {
    const d = i.lastDone;
    if (d.stat) return out([txt(`Done — +${d.stat.added} −${d.stat.removed} in ${d.stat.files} ${d.stat.files === 1 ? 'file' : 'files'}`)], 'done');
    return out([txt(d.files ? `Done — ${d.files} ${d.files === 1 ? 'file' : 'files'} changed` : `Done — ${d.task.key} finished`)], 'done');
  }

  return out([txt(a.role === 'solo' ? 'Ready to chat' : a.role === 'lead' ? 'Listening' : 'Idle')], 'idle');
}
