import { describe, expect, it } from 'vitest';
import type { Blocker } from '../gen/Blocker';
import { askMeOrder, isOverdue, waitedDays } from '../state/blockers.svelte';

const DAY = 86_400_000;
const NOW = 100 * DAY;
const card = (o: Partial<Blocker>): Blocker => ({
  id: 'b', workspace_id: 'w', agent_id: 'a', task_id: null, thread: 'lead', title: 't', body: 'b', severity: 'blocking', status: 'open',
  answer: null, created_at: NOW, updated_at: NOW, answered_at: null, reminded_at: null, ...o,
});

describe('ask me', () => {
  it('lists blocking cards before heads-ups, oldest first', () => {
    const fyi = card({ id: 'fyi', severity: 'heads_up', created_at: NOW - 5 * DAY });
    const young = card({ id: 'young', created_at: NOW - 1 * DAY });
    const old = card({ id: 'old', created_at: NOW - 3 * DAY });
    expect([fyi, young, old].sort(askMeOrder).map((b) => b.id)).toEqual(['old', 'young', 'fyi']);
  });

  it('counts whole waited days and nags after two (or when the core flagged it)', () => {
    expect(waitedDays(card({ created_at: NOW - 3.5 * DAY }), NOW)).toBe(3);
    expect(waitedDays(card({ created_at: NOW + DAY }), NOW)).toBe(0);
    expect(isOverdue(card({ created_at: NOW - DAY }), NOW)).toBe(false);
    expect(isOverdue(card({ created_at: NOW - 2 * DAY }), NOW)).toBe(true);
    expect(isOverdue(card({ created_at: NOW - DAY, reminded_at: NOW }), NOW)).toBe(true);
    expect(isOverdue(card({ created_at: NOW - 9 * DAY, status: 'answered' }), NOW)).toBe(false);
  });
});
