import { describe, expect, it } from 'vitest';
import { article, coins, dur, plural, usd } from '../util/format';

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

describe('a duration', () => {
  it('has one form per length: tenths under a second, then whole seconds ("1 s", never "1.0 s")', () => {
    expect([0, 450, 949, 950, 999, 1000, 1400, 59_999].map(dur)).toEqual(['0.0 s', '0.5 s', '0.9 s', '1 s', '1 s', '1 s', '1 s', '59 s']);
    expect([dur(61_000), dur(null)]).toEqual(['1m 01s', '–']);
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
