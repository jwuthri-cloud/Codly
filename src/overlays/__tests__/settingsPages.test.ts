import { describe, expect, it, vi } from 'vitest';
import { WATCHDOG_LIMITS } from '../../lib/util/watchdog';
import {
  MODE_TITLE,
  NO_REPO_LINE,
  SAFETY_PAGES,
  approvalsLine,
  checksLine,
  limitsLine,
  openAt,
  pageOf,
  safetyLink,
  sandboxLine,
  trustLine,
  worktreesLine,
  type SafetySection,
} from '../settingsPages';

describe('Safety pages', () => {
  it('are six, in the order of the list', () => {
    expect(SAFETY_PAGES.map((p) => p.label)).toEqual(['Approvals', 'Sandbox', 'Checks', 'Trust', 'Worktrees', 'Limits']);
  });

  it('show each section on its page', () => {
    const on: [SafetySection, string][] = [
      ['approval-mode', 'approvals'],
      ['approval-rules', 'approvals'],
      ['sandbox', 'sandbox'],
      ['review', 'checks'],
      ['required-checks', 'checks'],
      ['trust', 'trust'],
      ['shell-auth', 'trust'],
      ['worktrees', 'worktrees'],
      ['budgets', 'limits'],
      ['watchdog', 'limits'],
    ];
    for (const [section, page] of on) expect(pageOf(section), section).toBe(page);
  });
});

describe('where Settings opens', () => {
  it('on General, with the last Safety page kept, when nothing names a tab', () => {
    expect(openAt(null)).toEqual({ tab: 'general', page: null, section: null });
  });

  it('on another tab as it is named', () => {
    expect(openAt('team')).toEqual({ tab: 'team', page: null, section: null });
  });

  it('on Approvals for a link to Safety that names no section', () => {
    expect(openAt('safety')).toEqual({ tab: 'safety', page: 'approvals', section: null });
  });

  it('on the page of the section a link names, at that section', () => {
    expect(openAt(safetyLink('budgets'))).toEqual({ tab: 'safety', page: 'limits', section: 'budgets' });
    expect(openAt(safetyLink('shell-auth'))).toEqual({ tab: 'safety', page: 'trust', section: 'shell-auth' });
  });

  it('at the top of Safety for a section it does not know, and says so', () => {
    const said = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(openAt('safety/nowhere')).toEqual({ tab: 'safety', page: 'approvals', section: null });
    expect(said).toHaveBeenCalledOnce();
    said.mockRestore();
  });
});

