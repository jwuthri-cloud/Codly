// Pure helpers for sign-ins (accounts): ids, labels, quota windows, headroom, and the
// AccountConfig list the settings store. No state here; see state/accounts.svelte.ts.
import type { AccountConfig } from '../gen/AccountConfig';
import type { AccountInfo } from '../gen/AccountInfo';
import type { Provider } from '../gen/Provider';
import type { UsageWindow } from '../gen/UsageWindow';
import { clock } from './format';

/** The variable that points a CLI at one sign-in. Providers without one have a single account. */
export const ACCOUNT_VAR: Partial<Record<Provider, string>> = {
  claude: 'CLAUDE_CONFIG_DIR',
  codex: 'CODEX_HOME',
  gemini: 'GEMINI_CLI_HOME',
};

/** A folder name to suggest for a new sign-in (what `discover_homes` finds on disk). */
export const HOME_SUGGESTION: Partial<Record<Provider, string>> = {
  claude: '~/.claude-work',
  codex: '~/.codex-work',
  gemini: '~/.gemini-work',
};

export const supportsAccounts = (p: Provider) => !!ACCOUNT_VAR[p];

/** Same as the core's `account_id`: "claude" for the default sign-in, "claude:<home>" otherwise. */
export const accountId = (provider: Provider, home: string | null) => (home ? `${provider}:${home}` : provider);

/** What the user typed, tidied: trimmed, no trailing slash. `~` is expanded by the core. */
export function normalizeHome(input: string): string {
  const h = input.trim();
  return h.length > 1 ? h.replace(/\/+$/, '') : h;
}

/** A home Codly can use: absolute or under `~`. */
export const validHome = (h: string) => /^(\/|~\/)./.test(h);

/** Same as the core's `account_label`: "default", or the folder name without the CLI prefix. */
export function derivedLabel(provider: Provider, home: string | null): string {
  if (!home) return 'default';
  const name = home.replace(/\/+$/, '').split('/').pop() || home;
  const bare = name.replace(/^\.+/, '');
  const rest = bare.startsWith(provider) ? bare.slice(provider.length).replace(/^[-_.]+/, '') : '';
  const short = rest || bare;
  return short || name;
}

// ───────────── quota windows ─────────────

/** Contract timestamps are unix seconds; rate-limit events carry ms. Accept both. */
export const toMs = (ts: number) => (ts < 1e11 ? ts * 1000 : ts);

/** "5-hour", "Weekly", "Weekly · Opus", "Daily"… */
export function windowLabel(w: string): string {
  const name = w.replace(/^(claude|codex|gemini)\./, '');
  const fixed: Record<string, string> = {
    five_hour: '5-hour', seven_day: 'Weekly', weekly: 'Weekly', daily: 'Daily', one_day: 'Daily',
    primary: '5-hour', secondary: 'Weekly',
  };
  if (fixed[name]) return fixed[name];
  const m = /^(\d+)m$/.exec(name);
  if (m) {
    const mins = Number(m[1]);
    if (mins === 10080) return 'Weekly';
    if (mins === 1440) return 'Daily';
    if (mins % 1440 === 0) return `${mins / 1440}-day`;
    if (mins % 60 === 0) return `${mins / 60}-hour`;
    return `${mins}-minute`;
  }
  const sub = /^seven_day_(.+)$/.exec(name);
  if (sub) return `Weekly · ${sub[1][0].toUpperCase()}${sub[1].slice(1).replace(/_/g, ' ')}`;
  const s = name.replace(/_/g, ' ');
  return s ? s[0].toUpperCase() + s.slice(1) : w;
}

/** Shorter windows first: 5-hour, then daily, then weekly, then the rest. */
function windowRank(w: string): number {
  const l = windowLabel(w);
  if (l === '5-hour') return 0;
  if (/hour|minute/.test(l)) return 1;
  if (/day|Daily/.test(l)) return 2;
  if (l.startsWith('Weekly')) return l === 'Weekly' ? 3 : 4;
  return 5;
}
export const sortWindows = (ws: UsageWindow[]) => [...ws].sort((a, b) => windowRank(a.window) - windowRank(b.window));

/** The fraction used now: a window whose reset time has passed has started over. */
export function usedNow(w: UsageWindow, now: number): number {
  if (w.resets_at != null && toMs(w.resets_at) <= now) return 0;
  return Math.max(0, Math.min(1, w.used_fraction));
}

const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const dateFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** "resets 14:00" today, "resets Thu 09:00" this week, "resets Oct 30" later; "renewed" once past. */
export function resetLabel(ts: number | null | undefined, now: number): string {
  if (ts == null) return '';
  const ms = toMs(ts);
  if (ms <= now) return 'renewed';
  const d = new Date(ms);
  if (sameDay(d, new Date(now))) return `resets ${clock(ms)}`;
  if (ms - now < 6 * 86_400_000) return `resets ${dayFmt.format(d)} ${clock(ms)}`;
  return `resets ${dateFmt.format(d)}`;
}

