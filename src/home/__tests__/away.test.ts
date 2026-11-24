// When you come back after a long run, one sentence says what came while you were away, the
// things that need you first, each with the row it is about.
import { describe, expect, it } from 'vitest';
import type { Approval } from '../../lib/gen/Approval';
import { AWAY_MS, awayFor, awayKeys, awaySummary, hasNews, wasAway, withAway, type AwayNames } from '../away';
import type { Item } from '../spine';

const T = 1_760_000_000_000;
const MIN = 60_000;
/** You left at T and came back 47 minutes later. */
const w = { since: T, back: T + 47 * MIN };
const at = (m: number) => T + m * MIN;

const names: AwayNames = {
  agent: (id) => ({ ada: 'Ada', bo: 'Bo', lead: 'Lead' })[id] ?? 'An agent',
  task: (id) => ({ t1: { key: 't1', assignee: 'ada' }, t2: { key: 't2', assignee: 'ada' }, t4: { key: 't4', assignee: 'bo' } })[id] ?? null,
  planMerged: (id) => id === 'merged',
};

const say = (key: string, m: number): Item => ({ kind: 'say', key, ts: at(m), agentId: 'lead', itemId: null, text: 'x', phase: 'final', streaming: false, header: true, card: null, replyTo: null });
const mine = (key: string, m: number): Item => ({ kind: 'user', key, ts: at(m), msg: { id: key, thread: 'lead', author: 'user', text: 'hi', attachment: null, ts: at(m) }, outgoing: null });
const work = (key: string, m: number): Item => ({ kind: 'work', key, ts: at(m), agentId: 'lead', segs: [], thoughts: [], thinking: null, helpers: [] });
const card = (key: string, m: number, agent: string, status: Approval['status'] = 'pending', kind: 'question' | 'command' = 'question', decided: string | null = null): Item => ({
  kind: 'approval', key, ts: at(m), twins: [],
  approval: { id: key, agent_id: agent, status, decided_by: decided, created_at: at(m), subject: kind === 'question' ? { kind, questions: [] } : { kind, command: 'npm test', cwd: null } } as unknown as Approval,
});
const done = (key: string, m: number, taskId: string): Item => ({ kind: 'task-done', key, ts: at(m), taskId });
const trouble = (key: string, m: number, reason: string | null): Item => ({ kind: 'trouble', key, ts: at(m), agentId: 'lead', eventId: 1, what: 'failed', reason, retry: null });
const planDone = (key: string, m: number, planId: string): Item => ({ kind: 'system', key, ts: at(m), text: 'Plan done', card: { type: 'plan-done', planId } });

const texts = (items: Item[]) => awaySummary(items, w, names)?.facts.map((f) => f.text) ?? null;