describe('the line under each page', () => {
  it('Approvals: the mode and the saved rules', () => {
    expect(approvalsLine('balanced', 3)).toBe('Balanced · 3 saved rules');
    expect(approvalsLine('strict', 1)).toBe('Ask first · 1 saved rule');
    expect(approvalsLine('full', 0)).toBe('Full access · no saved rules');
    expect(approvalsLine('autonomous', null)).toBe('Autonomous');
  });

  it('Sandbox: both switches', () => {
    expect(sandboxLine({ claude_sandbox: true, codex_network_access: false })).toBe('Claude on · Codex network off');
    expect(sandboxLine({ claude_sandbox: false, codex_network_access: true })).toBe('Claude off · Codex network on');
  });

  it('Checks: the review and the required checks', () => {
    const cmds = (n: number) => Array.from({ length: n }, (_, i) => ({ command: `c${i}`, risks: [] }));
    expect(checksLine(true, { on: true, changed: false, commands: cmds(2) })).toBe('Review on · 2 required checks');
    expect(checksLine(true, { on: true, changed: false, commands: cmds(1) })).toBe('Review on · 1 required check');
    expect(checksLine(false, { on: false, changed: false, commands: cmds(2) })).toBe('Review off · checks off');
    expect(checksLine(true, { on: false, changed: false, commands: [] })).toBe('Review on · no required checks');
    expect(checksLine(true, { on: true, changed: true, commands: cmds(2) })).toBe('Review on · checks changed');
    expect(checksLine(false, null)).toBe('Review off');
  });

  it('Trust: your choice about this folder', () => {
    expect(trustLine({ state: 'trusted', changed: false })).toBe('This folder: trusted');
    expect(trustLine({ state: 'without', changed: false })).toBe('This folder: not trusted');
    expect(trustLine({ state: 'ask', changed: false })).toBe('This folder: not chosen yet');
    expect(trustLine({ state: 'ask', changed: true })).toBe('This folder: files changed');
    expect(trustLine({ state: 'nothing', changed: false })).toBe('This folder: nothing to trust');
    expect(trustLine(null)).toBe('');
  });

  it('Worktrees: the setup script', () => {
    const w = { script: 'npm ci', on: false, changed: false, problem: null };
    expect(worktreesLine({ ...w, on: true })).toBe('Setup script: on');
    expect(worktreesLine(w)).toBe('Setup script: off');
    expect(worktreesLine({ ...w, on: true, changed: true })).toBe('Setup script: changed');
    expect(worktreesLine({ ...w, script: null })).toBe('No setup script');
    expect(worktreesLine({ ...w, script: null, problem: 'a link, not a file' })).toBe('Setup script: off');
    expect(worktreesLine(null)).toBe('');
  });

  it('Limits: the budget and the watchdog', () => {
    const b = { enabled: true, token_budget: 0, cost_budget_usd: 20 };
    expect(limitsLine(b, { quiet_secs: 600 })).toBe('Budget $20.00 · watchdog 10 min');
    expect(limitsLine({ ...b, cost_budget_usd: 0, token_budget: 2_000_000 }, { quiet_secs: 600 })).toBe('2M tokens · watchdog 10 min');
    expect(limitsLine({ ...b, cost_budget_usd: 0 }, { quiet_secs: 0 })).toBe('No budget · watchdog off');
    expect(limitsLine({ ...b, enabled: false }, { quiet_secs: 3600 })).toBe('Breaker off · watchdog 1 h');
    expect(limitsLine(null, { quiet_secs: 600 })).toBe('watchdog 10 min');
  });

  // 'Budget $20.00 · watchdog 10 min', 31 characters, is the longest line seen on one row of an
  // entry (184 px, at 1024 px wide). A second row makes the list taller, and the first control of
  // the Trust page no longer shows without a scroll.
  it('each fits one row of its entry, in every state the settings offer', () => {
    const cmds = (n: number) => Array.from({ length: n }, (_, i) => ({ command: `c${i}`, risks: [] }));
    const lines: string[] = [NO_REPO_LINE];
    for (const mode of Object.keys(MODE_TITLE) as (keyof typeof MODE_TITLE)[]) {
      for (const rules of [null, 0, 1, 99]) lines.push(approvalsLine(mode, rules));
    }
    for (const claude_sandbox of [true, false]) {
      for (const codex_network_access of [true, false]) lines.push(sandboxLine({ claude_sandbox, codex_network_access }));
    }
    for (const review of [true, false]) {
      lines.push(checksLine(review, null));
      for (const n of [0, 1, 99]) {
        for (const on of [true, false]) {
          for (const changed of [true, false]) lines.push(checksLine(review, { on, changed, commands: cmds(n) }));
        }
      }
    }
    for (const state of ['nothing', 'trusted', 'without', 'ask'] as const) {
      for (const changed of [true, false]) lines.push(trustLine({ state, changed }));
    }
    for (const script of [null, 'npm ci']) {
      for (const on of [true, false]) {
        for (const changed of [true, false]) {
          for (const problem of [null, 'a link, not a file']) lines.push(worktreesLine({ script, on, changed, problem }));
        }
      }
    }
    // Cost budgets are kept to the cent (from $0.01) and stay under $10,000 here; token budgets
    // are typed in steps of half a million. The watchdog's choices are the ones its select offers.
    const budgets = [
      null,
      { enabled: false, token_budget: 0, cost_budget_usd: 0 },
      { enabled: true, token_budget: 0, cost_budget_usd: 0 },
      ...[0.01, 0.5, 20, 99.99, 250, 9_999].map((cost_budget_usd) => ({ enabled: true, token_budget: 0, cost_budget_usd })),
      ...[500_000, 2_000_000, 12_500_000, 999_500_000].map((token_budget) => ({ enabled: true, token_budget, cost_budget_usd: 0 })),
    ];
    for (const b of budgets) {
      for (const quiet_secs of WATCHDOG_LIMITS.find((l) => l.key === 'quiet_secs')!.choices) lines.push(limitsLine(b, { quiet_secs }));
    }
    const long = lines.filter((l) => l.length > 31);
    expect(long).toEqual([]);
  });
});
