// A used-up budget holds new work: no task starts until you raise the budget or reset the
// counters (Settings → Safety → Limits). The core says so once, with a `breaker`
// decision ("budget"), and once more when work goes on ("budget_cleared"); what holds right now
// is `breaker_status`. The top bar's mark shows `hold`, and one toast says it when it happens.
// The toast goes when the hold goes: it never says a budget holds work that has room again.
import { openSettings } from '../../overlays/settingsLink';
import { safetyLink } from '../../overlays/settingsPages';
import { api } from '../api';
import type { EventRow } from '../gen/EventRow';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';

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
  private why = $state<string | null>(null);
  /** The "Budget reached" toast, to take back when work goes on. */
  private said: number | undefined;
  private started = false;

  /** Why no new task starts ("the team has used … over the budget of …"); null: work goes on. */
  get hold() {
    return this.why;
  }

  /** What holds now, as the core or Settings last read it. Room again takes the toast back. */
  setHold(why: string | null) {
    this.why = why;
    if (!why) this.unsay();
  }

  /** Listen for the core's word (once). */
  start() {
    if (this.started) return;
    this.started = true;
    store.onEvent((ev) => this.observe(ev));
  }

  /** A budget is reached: the mark shows and a toast says it. Room again: both go. */
  observe(ev: EventRow) {
    const change = budgetChange(ev);
    if (change === undefined) return;
    this.setHold(change);
    if (!change) return;
    this.unsay();
    this.said = toast({ kind: 'attention', title: 'Budget reached', body: `No new task starts: ${change}.`, action: { label: 'Open Settings', run: () => this.openSettings() } });
  }

  private unsay() {
    if (this.said === undefined) return;
    toasts.dismiss(this.said);
    this.said = undefined;
  }

  /** What holds now, from the core: a budget used up before you quit still holds when you are back. */
  async refresh() {
    try {
      this.setHold((await api.breakerStatus()).hold);
    } catch (e) {
      console.error('[codly] could not read whether a budget holds new work', e);
    }
  }

  /** Settings on the Safety tab, at the budgets (also when Settings is open already). */
  openSettings() {
    openSettings(safetyLink('budgets'));
  }
}

export const budget = new Budget();
