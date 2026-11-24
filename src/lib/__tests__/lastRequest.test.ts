// The request you were last on in each office is also kept in the browser's storage, one key
// per office: after a restart you land where you were.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Workspace } from '../gen/Workspace';
import { openWorkspace, reopenLastRequest } from '../state/actions';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

const office = (id: string): Workspace => ({ id, repo_path: `/tmp/${id}`, name: id, default_branch: 'main', current_branch: 'main', head: null, dirty: false, created_at: 0 });
const asked = (thread: string, ts: number): ChatMessage => ({ id: `${thread}-${ts}`, thread, author: 'user', text: `Fix ${thread}`, attachment: null, ts });
const said = (threads: string[]) => threads.map((thread, i) => asked(thread, i + 1));

/** A storage that keeps what it is given, like the browser's. */
function fakeStorage(kept: Record<string, string> = {}) {
  const map = new Map(Object.entries(kept));
  vi.stubGlobal('localStorage', { getItem: (key: string) => map.get(key) ?? null, setItem: (key: string, value: string) => void map.set(key, value) });
  return map;
}

describe('the request you were last on, across a restart', () => {
  /** What each office's snapshot holds: the requests asked there. */
  let requestsOf: Record<string, string[]>;
  const open = (id: string) => openWorkspace(`/tmp/${id}`);

  beforeEach(() => {
    vi.spyOn(api, 'openWorkspace').mockImplementation(async (path) => office(path.replace('/tmp/', '')));
    vi.spyOn(api, 'memories').mockResolvedValue([]);
    vi.spyOn(api, 'triples').mockResolvedValue([]);
    vi.spyOn(store, 'resync').mockImplementation(async () => {
      store.chats = said(requestsOf[store.workspace?.id ?? ''] ?? []);
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    store.workspace = null;
    store.chats = [];
    ui.draftRequests = [];
    ui.room = 'home';
    ui.openRequest('lead');
  });

  it('is kept as you go, one key per office', async () => {
    const kept = fakeStorage();
    requestsOf = { k1: ['lead', 'lead:x'], k2: ['lead', 'lead:z'] };
    await open('k1');
    expect(kept.get('codly.request.k1')).toBe('lead');
    ui.openRequest('lead:x');
    expect(kept.get('codly.request.k1')).toBe('lead:x');

    // Another office: its own key. While it loads, nothing of the office you left is written under it.
    await open('k2');
    expect(Object.fromEntries(kept)).toEqual({ 'codly.request.k1': 'lead:x', 'codly.request.k2': 'lead' });
    ui.openRequest('lead:z');
    expect(Object.fromEntries(kept)).toEqual({ 'codly.request.k1': 'lead:x', 'codly.request.k2': 'lead:z' });
    // The Plan room's request chips take you to a request too.
    ui.openPlan(null, 'lead');
    expect(kept.get('codly.request.k2')).toBe('lead');
  });

  it('is read back when Codly starts on that office, and the room you were in stays', () => {
    fakeStorage({ 'codly.request.r1': 'lead:y' });
    store.workspace = office('r1'); // the first snapshot: the office the core had open
    store.chats = said(['lead', 'lead:y']);
    ui.room = 'plan';
    reopenLastRequest();
    expect([ui.request, ui.thread, ui.room]).toEqual(['lead:y', 'lead:y', 'plan']);
  });

  it('is read back when you open an office you were not in since the restart', async () => {
    fakeStorage({ 'codly.request.r2': 'lead:y' });
    requestsOf = { r2: ['lead', 'lead:x', 'lead:y'] };
    await open('r2');
    expect(ui.request).toBe('lead:y');
  });

  it('a request that is gone lands on the first one, as before', async () => {
    fakeStorage({ 'codly.request.g1': 'lead:gone', 'codly.request.g2': 'lead:gone' });
    store.workspace = office('g1');
    store.chats = said(['lead', 'lead:x']);
    reopenLastRequest();
    expect(ui.request).toBe('lead');

    requestsOf = { g2: ['lead:x', 'lead:y'] };
    await open('g2');
    expect(ui.request).toBe('lead:x');
  });

  it('an office with nothing kept starts on its first request', () => {
    fakeStorage();
    store.workspace = office('n1');
    store.chats = said(['lead', 'lead:x']);
    reopenLastRequest();
    expect(ui.request).toBe('lead');
    expect(ui.lastRequest('n1')).toBeNull();
  });

  it('a new request whose composer kept text is a tab again after a restart, and you land on it', () => {
    const items = new Map([
      ['codly.request.d1', 'lead:new'],
      ['codly.draft.d1.lead:new', JSON.stringify({ text: 'a long request I never sent', files: [], at: 7 })],
      ['codly.draft.d1.agent:ada', JSON.stringify({ text: 'to ada', files: [], at: 8 })],
    ]);
    vi.stubGlobal('localStorage', {
      get length() {
        return items.size;
      },
      key: (i: number) => [...items.keys()][i] ?? null,
      getItem: (key: string) => items.get(key) ?? null,
      setItem: (key: string, value: string) => void items.set(key, value),
      removeItem: (key: string) => void items.delete(key),
    });
    store.workspace = office('d1');
    store.chats = said(['lead']);
    reopenLastRequest();
    expect(ui.draftRequests).toEqual([{ thread: 'lead:new', ts: 7, kept: true }]);
    expect(ui.request).toBe('lead:new');
  });

  it('a storage that refuses (private mode) changes nothing else: you still move between requests', async () => {
    const refuse = () => {
      throw new Error('The operation is insecure.');
    };
    vi.stubGlobal('localStorage', { getItem: refuse, setItem: refuse });
    requestsOf = { p1: ['lead', 'lead:x'] };
    await open('p1');
    ui.openRequest('lead:x');
    expect(ui.request).toBe('lead:x');
    expect(ui.lastRequest('p1')).toBeNull();
    reopenLastRequest();
    expect(ui.request).toBe('lead:x');
  });
});
