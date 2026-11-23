import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { MergeState } from '../gen/MergeState';
import type { PrSource } from '../gen/PrSource';
import { mergeAgent, mergePlan } from '../state/actions';
import { behindLine, fileList, loadMergeState, mergeByHand, mergeKey, mergeUi, resolveMergeConflict, stopResolvingMerge } from '../state/mergeState.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';

const plan: PrSource = { kind: 'plan', id: 'p1' };
const clean: MergeState = { target: 'main', branch: 'codly/dark-mode-ab12/integration', repo: '/Users/me/web', conflicts: [], resolving: null, failed_checks: [], checks_not_run: null, behind: 0 };
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
    expect(resolve).toHaveBeenCalledWith(plan, null);
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
    expect(core).toHaveBeenCalledWith(agent, null);
    expect(said().at(-1)).toEqual(['success', 'An agent is resolving the conflict', 'Ada resolves it in its conversation. Merge again when its turn ends.']);
    expect(await resolveMergeConflict(agent, 'Ada')).toBe(false);
    expect(said().at(-1)).toEqual(['error', 'Couldn’t have an agent resolve it', 'Ada is still working; ask again when its turn ends']);
  });

  it('a plan that started from another branch is resolved only after you confirm that branch', async () => {
    const warning = { target: 'feature-x', message: 'This plan started from main, and your checkout is on feature-x now.' };
    const warn = vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(warning);
    const resolve = vi.spyOn(api, 'resolveMergeConflict').mockResolvedValue({ ...conflict, target: 'feature-x', resolving: 't9' });
    expect(await resolveMergeConflict(plan)).toBe(false);
    expect(resolve).not.toHaveBeenCalled();
    const ask = toasts.list.find((t) => t.title === 'Bring feature-x into this plan?');
    expect(ask?.kind).toBe('attention');
    expect(ask?.body).toBe(
      'This plan started from main, and your checkout is on feature-x now. To resolve the conflict, an agent brings feature-x into the plan’s branch: from then on the plan builds on feature-x.',
    );
    expect(ask?.action?.label).toBe('Go on');

    // Go on: the branch you confirmed goes with the request, and nothing asks twice.
    ask!.action!.run();
    await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith(plan, 'feature-x'));
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('checks that could not run on the merge result stop the merge until you choose Merge anyway', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(null);
    const why = 'the plan’s checks could not run on the merge result, so nothing was merged and main is unchanged: .codly/setup.sh failed (exit 1)';
    const merge = vi.spyOn(api, 'mergePlan').mockRejectedValueOnce(new Error(why)).mockResolvedValueOnce('abc1234def');
    vi.spyOn(api, 'mergeState').mockResolvedValue({ ...clean, checks_not_run: '.codly/setup.sh failed (exit 1)' });
    const chat = vi.spyOn(api, 'sendChat');
    await mergePlan('p1');
    expect(chat).not.toHaveBeenCalled();
    const stop = toasts.list.find((t) => t.title === 'Not merged into main');
    expect([stop?.body, stop?.action?.label]).toEqual([why, 'Merge anyway']);

    stop!.action!.run();
    await vi.waitFor(() => expect(merge).toHaveBeenLastCalledWith('p1', null, true));
    await vi.waitFor(() => expect(store.mergeOf('p1')?.sha).toBe('abc1234def'));
  });

  it('a solo agent that is on the conflict can be stopped; a merge then says what is left', async () => {
    const agent: PrSource = { kind: 'agent', id: 'a1' };
    const open: MergeState = { ...clean, branch: 'codly/solo/ada-1f2e', conflicts: ['notes.md'], resolving: 'a1' };
    // Merge while it is on it: the core's own words, and no second "have it resolve it".
    const left = 'Ada has not finished resolving a merge: notes.md still has conflict markers. Ask it to finish, then try again.';
    vi.spyOn(api, 'mergeAgent').mockRejectedValue(new Error(left));
    vi.spyOn(api, 'mergeState').mockResolvedValue(open);
    expect(await mergeAgent('a1')).toBe(false);
    const stop = toasts.list.find((t) => t.kind === 'error');
    expect([stop?.body, stop?.action]).toEqual([left, undefined]);

    const core = vi
      .spyOn(api, 'stopResolvingMerge')
      .mockResolvedValueOnce({ ...open, resolving: null })
      .mockRejectedValueOnce(new Error('Ada is not resolving a merge'));
    expect(await stopResolvingMerge('a1', 'Ada')).toBe(true);
    expect(core).toHaveBeenCalledWith('a1');
    expect(mergeUi.states[mergeKey(agent)]?.resolving).toBeNull();
    expect(said().at(-1)).toEqual(['success', 'Stopped resolving', 'Ada’s branch is as it was before the merge. The conflict is still there.']);
    expect(await stopResolvingMerge('a1', 'Ada')).toBe(false);
    expect(said().at(-1)).toEqual(['error', 'Couldn’t stop resolving', 'Ada is not resolving a merge']);
  });

  it('says how far the Codly branch is behind yours, before you merge', () => {
    expect(behindLine({ ...clean, behind: 1 })).toBe('This plan is behind main by 1 commit: they merge without a conflict.');
    expect(behindLine({ ...conflict, behind: 3 })).toBe('This plan is behind main by 3 commits.');
    expect(behindLine({ ...clean, target: 'feature-x', behind: 12 }, 'Ada')).toBe('Ada’s branch is behind feature-x by 12 commits: they merge without a conflict.');
  });

  it('checks that failed on the merge result can go to an agent: a task of the plan fixes them', async () => {
    vi.spyOn(api, 'mergePlanWarning').mockResolvedValue(null);
    vi.spyOn(api, 'mergePlan').mockRejectedValue(new Error('with the plan merged into main, `npm test` failed, so nothing was merged and main is unchanged'));
    vi.spyOn(api, 'mergeState').mockResolvedValue({ ...clean, behind: 2, failed_checks: ['npm test'] });
    await mergePlan('p1');
    // The toast keeps Merge anyway, and says where the other way is.
    const stop = toasts.list.find((t) => t.title === 'Not merged into main');
    expect(stop?.body).toBe('with the plan merged into main, `npm test` failed, so nothing was merged and main is unchanged. An agent can fix it: see the plan’s card.');

    // From the card: your branch is in the plan's branch, nothing conflicts, and a task is on it.
    const resolve = vi.spyOn(api, 'resolveMergeConflict').mockResolvedValue({ ...clean, resolving: 't12' });
    expect(await resolveMergeConflict(plan)).toBe(true);
    expect(resolve).toHaveBeenCalledWith(plan, null);
    const started = toasts.list.find((t) => t.title === 'An agent is fixing the checks');
    expect([started?.kind, started?.body, started?.action?.label]).toEqual([
      'success',
      'A task of the plan makes the checks pass with your branch merged in. When the plan is checked again, you can merge it.',
      'Open the task',
    ]);
    expect(mergeUi.states[mergeKey(plan)]?.resolving).toBe('t12');
  });
});
