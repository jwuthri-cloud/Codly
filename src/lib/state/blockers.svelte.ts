// "Ask me": the cards agents raise when they need a human decision (codly_ask_user).
// Loaded with `list_blockers` and kept live by `blocker_changed` events (store.onEvent), so it
// needs nothing from the main store. Blocking cards come first, oldest first; heads-ups last.
import { api } from '../api';
import type { Blocker } from '../gen/Blocker';
import { store } from './store.svelte';

/** Matches REMIND_AFTER_DAYS in crates/codly-core/src/blockers.rs. */
export const REMIND_AFTER_DAYS = 2;
const DAY = 86_400_000;

/** The order of the inbox: what stops work first, then by age (oldest waits longest). */
export function askMeOrder(a: Blocker, b: Blocker): number {
  const rank = (x: Blocker) => (x.severity === 'blocking' ? 0 : 1);
  return rank(a) - rank(b) || a.created_at - b.created_at;
}

/** Whole days an open card has waited ("waiting 3 days"), 0 below one day. */
export function waitedDays(b: Blocker, now: number): number {
  return Math.max(0, Math.floor((now - b.created_at) / DAY));
}

/** Waited long enough to nag: flagged by the core, or old enough that it is about to be. */
export function isOverdue(b: Blocker, now: number): boolean {
  return b.status === 'open' && (b.reminded_at != null || now - b.created_at >= REMIND_AFTER_DAYS * DAY);
}

class Blockers {
  list = $state.raw<Blocker[]>([]);
  /** The popover is open. */
  panel = $state(false);
  error = $state<string | null>(null);
  open = $derived(this.list.filter((b) => b.status === 'open').sort(askMeOrder));
  blocking = $derived(this.open.filter((b) => b.severity === 'blocking'));
  /** Answered and dismissed cards, newest first (history stays readable). */
  closed = $derived(this.list.filter((b) => b.status !== 'open').sort((a, b) => (b.answered_at ?? b.updated_at) - (a.answered_at ?? a.updated_at)));
  private started = false;

  /** Listen for live changes (once). */
  start() {
    if (this.started) return;
    this.started = true;
    store.onEvent((ev) => {
      if (ev.kind.type === 'blocker_changed') this.apply(ev.kind.blocker);
    });
  }

  private apply(b: Blocker) {
    const i = this.list.findIndex((x) => x.id === b.id);
    if (i < 0) this.list = [...this.list, b];
    else if (this.list[i].updated_at <= b.updated_at) this.list = this.list.map((x) => (x.id === b.id ? b : x));
  }

  async load() {
    try {
      this.list = await api.listBlockers();
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  /** "Respond & unblock": the agent that waits gets your answer and carries on. */
  async answer(id: string, text: string) {
    this.apply(await api.answerBlocker(id, text));
  }

  /** A heads-up: read. A blocking card: skipped, the agent uses its best judgment. */
  async dismiss(id: string) {
    this.apply(await api.dismissBlocker(id));
  }
}

export const blockers = new Blockers();
