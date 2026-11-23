// While a card waits for you, the thread holds the oldest one and counts the rows below.
import { describe, expect, it } from 'vitest';
import type { Approval } from '../../lib/gen/Approval';
import type { Item } from '../spine';
import { newBelow, oldestWaiting, revealTop } from '../threadScroll';

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
