import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { api } from '../api';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Workspace } from '../gen/Workspace';
import { openWorkspace } from '../state/actions';
import { closeWithUndo, flushClosing, UNDO_MS } from '../state/closing';
import { requests } from '../state/requests.svelte';
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
