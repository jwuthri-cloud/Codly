import { describe, expect, it } from 'vitest';
import { prettyModel } from '../labels';

describe('prettyModel', () => {
  it('names models the way people say them', () => {
    expect(prettyModel('claude-sonnet-5-5')).toBe('Sonnet 5.5');
    expect(prettyModel('gpt-6-astra')).toBe('GPT-6 Astra');
    expect(prettyModel('opus[1m]')).toBe('Opus (1M)');
    expect(prettyModel('haiku')).toBe('Haiku');
    expect(prettyModel('fable')).toBe('Fable');
    expect(prettyModel('claude-fable-5-2')).toBe('Fable 5.2');
    expect(prettyModel(null)).toBe('');
  });
});
