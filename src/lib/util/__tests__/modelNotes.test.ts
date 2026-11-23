import { describe, expect, it } from 'vitest';
import { listTroubleRows, switchNote } from '../modelNotes';

describe('what a switch of model does to the conversation', () => {
  const kept = 'The agent keeps its conversation: its next turn runs on the new model.';
  const resumed = 'The next message resumes the same conversation on the new model.';

  it('a Claude agent with a live process takes the new model with no restart', () => {
    expect(switchNote({ provider: 'claude', run_id: 'r1', status: 'idle' })).toBe(kept);
    expect(switchNote({ provider: 'claude', run_id: 'r1', status: 'working' })).toBe(kept);
  });

  it('a Claude agent that is not running starts again with your next message', () => {
    expect(switchNote({ provider: 'claude', run_id: null, status: 'offline' })).toBe(resumed);
    expect(switchNote({ provider: 'claude', run_id: null, status: 'idle' })).toBe(resumed);
    expect(switchNote({ provider: 'claude', run_id: 'r1', status: 'offline' })).toBe(resumed);
  });

  it('an agent of another CLI is restarted, live or not', () => {
    for (const provider of ['codex', 'gemini', 'opencode', 'cursor', 'copilot'] as const) {
      expect(switchNote({ provider, run_id: 'r1', status: 'idle' }), provider).toBe(resumed);
      expect(switchNote({ provider, run_id: null, status: 'offline' }), provider).toBe(resumed);
    }
  });
});

describe('a model menu whose list is not the CLI’s own', () => {
  it('says that the CLI could not be asked when Codly’s own names are listed', () => {
    expect(listTroubleRows('Codex', true)).toEqual(['Couldn’t ask Codex', 'These are the names Codly knows']);
    expect(listTroubleRows('Claude Code', true)[0]).toBe('Couldn’t ask Claude Code');
  });

  it('says that the models could not be listed only when nothing is listed', () => {
    expect(listTroubleRows('Codex', false)).toEqual(['Couldn’t list the models']);
  });
});
