import { describe, expect, it } from 'vitest';
import { afterSearch, messageRow } from '../findSeek';

describe('the find bar opened on a message found by the palette', () => {
  const seek = { query: 'login', key: 'c:m7' };

  it('goes to the match in the picked message, not to the thread’s first', () => {
    expect(afterSearch(seek, 'login', 2, 1)).toEqual({ seek: null, go: 1 });
  });

  it('with no match in that message (not loaded), goes to the first match in view', () => {
    expect(afterSearch(seek, 'login', 2, -1)).toEqual({ seek: null, go: 'view' });
  });

  it('keeps seeking while the conversation shows no match yet', () => {
    expect(afterSearch(seek, 'login', 0, -1)).toEqual({ seek, go: null });
  });

  it('stops seeking when the text changes: a later message never moves the thread by itself', () => {
    expect(afterSearch(seek, 'logout', 0, -1)).toEqual({ seek: null, go: null });
    // The message with the new text streams in: no jump.
    expect(afterSearch(null, 'logout', 1, -1)).toEqual({ seek: null, go: null });
  });

  it('finds the row of the message as the conversation keys it', () => {
    expect(messageRow('m7', new Map())).toBe('c:m7');
    // A message you sent keeps the row it had while it was on its way.
    expect(messageRow('m7', new Map([['m7', 'o:out-3']]))).toBe('o:out-3');
  });
});
