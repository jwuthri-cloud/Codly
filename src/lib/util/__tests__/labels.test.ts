import { describe, expect, it } from 'vitest';
import { prettyModel, subagentMark, subagentTag } from '../labels';

describe('subagentMark', () => {
  it('marks a tool call a subagent made, and only that', () => {
    expect(subagentMark({ command: 'ls', subagent: { thread: 't1', name: 'Ada (explorer)' } })).toBe(' · helper Ada (explorer)');
    expect(subagentMark({ command: 'ls', subagent: { thread: 't1', name: null } })).toBe(' · helper');
    expect(subagentMark({ command: 'ls' })).toBe('');
    expect(subagentMark({ subagent: 'Explore' })).toBe('');
    expect(subagentMark(null)).toBe('');
    expect(subagentMark(['subagent'])).toBe('');
  });

  it('is the same words without the dot, for a place that shows them apart', () => {
    expect(subagentTag({ command: 'ls', subagent: { thread: 't1', name: 'Ada (explorer)' } })).toBe('helper Ada (explorer)');
    expect(subagentTag({ command: 'ls', subagent: { thread: 't1', name: null } })).toBe('helper');
    expect(subagentTag({ command: 'ls' })).toBe('');
  });
});

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
