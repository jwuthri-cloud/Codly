import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { api } from '../api';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Plan } from '../gen/Plan';
import type { PullRequest } from '../gen/PullRequest';
import type { Task } from '../gen/Task';
import type { Workspace } from '../gen/Workspace';
import { openWorkspace } from '../state/actions';
import { closeQuestion, closeWithUndo, flushClosing, UNDO_MS } from '../state/closing';
import { prUi } from '../state/pullRequest.svelte';
import { requests, type Request } from '../state/requests.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

const office = (id: string): Workspace => ({ id, repo_path: `/tmp/${id}`, name: id, default_branch: 'main', current_branch: 'main', head: null, dirty: false, created_at: 0 });
const asked = (thread: string, text: string, ts: number): ChatMessage => ({ id: `${thread}-${ts}`, thread, author: 'user', text, attachment: null, ts });
const tabs = () => requests.list.map((r) => r.thread);
const undoToast = () => toasts.list.find((t) => t.title === 'Request closed');

describe('closing a request, with Undo', () => {
  let core: MockInstance<typeof api.closeThread>;

  beforeEach(() => {
    vi.useFakeTimers();
    // The core moves the request's messages out of its thread; the resync brings that to the store.
    core = vi.spyOn(api, 'closeThread').mockImplementation(async (thread) => {
      store.chats = store.chats.filter((c) => c.thread !== thread);
    });
    vi.spyOn(store, 'resync').mockResolvedValue();
    store.workspace = office('w1');
    store.chats = [asked('lead:a', 'Fix A', 1), asked('lead:b', 'Fix B', 2), asked('lead:c', 'Fix C', 3)];
    ui.openRequest('lead:b');
  });

  afterEach(async () => {
    await vi.runAllTimersAsync(); // no close of one test waits into the next
    vi.useRealTimers();
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.workspace = null;
    store.chats = [];
    ui.closingRequests = [];
    ui.draftRequests = [];
    ui.openRequest('lead');
  });

  it('the tab leaves at once, and the core closes the request only when the time for Undo is up', async () => {
    const rev = ui.closedRev;
    closeWithUndo('lead:b', 'Fix B');
    expect(tabs()).toEqual(['lead:a', 'lead:c']);
    expect(ui.request).toBe('lead:c'); // the tab beside it
    expect(undoToast()).toMatchObject({ body: 'Fix B', action: { label: 'Undo' } });

    await vi.advanceTimersByTimeAsync(UNDO_MS - 1);
    expect(core).not.toHaveBeenCalled(); // its Lead still runs, its plan is not cancelled
    expect(ui.closedRev).toBe(rev);

    await vi.advanceTimersByTimeAsync(1);
    expect(core).toHaveBeenCalledTimes(1);
    expect(core).toHaveBeenCalledWith('lead:b');
    expect(ui.closedRev).toBe(rev + 1); // the closed requests' list reads again: it is in it now
    expect(undoToast()).toBeUndefined(); // too late to undo: the offer is gone
    expect(ui.closingRequests).toEqual([]);
    expect(tabs()).toEqual(['lead:a', 'lead:c']);
  });

  it('Undo brings the tab back, you are on it again, and the core never closed anything', async () => {
    const rev = ui.closedRev;
    closeWithUndo('lead:b', 'Fix B');
    undoToast()!.action!.run();
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']);
    expect(ui.request).toBe('lead:b');
    expect(undoToast()).toBeUndefined();

    await vi.advanceTimersByTimeAsync(UNDO_MS * 2);
    expect(core).not.toHaveBeenCalled();
    expect(ui.closedRev).toBe(rev);
  });

  it('closing the last tab lands on the one before; closing the only request opens a new one', () => {
    ui.openRequest('lead:c');
    closeWithUndo('lead:c', 'Fix C');
    expect(ui.request).toBe('lead:b');

    closeWithUndo('lead:a', 'Fix A'); // not the one you are on: you stay
    expect(ui.request).toBe('lead:b');

    closeWithUndo('lead:b', 'Fix B');
    expect(ui.request).not.toBe('lead:b');
    expect(tabs()).toEqual([ui.request]); // a new, empty request
    expect(requests.get(ui.request)?.phase).toBe('new');
  });

  it('closing the same request twice asks the core once', async () => {
    closeWithUndo('lead:a', 'Fix A');
    closeWithUndo('lead:a', 'Fix A');
    expect(toasts.list.filter((t) => t.title === 'Request closed')).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(core).toHaveBeenCalledTimes(1);
  });

  it('a request the core refuses to close comes back as a tab, with the reason', async () => {
    const rev = ui.closedRev;
    core.mockRejectedValue(new Error('Mira did not stop in time; try again'));
    closeWithUndo('lead:a', 'Fix A');
    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']);
    expect(ui.closedRev).toBe(rev);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t close the request', 'Mira did not stop in time; try again']]);
  });

  it('opening another repository closes what waits for Undo first, while its office is the open one', async () => {
    const open = vi.spyOn(api, 'openWorkspace').mockResolvedValue(office('w2'));
    vi.spyOn(api, 'memories').mockResolvedValue([]);
    vi.spyOn(api, 'triples').mockResolvedValue([]);
    closeWithUndo('lead:a', 'Fix A');
    closeWithUndo('lead:c', 'Fix C');

    await openWorkspace('/tmp/w2');
    expect(core.mock.calls).toEqual([['lead:a'], ['lead:c']]);
    // Both closes reached the core before it opened the next office.
    expect(Math.max(...core.mock.invocationCallOrder)).toBeLessThan(open.mock.invocationCallOrder[0]);
    expect(store.workspace?.id).toBe('w2');
    expect(undoToast()).toBeUndefined(); // closed for good: Undo is no longer on offer
    expect(ui.closingRequests).toEqual([]);

    // Their timers are gone: nothing is closed in the next office, where the names repeat.
    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(core).toHaveBeenCalledTimes(2);
  });

  it('a close the core is still working on is awaited before another repository opens', async () => {
    let answer!: () => void;
    core.mockImplementation(() => new Promise<void>((r) => (answer = r)));
    closeWithUndo('lead:a', 'Fix A');
    await vi.advanceTimersByTimeAsync(UNDO_MS); // the time is up: the core is closing it
    expect(core).toHaveBeenCalledTimes(1);

    let flushed = false;
    const flush = flushClosing().then(() => (flushed = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(flushed).toBe(false);

    answer();
    await flush;
    expect(core).toHaveBeenCalledTimes(1); // awaited, not asked again
    expect(ui.closingRequests).toEqual([]);
  });

  it('a request closed while the next repository was already opening stays open, and says so', async () => {
    closeWithUndo('lead:a', 'Fix A');
    // The switch landed with the close still waiting. The other office has a request of the same name.
    store.workspace = office('w2');
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']); // its tabs are not hidden

    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(core).not.toHaveBeenCalled(); // nothing is closed in the other office
    expect(ui.closingRequests).toEqual([]);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['attention', 'Request not closed', '“Fix A” is still open: another repository opened first.']]);
  });
});