/** Quota left on the tightest window (0..1); null when no window was ever reported. */
export function headroom(a: Pick<AccountInfo, 'limits'>, now: number): number | null {
  if (!a.limits.length) return null;
  return 1 - Math.max(...a.limits.map((w) => usedNow(w, now)));
}

/** Most quota left first; accounts with no known quota after the known ones (stable). */
export function byHeadroom<T extends Pick<AccountInfo, 'limits'>>(list: T[], now: number): T[] {
  return list
    .map((a, i) => ({ a, i, h: headroom(a, now) }))
    .sort((x, y) => (x.h == null ? (y.h == null ? x.i - y.i : 1) : y.h == null ? -1 : y.h - x.h || x.i - y.i))
    .map((x) => x.a);
}

/** The account closest to its limit (null when none has a known quota). */
export function mostConstrained<T extends Pick<AccountInfo, 'limits'>>(list: T[], now: number): T | null {
  const known = byHeadroom(list, now).filter((a) => headroom(a, now) != null);
  return known[known.length - 1] ?? null;
}

/** Upsert one window reported live (ignores reports older than what is known). */
export function observeWindow(limits: UsageWindow[], w: UsageWindow): UsageWindow[] {
  const prev = limits.find((x) => x.window === w.window);
  if (prev && toMs(prev.seen_at) > toMs(w.seen_at)) return limits;
  return prev ? limits.map((x) => (x === prev ? w : x)) : [...limits, w];
}

// ───────────── which accounts count ─────────────

/** Codly may run agents on it: enabled and not known to be signed out. */
export const usable = (a: Pick<AccountInfo, 'enabled' | 'logged_in'>) => a.enabled && a.logged_in !== false;

/** Providers with more than one usable account: only these show account labels elsewhere. */
export function multiProviders(list: Pick<AccountInfo, 'provider' | 'enabled' | 'logged_in'>[]): Set<Provider> {
  const n = new Map<Provider, number>();
  for (const a of list) if (usable(a)) n.set(a.provider, (n.get(a.provider) ?? 0) + 1);
  return new Set([...n].filter(([, c]) => c > 1).map(([p]) => p));
}

/** An agent's account: its own, else its provider's primary sign-in. */
export function accountFor(agent: { provider: Provider; account: string | null }, list: AccountInfo[]): AccountInfo | null {
  if (agent.account) {
    const a = list.find((x) => x.id === agent.account);
    if (a) return a;
  }
  return list.find((x) => x.provider === agent.provider && x.primary) ?? list.find((x) => x.provider === agent.provider) ?? null;
}

// ───────────── settings.accounts ─────────────

/**
 * The provider's accounts as configs, in the given order, keeping each one's enabled state.
 * Writing every listed account (not just the edited one) matters: once a provider has any
 * config, the core no longer enables its unlisted terminal sign-in on its own.
 */
export function asConfigs(list: AccountInfo[], configs: AccountConfig[]): AccountConfig[] {
  return list.map((a) => {
    const c = configs.find((x) => x.provider === a.provider && accountId(x.provider, x.home) === a.id);
    return { provider: a.provider, home: a.home, label: c?.label ?? null, enabled: a.enabled };
  });
}

/** Replace one provider's configs, keeping the others (and where that provider sat). */
export function withProvider(configs: AccountConfig[], provider: Provider, next: AccountConfig[]): AccountConfig[] {
  const at = configs.findIndex((c) => c.provider === provider);
  const others = configs.filter((c) => c.provider !== provider);
  if (at < 0) return [...others, ...next];
  const before = configs.slice(0, at).filter((c) => c.provider !== provider).length;
  return [...others.slice(0, before), ...next, ...others.slice(before)];
}

/** Optimistic view of the list after saving `next` for `provider` (the core answers with the truth). */
export function applyConfigs(list: AccountInfo[], provider: Provider, next: AccountConfig[]): AccountInfo[] {
  const mine = list.filter((a) => a.provider === provider);
  const ordered: AccountInfo[] = [];
  for (const c of next) {
    const id = accountId(c.provider, c.home);
    const a = mine.find((x) => x.id === id);
    const label = c.label?.trim() || derivedLabel(c.provider, c.home);
    ordered.push(a ? { ...a, enabled: c.enabled, label, configured: true } : {
      id, provider, home: c.home, label, enabled: c.enabled, primary: false, configured: true, logged_in: null,
      billing: 'unknown', plan: null, identity: null, problem: null, login_command: null, limits: [],
    });
  }
  const rest = mine.filter((a) => !ordered.some((o) => o.id === a.id));
  const out: AccountInfo[] = [];
  let placed = false;
  for (const a of list) {
    if (a.provider !== provider) out.push(a);
    else if (!placed) {
      out.push(...ordered, ...rest);
      placed = true;
    }
  }
  if (!placed) out.push(...ordered, ...rest);
  return out;
}

/** Move the item at `from` to `to` (both clamped). */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  const next = list.slice();
  const j = Math.max(0, Math.min(list.length - 1, to));
  if (from < 0 || from >= list.length || from === j) return list;
  const [x] = next.splice(from, 1);
  next.splice(j, 0, x);
  return next;
}
