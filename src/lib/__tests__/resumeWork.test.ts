import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { resumeWork } from '../state/resumeWork.svelte';
import { toasts } from '../state/toasts.svelte';
import { asksNow, resumeQuestion } from '../util/resumeWork';

describe('the launch question, in words', () => {
  it('says how many tasks were running', () => {
    expect(resumeQuestion({ tasks: 1 })).toBe('1 task was running when Codly closed. Resume it?');
    expect(resumeQuestion({ tasks: 3 })).toBe('3 tasks were running when Codly closed. Resume them?');
  });

  it('says so when work only waited to go on', () => {
    expect(resumeQuestion({ tasks: 0 })).toBe('Work was waiting to go on when Codly closed. Resume it?');
  });

  it('is on screen with Ask, until you say Not now', () => {
    expect(asksNow('ask', false)).toBe(true);
    expect(asksNow('ask', true)).toBe(false);
    // Never: no question, only the Resume work button. (Always holds nothing.)
    expect(asksNow('never', false)).toBe(false);
  });
});

describe('the launch question, on screen', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    resumeWork.offer = null;
    resumeWork.later = false;
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  it('shows what the core holds, and is gone once you resume', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 2 }).mockResolvedValueOnce(null);
    await resumeWork.check();
    expect(resumeWork.offer).toEqual({ tasks: 2 });
    expect(core).toHaveBeenLastCalledWith();

    await resumeWork.resume();
    expect(core).toHaveBeenLastCalledWith(true);
    expect(resumeWork.offer).toBeNull();
    expect(resumeWork.busy).toBe(false);
  });

  it('keeps the work on hold after Not now', async () => {
    vi.spyOn(api, 'resumeWork').mockResolvedValue({ tasks: 1 });
    await resumeWork.check();
    resumeWork.notNow();
    // Nothing was sent to the core: the Resume work button stays.
    expect(api.resumeWork).toHaveBeenCalledTimes(1);
    expect(resumeWork.offer).toEqual({ tasks: 1 });
    expect(resumeWork.later).toBe(true);
  });

  it('keeps the button when the resume fails, and says why', async () => {
    vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 1 }).mockRejectedValueOnce(new Error('the store is locked'));
    await resumeWork.check();
    await resumeWork.resume();
    expect(resumeWork.offer).toEqual({ tasks: 1 });
    expect(resumeWork.busy).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t resume the work', 'the store is locked']]);
  });

  it('offers the read again when it fails', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockRejectedValueOnce(new Error('no answer')).mockResolvedValueOnce({ tasks: 4 });
    await resumeWork.check();
    expect(resumeWork.offer).toBeNull();
    const [failed] = toasts.list;
    expect(failed.title).toBe('Couldn’t check for work to resume');
    failed.action!.run();
    await vi.waitFor(() => expect(resumeWork.offer).toEqual({ tasks: 4 }));
    expect(core).toHaveBeenCalledTimes(2);
  });
});
