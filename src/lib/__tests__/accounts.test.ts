import { describe, expect, it } from 'vitest';
import type { AccountConfig } from '../gen/AccountConfig';
import type { AccountInfo } from '../gen/AccountInfo';
import type { UsageWindow } from '../gen/UsageWindow';
import {
  accountFor, applyConfigs, asConfigs, byHeadroom, derivedLabel, headroom, mostConstrained, moveItem, multiProviders,
  normalizeHome, observeWindow, parked, resetLabel, toMs, usedNow, windowLabel, withProvider,
} from '../util/accounts';
import { clock } from '../util/format';

const NOW = new Date(2026, 9, 26, 10, 0, 0).getTime();
const win = (window: string, used: number, resetsAt: number | null = null, seen = NOW): UsageWindow => ({ window, used_fraction: used, resets_at: resetsAt, status: 'allowed', seen_at: seen });
const acct = (id: string, o: Partial<AccountInfo> = {}): AccountInfo => ({
  id, provider: 'claude', home: id.includes(':') ? id.split(':')[1] : null, label: id, enabled: true, primary: !id.includes(':'), configured: true,
  logged_in: true, billing: 'subscription', plan: null, identity: null, problem: null, login_command: null, limits: [], ...o,
});

describe('reset times', () => {
  it('reads unix seconds and ms alike', () => {
    expect(toMs(1_790_000_000)).toBe(1_790_000_000_000);
    expect(toMs(1_790_000_000_000)).toBe(1_790_000_000_000);
  });

  it('says the time today, the weekday this week, the date later', () => {
    const today = new Date(2026, 9, 26, 14, 0).getTime();
    expect(resetLabel(today / 1000, NOW)).toBe(`resets ${clock(today)}`);
    const thu = new Date(2026, 9, 29, 9, 0).getTime();
    expect(resetLabel(thu, NOW)).toMatch(new RegExp(`^resets \\S+ ${clock(thu).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`));
    expect(resetLabel(new Date(2026, 10, 9).getTime(), NOW)).toMatch(/^resets \D*\d+\D*$/);
    expect(resetLabel(null, NOW)).toBe('');
  });

  it('a window past its reset has started over', () => {
    const past = (NOW - 60_000) / 1000;
    expect(resetLabel(past, NOW)).toBe('renewed');
    expect(usedNow(win('five_hour', 0.9, past), NOW)).toBe(0);
    expect(usedNow(win('five_hour', 0.9, (NOW + 60_000) / 1000), NOW)).toBe(0.9);
  });
});

describe('headroom', () => {
  const fresh = (NOW + 3_600_000) / 1000;
  const def = acct('claude', { limits: [win('five_hour', 0.62, fresh), win('seven_day', 0.31, fresh)] });
  const work = acct('claude:/u/.claude-work', { limits: [win('five_hour', 0.18, fresh), win('seven_day', 0.09, fresh)] });
  const blank = acct('claude:/u/.claude-x');

  it('is what is left on the tightest window', () => {
    expect(headroom(def, NOW)).toBeCloseTo(0.38);
    expect(headroom(work, NOW)).toBeCloseTo(0.82);
    expect(headroom(blank, NOW)).toBeNull();
  });

  it('orders most quota left first, unknown last', () => {
    expect(byHeadroom([def, blank, work], NOW).map((a) => a.id)).toEqual([work.id, def.id, blank.id]);
    expect(mostConstrained([work, blank, def], NOW)?.id).toBe(def.id);
    expect(mostConstrained([blank], NOW)).toBeNull();
  });

  it('parks a sign-in at 85% or rejected, until the window resets', () => {
    const at = NOW + 3_600_000;
    expect(parked(acct('a', { limits: [win('five_hour', 0.84, at)] }), NOW)).toBeNull();
    expect(parked(acct('a', { limits: [win('five_hour', 0.86, at), win('seven_day', 0.2, at + 1)] }), NOW)).toEqual({ until: at });
    expect(parked(acct('a', { limits: [{ ...win('five_hour', 0.1, at), status: 'rejected' }] }), NOW)).toEqual({ until: at });
    expect(parked(acct('a', { limits: [win('five_hour', 0.95, NOW - 1)] }), NOW)).toBeNull();
    expect(parked(acct('a', { limits: [win('five_hour', 0.9, null, NOW - 60_000)] }), NOW)).toEqual({ until: null });
    expect(parked(acct('a', { limits: [win('five_hour', 0.9, null, NOW - 31 * 60_000)] }), NOW)).toBeNull();
  });

  it('keeps the newest report of a window', () => {
    const l = [win('five_hour', 0.5, null, NOW)];
    expect(observeWindow(l, win('five_hour', 0.4, null, NOW - 1000))).toBe(l);
    expect(observeWindow(l, win('five_hour', 0.6, null, NOW + 1000))[0].used_fraction).toBe(0.6);
    expect(observeWindow(l, win('seven_day', 0.1))).toHaveLength(2);
  });
});

