// Sign-ins (accounts) of the agent CLIs, with their quota. Filled from the snapshot and
// `list_accounts`, kept live by rate-limit events (store.svelte.ts calls `observe`).
// Labels show elsewhere only when a CLI has more than one usable account.
import type { AccountInfo } from '../gen/AccountInfo';
import type { Agent } from '../gen/Agent';
import type { Provider } from '../gen/Provider';
import type { Task } from '../gen/Task';
import { accountFor, multiProviders, observeWindow } from '../util/accounts';
import { PROVIDER_LABEL } from '../util/labels';

class Accounts {
  list = $state.raw<AccountInfo[]>([]);
  byId = $derived(new Map(this.list.map((a) => [a.id, a])));
  /** CLIs with several usable sign-ins (their agents carry an account label). */
  multi = $derived(multiProviders(this.list));

  set(list: AccountInfo[]) {
    this.list = list;
  }

  of(agent: Pick<Agent, 'provider' | 'account'>): AccountInfo | null {
    return accountFor(agent, this.list);
  }

  /** Stable key of the agent's account (falls back to the provider before accounts load). */
  idOf(agent: Pick<Agent, 'provider' | 'account'>): string {
    return this.of(agent)?.id ?? agent.account ?? agent.provider;
  }

  /** The agent's account label, only when its CLI has more than one usable sign-in. */
  tag(agent: Pick<Agent, 'provider' | 'account'>): string | null {
    if (!this.multi.has(agent.provider)) return null;
    return this.of(agent)?.label ?? null;
  }

  /** "Claude Code · work" (or just "Claude Code" with one sign-in). */
  who(agent: Pick<Agent, 'provider' | 'account'>): string {
    const t = this.tag(agent);
    return t ? `${PROVIDER_LABEL[agent.provider]} · ${t}` : PROVIDER_LABEL[agent.provider];
  }

  /** The account the Lead pinned a task to, as a label (null when not pinned or not useful). */
  hintTag(task: Pick<Task, 'account_hint'>, provider: Provider | null): string | null {
    if (!task.account_hint) return null;
    const a = this.byId.get(task.account_hint);
    if (!a || (provider && a.provider !== provider) || !this.multi.has(a.provider)) return null;
    return a.label;
  }

  /** A live quota report from one of the account's agents. */
  observe(agent: Pick<Agent, 'provider' | 'account'>, w: { window: string; used_fraction: number | null; resets_at: number | null; status: string | null }, ts: number) {
    if (w.used_fraction == null) return;
    const acc = this.of(agent);
    if (!acc) return;
    const limits = observeWindow(acc.limits, { window: w.window, used_fraction: w.used_fraction, resets_at: w.resets_at, status: w.status, seen_at: ts });
    if (limits !== acc.limits) this.list = this.list.map((a) => (a.id === acc.id ? { ...a, limits } : a));
  }
}

export const accounts = new Accounts();
