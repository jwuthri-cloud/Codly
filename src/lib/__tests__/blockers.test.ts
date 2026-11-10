import { describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Blocker } from '../gen/Blocker';
import type { Workspace } from '../gen/Workspace';
import { askMeOrder, blockers, isOverdue, waitedDays } from '../state/blockers.svelte';
import { store } from '../state/store.svelte';

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

  it('cards of the office you left never show in the next one', async () => {
    const lists: ((l: Blocker[]) => void)[] = [];
    vi.spyOn(api, 'listBlockers').mockImplementation(() => new Promise((r) => lists.push(r)));
    const answered = vi.spyOn(api, 'answerBlocker').mockResolvedValue(card({ id: 'a1', workspace_id: 'a', status: 'answered' }));
    store.setWorkspace({ id: 'a' } as Workspace);
    const first = blockers.load();
    lists[0]([card({ id: 'a1', workspace_id: 'a' })]);
    await first;
    expect(blockers.list.map((b) => b.id)).toEqual(['a1']);

    // You switch while office A's list is being read again: B starts empty and stays B's.
    const late = blockers.load();
    store.setWorkspace({ id: 'b' } as Workspace);
    const next = blockers.load();
    expect(blockers.list).toEqual([]);
    lists[2]([card({ id: 'b1', workspace_id: 'b' })]);
    await next;
    lists[1]([card({ id: 'a1', workspace_id: 'a' })]);
    await late;
    expect(blockers.list.map((b) => b.id)).toEqual(['b1']);
    // An answer sent in A that lands after the switch is A's card, not B's.
    await blockers.answer('a1', 'yes');
    expect(answered).toHaveBeenCalled();
    expect(blockers.list.map((b) => b.id)).toEqual(['b1']);
    vi.restoreAllMocks();
    store.setWorkspace(null);
  });
});
