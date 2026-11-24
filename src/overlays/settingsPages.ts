// Settings → Safety in six short pages: which page each section is on, where a link opens the
// sheet, and the one line under each page's name that says where its settings stand. A line
// must fit one row of its entry (184 px in a small window, about 31 characters): a second row
// makes the whole list taller and pushes a page's first control out of view.

import type { ApprovalMode } from '../lib/gen/ApprovalMode';
import type { BreakerConfig } from '../lib/gen/BreakerConfig';
import type { RequiredChecks } from '../lib/gen/RequiredChecks';
import type { Settings } from '../lib/gen/Settings';
import type { WatchdogSettings } from '../lib/gen/WatchdogSettings';
import type { WorkspaceTrust } from '../lib/gen/WorkspaceTrust';
import type { WorktreeSetup } from '../lib/gen/WorktreeSetup';
import { tokens, usd } from '../lib/util/format';
import { limitLabel } from '../lib/util/watchdog';

export type SafetyPage = 'approvals' | 'sandbox' | 'checks' | 'trust' | 'worktrees' | 'limits';

export const SAFETY_PAGES: { id: SafetyPage; label: string }[] = [
  { id: 'approvals', label: 'Approvals' },
  { id: 'sandbox', label: 'Sandbox' },
  { id: 'checks', label: 'Checks' },
  { id: 'trust', label: 'Trust' },
  { id: 'worktrees', label: 'Worktrees' },
  { id: 'limits', label: 'Limits' },
];

/** The sections of the Safety tab, by the page that shows them, in the order they show. */
const SECTION_PAGE = {
  'approval-mode': 'approvals',
  'approval-rules': 'approvals',
  sandbox: 'sandbox',
  review: 'checks',
  'required-checks': 'checks',
  trust: 'trust',
  'shell-auth': 'trust',
  worktrees: 'worktrees',
  budgets: 'limits',
  watchdog: 'limits',
} as const satisfies Record<string, SafetyPage>;

export type SafetySection = keyof typeof SECTION_PAGE;

export function pageOf(section: SafetySection): SafetyPage {
  return SECTION_PAGE[section];
}

/** The value of `ui.settingsTab` that opens Settings on the Safety page of `section`, at it. */
export function safetyLink(section: SafetySection): string {
  return `safety/${section}`;
}

export interface SettingsTarget {
  tab: string;
  /** The Safety page to show; null = the one you were on last. */
  page: SafetyPage | null;
  /** The section to scroll to. */
  section: SafetySection | null;
}

/**
 * Where Settings opens for `link` (`ui.settingsTab`): null is General; a tab id is that tab;
 * "safety" alone is its first page, Approvals; "safety/<section>" (`safetyLink`) is the page
 * of that section, at it.
 */
export function openAt(link: string | null): SettingsTarget {
  if (link === null) return { tab: 'general', page: null, section: null };
  const [tab, section] = link.split('/', 2);
  if (tab !== 'safety') return { tab, page: null, section: null };
  if (section === undefined) return { tab, page: 'approvals', section: null };
  // Its own keys only: "toString" or "constructor" is no section.
  if (!Object.hasOwn(SECTION_PAGE, section)) {
    // A link built without `safetyLink`: the sheet still opens, at the top of the tab.
    console.error(`[codly] Settings has no Safety section “${section}”`);
    return { tab, page: 'approvals', section: null };
  }
  const known = section as SafetySection;
  return { tab, page: pageOf(known), section: known };
}

/** The approval modes by the names of their cards. */
export const MODE_TITLE: Record<ApprovalMode, string> = { balanced: 'Balanced', strict: 'Ask first', autonomous: 'Autonomous', full: 'Full access' };

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "Balanced · 3 saved rules". `rules` null: not known (no repo open, or not read yet). */
export function approvalsLine(mode: ApprovalMode, rules: number | null): string {
  if (rules === null) return MODE_TITLE[mode];
  return `${MODE_TITLE[mode]} · ${rules ? count(rules, 'saved rule', 'saved rules') : 'no saved rules'}`;
}

/** "Claude on · Codex network off". */
export function sandboxLine(s: Pick<Settings, 'claude_sandbox' | 'codex_network_access'>): string {
  const word = (on: boolean) => (on ? 'on' : 'off');
  return `Claude ${word(s.claude_sandbox)} · Codex network ${word(s.codex_network_access)}`;
}

/** "Review on · 2 required checks". `checks` null: not known. */
export function checksLine(reviewGate: boolean, checks: Pick<RequiredChecks, 'on' | 'changed' | 'commands'> | null): string {
  const review = reviewGate ? 'Review on' : 'Review off';
  if (!checks) return review;
  if (checks.changed) return `${review} · checks changed`;
  if (checks.on) return `${review} · ${count(checks.commands.length, 'required check', 'required checks')}`;
  return `${review} · ${checks.commands.length ? 'checks off' : 'no required checks'}`;
}

/** The line of a page whose settings are the open repo's, with no repo open. */
export const NO_REPO_LINE = 'No repository open';

/** "This folder: trusted". `t` null: not known ('' shows no line). */
export function trustLine(t: Pick<WorkspaceTrust, 'state' | 'changed'> | null): string {
  if (!t) return '';
  switch (t.state) {
    case 'nothing':
      return 'This folder: nothing to trust';
    case 'trusted':
      return 'This folder: trusted';
    case 'without':
      return 'This folder: not trusted';
    case 'ask':
      return t.changed ? 'This folder: files changed' : 'This folder: not chosen yet';
  }
}

/** "Setup script: on". `w` null: not known ('' shows no line). */
export function worktreesLine(w: Pick<WorktreeSetup, 'script' | 'on' | 'changed' | 'problem'> | null): string {
  if (!w) return '';
  if (w.changed) return 'Setup script: changed';
  if (w.on) return 'Setup script: on';
  return w.script === null && !w.problem ? 'No setup script' : 'Setup script: off';
}

/** "Budget $20.00 · watchdog 10 min". `b` null: not known, only the watchdog shows. */
export function limitsLine(b: Pick<BreakerConfig, 'enabled' | 'token_budget' | 'cost_budget_usd'> | null, w: Pick<WatchdogSettings, 'quiet_secs'>): string {
  const watchdog = `watchdog ${w.quiet_secs ? limitLabel(w.quiet_secs) : 'off'}`;
  if (!b) return watchdog;
  const breaker = !b.enabled
    ? 'Breaker off'
    : b.cost_budget_usd > 0
      ? `Budget ${usd(b.cost_budget_usd)}`
      : b.token_budget > 0
        ? `${tokens(b.token_budget)} tokens`
        : 'No budget';
  return `${breaker} · ${watchdog}`;
}
