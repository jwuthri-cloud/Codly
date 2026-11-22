import { describe, expect, it } from 'vitest';
import { findMatches, fold, stepMatch, type Piece } from '../find';

/** Text nodes of one paragraph each. */
const paragraphs = (...texts: string[]): Piece[] => texts.map((text, block) => ({ text, block }));
/** Text nodes that are all in the same paragraph (bold, code and links split a paragraph's text). */
const inline = (...texts: string[]): Piece[] => texts.map((text) => ({ text, block: 0 }));
/** The text a match covers, read back from the pieces. */
function read(pieces: Piece[], m: { start: number; startOffset: number; end: number; endOffset: number }): string {
  if (m.start === m.end) return pieces[m.start].text.slice(m.startOffset, m.endOffset);
  const middle = pieces.slice(m.start + 1, m.end).map((p) => p.text).join('');
  return pieces[m.start].text.slice(m.startOffset) + middle + pieces[m.end].text.slice(0, m.endOffset);
}

describe('findMatches', () => {
  it('finds every occurrence, in order, with where each starts and ends', () => {
    const pieces = paragraphs('Run the migration.', 'The migration ran; migration 0042 is in.');
    expect(findMatches(pieces, 'migration')).toEqual([
      { start: 0, startOffset: 8, end: 0, endOffset: 17 },
      { start: 1, startOffset: 4, end: 1, endOffset: 13 },
      { start: 1, startOffset: 19, end: 1, endOffset: 28 },
    ]);
  });

  it('ignores case, and gives back the text as it is written', () => {
    const pieces = paragraphs('Fix the Login page', 'LOGIN works');
    const found = findMatches(pieces, 'login');
    expect(found.map((m) => read(pieces, m))).toEqual(['Login', 'LOGIN']);
    expect(findMatches(pieces, 'LoGiN')).toEqual(found);
  });

  it('a match goes across bold, code and links inside one paragraph', () => {
    // "Ran `pnpm test` twice" is three text nodes.
    const pieces = inline('Ran ', 'pnpm test', ' twice');
    const [m] = findMatches(pieces, 'an pnpm test tw');
    expect(m).toEqual({ start: 0, startOffset: 1, end: 2, endOffset: 3 });
    expect(read(pieces, m)).toBe('an pnpm test tw');
    // One that ends exactly where a node ends stays in that node.
    expect(findMatches(pieces, 'ran ')).toEqual([{ start: 0, startOffset: 0, end: 0, endOffset: 4 }]);
    expect(findMatches(pieces, 'pnpm test')).toEqual([{ start: 1, startOffset: 0, end: 1, endOffset: 9 }]);
  });

  it('a match never goes from one paragraph into the next', () => {
    expect(findMatches(paragraphs('first line', 'second line'), 'linesecond')).toEqual([]);
    expect(findMatches(paragraphs('first line', 'second line'), 'line')).toHaveLength(2);
  });

  it('matches do not overlap', () => {
    expect(findMatches(paragraphs('aaaa'), 'aa').map((m) => m.startOffset)).toEqual([0, 2]);
  });

  it('an empty search matches nothing; spaces are text like any other', () => {
    expect(findMatches(paragraphs('some text'), '')).toEqual([]);
    expect(findMatches(paragraphs('some text'), ' ')).toHaveLength(1);
  });

  it('stops at the limit: one letter in a long conversation matches everywhere', () => {
    const pieces = paragraphs('eeee', 'eeee', 'eeee');
    expect(findMatches(pieces, 'e', 5)).toHaveLength(5);
    expect(findMatches(pieces, 'e')).toHaveLength(12);
  });

  it('empty text nodes between two others change nothing', () => {
    const pieces = inline('ab', '', 'cd');
    expect(findMatches(pieces, 'bc')).toEqual([{ start: 0, startOffset: 1, end: 2, endOffset: 1 }]);
    expect(findMatches(pieces, 'cd')).toEqual([{ start: 2, startOffset: 0, end: 2, endOffset: 2 }]);
  });

  it('offsets stay those of the text when a letter changes length with its case', () => {
    // "İ" lower-cases to two characters; the match after it must not shift.
    const pieces = paragraphs('İstanbul LOGIN');
    const [m] = findMatches(pieces, 'login');
    expect(read(pieces, m)).toBe('LOGIN');
    expect(fold('İstanbul').length).toBe('İstanbul'.length);
    expect(fold('ABC déjà')).toBe('abc déjà');
  });
});

describe('stepMatch', () => {
  it('goes to the next and the previous match, round at the ends', () => {
    expect(stepMatch(0, 3, 1)).toBe(1);
    expect(stepMatch(2, 3, 1)).toBe(0);
    expect(stepMatch(0, 3, -1)).toBe(2);
    expect(stepMatch(1, 3, -1)).toBe(0);
  });

  it('with no match there is nowhere to go', () => {
    expect(stepMatch(0, 0, 1)).toBe(0);
    expect(stepMatch(0, 0, -1)).toBe(0);
  });
});
