import { describe, expect, it } from 'vitest';
import { uniqueKeys } from '../keys';

const id = (r: { id: string }) => r.id;
const rows = (...ids: string[]) => ids.map((id) => ({ id }));

describe('uniqueKeys', () => {
  it('gives each row its own key when no two are the same', () => {
    expect(uniqueKeys(rows('call-1', 'call-2', 'call-3'), id)).toEqual(['call-1', 'call-2', 'call-3']);
    expect(uniqueKeys([], id)).toEqual([]);
  });

  it('adds its place to a later row with a key that is taken', () => {
    expect(uniqueKeys(rows('call-1', 'call-2', 'call-1', 'call-1'), id)).toEqual(['call-1', 'call-2', 'call-1#2', 'call-1#3']);
  });

  it('keeps the keys of the rows before when one more row arrives', () => {
    const before = uniqueKeys(rows('a', 'a', 'b'), id);
    const after = uniqueKeys(rows('a', 'a', 'b', 'a', 'b'), id);
    expect(after.slice(0, before.length)).toEqual(before);
    expect(after).toEqual(['a', 'a#2', 'b', 'a#3', 'b#2']);
  });

  it('never gives a key twice, also when a row’s own key looks like one it made', () => {
    const keys = uniqueKeys(rows('a', 'a#2', 'a', 'a', 'a#2'), id);
    expect(keys).toEqual(['a', 'a#2', 'a#3', 'a#4', 'a#2#2']);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