describe('what a close asks first', () => {
  const BRANCH = 'codly/ab12/integration';
  const plan = (o: Partial<Plan> = {}): Plan => ({ id: 'p1', thread: 'lead:a', status: 'done', integration_branch: BRANCH, merged_sha: null, kept: false, created_at: 1, ...o }) as Plan;
  const task = (id: string, status: Task['status'], plan_id = 'p1'): Task => ({ id, plan_id, status }) as Task;
  const request = (o: Partial<Request> = {}): Request => ({ thread: 'lead:a', title: 'Fix A', phase: 'done', ...o }) as Request;
  const pr = (state: PullRequest['state']): PullRequest => ({ url: 'https://github.com/acme/web/pull/7', remote: 'origin', branch: BRANCH, base: 'main', created_at: 1, state, checks: null });
  const kept = `Nothing is deleted: ${BRANCH} stays, and you can still merge it in Changes.`;
  const stays = 'The pull request stays open: Codly does not close or merge it.';
  let record: MockInstance<typeof api.pullRequest>;

  beforeEach(() => {
    record = vi.spyOn(api, 'pullRequest').mockResolvedValue(null);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    store.plans = [];
    store.tasks = [];
    ui.keptPlans = [];
    prUi.opened = {};
  });

  it('a request that leaves nothing behind closes at once: no plan, a cancelled or failed one, merged work, a branch you chose to keep', async () => {
    expect(await closeQuestion(request({ phase: 'talking' }))).toBeNull();
    for (const p of [plan({ status: 'cancelled' }), plan({ status: 'failed' }), plan({ merged_sha: 'abc1234' }), plan({ kept: true })]) {
      store.plans = [p];
      expect(await closeQuestion(request())).toBeNull();
    }
    store.plans = [plan()];
    ui.keptPlans = ['p1']; // kept a moment ago: the plan on record does not say so yet
    expect(await closeQuestion(request())).toBeNull();
    // Another request's finished work is not this one's.
    ui.keptPlans = [];
    store.plans = [plan({ thread: 'lead:b' })];
    expect(await closeQuestion(request())).toBeNull();
  });

  it('finished work that is not merged asks first, and says that its branch stays', async () => {
    store.plans = [plan()];
    expect(await closeQuestion(request())).toEqual({ ask: 'Not merged yet: its branch stays. Close?', detail: kept, yes: 'Close' });
    // Also while its Lead answers a later message (the tab then says "Thinking").
    expect(await closeQuestion(request({ phase: 'planning' }))).toEqual({ ask: 'Not merged yet: its branch stays. Close?', detail: kept, yes: 'Close' });
    // An earlier plan of the request counts too.
    store.plans = [plan(), plan({ id: 'p2', created_at: 2, merged_sha: 'abc1234' })];
    expect((await closeQuestion(request({ phase: 'merged' })))?.ask).toBe('Not merged yet: its branch stays. Close?');
  });

  it('a failed or cancelled plan with done tasks asks first, and says that their work stays on its branch', async () => {
    const inChanges = 'Nothing is deleted: you can still merge it in Changes.';
    const some = { ask: 'Some tasks are done and not merged: their work stays on the plan’s branch. Close?', detail: inChanges, yes: 'Close' };
    store.tasks = [task('t1', 'done'), task('t2', 'done'), task('t3', 'failed')];
    for (const status of ['failed', 'cancelled'] as const) {
      store.plans = [plan({ status })];
      expect(await closeQuestion(request({ phase: status }))).toEqual(some);
    }
    // One done task: the question does not say "some".
    store.plans = [plan({ status: 'failed' })];
    store.tasks = [task('t1', 'done'), task('t3', 'failed')];
    expect(await closeQuestion(request({ phase: 'failed' }))).toEqual({ ask: 'A task is done and not merged: its work stays on the plan’s branch. Close?', detail: inChanges, yes: 'Close' });
    expect(record).not.toHaveBeenCalled(); // a plan that did not finish has no pull request to read
  });

  it('a failed or cancelled plan with no done task leaves nothing behind, and neither does one whose work is in your branch', async () => {
    store.plans = [plan({ status: 'failed' })];
    store.tasks = [task('t1', 'failed'), task('t2', 'cancelled'), task('t9', 'done', 'another-plan')];
    expect(await closeQuestion(request({ phase: 'failed' }))).toBeNull();
    // Its done tasks are merged, or you chose to keep the branch as it is.
    store.tasks = [task('t1', 'done'), task('t2', 'failed')];
    for (const p of [plan({ status: 'failed', merged_sha: 'abc1234' }), plan({ status: 'cancelled', kept: true })]) {
      store.plans = [p];
      expect(await closeQuestion(request({ phase: 'failed' }))).toBeNull();
    }
    // Another request's plan is not this one's.
    store.plans = [plan({ status: 'failed', thread: 'lead:b' })];
    expect(await closeQuestion(request({ phase: 'talking' }))).toBeNull();
  });

  it('a finished plan that is not merged asks about its branch, also beside a failed plan with done tasks', async () => {
    store.plans = [plan(), plan({ id: 'p0', status: 'failed', created_at: 0 })];
    store.tasks = [task('t1', 'done', 'p0')];
    expect(await closeQuestion(request())).toEqual({ ask: 'Not merged yet: its branch stays. Close?', detail: kept, yes: 'Close' });
  });

  it('an open pull request asks first, with unmerged work and without', async () => {
    record.mockResolvedValue(pr('open'));
    store.plans = [plan()];
    expect(await closeQuestion(request())).toEqual({ ask: 'Not merged yet: its branch and pull request stay. Close?', detail: `${kept} ${stays}`, yes: 'Close' });
    expect(record).toHaveBeenCalledWith({ kind: 'plan', id: 'p1' });

    store.plans = [plan({ merged_sha: 'abc1234' })];
    expect(await closeQuestion(request({ phase: 'merged' }))).toEqual({ ask: 'Its pull request stays open. Close?', detail: stays, yes: 'Close' });
    expect(record).toHaveBeenCalledTimes(1); // on record here already: not read again

    // Merged or closed on its host, as last read: nothing of it is left open.
    prUi.opened = { 'plan:p1': pr('merged') };
    expect(await closeQuestion(request({ phase: 'merged' }))).toBeNull();
  });

  it('a running plan asks as before, and reads nothing', async () => {
    store.plans = [plan({ status: 'running' })];
    expect(await closeQuestion(request({ phase: 'running' }))).toMatchObject({ ask: 'Cancel its running plan and close?', yes: 'Close' });
    expect(record).not.toHaveBeenCalled();
  });

  it('a Lead at work asks first: in a turn, or starting before it, or before it answered your message', async () => {
    const working = { ask: 'The Lead is working: close and stop it?', detail: 'Its turn stops where it is. Nothing is deleted.', yes: 'Close' };
    // The Lead starts or thinks and no turn of it runs yet, in a request with no plan: the request says "planning".
    expect(await closeQuestion(request({ phase: 'planning' }))).toEqual(working);
    // Your message is in, and the Lead has not begun: nothing answered it.
    store.chats = [asked('lead:a', 'Explain sessions', 5)];
    expect(await closeQuestion(request({ phase: 'talking' }))).toEqual({ ask: 'Your message has no answer yet: close anyway?', detail: 'The Lead stops before it answers. Nothing is deleted.', yes: 'Close' });
    // Answered: it closes at once.
    store.chats = [...store.chats, { ...asked('lead:a', 'Sessions live in Postgres.', 6), author: 'l3' }];
    expect(await closeQuestion(request({ phase: 'talking' }))).toBeNull();
    store.chats = [];
  });

  it('a draft asks only while its composer holds text, and the button says Discard', async () => {
    ui.newRequest();
    const draft = request({ thread: ui.request, title: 'New request', phase: 'new' });
    expect(await closeQuestion(draft)).toBeNull(); // empty: it closes at once
    ui.holdDraft(draft.thread, true); // you typed in it
    expect(await closeQuestion(draft)).toEqual({ ask: 'Discard this draft?', detail: 'What you wrote in it was not sent: it goes with the tab.', yes: 'Discard' });
    ui.holdDraft(draft.thread, false); // and cleared it again
    expect(await closeQuestion(draft)).toBeNull();
    expect(record).not.toHaveBeenCalled();
    ui.draftRequests = [];
    ui.openRequest('lead');
  });
});
