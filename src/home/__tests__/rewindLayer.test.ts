// A restore from "Undo a turn" or Last turn: the toast says what went back, and its Undo restores
// the point the core kept of the files just before.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import { toasts } from '../../lib/state/toasts.svelte';
import { putBack, rewindLayer } from '../rewindLayer.svelte';

afterEach(() => {
  vi.restoreAllMocks();
  for (const t of [...toasts.list]) toasts.dismiss(t.id);
});

describe('putting files back', () => {
  it('says what went back, with an Undo that restores the point kept just before', async () => {
    const core = vi.spyOn(api, 'restoreCheckpoint').mockResolvedValueOnce({ files: ['a.ts', 'b.ts'], undo: 7 }).mockResolvedValueOnce({ files: ['a.ts', 'b.ts'], undo: 8 });
    const before = rewindLayer.restored;
    await putBack('ada', 3, null, 'Wire the toggle');
    expect(core).toHaveBeenCalledWith('ada', 3, null);
    expect(rewindLayer.restored).toBe(before + 1);
    const t = toasts.list.at(-1)!;
    expect(t).toMatchObject({ kind: 'success', title: '2 files put back to before “Wire the toggle”', action: { label: 'Undo' } });
    t.action!.run();
    await vi.waitFor(() => expect(core).toHaveBeenLastCalledWith('ada', 7, null));
    await vi.waitFor(() => expect(toasts.list.at(-1)?.title).toBe('2 files back as they were'));
    expect(rewindLayer.restored).toBe(before + 2);
  });

  it('one file goes back alone', async () => {
    const core = vi.spyOn(api, 'restoreCheckpoint').mockResolvedValue({ files: ['a.ts'], undo: 9 });
    await putBack('ada', 3, 'a.ts', 'Wire the toggle');
    expect(core).toHaveBeenCalledWith('ada', 3, 'a.ts');
    expect(toasts.list.at(-1)?.title).toBe('1 file put back to before “Wire the toggle”');
  });

  it('a refusal of the core changes nothing and is thrown in its words', async () => {
    vi.spyOn(api, 'restoreCheckpoint').mockRejectedValue(new Error('Ada is still working; restore files when its turn ends'));
    const before = rewindLayer.restored;
    await expect(putBack('ada', 3, null, 'Wire the toggle')).rejects.toThrow('Ada is still working; restore files when its turn ends');
    expect(rewindLayer.restored).toBe(before);
    expect(toasts.list).toEqual([]);
  });
});

describe('the list leaves the screen', () => {
  afterEach(() => rewindLayer.close());

  it('when its conversation goes, it closes: back there later, it does not open by itself', () => {
    rewindLayer.open('ada');
    rewindLayer.release('ada');
    expect(rewindLayer.agentId).toBeNull();
  });

  it('a list opened since for another agent stays', () => {
    rewindLayer.open('bob');
    rewindLayer.release('ada');
    expect(rewindLayer.agentId).toBe('bob');
  });
});
