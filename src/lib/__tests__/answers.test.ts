// The closed card of a question and the agent's history show what you answered. The app
// remembers the answers it sent, so they show before the core's record of them comes back
// (that record: approvalSaid.test.ts).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { resolveApproval, resolveApprovals } from '../state/actions';
import { yourAnswers } from '../state/answers';
import { toasts } from '../state/toasts.svelte';

afterEach(() => {
  vi.restoreAllMocks();
  yourAnswers.clear();
  for (const t of toasts.list) toasts.dismiss(t.id);
});

describe('the answer you gave to a question', () => {
  it('is remembered once the core took it, for every request the card answered', async () => {
    const core = vi.spyOn(api, 'resolveApproval').mockResolvedValue();
    await resolveApprovals(['q1', 'q2'], 'allow_once', null, { 'Keep the fallback?': 'Keep it' });
    expect(core).toHaveBeenCalledWith('q1', { choice: 'allow_once', message: null, answers: { 'Keep the fallback?': 'Keep it' } });
    expect([...yourAnswers]).toEqual([['q1', 'Keep it'], ['q2', 'Keep it']]);
  });

  it('is not remembered when the core refused it, and an approval or a skipped question has none', async () => {
    const core = vi.spyOn(api, 'resolveApproval').mockRejectedValue(new Error('the agent that asked is no longer running; the request expired'));
    await resolveApproval('q1', 'allow_once', null, { 'Keep the fallback?': 'Keep it' });
    expect(yourAnswers.size).toBe(0);
    expect(toasts.list.map((t) => t.title)).toEqual(['Couldn’t send your answer']);
    core.mockResolvedValue();
    await resolveApproval('a1', 'allow_once');
    await resolveApproval('q2', 'deny');
    expect(yourAnswers.size).toBe(0);
  });
});
