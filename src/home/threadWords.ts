// Small pieces of the thread's words that depend on what they count. Pure.
import type { Approval } from '../lib/gen/Approval';
import type { Task } from '../lib/gen/Task';
import type { Tone } from '../lib/narrate';

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

/**
 * The mark of a task row. Its worker that waits for you comes first, then a task held since
 * Codly restarted (on hold, not at work). A task whose line says it cannot start (pending or
 * ready, after one that stopped, once the plan is approved) is blocked; any other task's
 * status says (a cancelled one stays struck through, whatever it waited on).
 */
export function taskTone(t: { status: Task['status']; held: boolean; stuck: boolean; proposing: boolean; worker: Tone | null }): Tone {
  if (t.worker === 'wait') return 'wait';
  if (t.held) return 'idle';
  if (t.stuck && !t.proposing && (t.status === 'pending' || t.status === 'ready')) return 'blocked';
  switch (t.status) {
    case 'running': case 'verifying': case 'reviewing': case 'merging': case 'ready': return 'live';
    case 'done': return 'done';
    case 'blocked': case 'failed': case 'changes_requested': return 'blocked';
    case 'cancelled': return 'off';
    default: return 'idle';
  }
}
