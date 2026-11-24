// A toast that offers to go on with a plan's merge ("Merge anyway", "Have an agent resolve it")
// goes once that offer is no longer true (lib/state/actions.ts `mergePlan`). Pure.
import type { MergeState } from '../gen/MergeState';

/** What held the merge back: your checkout is not the branch the plan started from, a
 *  conflict, or checks that failed (or could not run) on the merge result. */
export type MergeRefusal = 'branch' | 'conflict' | 'checks';

/**
 * Is the offer gone? Yes once the plan is merged, and once the core's last word on the merge
 * (`state`, read again when a card shows it) no longer has the conflict or the checks that
 * refused it. `state` undefined: not known, and what is not known keeps the offer. The branch
 * you are on is not asked: the window can learn of a switch later than the core, and a toast
 * that says why a merge did not go in must not go for that.
 */
export function mergeOfferGone(refusal: MergeRefusal, merged: boolean, state: MergeState | undefined): boolean {
  if (merged) return true;
  if (!state || refusal === 'branch') return false;
  if (refusal === 'conflict') return !state.conflicts.length;
  return !state.failed_checks.length && !state.checks_not_run;
}
