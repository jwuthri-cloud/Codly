import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { PullRequestDemo, type DemoWorld } from '../demo/pullrequest';
import type { Plan } from '../gen/Plan';
import type { PrChecks } from '../gen/PrChecks';
import type { PrDraft } from '../gen/PrDraft';
import type { PrSource } from '../gen/PrSource';
import type { PullRequest } from '../gen/PullRequest';
import type { Task } from '../gen/Task';
import { checksLine, checksState, createPullRequest, loadPullRequest, preparePullRequest, prKey, prUi, pushPrBranch, refreshPullRequest } from '../state/pullRequest.svelte';
import { toasts } from '../state/toasts.svelte';

const plan: PrSource = { kind: 'plan', id: 'p1' };
const confirm = { remote: 'origin', branch: 'codly/dark-mode-ab12/integration', base: 'main' };
const pr: PullRequest = { url: 'https://github.com/acme/web/pull/7', ...confirm, created_at: 1, checks: null };
const checks = (c: Partial<PrChecks>): PrChecks => ({ passed: 0, failed: 0, pending: 0, skipped: 0, failing: [], read_at: 1, ...c });
const said = () => toasts.list.map((t) => [t.kind, t.title, t.body]);

describe('the pull-request step', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    prUi.dialog = null;
    prUi.opened = {};
  });

  it('Create pull request… only drafts: nothing is pushed before the confirm', async () => {
    const draft: PrDraft = { ...confirm, title: 'Dark mode', body: '## Goal', gh_problem: null, warning: null };
    const drafted = vi.spyOn(api, 'draftPullRequest').mockResolvedValue(draft);
    const created = vi.spyOn(api, 'createPullRequest');
    const pushed = vi.spyOn(api, 'pushPrBranch');
    expect(await preparePullRequest(plan)).toBe(true);
    expect(drafted).toHaveBeenCalledWith(plan);
    expect(prUi.dialog).toEqual({ source: plan, draft });
    expect(created).not.toHaveBeenCalled();
    expect(pushed).not.toHaveBeenCalled();

    // A draft that cannot be made opens no dialog and says why.
    prUi.dialog = null;
    drafted.mockRejectedValue(new Error('this repository has no remote yet'));
    expect(await preparePullRequest(plan)).toBe(false);
    expect(prUi.dialog).toBeNull();
    expect(said()).toEqual([['error', 'Couldn’t prepare the pull request', 'this repository has no remote yet']]);
  });

  it('a pull request shows only once the core has opened it, with what you confirmed', async () => {
    const core = vi.spyOn(api, 'createPullRequest').mockRejectedValue(new Error('the GitHub CLI (gh) is not installed; nothing was pushed'));
    expect(await createPullRequest(plan, confirm, 'Dark mode', 'My words')).toBe(false);
    expect(prUi.opened[prKey(plan)]).toBeUndefined();
    expect(said()).toEqual([['error', 'Couldn’t open the pull request', 'the GitHub CLI (gh) is not installed; nothing was pushed']]);

    core.mockResolvedValue(pr);
    expect(await createPullRequest(plan, confirm, 'Dark mode', 'My words')).toBe(true);
    expect(core).toHaveBeenLastCalledWith(plan, confirm, 'Dark mode', 'My words');
    expect(prUi.opened[prKey(plan)]).toEqual(pr);
    expect(said()).toContainEqual(['success', 'Pull request opened', pr.url]);
  });

  it('Push branch answers with the compare page, and opens no pull request', async () => {
    const answer = { remote: 'origin', branch: confirm.branch, compare_url: 'https://github.com/acme/web/compare/main...x?expand=1' };
    const core = vi.spyOn(api, 'pushPrBranch').mockResolvedValue(answer);
    expect(await pushPrBranch(plan, confirm)).toEqual(answer);
    expect(core).toHaveBeenCalledWith(plan, confirm);
    expect(prUi.opened[prKey(plan)]).toBeUndefined();
    core.mockRejectedValue(new Error('Codly never force-pushes, so nothing was sent'));
    expect(await pushPrBranch(plan, confirm)).toBeUndefined();
    expect(said()).toContainEqual(['error', `Couldn’t push ${confirm.branch}`, 'Codly never force-pushes, so nothing was sent']);
  });

  it('the record is read without gh, and the checks only on Refresh', async () => {
    const record = vi.spyOn(api, 'pullRequest').mockResolvedValue(pr);
    const refresh = vi.spyOn(api, 'refreshPullRequest').mockResolvedValue({ ...pr, checks: checks({ passed: 2 }) });
    await loadPullRequest(plan);
    expect(record).toHaveBeenCalledWith(plan);
    expect(refresh).not.toHaveBeenCalled();
    expect(prUi.opened[prKey(plan)]?.checks).toBeNull();
    await refreshPullRequest(plan);
    expect(prUi.opened[prKey(plan)]?.checks?.passed).toBe(2);

    // A refresh that fails keeps the last answer and says why.
    refresh.mockRejectedValue(new Error('gh could not read the checks: HTTP 502'));
    await refreshPullRequest(plan);
    expect(prUi.opened[prKey(plan)]?.checks?.passed).toBe(2);
    expect(said()).toEqual([['error', 'Couldn’t read the checks', 'gh could not read the checks: HTTP 502']]);
  });

  it('the checks read as one line, the worst news first', () => {
    expect(checksLine(checks({}))).toBe('none reported yet');
    expect(checksLine(checks({ passed: 3 }))).toBe('3 passed');
    expect(checksLine(checks({ passed: 3, pending: 2, skipped: 1 }))).toBe('2 running · 3 passed · 1 skipped');
    expect(checksLine(checks({ passed: 1, failed: 1, failing: ['lint'] }))).toBe('1 failed (lint) · 1 passed');
    expect(checksLine(checks({ failed: 7, failing: ['a', 'b', 'c', 'd', 'e'] }))).toBe('7 failed (a, b, c, d, e, …)');
    expect([checks({}), checks({ passed: 1 }), checks({ passed: 1, pending: 1 }), checks({ passed: 1, pending: 1, failed: 1 })].map(checksState)).toEqual(['none', 'ok', 'wait', 'bad']);
  });
});

