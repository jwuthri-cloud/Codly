// What a plan's tasks say about the plan as a whole, for its card and the task sheet.
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import type { Decision } from '../state/store.svelte';

/** In flight: the task has a worker, or Codly checks, reviews or merges its work. */
const ACTIVE = ['running', 'verifying', 'reviewing', 'changes_requested', 'merging'];
/** The reason of a task that waits for your answer starts with this (ask_me.rs's `WAITING`). */
const WAITING = 'waiting for you:';

export type Stuck = {
  /** The tasks that stopped the plan: blocked, failed, or cancelled while others wait for them. */
  stopped: Task[];
  /** The tasks that cannot start because of them. */
  waiting: Task[];
  /** Tasks merged into the plan's integration branch. */
  merged: number;
};

/** A running plan that cannot go on by itself (the core's `scheduler::stuck_plans`): nothing of it
 *  runs, is ready or can become ready, and work is left. Null: it goes on, or it only waits for
 *  your answers (each of those has its own card). `tasks`: the plan's. */
export function stuckPlan(plan: Plan, tasks: Task[]): Stuck | null {
  if (plan.status !== 'running') return null;
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const startable = (t: Task) => t.status === 'pending' && t.depends_on.every((d) => byId.get(d)?.status === 'done');
  if (tasks.some((t) => t.status === 'ready' || ACTIVE.includes(t.status) || startable(t))) return null;
  const waiting = tasks.filter((t) => t.status === 'pending');
  const stopped = tasks.filter(
    (t) => t.status === 'blocked' || t.status === 'failed' || (t.status === 'cancelled' && waiting.some((w) => w.depends_on.includes(t.id))),
  );
  if (!stopped.length && !waiting.length) return null;
  if (stopped.length && stopped.every((t) => t.status === 'blocked' && (t.status_reason ?? '').startsWith(WAITING))) return null;
  return { stopped, waiting, merged: tasks.filter((t) => t.status === 'done').length };
}

/** The tasks that wait for task `id`, directly or through other tasks, and are not over yet:
 *  once `id` is cancelled they can never start. `tasks`: the plan's, returned in that order. */
export function waitingOn(tasks: Task[], id: string): Task[] {
  const found = new Set([id]);
  // Plans are small: look again until a pass finds nothing new.
  for (let before = 0; before !== found.size; ) {
    before = found.size;
    for (const t of tasks) if (t.depends_on.some((d) => found.has(d))) found.add(t.id);
  }
  return tasks.filter((t) => t.id !== id && found.has(t.id) && t.status !== 'done' && t.status !== 'cancelled');
}

/** A task whose verify commands prove nothing about the work: Codly would pass it whatever its worker did. */
export type WeakTask = {
  /** The task's key in the plan ("t1"). */
  task: string;
  /** Each of its verify commands, and why it proves nothing ("always passes"). */
  checks: { command: string; why: string }[];
};

/** The tasks of plan `planId` that the planner marked (the core's `weak_checks`): none of their verify
 *  commands can fail on the work. Read from the plan's `plan_validate` decisions, rule `weak_verify`. */
export function weakTasks(decisions: Decision[], planId: string): WeakTask[] {
  const marked: WeakTask[] = [];
  for (const { type, detail } of decisions) {
    if (type !== 'plan_validate' || !detail || typeof detail !== 'object' || Array.isArray(detail)) continue;
    const { plan_id, rule, task, checks } = detail;
    if (plan_id !== planId || rule !== 'weak_verify' || typeof task !== 'string' || !Array.isArray(checks)) continue;
    const read = checks.flatMap((c) =>
      c && typeof c === 'object' && !Array.isArray(c) && typeof c.command === 'string' && typeof c.why === 'string' ? [{ command: c.command, why: c.why }] : [],
    );
    if (read.length) marked.push({ task, checks: read });
  }
  return marked;
}

/** "t1", "t1 and t2", "t1, t2 and t3". */
export function keyList(tasks: Task[]): string {
  const keys = tasks.map((t) => t.key);
  return keys.length < 2 ? keys.join('') : `${keys.slice(0, -1).join(', ')} and ${keys[keys.length - 1]}`;
}
