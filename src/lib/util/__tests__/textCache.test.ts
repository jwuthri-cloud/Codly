import { describe, expect, it } from 'vitest';
import { textCache } from '../textCache';

describe('textCache', () => {
  it('makes a text once and gives the same value again', () => {
    const cached = textCache(10, 1000);
    let made = 0;
    const upper = (s: string) => (made++, s.toUpperCase());
    expect(cached('abc', upper)).toBe('ABC');
    expect(cached('abc', upper)).toBe('ABC');
    expect(made).toBe(1);
  });

  it('keeps the pairs used last, up to its number of pairs', () => {
    const cached = textCache(2, 1000);
    const made: string[] = [];
    const make = (s: string) => (made.push(s), s + '!');
    cached('a', make);
    cached('b', make);
    cached('a', make); // used again: "b" is now the oldest
    cached('c', make); // "b" leaves
    cached('a', make);
    cached('b', make);
    expect(made).toEqual(['a', 'b', 'c', 'b']);
  });

  it('keeps within its size, and never keeps a pair larger than that', () => {
    const cached = textCache(100, 20);
    const made: string[] = [];
    const make = (s: string) => (made.push(s), s.repeat(2));
    cached('aaaa', make); // 12 characters with its value
    cached('bbbb', make); // 24 in all: "aaaa" leaves
    cached('bbbb', make);
    cached('aaaa', make);
    expect(made).toEqual(['aaaa', 'bbbb', 'aaaa']);
    const big = 'x'.repeat(10); // 30 with its value: more than the cache holds
    expect(cached(big, make)).toBe(big.repeat(2));
    cached(big, make);
    cached('aaaa', make); // still kept: the large pair pushed nothing out
    expect(made).toEqual(['aaaa', 'bbbb', 'aaaa', big, big]);
  });
});
