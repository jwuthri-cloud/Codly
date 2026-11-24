// The closed card of a question and the agent's history show what you answered. The app
// remembers the answers it sent, so they show before the core's record of them comes back
// (that record: approvalSaid.test.ts).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Approval } from '../gen/Approval';
import { resolveApproval, resolveApprovals } from '../state/actions';
import { yourAnswers } from '../state/answers';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';

/** A request that waits, of the kind `subject` says. */
const waiting = (id: string, subject: Approval['subject']) =>
  ({ id, agent_id: 'eli', run_id: 'r', task_id: null, call_id: null, subject, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 0 }) as Approval;
const question: Approval['subject'] = { kind: 'question', questions: [{ question: 'Keep the fallback?' }] };

afterEach(() => {
  vi.restoreAllMocks();
  yourAnswers.clear();
  store.approvals = [];
  for (const t of toasts.list) toasts.dismiss(t.id);
});

describe('the answer you gave to a question', () => {
  it('is remembered once the core took it, for every request the card answered', async () => {
    store.approvals = [waiting('q1', question), waiting('q2', question)];
    const core = vi.spyOn(api, 'resolveApproval').mockResolvedValue();
    expect(await resolveApprovals(['q1', 'q2'], 'allow_once', null, { 'Keep the fallback?': 'Keep it' })).toBe(true);
    expect(core).toHaveBeenCalledWith('q1', { choice: 'allow_once', message: null, answers: { 'Keep the fallback?': 'Keep it' } });
    expect([...yourAnswers]).toEqual([['q1', 'Keep it'], ['q2', 'Keep it']]);
  });

  it('is not remembered when the core refused it, and an approval or a skipped question has none', async () => {
    store.approvals = [waiting('q1', question), waiting('q2', question), waiting('a1', { kind: 'command', command: 'ls', cwd: null })];
    const core = vi.spyOn(api, 'resolveApproval').mockRejectedValue(new Error('the agent that asked is no longer running; the request expired'));
    expect(await resolveApproval('q1', 'allow_once', null, { 'Keep the fallback?': 'Keep it' })).toBe(false);
    expect(yourAnswers.size).toBe(0);
    expect(toasts.list.map((t) => t.title)).toEqual(['Couldn’t send your answer']);
    core.mockResolvedValue();
    await resolveApproval('a1', 'allow_once');
    await resolveApproval('q2', 'deny');
    expect(yourAnswers.size).toBe(0);
  });
});
