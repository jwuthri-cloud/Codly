// Transfers between your offices (mirror of `transfer_agent` in
// crates/codly-core/src/orchestrator/careers.rs): who can go now, and which agent a transfer
// takes out of the open office. The core decides; this is for the click menu and the web demo.

import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import type { Task } from '../gen/Task';

const FINAL = new Set(['done', 'failed', 'cancelled']);
const MID_TURN = new Set(['starting', 'thinking', 'working', 'waiting_approval']);

/** Why it can't transfer right now (null: it can). The Lead never does, so its menu has no Transfer. */
export function transferBlock(a: Agent, tasks: Task[]): string | null {
  if (a.role === 'lead') return 'The Lead stays with its repo';
  if (a.member) return 'Team members get an agent of their own in every repo';
  const held = tasks.find((t) => t.id === a.task_id || (t.assignee === a.id && !FINAL.has(t.status)));
  if (a.task_id || held) return `On ${held?.key ?? 'a task'}: it can't transfer mid-task`;
  if (MID_TURN.has(a.status)) return 'In the middle of a turn: wait for it, or stop it first';
  return null;
}

/** The agent a `transfer` decision moves out of the office `wsId` (it walks out), else null. */
export function transferredOut(k: EventKind, wsId: string | null | undefined): string | null {
  if (k.type !== 'decision' || k.decision_type !== 'transfer' || !wsId) return null;
  const d = k.detail;
  if (!d || typeof d !== 'object' || Array.isArray(d)) return null;
  return d.from === wsId && d.to !== wsId && typeof d.agent === 'string' ? d.agent : null;
}
