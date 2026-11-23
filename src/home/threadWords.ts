// Small pieces of the thread's words that depend on what they count. Pure.
import type { Approval } from '../lib/gen/Approval';
import type { Task } from '../lib/gen/Task';

/** The chip that brings the waiting cards into view: a question is answered, the rest reviewed. */
export function reviewLabel(waiting: Pick<Approval, 'subject'>[]): string {
  const n = waiting.length;
  if (waiting.every((a) => a.subject.kind === 'question')) return n === 1 ? 'Answer the question' : `Answer ${n} questions`;
  if (waiting.some((a) => a.subject.kind === 'question')) return `Review ${n} requests`;
  return n === 1 ? 'Review the approval' : `Review ${n} approvals`;
}

/** A task it depends on that cannot finish by itself: it stopped, failed or was cancelled. */
const STUCK: Partial<Record<Task['status'], string>> = { cancelled: 'was cancelled', failed: 'failed', blocked: 'is blocked' };

/**
 * What a task that waits on others says, from the ones not done yet: "Waiting on t1, t2"; a
 * task among them that cannot finish is named first, since the task cannot start until you act
 * ("Can’t start · t1 was cancelled"). Null when it waits on none.
 */
export function waitLine(unmet: Pick<Task, 'key' | 'status'>[]): { text: string; stuck: boolean } | null {
  if (!unmet.length) return null;
  const stuck = unmet.flatMap((d) => (STUCK[d.status] ? [`${d.key} ${STUCK[d.status]}`] : []));
  if (stuck.length) return { text: `Can’t start · ${stuck.join(', ')}`, stuck: true };
  return { text: `Waiting on ${unmet.map((d) => d.key).join(', ')}`, stuck: false };
}
