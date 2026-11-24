// Drafts across a reload or a restart: the copy in the browser's storage. A restart is a fresh
// import of the module (its drafts of this session are gone, the storage stays).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Att } from '../drafts';

/** A storage that keeps its items in a map, as the browser's does. */
function fakeStorage() {
  const items = new Map<string, string>();
  return {
    items,
    get length() {
      return items.size;
    },
    key: (i: number) => [...items.keys()][i] ?? null,
    getItem: (k: string) => items.get(k) ?? null,
    setItem: (k: string, v: string) => void items.set(k, v),
    removeItem: (k: string) => void items.delete(k),
  };
}

let storage: ReturnType<typeof fakeStorage>;
/** The module as a fresh start of the app finds it. */
async function restart() {
  vi.resetModules();
  return import('../drafts');
}
const att = (name: string, path: string | null): Att => ({ id: 0, name, path, url: null, failed: false });

beforeEach(() => {
  storage = fakeStorage();
  vi.stubGlobal('localStorage', storage);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('a draft across a restart', () => {
  it('keeps its text and its saved files, per office and thread', async () => {
    const d = await restart();
    d.saveDraft(d.draftSlot('office-a', 'lead'), 'half a prompt', [att('shot.png', '/repo/.codly/attachments/a1/shot.png')]);
    expect([...storage.items.keys()]).toEqual(['codly.draft.office-a.lead']);
    const after = await restart();
    const back = after.loadDraft(after.draftSlot('office-a', 'lead'));
    expect(back.text).toBe('half a prompt');
    expect(back.files.map((f) => [f.name, f.path, f.url, f.failed])).toEqual([['shot.png', '/repo/.codly/attachments/a1/shot.png', null, false]]);
    expect(after.loadDraft(after.draftSlot('office-b', 'lead'))).toEqual({ text: '', files: [] });
  });

  it('drops a file still on its way, and one that failed, with no word', async () => {
    const d = await restart();
    d.saveDraft(d.draftSlot('o', 'lead'), 'look', [att('big.mov', null), { ...att('bad.png', null), failed: true }]);
    const after = await restart();
    expect(after.loadDraft(after.draftSlot('o', 'lead'))).toEqual({ text: 'look', files: [] });
  });

  it('is removed when the message is sent or the text emptied', async () => {
    const d = await restart();
    const slot = d.draftSlot('o', 'agent:ada');
    d.saveDraft(slot, 'fix it', []);
    d.saveDraft(slot, '', []);
    expect(storage.items.size).toBe(0);
    const after = await restart();
    expect(after.loadDraft(slot)).toEqual({ text: '', files: [] });
  });

  it('a text over the limit is not kept, and an older copy does not come back in its place', async () => {
    const d = await restart();
    const slot = d.draftSlot('o', 'lead');
    d.saveDraft(slot, 'short', []);
    d.saveDraft(slot, 'x'.repeat(d.DRAFT_MOST_CHARS + 1), []);
    expect(storage.items.size).toBe(0);
    // It still lives while the app is open.
    expect(d.loadDraft(slot).text.length).toBe(d.DRAFT_MOST_CHARS + 1);
    d.saveDraft(slot, 'x'.repeat(d.DRAFT_MOST_CHARS), []);
    expect(storage.items.size).toBe(1);
  });

  it('keeps the newest 50 drafts: the oldest goes first', async () => {
    vi.useFakeTimers();
    const d = await restart();
    for (let i = 0; i <= d.DRAFT_MOST; i++) {
      vi.setSystemTime(1000 + i);
      d.saveDraft(d.draftSlot('o', `lead:${i}`), `draft ${i}`, []);
    }
    expect(storage.items.size).toBe(d.DRAFT_MOST);
    expect(storage.items.has('codly.draft.o.lead:0')).toBe(false);
    expect(storage.items.has(`codly.draft.o.lead:${d.DRAFT_MOST}`)).toBe(true);
  });

  it('a copy it cannot read is dropped', async () => {
    storage.setItem('codly.draft.o.lead', '{not json');
    storage.setItem('codly.draft.o.lead:2', JSON.stringify({ text: 3 }));
    const d = await restart();
    expect(d.loadDraft(d.draftSlot('o', 'lead'))).toEqual({ text: '', files: [] });
    expect(d.loadDraft(d.draftSlot('o', 'lead:2'))).toEqual({ text: '', files: [] });
    expect(storage.items.has('codly.draft.o.lead:2')).toBe(false);
  });

  it('a storage that throws leaves the draft of this session as it was', async () => {
    const refuse = () => {
      throw new Error('blocked');
    };
    vi.stubGlobal('localStorage', { length: 0, key: refuse, getItem: refuse, setItem: refuse, removeItem: refuse });
    const d = await restart();
    const slot = d.draftSlot('o', 'lead');
    expect(d.loadDraft(slot)).toEqual({ text: '', files: [] });
    d.saveDraft(slot, 'still here', []);
    expect(d.loadDraft(slot).text).toBe('still here');
  });

  it('with no office open, nothing is kept across a restart', async () => {
    const d = await restart();
    d.saveDraft(d.draftSlot(null, 'lead'), 'no repo yet', []);
    expect(storage.items.size).toBe(0);
  });
});
