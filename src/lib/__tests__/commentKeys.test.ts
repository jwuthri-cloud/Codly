// The rows of your review comments each hold a "Remove" button. A row is its comment's own:
// when the comment is removed the row goes with it, and the button that had the focus is not
// handed to the next comment (a second press would remove a comment you did not mean).
import { describe, expect, it } from 'vitest';
import { commentKeys, withComment, type ReviewComment } from '../util/review';

const on = (path: string, line: number | null, quote: string | null, side: 'old' | 'new' | null = line == null ? null : 'new'): ReviewComment => ({ path, line, side, quote, text: 'Rename this.' });

describe('commentKeys', () => {
  it('gives each comment a key no other has, also with the same place and the same quoted line', () => {
    const list = [
      on('src/a.ts', 3, '+const a = 1;'),
      on('src/b.ts', 3, '+const a = 1;'), // the same line and words in another file
      on('src/a.ts', 3, '-const a = 1;', 'old'), // the removed line with the same number
      on('src/a.ts', 3, '+const a = 2;'), // the same line as it read before the worker changed it
      on('src/a.ts', null, null), // the whole file
    ];
    const keys = commentKeys(list);
    expect(keys).toHaveLength(list.length);
    expect(new Set(keys).size).toBe(list.length);
  });

  it('keeps the parts of a key apart, whatever a path or a quoted line holds', () => {
    // Joined with no mark between them, these two would read the same.
    const keys = commentKeys([on('a new:1', 12, '+x'), on('a', 1, ' new:12+x')]);
    expect(keys[0]).not.toBe(keys[1]);
  });

  it('gives two comments that are equal in all of it different keys', () => {
    // The review state keeps one comment per place; a list that still holds two must not stop the view.
    const twin = on('src/a.ts', 3, '+const a = 1;');
    const keys = commentKeys([twin, { ...twin }, on('src/a.ts', 4, '+const b = 2;')]);
    expect(new Set(keys).size).toBe(3);
  });

  it('a comment keeps its key from one render to the next, also when the one before it is removed', () => {
    const first = on('src/a.ts', 3, '+const a = 1;');
    const second = on('src/a.ts', 9, '+const b = 2;');
    const third = on('src/c.ts', null, null);
    const list = [first, second, third];
    const before = commentKeys(list);
    expect(commentKeys([...list])).toEqual(before);

    // "Remove" on the first one: the others keep their keys, so the first row is destroyed
    // (with the focus on its button) and the second comment does not move into it.
    const after = commentKeys(withComment(list, first, ''));
    expect(after).toEqual([before[1], before[2]]);
    expect(after).not.toContain(before[0]);
  });
});
