// While a card waits for you, the thread holds the oldest one and counts the rows below.
import { describe, expect, it } from 'vitest';
import type { Approval } from '../../lib/gen/Approval';
import type { Item } from '../spine';
import { meets, newBelow, nextMove, oldestWaiting, pillSpot, revealTop, type Rect, type ScrollState } from '../threadScroll';

const card = (key: string, status: string): Item => ({ kind: 'approval', key, ts: 0, approval: { status } as unknown as Approval, twins: [] });
const sys = (key: string): Item => ({ kind: 'system', key, ts: 0, text: 'x', card: null });

describe('oldestWaiting', () => {
  it('is the first card that still waits', () => {
    expect(oldestWaiting([sys('s'), card('a:1', 'approved'), card('a:2', 'pending'), card('a:3', 'pending')])).toBe('a:2');
  });
  it('is null when no card waits', () => {
    expect(oldestWaiting([sys('s'), card('a:1', 'denied')])).toBeNull();
  });
});

describe('revealTop', () => {
  const view = { top: 0, bottom: 300 };
  it('does not move a card that is in view', () => {
    expect(revealTop(500, view, { top: 40, bottom: 250 }, 28)).toBe(500);
  });
  it('brings a card above the view down to its top edge', () => {
    expect(revealTop(500, view, { top: -100, bottom: 150 }, 28)).toBe(372);
  });
  it('brings a card below the view up until its bottom is in', () => {
    expect(revealTop(500, view, { top: 200, bottom: 400 }, 28)).toBe(628);
  });
  it('a card taller than the view shows its top', () => {
    expect(revealTop(500, view, { top: 200, bottom: 700 }, 28)).toBe(672);
  });
});

describe('newBelow', () => {
  it('counts the rows below the view that you had not seen', () => {
    const rows = [{ key: 'a', top: 10 }, { key: 'b', top: 400 }, { key: 'c', top: 280 }, { key: 'd', top: 500 }];
    expect(newBelow(rows, new Set(['a', 'b']), 300)).toBe(1);
    expect(newBelow(rows, new Set(['a']), 250)).toBe(3);
  });
});

describe('newBelow, on a thread you open', () => {
  it('counts nothing: what the thread held then is not new, also below the view', () => {
    const rows = [{ key: 'a', top: 10 }, { key: 'card', top: 120 }, { key: 'b', top: 400 }, { key: 'c', top: 600 }];
    const seen = new Set(rows.map((r) => r.key));
    expect(newBelow(rows, seen, 300)).toBe(0);
    expect(newBelow([...rows, { key: 'd', top: 800 }], seen, 300)).toBe(1);
  });
});

describe('meets', () => {
  const view = { top: 0, bottom: 300 };
  it('is true while some of the box shows', () => {
    expect([meets(view, { top: -50, bottom: 10 }), meets(view, { top: 290, bottom: 400 }), meets(view, { top: 20, bottom: 80 })]).toEqual([true, true, true]);
  });
  it('is false for a box above or below the view', () => {
    expect([meets(view, { top: -200, bottom: 0 }), meets(view, { top: 300, bottom: 400 })]).toEqual([false, false]);
  });
});

describe('nextMove', () => {
  const none: ScrollState = { card: false, fresh: false, held: false, inView: false, atBottom: false, followed: false, added: false };
  const at = (s: Partial<ScrollState>) => nextMove({ ...none, ...s });

  it('with no card, follows the newest line while you are there, else marks what came in', () => {
    expect([at({ atBottom: true }), at({ added: true }), at({})]).toEqual(['bottom', 'mark', 'none']);
  });

  it('a card that becomes the oldest waiting one comes into view from the newest line or the card before it, not while you read further up', () => {
    expect([at({ card: true, fresh: true, atBottom: true }), at({ card: true, fresh: true, held: true }), at({ card: true, fresh: true, added: true })]).toEqual(['reveal', 'reveal', 'mark']);
  });

  it('holds the card while it shows', () => {
    expect(at({ card: true, held: true, inView: true, added: true })).toBe('hold');
  });

  it('a card out of view holds nothing: the thread follows the newest line while you are there', () => {
    // You sent a message (the view went to the end): the Lead's reply grows below.
    expect(at({ card: true, held: true, inView: false, atBottom: true, followed: true, added: true })).toBe('bottom');
    expect(at({ card: true, held: true, inView: false, added: true })).toBe('mark');
  });

  it('when the last card is answered, goes back to the newest line if you were there when the hold began', () => {
    expect(at({ held: true, followed: true })).toBe('bottom');
    expect(at({ held: true, followed: false, added: true })).toBe('mark');
  });
});

describe('where the "new below" pill goes (`pillSpot`)', () => {
  const r = (left: number, top: number, right: number, bottom: number): Rect => ({ left, top, right, bottom });
  // At 1024x640 with Ask-me open: the pill centred [257,426,379,456], at the edge, or above the buttons.
  const centre = r(257, 426, 379, 456);
  const edge = r(470, 426, 592, 456);
  const above = (top: number) => r(470, top - 38, 592, top - 8);

  it('stays centred while it lies on nothing of the waiting card', () => {
    const parts = [r(217, 120, 600, 144), r(217, 152, 600, 195), r(217, 300, 600, 340)];
    expect(pillSpot([centre, edge, above(300)], parts, parts[2])).toBe(0);
  });

  it('the review’s defect 4: over "Allow for this session" and "Deny" it goes to the edge, off the buttons', () => {
    const buttons = r(217, 415, 460, 455);
    const parts = [r(217, 210, 600, 234), r(217, 242, 600, 285), buttons];
    expect(pillSpot([centre, edge, above(buttons.top)], parts, buttons)).toBe(1);
  });

  it('above the buttons when the row is as wide as the thread, and never on the buttons when it can help it', () => {
    const buttons = r(217, 415, 600, 455);
    const parts = [r(217, 210, 600, 234), r(217, 242, 600, 285), buttons];
    expect(pillSpot([centre, edge, above(buttons.top)], parts, buttons)).toBe(2);
    // Above them it would lie on what the card asks: still off the buttons.
    const tall = [r(217, 300, 600, 324), r(217, 332, 600, 400), buttons];
    expect(pillSpot([centre, edge, above(buttons.top)], tall, buttons)).toBe(2);
    // No spot off the buttons: the last one.
    expect(pillSpot([centre, edge], parts, r(0, 0, 1000, 1000))).toBe(1);
  });
});
