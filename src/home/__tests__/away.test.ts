// When you come back after a long run, one sentence says what came while you were away, the
// things that need you first, each with the row it is about.
import { describe, expect, it } from 'vitest';
import type { Approval } from '../../lib/gen/Approval';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { AWAY_MS, Presence, STAMP_EVERY_MS, awayFor, awayKeys, awaySummary, hasNews, leadTroubles, planDoneWords, requestNews, wasAway, withAway, type AwayNames, type PlanEnd, type Spot } from '../away';
import type { Item } from '../spine';

const T = 1_760_000_000_000;
const MIN = 60_000;
/** You left at T and came back 47 minutes later. */
const w = { since: T, back: T + 47 * MIN };
const at = (m: number) => T + m * MIN;

const names: AwayNames = {
  agent: (id) => ({ ada: 'Ada', bo: 'Bo', lead: 'Lead' })[id] ?? 'An agent',
  task: (id) => ({ t1: { key: 't1', assignee: 'ada' }, t2: { key: 't2', assignee: 'ada' }, t4: { key: 't4', assignee: 'bo' } })[id] ?? null,
  plan: (id) => (({ merged: 'merged', kept: 'kept', conflict: 'conflict', checks: 'checks', waits: 'waits', running: 'other' }) as Record<string, PlanEnd>)[id] ?? 'ready',
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
const planReady = (key: string, m: number, planId: string): Item => ({ kind: 'say', key, ts: at(m), agentId: 'lead', itemId: null, text: 'Here is the plan', phase: 'final', streaming: false, header: true, card: { type: 'plan', planId }, replyTo: null });

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

  it('a finished plan is said in the words of where it stands, not always "ready to merge"', () => {
    const rest = [say('s1', 2), say('s2', 3)];
    const said = (planId: string) => texts([planDone('p1', 1, planId), ...rest])?.[0];
    expect(['ready', 'merged', 'kept', 'conflict', 'checks', 'running'].map(said)).toEqual([
      'the plan is ready to merge',
      'the plan was merged',
      'the plan finished, and its branch is kept',
      'the plan finished, with a conflict to resolve',
      'the plan finished, and its checks stopped the merge',
      'the plan finished',
    ]);
    expect(planDoneWords('waits')).toBe('the plan finished');
  });

  it('a plan that waits for your go is the first fact, also one that came before you left', () => {
    // "6 more rows" said nothing while the plan waited (round 8 review, part B, defect 5).
    const rows = [say('s1', 2), say('s2', 3), say('s3', 4), work('w1', 5), say('s4', 6), say('s5', 7)];
    expect(awaySummary([planReady('m0', -20, 'waits'), ...rows], w, names)?.facts).toEqual([
      { key: 'm0', text: 'the plan waits for your go' },
      { key: 's1', text: '6 more rows' },
    ]);
    expect(texts([card('a1', 1, 'bo'), planReady('m1', 2, 'waits'), ...rows])?.slice(0, 2)).toEqual(['Bo waits for your answer', 'the plan waits for your go']);
    // A plan that runs, or one that came after your return, is no such fact.
    expect(texts([planReady('m0', 1, 'running'), ...rows])).toEqual(['7 more rows']);
    expect(texts([...rows, planReady('m9', 60, 'waits')])).toEqual(['6 more rows']);
  });

  it('a card that came after your return is not a fact: it did not come while you were away', () => {
    const items = [say('s1', 2), say('s2', 3), say('s3', 4), card('a1', 60, 'bo')];
    expect(texts(items)).toEqual(['3 more rows']);
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

describe('requestNews', () => {
  const isRequest = (t: string) => t === 'lead' || t.startsWith('lead:');
  it('a finished task, a trouble and a card light the dot as a message does; your own message does not', () => {
    const newest = requestNews(
      {
        chats: [
          { thread: 'lead', author: 'user', ts: 50 },
          { thread: 'lead:b', author: 'lead-b', ts: 10 },
          { thread: 'agent:ada', author: 'ada', ts: 90 },
        ],
        asks: [{ thread: 'lead:b', ts: 20 }],
        finished: [{ thread: 'lead', ts: 30 }],
        troubles: [{ thread: 'lead:c', ts: 40 }],
      },
      isRequest,
    );
    expect(Object.fromEntries(newest)).toEqual({ lead: 30, 'lead:b': 20, 'lead:c': 40 });
  });
});

describe('leadTroubles', () => {
  let id = 0;
  const ev = (agent: string, ts: number, kind: unknown): EventRow => ({ id: ++id, ts, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
  const started = (agent: string, ts: number) => ev(agent, ts, { type: 'turn_started', input: 'go' });
  const ended = (agent: string, ts: number, status: string) => ev(agent, ts, { type: 'turn_completed', status, result_text: null, duration_ms: null });
  const leads = [{ thread: 'lead', agentId: 'l1' }, { thread: 'lead:b', agentId: 'l2' }];
  const none = () => [];

  it('lights the dot as the thread shows a trouble row: errors, failed turns and step limits, also of turns before the last', () => {
    const events = [
      started('l1', 10), ev('l1', 11, { type: 'error', message: 'rate limited', fatal: false }), ended('l1', 12, 'completed'),
      started('l1', 20), ended('l1', 21, 'failed'),
      started('l1', 30), ended('l1', 31, 'completed'),
      started('l2', 40), ended('l2', 41, 'max_turns'),
      started('ada', 50), ended('ada', 51, 'failed'), // a worker: not a Lead of a tab
    ];
    expect(leadTroubles(events, leads, none, none)).toEqual([{ thread: 'lead', ts: 11 }, { thread: 'lead', ts: 21 }, { thread: 'lead:b', ts: 41 }]);
  });

  it('a turn you stopped, by button or with Esc, or one you killed, is not news; one its CLI cut is', () => {
    const events = [started('l1', 10), ended('l1', 12, 'interrupted'), started('l2', 20), ended('l2', 22, 'interrupted'), started('l1', 30), ended('l1', 32, 'interrupted')];
    const stops = (agent: string) => (agent === 'l1' ? [10] : []);
    const kills = (agent: string) => (agent === 'l2' ? [21] : []);
    expect(leadTroubles(events, leads, stops, kills)).toEqual([{ thread: 'lead', ts: 32 }]);
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

describe('Presence', () => {
  const a: Spot = { office: 'o', thread: 'a' };
  const b: Spot = { office: 'o', thread: 'b' };
  /** The times kept, as the storage of lastSeen.ts keeps them across a restart. */
  const kept = () => {
    const times = new Map<string, number>();
    const p = (stored = times) =>
      new Presence(
        (s) => stored.get(s.thread),
        (s, t) => stored.set(s.thread, t),
      );
    return { times, p };
  };

  it('a thread you left 10 minutes ago or more sums up from your leave when you open it again', () => {
    const { times, p } = kept();
    const presence = p();
    expect(presence.show(a, true, at(0))).toBeNull();
    expect(presence.show(b, true, at(5))).toBeNull();
    expect(times.get('a')).toBe(at(5));
    expect(presence.show(a, true, at(14))).toBeNull();
    presence.show(b, true, at(20));
    expect(presence.show(a, true, at(30))).toEqual({ since: at(20), back: at(30) });
  });

  it('a page hidden or a window without the focus for 10 minutes is a time away', () => {
    const { times, p } = kept();
    const presence = p();
    presence.show(a, true, at(0));
    expect(presence.setPresent(false, at(3))).toBeNull();
    expect(times.get('a')).toBe(at(3));
    presence.tick(at(10));
    expect(times.get('a')).toBe(at(3));
    expect(presence.setPresent(true, at(13))).toEqual({ since: at(3), back: at(13) });
    expect(presence.setPresent(true, at(14))).toBeNull();
  });

  it('a quit without a leave event loses at most a minute of your look', () => {
    const { times, p } = kept();
    const before = p();
    before.show(a, true, at(0));
    for (let m = 0; m <= 240; m++) before.tick(at(m) + 30_000);
    expect(at(240) + 30_000 - (times.get('a') ?? 0)).toBeLessThanOrEqual(STAMP_EVERY_MS);
    // Codly is gone: the next start opens the thread the next day.
    const next = p().show(a, true, at(24 * 60));
    expect(next?.since).toBeGreaterThanOrEqual(at(240) + 30_000 - STAMP_EVERY_MS);
  });

  it('does not stamp a thread you do not look at, nor more than once a minute', () => {
    const { times, p } = kept();
    const presence = p();
    presence.show(a, false, at(0));
    presence.tick(at(5));
    expect(times.has('a')).toBe(false);
    presence.setPresent(true, at(6));
    presence.tick(at(6) + 30_000);
    expect(times.get('a')).toBe(at(6));
    presence.tick(at(7));
    expect(times.get('a')).toBe(at(7));
  });
});
