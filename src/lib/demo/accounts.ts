// The demo machine's sign-ins, and a small copy of the core's account rules (env.rs
// `detect_accounts`): configured homes in your order, then the terminal's sign-in; a found
// folder that is not signed in is left out; once a CLI has any config, an unlisted sign-in
// is off. Quota windows move as the demo's agents work (`bump`).
import type { AccountInfo } from '../gen/AccountInfo';
import type { Agent } from '../gen/Agent';
import type { Billing } from '../gen/Billing';
import type { CliInfo } from '../gen/CliInfo';
import type { Provider } from '../gen/Provider';
import type { Role } from '../gen/Role';
import type { Settings } from '../gen/Settings';
import type { UsageWindow } from '../gen/UsageWindow';
import { ACCOUNT_VAR, accountId, derivedLabel, headroom, parked, usable } from '../util/accounts';

type SignIn = { logged_in: boolean | null; billing: Billing; plan: string | null; identity: string | null; limits: UsageWindow[] };

const HOUR = 3_600_000;

const LOGIN: Partial<Record<Provider, string>> = { claude: 'claude auth login', codex: 'codex login', gemini: 'gemini' };
function loginCommand(p: Provider, home: string | null): string | null {
  const cmd = LOGIN[p];
  if (!cmd) return null;
  return home ? `${ACCOUNT_VAR[p]}=${/^[\w/.~+-]+$/.test(home) ? home : `'${home.replace(/'/g, `'\\''`)}'`} ${cmd}` : cmd;
}

export class DemoAccounts {
  private known = new Map<string, SignIn>();

  constructor(private home: string, now: number) {
    // Timestamps in ms, like the core.
    const w = (window: string, used: number, inHours: number): UsageWindow => ({ window, used_fraction: used, resets_at: now + Math.round(inHours * HOUR), status: 'allowed', seen_at: now - 240_000 });
    this.known.set('claude', { logged_in: true, billing: 'subscription', plan: 'Team', identity: 'demo@acme.dev · Acme Inc', limits: [w('five_hour', 0.62, 2.2), w('seven_day', 0.31, 86)] });
    this.known.set(`claude:${home}/.claude-work`, { logged_in: true, billing: 'subscription', plan: 'Max', identity: 'demo@globex.com · Globex', limits: [w('five_hour', 0.18, 3.9), w('seven_day', 0.09, 122)] });
    this.known.set('codex', { logged_in: true, billing: 'subscription', plan: 'Plus', identity: 'demo@acme.dev', limits: [w('five_hour', 0.16, 3.6), w('seven_day', 0.22, 97)] });
    this.known.set('gemini', { logged_in: true, billing: 'subscription', plan: 'Google AI Pro', identity: 'demo@gmail.com', limits: [] });
  }

  /** The core stores homes absolute: `~` expanded, no trailing slash. */
  normalize(settings: Settings): Settings {
    const fix = (h: string | null) => (h ? h.trim().replace(/^~(?=\/|$)/, this.home).replace(/(.)\/+$/, '$1') : null);
    return { ...settings, accounts: settings.accounts.map((c) => ({ ...c, home: fix(c.home), label: c.label?.trim() || null })) };
  }

  list(settings: Settings, clis: CliInfo[]): AccountInfo[] {
    const out: AccountInfo[] = [];
    for (const cli of clis.filter((c) => c.path)) {
      const p = cli.provider;
      const configs = settings.accounts.filter((c) => c.provider === p);
      if (!ACCOUNT_VAR[p]) {
        out.push({
          id: p, provider: p, home: null, label: 'default', enabled: true, primary: true, configured: false, logged_in: cli.logged_in,
          billing: cli.billing, plan: cli.plan, identity: null, problem: cli.problem, login_command: null, limits: [],
        });
        continue;
      }
      const homes: [string | null, (typeof configs)[number] | null][] = configs.map((c) => [c.home, c]);
      if (!homes.some(([h]) => h === null)) homes.push([null, null]);
      for (const [home, cfg] of homes) {
        const id = accountId(p, home);
        const primary = home === null;
        const k = this.known.get(id);
        const logged_in = k ? k.logged_in : false;
        if (!cfg && !primary && logged_in !== true) continue;
        const login = loginCommand(p, home);
        out.push({
          id, provider: p, home,
          label: cfg?.label?.trim() || derivedLabel(p, home),
          enabled: cfg ? cfg.enabled : primary && !configs.length,
          primary, configured: !!cfg, logged_in,
          billing: k?.billing ?? 'unknown', plan: k?.plan ?? null, identity: k?.identity ?? null,
          problem: logged_in === false && login ? `not signed in: run \`${login}\` in a terminal` : null,
          login_command: login,
          limits: k ? k.limits.map((x) => ({ ...x })) : [],
        });
      }
    }
    return out;
  }

  /** The sign-in a new agent runs on: the Lead's pinned one, else by the strategy. */
  pick(provider: Provider, role: Role, settings: Settings, clis: CliInfo[], agents: Agent[], now: number): string | null {
    const list = this.list(settings, clis).filter((a) => a.provider === provider && usable(a));
    if (list.length <= 1) return list[0]?.id ?? null;
    if (role === 'lead' && settings.lead_account && list.some((a) => a.id === settings.lead_account)) return settings.lead_account;
    // Like the core: a sign-in near its limit takes no new work; unknown usage counts as half.
    const open = list.filter((a) => !parked(a, now));
    const pool = open.length ? open : list;
    if (settings.account_strategy === 'in_order') return pool[0].id;
    // Spread: most quota left, counting the agents already working on each sign-in.
    const on = (id: string) => agents.filter((x) => x.account === id && x.status !== 'offline').length;
    return pool.map((a) => ({ id: a.id, h: (headroom(a, now) ?? 0.5) - 0.5 * on(a.id) })).sort((x, y) => y.h - x.h)[0].id;
  }

  /** One turn's worth of quota used on this sign-in; returns the windows to report. */
  bump(id: string, now: number): UsageWindow[] {
    const k = this.known.get(id);
    if (!k) return [];
    k.limits = k.limits.map((w) => ({ ...w, used_fraction: Math.min(0.97, w.used_fraction + (w.window === 'five_hour' ? 0.006 : 0.0015)), seen_at: now }));
    return k.limits;
  }
}
