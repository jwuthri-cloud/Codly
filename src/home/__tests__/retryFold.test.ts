// Retry sends your message again: the thread draws the second copy as "Sent again", and the head
// of a 1:1 thread offers the same Retry as the failed row (round 7 review, defect 19).
import { describe, expect, it } from 'vitest';
import type { Outgoing } from '../../lib/state/store.svelte';
import { headRetry, retryNow, sentAgain } from '../retryFold';
import type { Item } from '../spine';
import type { TroubleKind } from '../outcomes';

let ts = 0;
const mine = (key: string, text: string, outgoing: Outgoing | null = null): Item => ({ kind: 'user', key, ts: ++ts, msg: { id: key, thread: 'agent:a', author: 'user', text, attachment: null, ts }, outgoing });
const trouble = (key: string, what: TroubleKind, retry: string | null = 'Fix it'): Item => ({ kind: 'trouble', key, ts: ++ts, agentId: 'a', eventId: ts, what, reason: null, retry });
const says = (key: string): Item => ({ kind: 'say', key, ts: ++ts, agentId: 'a', itemId: null, text: 'Done.', phase: 'final', streaming: false, header: true, card: null, replyTo: null });
const work = (key: string): Item => ({ kind: 'work', key, ts: ++ts, agentId: 'a', segs: [], thoughts: [], thinking: null, helpers: [] });

describe('sentAgain', () => {
  it('is your message that repeats the one before it across a failed or stopped turn', () => {
    expect([...sentAgain([mine('u1', 'Fix it'), work('w1'), trouble('e1', 'failed'), mine('u2', 'Fix  it\n')])]).toEqual(['u2']);
    expect([...sentAgain([mine('u1', 'Fix it'), trouble('e1', 'stopped'), mine('u2', 'Fix it'), trouble('e2', 'failed'), mine('u3', 'Fix it')])]).toEqual(['u2', 'u3']);
  });

  it('is not a message you wrote again yourself, other words, or one after a turn that ended well', () => {
    expect(sentAgain([mine('u1', 'Fix it'), says('s1'), mine('u2', 'Fix it')]).size).toBe(0);
    expect(sentAgain([mine('u1', 'Fix it'), trouble('e1', 'failed'), mine('u2', 'Fix the other one')]).size).toBe(0);
    // An interruption or a step limit: sending again is not what Retry does there.
    expect(sentAgain([mine('u1', 'Fix it'), trouble('e1', 'interrupted', null), mine('u2', 'Fix it')]).size).toBe(0);
    expect(sentAgain([mine('u1', 'Fix it'), trouble('e1', 'limit', null), mine('u2', 'Fix it')]).size).toBe(0);
  });

  it('keeps a message that is on its way or was not sent whole', () => {
    const out: Outgoing = { id: 'o1', thread: 'agent:a', text: 'Fix it', ts: 0, ws: null, error: 'offline' };
    expect(sentAgain([mine('u1', 'Fix it'), trouble('e1', 'failed'), mine('o:o1', 'Fix it', out)]).size).toBe(0);
  });
});

describe('Retry', () => {
  it('is offered by the newest failed row while nothing was said after it and the agent is free', () => {
    const rows = [mine('u1', 'Fix it'), trouble('e1', 'failed'), work('w1')];
    expect(retryNow(rows, 1, false)).toBe('Fix it');
    expect(headRetry(rows, false)).toBe('Fix it');
    expect(headRetry(rows, true)).toBeNull();
    expect(headRetry([...rows, says('s1')], false)).toBeNull();
    expect(retryNow([...rows, mine('u2', 'Fix it')], 1, false)).toBeNull();
  });

  it('the head finds the failed row behind a newer row that offers nothing', () => {
    const rows = [mine('u1', 'Fix it'), trouble('e1', 'failed'), trouble('e2', 'interrupted', null)];
    expect(headRetry(rows, false)).toBe('Fix it');
    expect(headRetry([mine('u1', 'Hi'), trouble('e1', 'problem', null)], false)).toBeNull();
  });
});