describe('labels', () => {
  it('names windows plainly', () => {
    expect(windowLabel('five_hour')).toBe('5-hour');
    expect(windowLabel('seven_day')).toBe('Weekly');
    expect(windowLabel('seven_day_opus')).toBe('Weekly · Opus');
    expect(windowLabel('1440m')).toBe('Daily');
    expect(windowLabel('120m')).toBe('2-hour');
  });

  it('derives an account name like the core', () => {
    expect(derivedLabel('claude', null)).toBe('default');
    expect(derivedLabel('claude', '/Users/me/.claude-work')).toBe('work');
    expect(derivedLabel('gemini', '/Users/me/gemini-perso')).toBe('perso');
    expect(derivedLabel('codex', '/Users/me/codex')).toBe('codex');
    expect(normalizeHome('  ~/.claude-work/ ')).toBe('~/.claude-work');
  });

  it('labels agents only for a CLI with several usable sign-ins', () => {
    const list = [acct('claude'), acct('claude:/u/.claude-work'), acct('codex', { provider: 'codex' }), acct('codex:/u/.codex-p', { provider: 'codex', logged_in: false })];
    expect([...multiProviders(list)]).toEqual(['claude']);
    expect(accountFor({ provider: 'claude', account: null }, list)?.id).toBe('claude');
    expect(accountFor({ provider: 'claude', account: 'claude:/u/.claude-work' }, list)?.id).toBe('claude:/u/.claude-work');
    expect(accountFor({ provider: 'claude', account: 'claude:/u/.gone' }, list)).toBeNull();
  });
});

describe('settings.accounts', () => {
  const list = [acct('claude', { label: 'default' }), acct('claude:/u/.claude-work', { label: 'job', configured: true, enabled: false })];
  const configs: AccountConfig[] = [{ provider: 'codex', home: null, label: null, enabled: true }, { provider: 'claude', home: '/u/.claude-work', label: 'job', enabled: false }];

  it('writes every listed sign-in of the CLI, keeping labels and switches', () => {
    expect(asConfigs(list, configs)).toEqual([
      { provider: 'claude', home: null, label: null, enabled: true },
      { provider: 'claude', home: '/u/.claude-work', label: 'job', enabled: false },
    ]);
  });

  it('replaces only that CLI, where it sat', () => {
    const next = asConfigs(moveItem(list, 1, 0), configs);
    expect(withProvider(configs, 'claude', next).map((c) => `${c.provider}:${c.home}`)).toEqual(['codex:null', 'claude:/u/.claude-work', 'claude:null']);
    expect(withProvider([], 'claude', next)).toEqual(next);
  });

  it('shows the saved order and switches before the core answers', () => {
    const next = asConfigs(moveItem(list, 1, 0), configs).map((c) => ({ ...c, enabled: true }));
    const out = applyConfigs([acct('codex', { provider: 'codex' }), ...list], 'claude', [...next, { provider: 'claude', home: '/u/.claude-new', label: null, enabled: true }]);
    expect(out.map((a) => `${a.id}:${a.enabled}:${a.label}`)).toEqual(['codex:true:codex', 'claude:/u/.claude-work:true:job', 'claude:true:default', 'claude:/u/.claude-new:true:new']);
  });
});
