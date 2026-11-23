// A used-up budget holds new work: no task starts until you raise the budget or reset the
// counters (Settings → Safety → Autonomy & Budgets). The core says so once, with a `breaker`
// decision ("budget"), and once more when work goes on ("budget_cleared"); what holds right now
// is `breaker_status`. The top bar's mark shows `hold`, and one toast says it when it happens.
import { api } from '../api';
import type { EventRow } from '../gen/EventRow';
import { store } from './store.svelte';
import { toast } from './toasts.svelte';
import { ui } from './ui.svelte';

/** What a `breaker` decision says about the budgets: why no task starts, or null when work goes
 *  on again. Undefined for every other event (an agent over a limit of its own, say). */
export function budgetChange(ev: EventRow): string | null | undefined {
  const k = ev.kind;
  if (k.type !== 'decision' || k.decision_type !== 'breaker') return undefined;
  const d = k.detail as { action?: unknown; reason?: unknown } | null;
  if (d?.action === 'budget_cleared') return null;
  if (d?.action !== 'budget') return undefined;
  return typeof d.reason === 'string' && d.reason ? d.reason : 'a budget is used up';
}

class Budget {
  /** Why no new task starts ("the team has used … over the budget of …"); null: work goes on. */
  hold = $state<string | null>(null);
  /** Settings was opened from here: its budget section scrolls into view, once. */
  reveal = $state(false);
  private started = false;

  /** Listen for the core's word (once). */
  start() {
    if (this.started) return;
    this.started = true;
    store.onEvent((ev) => this.observe(ev));
  }

  /** A budget is reached: the mark shows and a toast says it. Room again: the mark goes. */
  observe(ev: EventRow) {
    const change = budgetChange(ev);
    if (change === undefined) return;
    this.hold = change;
    if (change) toast({ kind: 'attention', title: 'Budget reached', body: `No new task starts: ${change}.`, action: { label: 'Open Settings', run: () => this.openSettings() } });
  }

  /** What holds now, from the core: a budget used up before you quit still holds when you are back. */
  async refresh() {
    try {
      this.hold = (await api.breakerStatus()).hold;
    } catch (e) {
      console.error('[codly] could not read whether a budget holds new work', e);
    }
  }

  /** Settings on the Safety tab, at the budgets. */
  openSettings() {
    this.reveal = true;
    ui.settingsTab = 'safety';
    ui.settingsOpen = true;
  }
}

export const budget = new Budget();
