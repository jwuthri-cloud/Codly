// What each plan's tasks used so far (work tokens and reported cost), asked from the core and
// kept until the plan moves on. The plan card and the plan-done card read the same numbers.
import type { Spent } from '../gen/Spent';
import { api } from '../api';

class PlanSpent {
  plan = $state<Record<string, Spent>>({});
  private seen = new Map<string, string>();

  /** Ask for a plan's total; `version` (its task states, a clock step while it runs) asks again when it changes. */
  ensure(id: string, version: string) {
    if (this.seen.get(id) === version) return;
    this.seen.set(id, version);
    api.planSpent(id).then(
      (s) => (this.plan = { ...this.plan, [id]: s }),
      () => this.seen.delete(id),
    );
  }
}

export const planSpent = new PlanSpent();

/** How often a running plan's total is asked again. */
const STEP_MS = 30_000;

/** The `version` for `ensure`: a change of any task's state, and every half minute while the plan runs. */
export const spentVersion = (statuses: string[], live: boolean, now: number) => `${statuses.join()}:${live ? Math.floor(now / STEP_MS) : ''}`;
