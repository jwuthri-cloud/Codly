// The store-bound narration: call `narrate(agentId)` inside a `$derived` (it reads the store's
// version counters and the 1 Hz clock, so it re-runs when anything relevant changes).
// Pure rules live in ../narrate.ts.
import { narrateFrom, type Narration } from '../narrate';
import type { Task } from '../gen/Task';
import { accounts } from './accounts.svelte';
import { diffstats } from './diffstats.svelte';
import { store } from './store.svelte';
import { ui } from './ui.svelte';

/** The agent's most recently finished task (newest first). */
export function lastDoneTask(agentId: string): Task | null {
  let best: Task | null = null;
  for (const t of store.tasks) if (t.assignee === agentId && t.status === 'done' && (!best || t.updated_at > best.updated_at)) best = t;
  return best;
}

export function narrate(agentId: string, now = ui.now): Narration | null {
  void store.rev;
  void store.liveRev;
  const agent = store.agentMap.get(agentId);
  if (!agent) return null;
  const live = store.live[agentId];
  let streaming: 'text' | 'reasoning' | null = null;
  for (const s of store.streams.values()) if (s.agentId === agentId) streaming = s.kind;
  const task = agent.task_id ? (store.taskMap.get(agent.task_id) ?? null) : null;
  const done = !task || task.status === 'done' ? lastDoneTask(agentId) : null;
  const files = done ? (store.taskMarks[done.id] ?? []).find((m) => m.kind === 'committed')?.files?.length ?? 0 : 0;
  let rate = null;
  const account = accounts.idOf(agent);
  for (const r of store.rateLimits) if (r.provider === agent.provider && r.account === account && (!rate || (r.usedFraction ?? 0) > (rate.usedFraction ?? 0))) rate = r;
  return narrateFrom({
    agent,
    now,
    approvals: store.pendingApprovals.filter((a) => a.agent_id === agentId),
    live,
    streaming,
    task,
    lastDone: done ? { task: done, stat: diffstats.task[done.id] ?? null, files } : null,
    rate,
  });
}
