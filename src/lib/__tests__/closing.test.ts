import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { api } from '../api';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Workspace } from '../gen/Workspace';
import { closeWithUndo, UNDO_MS } from '../state/closing';
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
    closeWithUndo('lead:b', 'Fix B');
    expect(tabs()).toEqual(['lead:a', 'lead:c']);
    expect(ui.request).toBe('lead:c'); // the tab beside it
    expect(undoToast()).toMatchObject({ body: 'Fix B', action: { label: 'Undo' } });

    await vi.advanceTimersByTimeAsync(UNDO_MS - 1);
    expect(core).not.toHaveBeenCalled(); // its Lead still runs, its plan is not cancelled

    await vi.advanceTimersByTimeAsync(1);
    expect(core).toHaveBeenCalledTimes(1);
    expect(core).toHaveBeenCalledWith('lead:b');
    expect(undoToast()).toBeUndefined(); // too late to undo: the offer is gone
    expect(ui.closingRequests).toEqual([]);
    expect(tabs()).toEqual(['lead:a', 'lead:c']);
  });

  it('Undo brings the tab back, you are on it again, and the core never closed anything', async () => {
    closeWithUndo('lead:b', 'Fix B');
    undoToast()!.action!.run();
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']);
    expect(ui.request).toBe('lead:b');
    expect(undoToast()).toBeUndefined();

    await vi.advanceTimersByTimeAsync(UNDO_MS * 2);
    expect(core).not.toHaveBeenCalled();
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
    core.mockRejectedValue(new Error('Mira did not stop in time; try again'));
    closeWithUndo('lead:a', 'Fix A');
    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t close the request', 'Mira did not stop in time; try again']]);
  });

  it('another office opened meanwhile: nothing is closed there, and its tabs are not hidden', async () => {
    closeWithUndo('lead:a', 'Fix A');
    // The other office has a request with the same thread name.
    store.workspace = office('w2');
    expect(tabs()).toEqual(['lead:a', 'lead:b', 'lead:c']);

    await vi.advanceTimersByTimeAsync(UNDO_MS);
    expect(core).not.toHaveBeenCalled();
    expect(ui.closingRequests).toEqual([]);
  });
});