describe('the demo’s pull requests', () => {
  const world: DemoWorld = {
    plans: new Map([['p1', { id: 'p1', summary: 'Add a dark mode', request: 'Please add a dark mode', integration_branch: confirm.branch, base_branch: null } as Plan]]),
    tasks: new Map([
      ['t1', { id: 't1', plan_id: 'p1', key: 't1', title: 'Theme tokens', status: 'done', verify: ['pnpm test theme'] } as Task],
      ['t2', { id: 't2', plan_id: 'p1', key: 't2', title: 'Toggle', status: 'changes_requested', verify: ['pnpm test toggle'] } as Task],
    ]),
    agents: new Map(),
    now: () => 1000,
  };

  it('drafts from the plan, opens one pull request per branch, and its checks finish', () => {
    const demo = new PullRequestDemo();
    const args = { source: plan, confirm };
    expect(demo.handle('pull_request', args, world)).toBeNull();
    expect(demo.handle('draft_pull_request', args, world)).toEqual({
      ...confirm,
      title: 'Add a dark mode',
      body: '## Goal\n\nPlease add a dark mode\n\n## Tasks\n\n- **t1** Theme tokens (done)\n- **t2** Toggle (changes requested)\n\n## Checks\n\n- t1: `pnpm test theme` passed\n',
      gh_problem: null,
      warning: null,
    });
    expect(demo.handle('pull_request', args, world)).toBeNull();
    expect(demo.handle('push_pr_branch', args, world)).toEqual({ remote: 'origin', branch: confirm.branch, compare_url: `https://github.com/acme/web/compare/main...${confirm.branch}?expand=1` });
    expect(demo.handle('pull_request', args, world)).toBeNull();

    const made = demo.handle('create_pull_request', args, world) as PullRequest;
    expect(made).toEqual({ url: 'https://github.com/acme/web/pull/213', ...confirm, created_at: 1000, checks: null });
    expect(demo.handle('create_pull_request', args, world)).toEqual(made);
    expect(demo.handle('pull_request', args, world)).toEqual(made);
    const first = demo.handle('refresh_pull_request', args, world) as PullRequest;
    expect([first.checks?.pending, first.checks?.passed]).toEqual([2, 1]);
    const second = demo.handle('refresh_pull_request', args, world) as PullRequest;
    expect([second.checks?.pending, second.checks?.passed]).toEqual([0, 3]);
    expect(() => demo.handle('refresh_pull_request', { source: { kind: 'agent', id: 'nobody' } }, world)).toThrow('has not opened a pull request');
  });
});
