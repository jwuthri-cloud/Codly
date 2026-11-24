// Where a finished plan stands, for the summary of your time away: read from the store and from
// what the core last said about its merge, in the order the plan's own cards say it.
import { afterEach, describe, expect, it } from 'vitest';
import type { MergeState } from '../../lib/gen/MergeState';
import type { Plan } from '../../lib/gen/Plan';
import { mergeKey, mergeUi } from '../../lib/state/mergeState.svelte';
import { store } from '../../lib/state/store.svelte';
import { ui } from '../../lib/state/ui.svelte';
import { planEnd } from '../planEnd';

const plan = (o: Partial<Plan> = {}): Plan => ({
  id: 'p1', workspace_id: 'w1', thread: 'lead', request: 'Add dark mode', summary: '', assumptions: [], open_questions: [], weak_checks: [],
  status: 'done', base_commit: null, base_branch: 'main', integration_branch: 'codly/p1/integration', integration_worktree: null,
  merged_sha: null, kept: false, created_at: 0, updated_at: 0, ...o,
});
const clean: MergeState = { target: 'main', branch: 'codly/p1/integration', repo: '/Users/me/web', conflicts: [], resolving: null, failed_checks: [], checks_not_run: null, behind: 0 };
const said = (merge: MergeState) => {
  mergeUi.states[mergeKey({ kind: 'plan', id: 'p1' })] = merge;
};

afterEach(() => {
  store.plans = [];
  store.planMerges = {};
  mergeUi.states = {};
  ui.keptPlans = [];
});

describe('where a plan stands, for the summary of your time away', () => {
  it('a plan the window does not know, or one at work, is just a plan', () => {
    expect(planEnd('p1')).toBe('other');
    store.plans = [plan({ status: 'running' })];
    expect(planEnd('p1')).toBe('other');
  });

  it('a plan that waits for your go waits', () => {
    store.plans = [plan({ status: 'awaiting_approval' })];
    expect(planEnd('p1')).toBe('waits');
  });

  it('a finished plan: merged, then kept, then what stops the merge, else ready', () => {
    store.plans = [plan()];
    expect(planEnd('p1')).toBe('ready'); // the merge state is not read yet: nothing stops it
    said({ ...clean, checks_not_run: 'no checks on the merge result' });
    expect(planEnd('p1')).toBe('checks');
    said({ ...clean, failed_checks: ['pnpm test'] });
    expect(planEnd('p1')).toBe('checks');
    said({ ...clean, conflicts: ['src/theme.ts'], failed_checks: ['pnpm test'] });
    expect(planEnd('p1')).toBe('conflict');
    ui.keptPlans = ['p1'];
    expect(planEnd('p1')).toBe('kept');
    ui.keptPlans = [];
    store.plans = [plan({ kept: true })];
    expect(planEnd('p1')).toBe('kept');
    store.plans = [plan({ kept: true, merged_sha: 'abc123' })];
    expect(planEnd('p1')).toBe('merged');
    store.plans = [plan()];
    store.markPlanMerged('p1', 'def456');
    expect(planEnd('p1')).toBe('merged');
  });
});
