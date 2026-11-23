import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Workspace } from '../gen/Workspace';
import { openWorkspace } from '../state/actions';
import { requests } from '../state/requests.svelte';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

const tabs = () => requests.list.map((r) => r.thread);

describe('a new request that has no message yet', () => {
  afterEach(() => {
    ui.draftRequests = [];
    ui.composerSeed = null;
    ui.openRequest('lead');
  });

  it('is dropped when you leave it with nothing in its composer', () => {
    ui.newRequest();
    expect(tabs()).toEqual([ui.request]);
    ui.openRequest('lead');
    expect(ui.draftRequests).toEqual([]);
    expect(tabs()).toEqual([]);
  });

  it('stays a tab while its composer holds text or files, whatever you open meanwhile', () => {
    ui.newRequest();
    const draft = ui.request;
    ui.holdDraft(draft, true); // its composer: you typed something
    ui.openRequest('lead');
    expect(tabs()).toEqual([draft]);
    ui.newRequest(); // ⌘N
    expect(tabs()).toEqual([draft, ui.request]);
    ui.openRequest(draft);
    expect(tabs()).toEqual([draft]); // the empty one is gone, as before

    ui.holdDraft(draft, false); // sent, or cleared by hand
    ui.openRequest('lead');
    expect(ui.draftRequests).toEqual([]);
  });

  it('holds the text another surface hands it, before its composer is on screen', () => {
    ui.newRequest();
    const draft = ui.request;
    ui.seedComposer(draft, 'Bring t1 into main.');
    ui.openRequest('lead');
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([draft]);
  });
});

describe('the request you land on when a repository opens', () => {
  const office = (id: string): Workspace => ({ id, repo_path: `/tmp/${id}`, name: id, default_branch: 'main', current_branch: 'main', head: null, dirty: false, created_at: 0 });
  const asked = (thread: string, ts: number): ChatMessage => ({ id: `${thread}-${ts}`, thread, author: 'user', text: `Fix ${thread}`, attachment: null, ts });
  /** What each office's snapshot holds: the requests asked there. */
  let said: Record<string, string[]>;
  const open = (id: string) => openWorkspace(`/tmp/${id}`);

  beforeEach(() => {
    vi.spyOn(api, 'openWorkspace').mockImplementation(async (path) => office(path.replace('/tmp/', '')));
    vi.spyOn(api, 'memories').mockResolvedValue([]);
    vi.spyOn(api, 'triples').mockResolvedValue([]);
    vi.spyOn(store, 'resync').mockImplementation(async () => {
      store.chats = (said[store.workspace?.id ?? ''] ?? []).map((thread, i) => asked(thread, i + 1));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    store.workspace = null;
    store.chats = [];
    ui.draftRequests = [];
    ui.openRequest('lead');
  });

  it('is the one you last had open there', async () => {
    said = { a1: ['lead', 'lead:x', 'lead:y'], b1: ['lead', 'lead:z'] };
    await open('a1');
    expect(ui.request).toBe('lead'); // the first time: its first request
    ui.openRequest('lead:y');

    await open('b1');
    expect(ui.request).toBe('lead');
    ui.openRequest('lead:z');

    await open('a1');
    expect(ui.request).toBe('lead:y');
    await open('b1');
    expect(ui.request).toBe('lead:z');
    // The same repository opened again: you stay where you are.
    await open('b1');
    expect(ui.request).toBe('lead:z');
  });

  it('is the first request there when that one is gone, or "lead" was closed: never an empty one beside real tabs', async () => {
    said = { a2: ['lead:x', 'lead:y'], b2: ['lead'] };
    await open('a2');
    expect(ui.request).toBe('lead:x');
    ui.openRequest('lead:y');

    await open('b2');
    said.a2 = ['lead:x']; // closed meanwhile
    await open('a2');
    expect(ui.request).toBe('lead:x');
  });

  it('is "lead" in a repository where nothing was asked yet', async () => {
    said = {};
    await open('a3');
    expect(ui.request).toBe('lead');
    expect(tabs()).toEqual([]);
  });

  it('a new request that holds text is a tab of its own repository only, and you are back on it', async () => {
    said = { a4: ['lead'], b4: ['lead'] };
    await open('a4');
    ui.newRequest();
    const draft = ui.request;
    ui.holdDraft(draft, true);
    ui.newRequest(); // an empty one: not worth keeping
    ui.openRequest(draft);

    await open('b4');
    expect(tabs()).toEqual(['lead']);
    expect(ui.draftRequests).toEqual([]);

    await open('a4');
    expect(tabs()).toEqual(['lead', draft]);
    expect(ui.request).toBe(draft);
  });
});
