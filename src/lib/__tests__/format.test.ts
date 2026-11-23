import { describe, expect, it } from 'vitest';
import { article, coins, plural, usd } from '../util/format';

describe('words around a name or a count', () => {
  it('"a" or "an" follows the first sound of the name', () => {
    expect(['OpenCode', 'Codex', 'Claude Code', 'Gemini CLI', 'Amp', 'unknown CLI'].map(article)).toEqual(['an', 'a', 'a', 'a', 'an', 'an']);
    // "u" that sounds like "you" takes "a".
    expect(['user', 'unit', 'utility'].map(article)).toEqual(['a', 'a', 'a']);
  });

  it('one of a thing is singular', () => {
    expect([plural(1, 'file'), plural(2, 'file'), plural(0, 'file')]).toEqual(['1 file', '2 files', '0 files']);
  });
});

describe('coins', () => {
  it('are 🪙, never $ (real API cost)', () => {
    expect(coins(0)).toBe('🪙 0');
    expect(coins(250)).toBe('🪙 250');
    expect(coins(10000)).toBe('🪙 10,000');
    expect(coins(12.6)).toBe('🪙 13');
    expect(coins(-40)).toBe('🪙 -40');
    expect(usd(250).startsWith('$')).toBe(true);
  });
});
