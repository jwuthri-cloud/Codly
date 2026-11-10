import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import { STEER_GRACE, waitingOf, type Sent, type Turn } from '../state/queue';
import { store } from '../state/store.svelte';

const msg = (key: string, ts: number, text = key): Sent => ({ key, ts, text });
const turn = (start: number, end: number | null, input: string | null = null): Turn => ({ start, end, input });
const keys = (r: { waiting: Sent[] }) => r.waiting.map((m) => m.key);

describe('waitingOf', () => {
  it('a message to an idle agent never queues', () => {
    expect(keys(waitingOf([turn(100, 200, 'hello')], [msg('a', 90, 'hello')], 300))).toEqual([]);
  });

  it('a message sent mid-turn waits until the turn that carries it starts', () => {
    const sent = [msg('a', 150, 'also do X')];
    expect(keys(waitingOf([turn(100, null, 'plan it')], sent, 160))).toEqual(['a']);
    expect(keys(waitingOf([turn(100, 400, 'plan it'), turn(401, null, 'context…\n\nalso do X')], sent, 410))).toEqual([]);
  });

  it('several queued messages keep their order and leave one turn at a time', () => {
    const sent = [msg('a', 150, 'first'), msg('b', 160, 'second')];
    const turns = [turn(100, 400, 'go'), turn(401, null, 'first')];
    expect(keys(waitingOf(turns, sent, 410))).toEqual(['b']);
  });

  it('an unrelated turn (a worker question) does not deliver your message', () => {
    const sent = [msg('a', 150, 'also do X')];
    const turns = [turn(100, 400, 'go'), turn(401, null, 'Cy asks: above or below?')];
    expect(keys(waitingOf(turns, sent, 410))).toEqual(['a']);
  });

  it('a turn without a recorded prompt takes the oldest', () => {
    const sent = [msg('a', 150), msg('b', 160)];
    expect(keys(waitingOf([turn(100, 400), turn(401, null, null)], sent, 410))).toEqual(['b']);
  });

  it('steered into the running turn: nothing waits once the agent has been idle a while', () => {
    const sent = [msg('a', 150)];
    const turns = [turn(100, 400, 'go')];
    expect(keys(waitingOf(turns, sent, 400 + STEER_GRACE - 1))).toEqual(['a']);
    expect(keys(waitingOf(turns, sent, 400 + STEER_GRACE + 1))).toEqual([]);
  });

  it('all Lead slots busy: what you sent since the last turn waits for a free Lead', () => {
    const r = waitingOf([], [msg('a', 100), msg('b', 120)], 500, { ts: 105 });
    expect(r.slot).toBe(true);
    expect(keys(r)).toEqual(['a', 'b']);
    // Once the Lead starts, the note no longer holds anything.
    expect(waitingOf([turn(600, null, 'a')], [msg('a', 100)], 700, { ts: 105 }).slot).toBe(false);
  });
});

describe('approvalThread', () => {
  // Only the fields approvalThread reads; the broker's rule is ingest.rs (task → request, Lead → its thread, else 1:1).
  const agent = (o: Partial<Agent>) => ({ id: 'a1', role: 'solo', task_id: null, thread: null, ...o }) as Agent;
  const approval = (agent_id: string) => ({ id: 'ap1', agent_id, task_id: null }) as Approval;

  it('approvalThread routes a task-less agent to its 1:1', () => {
    store.agents = [agent({ id: 'solo1' }), agent({ id: 'lead2', role: 'lead', thread: 'lead:x' })];
    expect(store.approvalThread(approval('solo1'))).toBe('agent:solo1');
    expect(store.approvalThread(approval('lead2'))).toBe('lead:x');
    expect(store.approvalThread(approval('gone'))).toBe('lead');
    store.agents = [];
  });
});
