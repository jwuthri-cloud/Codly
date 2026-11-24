import { describe, expect, it } from 'vitest';
import { afterSearch, messageRows, pickRow } from '../findSeek';

describe('the find bar opened on a message found by the palette', () => {
  const seek = { query: 'login', keys: ['c:m7'] };

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
    expect(pickRow(messageRows('m7', null, new Map()), ['c:m1', 'c:m7', 'c:m9'])).toBe('c:m7');
    // A message you sent keeps the row it had while it was on its way.
    expect(pickRow(messageRows('m7', null, new Map([['m7', 'o:out-3']])), ['c:m1', 'o:out-3'])).toBe('o:out-3');
  });

  it('finds an agent’s reply by the message it shows', () => {
    // The row of the conversation's own agent is keyed by the message it said, not by the chat id.
    const keys = messageRows('m7', 'msg_42', new Map());
    expect(pickRow(keys, ['c:m1', 'say:msg_40', 'say:msg_42', 'c:m9'])).toBe('say:msg_42');
    // Not matched to a held message: the row keeps the chat id.
    expect(pickRow(keys, ['c:m7', 'say:msg_42'])).toBe('c:m7');
    // An id a CLI gave again in a later session: the later row (`#2`) is that new message, not this one.
    expect(pickRow(keys, ['say:msg_42', 'c:m8', 'say:msg_42#2'])).toBe('say:msg_42');
    expect(pickRow(keys, ['say:msg_421', 'say:msg_42#2', 'c:m1'])).toBeNull();
  });
});
