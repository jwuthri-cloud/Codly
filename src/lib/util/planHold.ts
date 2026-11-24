// A plan whose work is on hold since Codly restarted: it is running on record, but its tasks wait
// for your "Resume work" and none of them works (held.ts). Its room says so, in the words of the
// plan card in the thread (home/cards/PlanCard.svelte). Pure.
import type { PlanStatus } from '../gen/PlanStatus';
import type { Task } from '../gen/Task';
import { isHeld } from './held';

/** What a plan on hold since Codly restarted says after "Plan ·". */
export const ON_HOLD = 'on hold since Codly restarted';

/** The task states in which a task works now. */
const WORKING: Task['status'][] = ['running', 'verifying', 'reviewing', 'merging'];

/** Is the plan on hold since Codly restarted? A running (or integrating) plan with a held task
 *  and no task that works. */
export function onHold(status: PlanStatus, tasks: Pick<Task, 'status' | 'status_reason'>[]): boolean {
  if (status !== 'running' && status !== 'integrating') return false;
  return tasks.some(isHeld) && !tasks.some((t) => WORKING.includes(t.status));
}
