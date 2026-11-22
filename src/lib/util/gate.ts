// What the gate's record says about a task, for its card.
import type { Decision } from '../state/store.svelte';

/** Keys in a gate decision's detail that say the task's checks stopped it (the core's `CHECK_STOPS`):
 *  one failed its last fix round, one ran out of time, some wait for your OK. */
const CHECK_STOPS = ['failed_check', 'timed_out', 'held'];

/** Did its checks stop the task? Then its last decision is the gate's, about its checks. Stopped by you,
 *  its worker, the scope check or a reviewer: another decision is on record last, and the core refuses
 *  Merge anyway (there is no check to skip). */
export function stoppedByChecks(decisions: Decision[], taskId: string): boolean {
  const last = decisions.findLast((d) => d.taskId === taskId);
  const detail = last?.type === 'gate' ? last.detail : null;
  if (!detail || typeof detail !== 'object' || Array.isArray(detail)) return false;
  return CHECK_STOPS.some((k) => detail[k] != null);
}
