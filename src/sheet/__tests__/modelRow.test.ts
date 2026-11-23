// The "Model" row of the agent sheet's Controls. Before, the model was only read-only text in
// the sheet, and "Change model…" was hidden in its menu.
import { describe, expect, it } from 'vitest';
import { modelChangeOff, modelLine } from '../modelRow';

describe('the model among the controls', () => {
  it('names the model and the thinking effort picked with it', () => {
    expect(modelLine('Opus', 'opus:high')).toBe('Opus · High effort');
    expect(modelLine('GPT-5.5 Codex', 'gpt-5.5-codex:xhigh')).toBe('GPT-5.5 Codex · Extra high effort');
    // No effort picked: the model alone (the CLI's own default effort applies).
    expect(modelLine('Sonnet', 'sonnet')).toBe('Sonnet');
    expect(modelLine('default model', null)).toBe('default model');
  });

  it('“Change…” runs for a 1:1 agent, as the menu entry does; for another it says why not', () => {
    expect(modelChangeOff({ role: 'solo' })).toBeNull();
    expect(modelChangeOff({ role: 'lead' })).toBe('Only a 1:1 agent’s model can change here.');
    expect(modelChangeOff({ role: 'worker' })).toBe('Only a 1:1 agent’s model can change here.');
  });
});
