// Where a plan stands, for the summary of your time away (away.ts `PlanEnd`): read from the
// store, as the plan's own cards say it.
import { mergeBlock, mergeKey, mergeUi } from '../lib/state/mergeState.svelte';
import { store } from '../lib/state/store.svelte';
import { ui } from '../lib/state/ui.svelte';
import type { PlanEnd } from './away';

export function planEnd(planId: string): PlanEnd {
  const plan = store.planMap.get(planId);
  if (!plan) return 'other';
  if (plan.status === 'awaiting_approval') return 'waits';
  if (store.mergeOf(planId)) return 'merged';
  if (plan.status !== 'done') return 'other';
  if (plan.kept || ui.keptPlans.includes(planId)) return 'kept';
  const block = mergeBlock(mergeUi.states[mergeKey({ kind: 'plan', id: planId })]);
  return block === 'conflict' ? 'conflict' : block ? 'checks' : 'ready';
}