describe('awaySummary', () => {
  it('says each kind of fact, the ones that need you first, with the row each is about', () => {
    const items = [
      say('s0', -5),
      done('d1', 5, 't1'),
      trouble('e1', 8, 'tests did not pass in the checkout suite after the change'),
      done('d2', 12, 't2'),
      card('a1', 20, 'bo'),
      planDone('p1', 30, 'p'),
      card('a2', 31, 'ada', 'approved', 'command', 'policy'),
    ];
    const s = awaySummary(items, w, names);
    expect(s?.minutes).toBe(47);
    expect(s?.facts).toEqual([
      { key: 'a1', text: 'Bo waits for your answer' },
      { key: 'e1', text: 'Lead’s turn failed: tests did not pass in the checkout suite…' },
      { key: 'a2', text: 'a request was answered without you' },
      { key: 'd1', text: 'Ada finished t1 and t2' },
      { key: 'p1', text: 'the plan is ready to merge' },
    ]);
  });

  it('counts the other rows last, from the oldest of them', () => {
    expect(awaySummary([say('s1', 3), work('w1', 4), done('d4', 6, 't4'), say('s2', 9)], w, names)?.facts).toEqual([
      { key: 'd4', text: 'Bo finished t4' },
      { key: 's1', text: '3 more rows' },
    ]);
    expect(texts([done('d1', 3, 't1'), planDone('p1', 4, 'merged'), say('s1', 5)])).toEqual(['Ada finished t1', 'the plan was merged', '1 more row']);
  });

  it('a card that waits since before you left is the first fact: it is the card the thread holds', () => {
    const items = [card('a0', -3, 'ada', 'pending', 'command'), card('a1', 4, 'bo', 'pending', 'question'), say('s1', 5), say('s2', 6)];
    expect(awaySummary(items, w, names)?.facts[0]).toEqual({ key: 'a0', text: 'Ada and Bo wait for you' });
    expect(texts([card('a0', 2, 'ada', 'pending', 'command'), say('s1', 5), say('s2', 6)])?.[0]).toBe('Ada waits for your approval');
  });

  it('says five facts at most: the ones cut are counted with the other rows', () => {
    const items = [
      card('a1', 1, 'bo'),
      trouble('e1', 2, null),
      trouble('e2', 3, 'out of credits'),
      card('a2', 4, 'ada', 'denied', 'command', 'policy'),
      done('d1', 5, 't1'),
      planDone('p1', 6, 'p'),
      say('s1', 7),
      say('s2', 8),
    ];
    expect(awaySummary(items, w, names)?.facts).toEqual([
      { key: 'a1', text: 'Bo waits for your answer' },
      { key: 'e1', text: 'Lead’s turn failed' },
      { key: 'e2', text: 'Lead’s turn failed: out of credits' },
      { key: 'a2', text: 'a request was answered without you' },
      // t1 and the plan did not fit: with the two other rows, from the oldest of them.
      { key: 'd1', text: '4 more rows' },
    ]);
    // Five facts and no other row: all five.
    expect(texts(items.slice(0, 5))).toEqual(['Bo waits for your answer', 'Lead’s turn failed', 'Lead’s turn failed: out of credits', 'a request was answered without you', 'Ada finished t1']);
  });

  it('gives none for fewer than 3 new rows, or less than 10 minutes gone', () => {
    expect(awaySummary([say('s0', -1), say('s1', 3), say('s2', 4)], w, names)).toBeNull();
    const many = [say('s1', 1), say('s2', 2), say('s3', 3)];
    expect(awaySummary(many, w, names)).not.toBeNull();
    expect(awaySummary(many, { since: T, back: T + AWAY_MS - 1 }, names)).toBeNull();
  });

  it('gives none for a thread with only your own messages, and rows after your return are not news', () => {
    expect(awaySummary([mine('u1', 2), mine('u2', 3), mine('u3', 4), say('s1', 5)], w, names)).toBeNull();
    expect(awaySummary([say('s1', 2), say('s2', 3), say('s3', 50), say('s4', 51)], w, names)).toBeNull();
  });
});

describe('the row of the summary', () => {
  const items = [say('s0', -5), say('s1', 3), say('s2', 4)];
  const row: Item & { kind: 'away' } = { kind: 'away', key: 'away', ts: T, summary: { minutes: 47, facts: [] } };

  it('goes before the first row newer than your leave, and nowhere with none', () => {
    expect(withAway(items, row).map((it) => it.key)).toEqual(['s0', 'away', 's1', 's2']);
    expect(withAway([say('s0', -5)], row).map((it) => it.key)).toEqual(['s0']);
  });

  it('speaks of the rows between your leave and your return', () => {
    expect([...awayKeys([...items, say('s3', 50)], w)]).toEqual(['s1', 's2']);
  });

  it('says how long you were away', () => {
    expect([awayFor(47), awayFor(60), awayFor(72)]).toEqual(['47 min', '1 h', '1 h 12 min']);
    expect([wasAway(T, T + AWAY_MS), wasAway(T, T + AWAY_MS - 1), wasAway(undefined, T)]).toEqual([true, false, false]);
  });
});

describe('hasNews', () => {
  const tab = { onScreen: false, needsYou: false, seen: T, newest: T + MIN };
  it('a request you are not on that got rows since you last had it on screen', () => {
    expect(hasNews(tab)).toBe(true);
  });
  it('none on screen, none when nothing came, none when it is not known when you saw it', () => {
    expect([hasNews({ ...tab, onScreen: true }), hasNews({ ...tab, newest: T }), hasNews({ ...tab, newest: null }), hasNews({ ...tab, seen: undefined })]).toEqual([false, false, false, false]);
  });
  it('the amber of a request that needs you wins over it', () => {
    expect(hasNews({ ...tab, needsYou: true })).toBe(false);
  });
});
