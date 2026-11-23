// Words for rows of an agent's History that are built from ids or lists: a fact of the knowledge
// graph, a task that changed, a change of what runs in the background. And which rows of a task
// say something new. Pure, so the sheet and the tests agree; the sheet passes the names.
import type { EventKind } from '../lib/gen/EventKind';
import type { Task } from '../lib/gen/Task';
import type { Triple } from '../lib/gen/Triple';
import { TASK_STATUS_LABEL } from '../lib/util/labels';
import { oneLine } from '../lib/util/sanitize';

/** How the sheet looks up what an id is called: an agent's name, a task's key ("t1"). */
export type Names = { agent: (id: string) => string | undefined; task: (id: string) => string | undefined };

/**
 * A graph entity ("agent:<id>", "task:<id>", "file:<path>", "concept:<name>") as a person reads
 * it. An agent or a task that is not known any more is "an agent" / "a task", never its raw id.
 */
export function entityWord(entity: string, names: Names): string {
  const at = entity.indexOf(':');
  if (at < 0) return entity;
  const type = entity.slice(0, at);
  const value = entity.slice(at + 1);
  if (type === 'agent') return names.agent(value) ?? 'an agent';
  if (type === 'task') return names.task(value) ?? 'a task';
  return value || entity;
}

/** "Ada created src/theme/ThemeProvider.tsx" */
export function tripleLine(t: Pick<Triple, 'subject' | 'predicate' | 'object'>, names: Names): string {
  return `${entityWord(t.subject, names)} ${t.predicate.replace(/_/g, ' ')} ${entityWord(t.object, names)}`;
}

/** "Task t1 is now In review" */
export function taskChangedLine(task: Pick<Task, 'key' | 'status'>): string {
  return `Task ${task.key} is now ${TASK_STATUS_LABEL[task.status]}`;
}

/**
 * History rows (newest first) without the `task_changed` rows that say nothing new. The core
 * writes one for every update of a task (a new assignee, a branch, a summary), and the row
 * says only the status: a row shows when its status is not the one of that task's row before
 * it. The oldest row of a task in the list always shows.
 */
export function withoutRepeatedTaskRows<T extends { kind: EventKind }>(rows: T[]): T[] {
  const last = new Map<string, string>();
  const keep: T[] = [];
  for (let i = rows.length - 1; i >= 0; i--) {
    const k = rows[i].kind;
    if (k.type === 'task_changed') {
      const same = last.get(k.task.id) === k.task.status;
      last.set(k.task.id, k.task.status);
      if (same) continue;
    }
    keep.push(rows[i]);
  }
  return keep.reverse();
}

type BackgroundTasks = Extract<EventKind, { type: 'background_tasks' }>;

/** "Background command", "Helper": what a piece of background work is called in a row. */
const backgroundWord = (kind: string) => (kind === 'command' ? 'Background command' : kind === 'helper' ? 'Helper' : 'Background task');

/**
 * One change of what an agent runs in the background, as its CLI lists it: what ended, with the
 * CLI's word for it, then what still runs. "Background command stopped: npm run dev · still
 * runs: npm test"; with nothing ended, "Runs in the background: npm run dev".
 */
export function backgroundLine(k: Pick<BackgroundTasks, 'tasks' | 'ended'>): string {
  const runs = oneLine(k.tasks.map((t) => t.description).join(', '), 120);
  if (!k.ended.length) return k.tasks.length ? `Runs in the background: ${runs}` : 'Nothing runs in the background';
  const ended = k.ended.map((e) => `${backgroundWord(e.kind)} ${e.status}: ${oneLine(e.description, 90)}`).join('; ');
  return k.tasks.length ? `${ended} · still runs: ${runs}` : ended;
}
