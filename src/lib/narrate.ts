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
import type { Provider } from './gen/Provider';
import type { Task } from './gen/Task';
import type { AgentLive, RateWindow, Seg } from './state/store.svelte';
import type { DiffStat } from './state/diffstats.svelte';
import { timer } from './util/format';
import { isPlanCall, PLAN_ASK, planStep, showsPlan } from './util/planRequest';
import { firstQuestion } from './util/outcome';
import { permissionsAsked } from './util/permissionsAsked';
import { sandboxHost } from './util/sandbox';
import { basename, oneLine, truncate } from './util/sanitize';

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
  /** Its session is in your terminal ("Open in my terminal"): Codly runs nothing for it until you give it back. */
  inTerminal: boolean;
};

const code = (t: string): Part => ({ t, code: true });
const txt = (t: string): Part => ({ t });

const fileOf = (s: Seg) => {
  const p = s.target.path ?? s.target.paths?.[0] ?? null;
  return p ? basename(p) : s.label.replace(/^“|”$/g, '');
};
const cmdOf = (s: Seg) => oneLine(s.target.command ?? s.label ?? s.name, 44);

export const CODLY_TOOL: Record<string, string> = {
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

/** What an open tool call is doing, as parts. `provider`: the CLI of the agent that makes the call. */
export function activityParts(s: Seg, provider: Provider | null | undefined): Part[] {
  // A Claude agent's plan shown to you is said as that, whatever kind of step the call is.
  if (isPlanCall(s.name, provider)) return [txt(planStep(true))];
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

/** `provider`: the CLI of the agent that asks. */
function approvalParts(ap: Approval, provider: Provider): Part[] {
  const s = ap.subject;
  switch (s.kind) {
    case 'command':
      return [txt('Waiting for you: '), code(oneLine(s.command, 44))];
    case 'file_change':
      return s.paths.length === 1 ? [txt('Waiting for you: change '), code(basename(s.paths[0]))] : [txt(`Waiting for you: change ${s.paths.length} files`)];
    case 'tool': {
      // A Claude agent's plan to read, or a host that a command in Claude Code's sandbox wants:
      // said as that, not by the request's tool name.
      if (showsPlan(s, provider)) return [txt(`Waiting for you: ${PLAN_ASK}`)];
      const host = sandboxHost(s, provider);
      if (host === null) return [txt('Waiting for you: use '), code(oneLine(s.name, 32))];
      return host ? [txt('Waiting for you: reach '), code(truncate(host, 44))] : [txt('Waiting for you: use the network')];
    }
    case 'question': {
      const raw = s.questions as unknown;
      const list = Array.isArray(raw) ? raw : raw && typeof raw === 'object' ? ((raw as { questions?: unknown[] }).questions ?? []) : [];
      const q = list[0] as { question?: string; header?: string } | undefined;
      const text = q?.question ?? q?.header ?? firstQuestion(ap) ?? 'a question';
      return [txt(`Asking you: ${oneLine(text, 64)}`)];
    }
    case 'permissions':
      return [txt(`Waiting for you: ${oneLine(permissionsAsked(s.detail).wants, 56)}`)];
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

/**
 * Whether a turn of the agent runs: by the store's record of its turn when it has one (as
 * queue.ts `busy` reads it), else by the core's word for its state. The state alone is not
 * enough: a helper's tool calls set its agent "working" and "thinking" between turns too (the
 * Claude adapter sets the state for every tool call, a helper's too). The store's `now` reads it
 * the same way. Between turns the agent rests, whatever its helpers still do.
 */
export const turnRuns = (a: Pick<Agent, 'status'>, live: Pick<AgentLive, 'turn'> | null | undefined): boolean =>
  live?.turn
    ? live.turn.endedAt == null
    : a.status === 'working' || a.status === 'thinking' || a.status === 'starting' || a.status === 'waiting_approval';

/** The newest open tool call of `live` that `mine` accepts. */
function newestOpen(live: AgentLive | null | undefined, mine: (s: Seg) => boolean): Seg | null {
  let open: Seg | null = null;
  if (live) for (const idx of Object.values(live.open)) {
    const s = live.segments[idx];
    if (s && s.end == null && mine(s) && (!open || s.start >= open.start)) open = s;
  }
  return open;
}

/**
 * What the agent left at work in the background while it rests: " · 1 helper at work in the
 * background", " · 2 helpers and 1 command at work in the background"; '' when nothing is.
 * Between turns every helper that runs is in the background. A command counts while the list
 * of the agent's live run names it running.
 */
export function backgroundAtWork(agent: Pick<Agent, 'run_id'>, live: AgentLive | null | undefined): string {
  if (!live) return '';
  const helpers = live.subagents.filter((s) => s.status === 'running').length;
  const commands = live.background.filter((w) => agent.run_id != null && w.runId === agent.run_id && w.status === 'running' && w.kind === 'command').length;
  const count = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;
  const what = [helpers && count(helpers, 'helper'), commands && count(commands, 'command')].filter(Boolean).join(' and ');
  return what ? ` · ${what} at work in the background` : '';
}

export function narrateFrom(i: NarrationInput): Narration {
  const { agent: a, now, live } = i;
  const todo = live?.todo ?? [];
  const progress = todo.length ? todo.filter((s) => s.status === 'completed').length / todo.length : null;
  const out = (parts: Part[], tone: Tone, since: number | null = null, showTimer = false): Narration => {
    const key = plainOf(parts);
    const all = showTimer && since != null ? [...parts, txt(' · '), { t: timer(now - since), timer: true }] : parts;
    return { parts: all, text: plainOf(all), key, tone, since, progress: tone === 'done' && progress != null ? 1 : progress };
  };

  // 0 · Its session is in your terminal: whatever its row says, it is yours until you give it back.
  if (i.inTerminal) return out([txt('In your terminal')], 'idle');

  if (a.status === 'offline') return out([txt('Stopped')], 'off');

  // 1 · It needs you: say exactly what is asked.
  const ap = [...i.approvals].sort((x, y) => y.created_at - x.created_at)[0];
  if (ap) return out(approvalParts(ap, a.provider), 'wait', ap.created_at, true);
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
    // "waiting for you:" is ask_me.rs's WAITING prefix: the task waits on your answer, so it is amber, not red.
    return out([txt(why ? `Blocked: ${oneLine(why, 64)}` : 'Blocked')], why?.startsWith('waiting for you:') ? 'wait' : 'blocked');
  }

  // 3 · An open tool call of its own (the most recent one wins). A call of one of its helpers
  // (`parent`) is the helper's work, never the agent's own: after its turn ended the agent rests.
  const open = newestOpen(live, (s) => s.parent == null);
  if (open) {
    const slow = open.activity === 'execute' || open.activity === 'test';
    return out(activityParts(open, a.provider), 'live', open.start, slow);
  }

  // 4 · Streaming.
  if (i.streaming === 'text') return out([txt('Writing a reply')], 'live');
  if (i.streaming === 'reasoning') return out([txt('Thinking')], 'live');

  // 4b · Its turn runs and only its helpers' calls are open: it waits for its helper.
  const runs = turnRuns(a, live);
  const helping = runs ? newestOpen(live, (s) => s.parent != null) : null;
  if (helping) {
    const h = live?.subagents.find((s) => s.callId === helping.parent);
    return out(h?.description ? [txt('Waiting for its helper '), txt(`“${oneLine(h.description, 48)}”`)] : [txt('Waiting for its helper')], 'live', helping.start);
  }

  if (a.status === 'starting') return out([txt('Starting up')], 'live');
  if (runs && (a.status === 'working' || a.status === 'thinking')) {
    const last = live?.segments[live.segments.length - 1];
    const counts = last && last.activity === 'test' && last.end != null ? testCounts(last) : null;
    return out([txt(counts ? `Thinking · last run: ${counts}` : 'Thinking')], 'live');
  }

  // From here it rests: what its helpers and commands still do in the background ends the line.
  const away = backgroundAtWork(a, live);
  const rest = (said: string, tone: Tone) => out([txt(`${said}${away}`)], tone);

  // 5 · Its turn ended; Codly carries the task through the gate.
  const t = i.task;
  if (t && (t.status === 'verifying' || t.status === 'merging' || t.status === 'reviewing' || t.status === 'changes_requested')) {
    const step = t.status === 'verifying' ? 'Codly is verifying' : t.status === 'merging' ? 'in the merge queue' : t.status === 'reviewing' ? 'in review' : 'changes requested';
    return rest(`Handed off · ${step}`, t.status === 'changes_requested' ? 'wait' : 'done');
  }

  // 5b · Its last turn failed: it waits for you, and is not ready as if nothing happened.
  const turn = live?.turn;
  if (turn?.status === 'failed' && turn.endedAt != null) {
    const err = [...(live?.errors ?? [])].reverse().find((e) => e.ts >= turn.startedAt);
    return rest(err ? `Its last turn failed: ${oneLine(err.message, 60)}` : 'Its last turn failed', 'blocked');
  }

  // 6 · Done (with the diff it produced).
  if (i.lastDone) {
    const d = i.lastDone;
    if (d.stat) return rest(`Done — +${d.stat.added} −${d.stat.removed} in ${d.stat.files} ${d.stat.files === 1 ? 'file' : 'files'}`, 'done');
    return rest(d.files ? `Done — ${d.files} ${d.files === 1 ? 'file' : 'files'} changed` : `Done — ${d.task.key} finished`, 'done');
  }

  return rest(a.role === 'solo' ? 'Ready to chat' : a.role === 'lead' ? 'Listening' : 'Idle', 'idle');
}
