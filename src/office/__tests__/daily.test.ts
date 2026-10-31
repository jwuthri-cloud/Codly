// The daily edition and the standup: lines and headlines from the core's digest, once a day.
import { describe, expect, it } from 'vitest';
import type { Digest } from '../../lib/gen/Digest';
import { edition, editionDue, standupLine } from '../daily';

const stats = (o: Partial<Record<string, number>> = {}) => ({ commits: 0, checks_passed: 0, checks_failed: 0, approved: 0, changes_requested: 0, reviews_done: 0, merged: 0, shifts: 0, sick: 0, ...o });

describe('daily edition', () => {
  it('says only what it really did', () => {
    expect(standupLine({ agent_id: 'a', xp: 70, stats: stats({ merged: 1, commits: 2, checks_passed: 1 }) })).toBe('Yesterday: merged 1, 2 commits, 1 check passed');
    expect(standupLine({ agent_id: 'a', xp: 0, stats: stats({ checks_failed: 3 }) })).toBe('Yesterday: nothing landed, ready for today');
  });

  it('headlines what shipped and crowns the MVP', () => {
    const d: Digest = {
      since: 0,
      reports: [{ agent_id: 'ada', xp: 85, stats: stats({ merged: 1 }) }, { agent_id: 'bo', xp: 10, stats: stats({ reviews_done: 1 }) }],
      milestones: [['bo', { at: 1, text: 'First commit' }], ['ada', { at: 2, text: 'Promoted to Junior' }]],
      done: ['Dark mode'],
    };
    const e = edition(d);
    expect(e.headline).toBe('1 task shipped');
    expect(e.mvp).toEqual({ id: 'ada', xp: 85 });
    expect(e.notes[0].text).toBe('Promoted to Junior');
    expect(edition({ since: 0, reports: [], milestones: [], done: [] })).toMatchObject({ headline: 'A quiet day at the office', mvp: null });
  });

  it('comes out once a day after 5 am, covering at most 3 days', () => {
    const at = (d: string) => new Date(d).getTime();
    expect(editionDue(null, at('2026-10-29T04:00'))).toBeNull();
    expect(editionDue(null, at('2026-10-29T09:00'))).toEqual({ since: at('2026-10-28T09:00') });
    expect(editionDue(at('2026-10-29T08:00'), at('2026-10-29T17:00'))).toBeNull();
    expect(editionDue(at('2026-10-28T08:00'), at('2026-10-29T09:00'))).toEqual({ since: at('2026-10-28T08:00') });
    expect(editionDue(at('2026-10-01T08:00'), at('2026-10-29T09:00'))).toEqual({ since: at('2026-10-26T09:00') });
  });
});
