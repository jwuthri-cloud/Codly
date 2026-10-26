import { describe, expect, it } from 'vitest';
import { basename, clean, commandHead, truncate } from '../text';

describe('sanitization (design-world §8.6)', () => {
  it('strips ANSI escapes and control characters', () => {
    expect(clean('\u001b[31mred\u001b[0m text\u0007')).toBe('red text');
    expect(clean('a\u0000b\u009bc')).toBe('a b c');
  });

  it('strips bidi overrides, zero-width characters and line separators', () => {
    expect(clean('safe‮txt.exe')).toBe('safetxt.exe');
    expect(clean('ze​ro⁠wi﻿dth')).toBe('zerowidth');
    expect(clean('one two three')).toBe('one two three');
    expect(clean('lone \uD800 surrogate')).toBe('lone surrogate');
  });

  it('collapses whitespace and truncates by grapheme, never splitting an emoji', () => {
    expect(clean('  many\n\n  spaces\t here ')).toBe('many spaces here');
    const family = '👩‍👩‍👧‍👦';
    const t = truncate(`ab${family}cdef`, 4);
    expect(t).toBe(`ab${family}…`);
    expect(clean('x'.repeat(50), 10)).toHaveLength(10);
  });

  it('shows basenames and the first tokens of commands', () => {
    expect(basename('/Users/me/repo/src/app.ts')).toBe('app.ts');
    expect(commandHead('cd web && NODE_ENV=test pnpm test -- --run auth')).toBe('pnpm test …');
    expect(commandHead('ls')).toBe('ls');
  });
});
