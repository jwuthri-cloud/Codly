import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { PullRequestDemo, type DemoWorld } from '../demo/pullrequest';
import type { ChatMessage } from '../gen/ChatMessage';
import type { IntakeSource } from '../gen/IntakeSource';
import type { Plan } from '../gen/Plan';
import type { PrChecks } from '../gen/PrChecks';
import type { PrDraft } from '../gen/PrDraft';
import type { PrSource } from '../gen/PrSource';
import type { PullRequest } from '../gen/PullRequest';
import type { Task } from '../gen/Task';
import {
  checksLine,
  checksState,
  confirmMode,
  createPullRequest,
  loadPullRequest,
  preparePullRequest,
  prKey,
  prUi,
  pushPrBranch,
  readsByItself,
  readWhileShown,
  refreshPullRequest,
  type Page,
} from '../state/pullRequest.svelte';
import { toasts } from '../state/toasts.svelte';

const plan: PrSource = { kind: 'plan', id: 'p1' };
const confirm = { remote: 'origin', branch: 'codly/dark-mode-ab12/integration', base: 'main' };
const pr: PullRequest = { url: 'https://github.com/acme/web/pull/7', ...confirm, created_at: 1, state: 'open', checks: null };
const drafted: PrDraft = { ...confirm, title: 'Dark mode', body: '## Goal', gh_problem: null, warning: null, pr_state: null };
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
    const core = vi.spyOn(api, 'draftPullRequest').mockResolvedValue(drafted);
    const created = vi.spyOn(api, 'createPullRequest');
    const pushed = vi.spyOn(api, 'pushPrBranch');
    expect(await preparePullRequest(plan)).toBe(true);
    expect(core).toHaveBeenCalledWith(plan);
    expect(prUi.dialog).toEqual({ source: plan, draft: drafted });
    expect(created).not.toHaveBeenCalled();
    expect(pushed).not.toHaveBeenCalled();

    // A draft that cannot be made opens no dialog and says why.
    prUi.dialog = null;
    core.mockRejectedValue(new Error('this repository has no remote yet'));
    expect(await preparePullRequest(plan)).toBe(false);
    expect(prUi.dialog).toBeNull();
    expect(said()).toEqual([['error', 'Couldn’t prepare the pull request', 'this repository has no remote yet']]);
  });

  it('a pull request that was merged or closed makes room for a new one', async () => {
    // Open as gh just said: new commits join it. Not asked (no pull request on record): create.
    expect(confirmMode(null, drafted)).toBe('create');
    expect(confirmMode(pr, { ...drafted, pr_state: 'open' })).toBe('join');
    expect(confirmMode({ ...pr, state: 'merged' }, { ...drafted, pr_state: 'merged' })).toBe('create');
    expect(confirmMode({ ...pr, state: 'closed' }, { ...drafted, pr_state: 'closed' })).toBe('create');
    // gh cannot open one, or cannot say where the one on record stands: only a push, no claim.
    expect(confirmMode(null, { ...drafted, gh_problem: 'gh is not installed' })).toBe('push');
    expect(confirmMode(pr, { ...drafted, gh_problem: 'Codly does not know whether the pull request it opened is still open' })).toBe('push');
    expect(confirmMode(pr, drafted)).toBe('push');

    // The row learns the state the draft read, before the dialog opens.
    prUi.opened[prKey(plan)] = pr;
    vi.spyOn(api, 'draftPullRequest').mockResolvedValue({ ...drafted, pr_state: 'merged' });
    expect(await preparePullRequest(plan)).toBe(true);
    expect(prUi.opened[prKey(plan)]).toEqual({ ...pr, state: 'merged' });
    expect(confirmMode(prUi.opened[prKey(plan)], prUi.dialog!.draft)).toBe('create');
    // A draft that could not ask leaves the last answer as it is.
    vi.spyOn(api, 'draftPullRequest').mockResolvedValue(drafted);
    expect(await preparePullRequest(plan)).toBe(true);
    expect(prUi.opened[prKey(plan)]?.state).toBe('merged');
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

  it('the record is read without gh, and the state and checks only on Refresh', async () => {
    const record = vi.spyOn(api, 'pullRequest').mockResolvedValue(pr);
    const refresh = vi.spyOn(api, 'refreshPullRequest').mockResolvedValue({ ...pr, state: 'merged', checks: checks({ passed: 2 }) });
    await loadPullRequest(plan);
    expect(record).toHaveBeenCalledWith(plan);
    expect(refresh).not.toHaveBeenCalled();
    expect(prUi.opened[prKey(plan)]?.checks).toBeNull();
    await refreshPullRequest(plan);
    expect(prUi.opened[prKey(plan)]?.checks?.passed).toBe(2);
    expect(prUi.opened[prKey(plan)]?.state).toBe('merged');

    // A refresh that fails keeps the last answer and says why.
    refresh.mockRejectedValue(new Error('gh could not read the pull request: HTTP 502'));
    await refreshPullRequest(plan);
    expect(prUi.opened[prKey(plan)]?.checks?.passed).toBe(2);
    expect(said()).toEqual([['error', 'Couldn’t read the pull request', 'gh could not read the pull request: HTTP 502']]);
  });

  it('reads by itself only an open pull request whose row is in view, and not after you merged it here', () => {
    expect(readsByItself(pr, true, false)).toBe(true);
    expect(readsByItself(pr, false, false)).toBe(false);
    expect(readsByItself(pr, true, true)).toBe(false);
    expect(readsByItself({ ...pr, state: 'merged' }, true, false)).toBe(false);
    expect(readsByItself(null, true, false)).toBe(false);
    expect(readsByItself(undefined, true, false)).toBe(false);
  });

  it('one timer per pull request, however many of its rows are in view, and none once they all left', () => {
    const ticks: (() => void)[] = [];
    const focus: (() => void)[] = [];
    let stopped = 0;
    let visible = true;
    const page: Page = {
      every: (_ms, run) => {
        ticks.push(run);
        return () => void stopped++;
      },
      onFocus: (run) => {
        focus.push(run);
        return () => {};
      },
      visible: () => visible,
    };
    const refresh = vi.spyOn(api, 'refreshPullRequest').mockResolvedValue(pr);
    const first = readWhileShown(plan, page);
    const second = readWhileShown(plan, page);
    expect(ticks).toHaveLength(1);
    ticks[0]();
    expect(refresh).toHaveBeenCalledTimes(1);
    // The page hidden: no read.
    visible = false;
    ticks[0]();
    focus[0]();
    expect(refresh).toHaveBeenCalledTimes(1);
    first();
    first(); // a second release of the same row counts once
    expect(stopped).toBe(0);
    second();
    expect(stopped).toBe(1);
    // Another pull request has its own timer.
    readWhileShown({ kind: 'agent', id: 'a1' }, page)();
    expect(ticks).toHaveLength(2);
  });

  it('a read the row makes by itself that fails shows no toast: the row keeps the last reading and its age', async () => {
    prUi.opened[prKey(plan)] = { ...pr, checks: checks({ passed: 1, read_at: 5 }) };
    vi.spyOn(api, 'refreshPullRequest').mockRejectedValue(new Error('gh could not read the pull request: HTTP 502'));
    await refreshPullRequest(plan, true);
    expect(said()).toEqual([]);
    expect(prUi.opened[prKey(plan)]?.checks).toEqual(checks({ passed: 1, read_at: 5 }));
  });

  it('a read by itself is not made while a read of that pull request is on its way', async () => {
    let answer!: (p: PullRequest) => void;
    const refresh = vi.spyOn(api, 'refreshPullRequest').mockReturnValue(new Promise<PullRequest>((r) => (answer = r)));
    const asked = refreshPullRequest(plan);
    await refreshPullRequest(plan, true);
    expect(refresh).toHaveBeenCalledTimes(1);
    answer(pr);
    await asked;
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
    chats: [],
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
      pr_state: null,
    });
    expect(demo.handle('pull_request', args, world)).toBeNull();
    expect(demo.handle('push_pr_branch', args, world)).toEqual({ remote: 'origin', branch: confirm.branch, compare_url: `https://github.com/acme/web/compare/main...${confirm.branch}?expand=1` });
    expect(demo.handle('pull_request', args, world)).toBeNull();

    const made = demo.handle('create_pull_request', args, world) as PullRequest;
    expect(made).toEqual({ url: 'https://github.com/acme/web/pull/213', ...confirm, created_at: 1000, state: 'open', checks: null });
    expect((demo.handle('draft_pull_request', args, world) as PrDraft).pr_state).toBe('open');
    expect(demo.handle('create_pull_request', args, world)).toEqual(made);
    expect(demo.handle('pull_request', args, world)).toEqual(made);
    const first = demo.handle('refresh_pull_request', args, world) as PullRequest;
    expect([first.checks?.pending, first.checks?.passed]).toEqual([2, 1]);
    const second = demo.handle('refresh_pull_request', args, world) as PullRequest;
    expect([second.checks?.pending, second.checks?.passed]).toEqual([0, 3]);
    expect(() => demo.handle('refresh_pull_request', { source: { kind: 'agent', id: 'nobody' } }, world)).toThrow('has not opened a pull request');
  });

  it('ends the description with the issue or pull request the plan’s request started from', () => {
    const source: IntakeSource = { kind: 'issue', host: 'github.com', owner: 'acme', repo: 'web', number: 12, url: 'https://github.com/acme/web/issues/12', title: 'Dark mode resets on reload' };
    const from = (ts: number, intake: IntakeSource): ChatMessage => ({ id: `m${ts}`, thread: 'lead', author: 'user', text: 'Work on it.', attachment: { intake }, ts });
    const p1 = { ...world.plans.get('p1')!, thread: 'lead', status: 'running', created_at: 500 } as Plan;
    const drafted = (chats: ChatMessage[], plans: Plan[] = [p1]) => {
      const w: DemoWorld = { ...world, chats, plans: new Map(plans.map((p) => [p.id, p])) };
      return (new PullRequestDemo().handle('draft_pull_request', { source: plan, confirm }, w) as PrDraft).body;
    };
    const plain = drafted([]);
    expect(plain).not.toMatch(/Closes|Related to/);
    // An issue of the repo the pull request opens in closes with it; of another repo, by its full name.
    expect(drafted([from(100, source)])).toBe(`${plain}\nCloses #12\n`);
    expect(drafted([from(100, { ...source, owner: 'Acme', repo: 'WEB' })])).toBe(`${plain}\nCloses #12\n`);
    expect(drafted([from(100, { ...source, repo: 'api', number: 3 })])).toBe(`${plain}\nCloses acme/api#3\n`);
    // A pull request, or another host, is only named.
    expect(drafted([from(100, { ...source, kind: 'pull_request', number: 7 })])).toBe(`${plain}\nRelated to #7\n`);
    expect(drafted([from(100, { ...source, host: 'git.example.com' })])).toBe(`${plain}\nRelated to https://github.com/acme/web/issues/12\n`);
    // The newest message before the plan counts; one sent after the plan, or in another request, does not.
    expect(drafted([from(100, source), from(200, { ...source, number: 40 })])).toBe(`${plain}\nCloses #40\n`);
    expect(drafted([from(900, source)])).toBe(plain);
    expect(drafted([{ ...from(100, source), thread: 'lead:other' }])).toBe(plain);
    // An earlier plan made after that message is done: its work was delivered, this plan is new work.
    const earlier = { ...p1, id: 'p0', status: 'done', created_at: 300 } as Plan;
    expect(drafted([from(100, source)], [earlier, p1])).toBe(plain);
  });
});
