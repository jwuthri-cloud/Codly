import { describe, expect, it, vi } from 'vitest';
import {
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
    expect(sandboxLine({ claude_sandbox: true, codex_network_access: false })).toBe('Claude: on · Codex network: off');
    expect(sandboxLine({ claude_sandbox: false, codex_network_access: true })).toBe('Claude: off · Codex network: on');
  });

  it('Checks: the review and the required checks', () => {
    const cmds = (n: number) => Array.from({ length: n }, (_, i) => ({ command: `c${i}`, risks: [] }));
    expect(checksLine(true, { on: true, changed: false, commands: cmds(2) })).toBe('Review on · 2 required checks');
    expect(checksLine(true, { on: true, changed: false, commands: cmds(1) })).toBe('Review on · 1 required check');
    expect(checksLine(false, { on: false, changed: false, commands: cmds(2) })).toBe('Review off · required checks off');
    expect(checksLine(true, { on: false, changed: false, commands: [] })).toBe('Review on · no required checks');
    expect(checksLine(true, { on: true, changed: true, commands: cmds(2) })).toBe('Review on · required checks changed');
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
    expect(limitsLine({ ...b, cost_budget_usd: 0, token_budget: 2_000_000 }, { quiet_secs: 600 })).toBe('Budget 2M tokens · watchdog 10 min');
    expect(limitsLine({ ...b, cost_budget_usd: 0 }, { quiet_secs: 0 })).toBe('No budget · watchdog off');
    expect(limitsLine({ ...b, enabled: false }, { quiet_secs: 3600 })).toBe('Circuit breaker off · watchdog 1 h');
    expect(limitsLine(null, { quiet_secs: 600 })).toBe('watchdog 10 min');
  });
});
