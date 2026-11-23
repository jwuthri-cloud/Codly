import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import type { Agent } from '../../lib/gen/Agent';
import type { IdeInfo } from '../../lib/gen/IdeInfo';
import { store } from '../../lib/state/store.svelte';
import { toasts } from '../../lib/state/toasts.svelte';
import { ide } from '../ide.svelte';

describe('the editor shows one agent’s workspace at a time', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    store.agents = [];
  });

  it('keeps unsaved edits when a file of another agent is opened', async () => {
    store.agents = [{ id: 'a1', name: 'Ana' } as Agent, { id: 'a2', name: 'Ben' } as Agent];
    const info = (id: string): IdeInfo => ({ agent_id: id, agent_name: id, root: `/wt/${id}`, project: 'web', branch: null, changed: 0 });
    vi.spyOn(api, 'ideInfo').mockImplementation(async (id) => info(id));
    vi.spyOn(api, 'ideList').mockResolvedValue([]);
    vi.spyOn(api, 'ideChanges').mockResolvedValue([]);
    vi.spyOn(api, 'ideRead').mockImplementation(async (_id, path) => ({ path, content: 'one\n', size: 4 }));
    const write = vi.spyOn(api, 'ideWrite').mockResolvedValue(undefined);

    await ide.openPath('a1', 'src/a.ts');
    ide.edit('file:src/a.ts', 'one\ntwo\n');
    ide.close(); // only hidden: the edit is kept
    expect(ide.dirtyCount).toBe(1);

    // "Open in editor" on a file of Ben's (a plan's diff has files of several agents).
    await ide.openPath('a2', 'src/b.ts');
    expect(ide.agentId).toBe('a1');
    expect(ide.visible).toBe(false);
    expect(ide.tabs.map((t) => t.id)).toEqual(['home', 'file:src/a.ts']);
    expect(ide.dirtyCount).toBe(1);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([
      ['attention', 'Save your edits in Ana’s workspace first', '1 file is not saved there, and the editor shows one workspace at a time. Save it, or close its tab to drop the edits.'],
    ]);
    toasts.list[0].action?.run();
    expect(ide.visible).toBe(true);

    // Another file of the same agent opens beside it.
    await ide.openPath('a1', 'src/c.ts');
    expect(ide.tabs.map((t) => t.id)).toEqual(['home', 'file:src/a.ts', 'file:src/c.ts']);
    expect(ide.dirtyCount).toBe(1);

    // Saved: Ben's file opens, in a clean editor.
    await ide.save('file:src/a.ts');
    expect(write).toHaveBeenCalledWith('a1', 'src/a.ts', 'one\ntwo\n');
    await ide.openPath('a2', 'src/b.ts');
    expect(ide.agentId).toBe('a2');
    expect(ide.tabs.map((t) => t.id)).toEqual(['home', 'file:src/b.ts']);
  });
});
