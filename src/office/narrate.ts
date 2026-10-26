// One plain-language line per agent, derived from events only (no LLM), in the bubble copy
// of design-world §8.6: present progressive verb + short target, "…" while running; outcomes
// state the result, never a feeling. Used when the app does not provide its own `narrate`
// (the app's crew-strip sentence wins when given, so labels read the same everywhere).

import type { ActivityKind } from '../lib/gen/ActivityKind';
import { dominant } from './behavior';
import type { AgentState, Tone, ToolCall } from './model';
import { basename, clean, clock, commandHead, hhmm, host } from './text';

export interface Sentence {
  /** Plain text (aria, ticker). */
  text: string;
  /** Prefix, code-styled target, suffix — rendered with textContent only. */
  pre: string;
  code: string | null;
  post: string;
  tone: Tone;
  /** Includes a running clock: re-narrate every second while shown. */
  live: boolean;
}

export const OUTCOME_MS = 4500;

const VERB: Partial<Record<ActivityKind, string>> = {
  read: 'Reading',
  search: 'Searching',
  edit: 'Editing',
  create: 'Creating',
  delete: 'Deleting',
  execute: 'Running',
  test: 'Running',
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
  compacting: 'Tidying notes',
  thinking: 'Thinking',
  replying: 'Writing a reply',
};

function make(pre: string, code: string | null = null, post = '', tone: Tone = 'normal', live = false): Sentence {
  const text = [pre, code].filter(Boolean).join(' ') + post;
  return { text, pre, code, post, tone, live };
}

function target(t: ToolCall): string | null {
  const p = t.path ?? t.paths[0];
  return p ? clean(basename(p), 28) : null;
}

/** What the agent is doing in one tool call. */
export function toolSentence(t: ToolCall, now: number, burst = 1): Sentence {
  const verb = VERB[t.kind] ?? 'Working';
  const elapsed = now - t.since;
  const since = elapsed >= 3000 ? ` · ${clock(elapsed)}` : '';
  switch (t.kind) {
    case 'execute':
    case 'test':
      return make('Running', t.command ? commandHead(t.command) : t.kind === 'test' ? 'tests' : 'a command', '…' + since, 'normal', true);
    case 'search':
      if (t.query) return make('Searching', `“${clean(t.query, 24)}”`, '…' + since);
      return make('Searching', target(t), '…' + since);
    case 'fetch':
      if (t.url) return make('Reading', host(t.url), '…' + since);
      return make('Reading', target(t), '…' + since);
    case 'mcp':
      return make('Asking', clean(t.server ?? t.tool ?? t.name, 24), '…');
    case 'message':
      return make(/ask_lead/.test(t.name) ? 'Asking the Lead…' : 'Sending a message…');
    case 'read': {
      if (burst > 2) return make(`Reading ${burst} files…`);
      const tg = target(t);
      return tg ? make('Reading', tg, '…' + since) : make('Reading…');
    }
    case 'edit':
    case 'create':
    case 'delete': {
      const tg = target(t);
      const more = t.paths.length > 1 ? ` +${t.paths.length - 1}` : '';
      return tg ? make(verb, tg, more + '…' + since) : make(verb + ' files…');
    }
    case 'await_approval':
      return make('Waiting for you:', t.query ? clean(t.query, 40) : 'a question', '', 'warn');
    default:
      return make(verb + '…', null, since);
  }
}

export function sentence(a: AgentState, now: number): Sentence {
  if (!a.present) return make('Offline', null, '', 'muted');
  if (a.approval) {
    if (a.approval.id.startsWith('q:')) return make('Your turn:', a.approval.text, '', 'warn');
    return a.approval.text ? make('May I run', a.approval.text, '?', 'warn') : make('Waiting for you', null, '', 'warn');
  }
  if (a.rateLimited || a.status === 'rate_limited') {
    const r = a.rateLimited?.resetsAt;
    return make(r ? `Out of juice until ${hhmm(r)}` : 'Out of juice (rate limited)', null, '', 'muted');
  }
  if (a.blocked) return make('Stuck:', null, ` ${clean(a.blocked.reason, 48)}`, 'danger');
  if (a.status === 'blocked' && !a.tools.size) return a.lastError ? make('Stuck:', null, ` ${clean(a.lastError, 48)}`, 'danger') : make('Stuck', null, '', 'danger');
  if (a.status === 'failed') return make('Failed', null, '', 'danger');
  if (a.outcome && now - a.outcome.at < OUTCOME_MS) return make(a.outcome.text, a.outcome.code, '', a.outcome.tone);
  const t = dominant(a);
  if (t) return toolSentence(t, now, t.kind === 'read' && now - a.readBurst.at < 3000 ? a.readBurst.count : 1);
  if (a.retrying) return make(`Retrying (${a.retrying.attempt})…`, null, '', 'muted');
  if (a.compactingSince != null) return make('Tidying notes…');
  if (a.replyingSince != null) return make('Writing a reply…');
  if (a.thinkingSince != null) return make('Thinking…');
  if (a.subagents.size) return make('Briefing a helper…');
  switch (a.status) {
    case 'starting':
      return make('Starting…');
    case 'done':
      return make(a.filesTouched ? `Done ✓ · ${a.filesTouched} file${a.filesTouched === 1 ? '' : 's'}` : 'Done ✓', null, '', 'success');
    case 'waiting_approval':
      return make('Waiting for you', null, '', 'warn');
    case 'working':
      return make('Working…');
    case 'thinking':
      return make('Thinking…');
    case 'blocked':
      return make('Stuck', null, '', 'danger');
    default:
      return make(`Idle since ${hhmm(a.lastEventAt)}`, null, '', 'muted');
  }
}
