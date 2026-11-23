// Find in the conversation (⌘F): where a text occurs in what is on screen. The find bar hands
// over the page's text as it is in the DOM, one piece per text node, and gets back where each
// match starts and ends. Kept apart from the DOM so it can be tested.

/** One text node's text. Pieces with the same `block` are one run of text (a paragraph, a code
 *  block, a line): a match may go from one piece to the next only inside a block. */
export type Piece = { text: string; block: number };

/** A match: from `startOffset` in piece `start` to `endOffset` (exclusive) in piece `end`. */
export type Match = { start: number; startOffset: number; end: number; endOffset: number };

/** Lower case, one character for one: the offsets found in the result are the text's own
 *  (a few letters change length when their case changes; those stay as they are). */
export function fold(s: string): string {
  const lower = s.toLowerCase();
  if (lower.length === s.length) return lower;
  let out = '';
  for (const ch of s) {
    const l = ch.toLowerCase();
    out += l.length === ch.length ? l : ch;
  }
  return out;
}

/**
 * Every place `query` occurs in `pieces`, in order, ignoring case. Matches do not overlap.
 * At most `limit` are returned: a single letter in a long conversation matches everywhere.
 */
export function findMatches(pieces: Piece[], query: string, limit = 5000): Match[] {
  const q = fold(query);
  const out: Match[] = [];
  if (!q) return out;
  for (let first = 0; first < pieces.length && out.length < limit; ) {
    // One block: its pieces joined, and where each piece starts in the joined text.
    let last = first;
    const starts: number[] = [];
    let joined = '';
    for (; last < pieces.length && pieces[last].block === pieces[first].block; last++) {
      starts.push(joined.length);
      joined += fold(pieces[last].text);
    }
    /** The piece (as an index into `starts`) that holds the character at `at`. */
    const pieceAt = (at: number, from: number): number => {
      let i = from;
      while (i + 1 < starts.length && starts[i + 1] <= at) i++;
      return i;
    };
    let piece = 0;
    for (let at = joined.indexOf(q); at >= 0 && out.length < limit; at = joined.indexOf(q, at + q.length)) {
      piece = pieceAt(at, piece);
      const endPiece = pieceAt(at + q.length - 1, piece);
      out.push({ start: first + piece, startOffset: at - starts[piece], end: first + endPiece, endOffset: at + q.length - starts[endPiece] });
    }
    first = last;
  }
  return out;
}

/** The match after (`by` 1) or before (-1) match `current`, going round at the ends. */
export function stepMatch(current: number, total: number, by: 1 | -1): number {
  return total ? (current + by + total) % total : 0;
}
