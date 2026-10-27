// Action chips: the two-or-three-word bubble over an agent that is not showing its full
// sentence ("using Bash", "compacting context", "idle"). Derived from the same open tool
// calls and status flags the reducer keeps from real events — nothing is invented, and an
// agent with nothing real to say gets no chip at all.

import type { ActivityKind } from '../lib/gen/ActivityKind';
import { dominant } from './behavior';
import type { AgentState, Tone } from './model';
import { clean } from './text';

export interface Chip {
  text: string;
  tone: Tone;
}

/** What a camelCase protocol item type ("fileChange", "commandExecution") stands for. */
const KIND_NOUN: Partial<Record<ActivityKind, string>> = {
  read: 'file read',
  search: 'search',
  edit: 'file edit',
  create: 'file edit',
  delete: 'file edit',
  execute: 'shell',
  test: 'shell',
  fetch: 'web',
};

/** The tool as a person would name it: "Bash", or "fetch" for an MCP call on server "fetch". */
function toolLabel(a: AgentState): string | null {
  const t = dominant(a);
  if (!t) return null;
  if (t.kind === 'mcp') return clean(t.server ?? t.tool ?? t.name, 18) || null;
  // Codex reports item types, not tool names: say what the item does instead.
  if (/^[a-z]+[A-Z]/.test(t.name) && KIND_NOUN[t.kind]) return KIND_NOUN[t.kind]!;
  return clean(t.name, 18) || null;
}

/** The chip for an agent right now, or null when nothing real is happening to show. */
export function chipFor(a: AgentState): Chip | null {
  if (!a.present) return null;
  if (a.approval) return { text: 'needs you', tone: 'warn' };
  if (a.rateLimited || a.status === 'rate_limited') return { text: 'rate limited', tone: 'muted' };
  if (a.blocked || a.status === 'blocked' || a.status === 'failed') return { text: a.status === 'failed' ? 'failed' : 'stuck', tone: 'danger' };
  const tool = toolLabel(a);
  if (tool) return { text: `using ${tool}`, tone: 'normal' };
  if (a.retrying) return { text: 'retrying', tone: 'muted' };
  if (a.compactingSince != null) return { text: 'compacting context', tone: 'normal' };
  if (a.replyingSince != null) return { text: 'replying', tone: 'normal' };
  if (a.thinkingSince != null || a.status === 'thinking') return { text: 'thinking', tone: 'normal' };
  if (a.subagents.size) return { text: 'briefing a helper', tone: 'normal' };
  switch (a.status) {
    case 'starting':
      return { text: 'starting', tone: 'muted' };
    case 'done':
      return { text: 'done', tone: 'success' };
    case 'idle':
      return { text: 'idle', tone: 'muted' };
    case 'working':
      return { text: 'working', tone: 'normal' };
    default:
      return null;
  }
}
