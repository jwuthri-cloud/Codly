// Kill of an agent that is in your terminal. The fourth browser review (defect 10): a Kill in
// that state ended nothing, and "In your terminal" stayed in the header, the pill and the sheet.
// Codly has no process for such an agent: the core stopped its own before the hand-over, and
// the mark stays until you give it back (orchestrator/takeover.rs).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { stopAgent } from '../state/actions';
import { killRefusal } from '../state/killRule';
import { takeovers } from '../state/takeover.svelte';
import { toasts } from '../state/toasts.svelte';

const THERE = 'Ada is in your terminal: Codly runs nothing for it. Give it back to work with it here.';

afterEach(() => {
  vi.restoreAllMocks();
  takeovers.set('a1', { takeover: null, blocked: null });
  for (const t of toasts.list) toasts.dismiss(t.id);
});

describe('Kill of an agent in your terminal', () => {
  it('is off, with the reason and what to do', () => {
    expect(killRefusal('Ada', true)).toBe(THERE);
    expect(killRefusal('Ada', false)).toBeNull();
  });

  it('stopAgent refuses with that sentence and asks the core nothing', async () => {
    takeovers.set('a1', { takeover: { agent_id: 'a1', cwd: '/wt', command: 'claude --resume s1', copy_command: 'claude --resume s1', since: 1 }, blocked: null });
    const stop = vi.spyOn(api, 'stopAgent');
    await stopAgent('a1');
    expect(stop).not.toHaveBeenCalled();
    // (the store does not know the agent here: its name is "This agent")
    expect(toasts.list.map((t) => t.title)).toEqual([killRefusal('This agent', true)]);
  });
});
