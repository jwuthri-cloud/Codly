import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { MergeState } from '../gen/MergeState';
import type { PrSource } from '../gen/PrSource';
import { mergeAgent, mergePlan } from '../state/actions';
import { fileList, loadMergeState, mergeByHand, mergeKey, mergeUi, resolveMergeConflict } from '../state/mergeState.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';

const plan: PrSource = { kind: 'plan', id: 'p1' };
const clean: MergeState = { target: 'main', branch: 'codly/dark-mode-ab12/integration', repo: '/Users/me/web', conflicts: [], resolving: null, failed_checks: [] };
const conflict: MergeState = { ...clean, conflicts: ['src/theme.ts', 'README.md'] };
const said = () => toasts.list.map((t) => [t.kind, t.title, t.body]);

describe('a merge that cannot go straight in', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    mergeUi.states = {};
    store.planMerges = {};
  });

  it('I resolve it myself: the exact branch and commands, quoted where the shell needs it', () => {
    expect(mergeByHand(conflict)).toBe(
      [
        'cd /Users/me/web',
        'git merge codly/dark-mode-ab12/integration',
        '# fix the conflicts in the files git names, then:',
        'git add src/theme.ts README.md',
        'git commit',
      ].join('\n'),
    );
    const odd = mergeByHand({ repo: '/Users/me/my web', branch: 'codly/x/integration', conflicts: ["it's.md", 'app/[id]/page.tsx'] });
    expect(odd).toContain("cd '/Users/me/my web'");
    expect(odd).toContain("git add 'it'\\''s.md' 'app/[id]/page.tsx'");
    expect(fileList(['a', 'b', 'c'])).toBe('a, b, c');
    expect(fileList(['a', 'b', 'c', 'd', 'e'])).toBe('a, b, c and 2 more');
  });

  it('the state is what the core last said; one that cannot be read shows nothing', async () => {
    const core = vi.spyOn(api, 'mergeState').mockResolvedValueOnce(conflict).mockRejectedValueOnce(new Error('your checkout is on a detached HEAD'));
    expect(await loadMergeState(plan)).toEqual(conflict);
    expect(mergeUi.states[mergeKey(plan)]).toEqual(conflict);
    expect(await loadMergeState(plan)).toBeNull();
    expect(mergeUi.states[mergeKey(plan)]).toBeUndefined();
    expect(core).toHaveBeenCalledTimes(2);
    expect(said()).toEqual([]);
  });

  it('a plan that conflicts is not sent to the Lead: an agent can resolve it, from the toast', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(null);
    vi.spyOn(api, 'mergePlan').mockRejectedValue(new Error('merging into main conflicts in src/theme.ts, README.md'));
    vi.spyOn(api, 'mergeState').mockResolvedValue(conflict);
    const chat = vi.spyOn(api, 'sendChat');
    const resolve = vi.spyOn(api, 'resolveMergeConflict').mockResolvedValue({ ...conflict, resolving: 't9' });
    await mergePlan('p1');
    expect(chat).not.toHaveBeenCalled();
    const stop = toasts.list.find((t) => t.title === 'Couldn’t merge into main');
    expect(stop?.kind).toBe('error');
    expect(stop?.body).toContain('main and this plan both changed src/theme.ts, README.md. main is unchanged.');
    expect(stop?.action?.label).toBe('Have an agent resolve it');
    expect(mergeUi.states[mergeKey(plan)]).toEqual(conflict);

    stop!.action!.run();
    await vi.waitFor(() => expect(toasts.list.some((t) => t.title === 'An agent is resolving the conflict')).toBe(true));
    expect(resolve).toHaveBeenCalledWith(plan);
    expect(mergeUi.states[mergeKey(plan)]?.resolving).toBe('t9');
    // The task that resolves it is one click away.
    expect(toasts.list.find((t) => t.title === 'An agent is resolving the conflict')?.action?.label).toBe('Open the task');
  });

  it('checks that failed on the merge result stop the merge until you choose Merge anyway', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(null);
    const merge = vi
      .spyOn(api, 'mergePlan')
      .mockRejectedValueOnce(new Error('with the plan merged into main, `npm test` failed, so nothing was merged and main is unchanged'))
      .mockResolvedValueOnce('abc1234def');
    vi.spyOn(api, 'mergeState').mockResolvedValue({ ...clean, failed_checks: ['npm test'] });
    await mergePlan('p1');
    expect(merge).toHaveBeenLastCalledWith('p1', null, false);
    const stop = toasts.list.find((t) => t.title === 'Not merged into main');
    expect(stop?.body).toContain('`npm test` failed');
    expect(stop?.action?.label).toBe('Merge anyway');

    stop!.action!.run();
    await vi.waitFor(() => expect(merge).toHaveBeenLastCalledWith('p1', null, true));
    await vi.waitFor(() => expect(store.mergeOf('p1')?.sha).toBe('abc1234def'));
  });

  it('a solo agent resolves its own conflict, in its conversation', async () => {
    const agent: PrSource = { kind: 'agent', id: 'a1' };
    vi.spyOn(api, 'mergeAgent').mockRejectedValue(new Error('merging into main conflicts in notes.md'));
    vi.spyOn(api, 'mergeState').mockResolvedValue({ ...clean, branch: 'codly/solo/ada-1f2e', conflicts: ['notes.md'] });
    expect(await mergeAgent('a1')).toBe(false);
    const stop = toasts.list.find((t) => t.kind === 'error');
    expect([stop?.title, stop?.body, stop?.action?.label]).toEqual(['Couldn’t merge into main', 'main and the agent both changed notes.md. main is unchanged.', 'Have the agent resolve it']);

    // Started: the agent works on it. Not started: the toast says why, and nothing else changed.
    const core = vi
      .spyOn(api, 'resolveMergeConflict')
      .mockResolvedValueOnce({ ...clean, branch: 'codly/solo/ada-1f2e', conflicts: ['notes.md'] })
      .mockRejectedValueOnce(new Error('Ada is still working; ask again when its turn ends'));
    expect(await resolveMergeConflict(agent, 'Ada')).toBe(true);
    expect(core).toHaveBeenCalledWith(agent);
    expect(said().at(-1)).toEqual(['success', 'An agent is resolving the conflict', 'Ada resolves it in its conversation. Merge again when its turn ends.']);
    expect(await resolveMergeConflict(agent, 'Ada')).toBe(false);
    expect(said().at(-1)).toEqual(['error', 'Couldn’t have an agent resolve it', 'Ada is still working; ask again when its turn ends']);
  });
});
