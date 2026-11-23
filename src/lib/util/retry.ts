// "Retry to resume": the words the core (and the demo) put on a task that you stopped, or that
// stopped with its agent. Wherever the UI shows them, a Retry button beside them does what the
// task sheet's Retry does: the task goes back to Ready and its agent resumes it.
import type { Task } from '../gen/Task';

/** The task a Retry next to these words resumes, or null: none of them is shown for it. */
export function retryTarget(task: Pick<Task, 'id' | 'status' | 'status_reason'> | null | undefined): string | null {
  if (!task || (task.status !== 'blocked' && task.status !== 'failed')) return null;
  return /retry to resume/i.test(task.status_reason ?? '') ? task.id : null;
}
