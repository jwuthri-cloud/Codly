import { describe, expect, it } from 'vitest';
import { findModels } from '../team';

describe('findModels', () => {
  const list = ['anthropic/claude-sonnet-4', 'anthropic/claude-opus-4', 'openai/gpt-5', 'google/gemini-2.5-pro'];

  it('keeps the first n with no search', () => {
    expect(findModels(list, '', 2)).toEqual({ shown: list.slice(0, 2), total: 4 });
  });

  it('matches every word, in any case', () => {
    expect(findModels(list, 'Claude SON', 8)).toEqual({ shown: ['anthropic/claude-sonnet-4'], total: 1 });
    expect(findModels(list, 'anthropic', 1)).toEqual({ shown: ['anthropic/claude-sonnet-4'], total: 2 });
  });
});
