// A long session must not make each new event cost more: the thread reads the event log once
// for its troubles, its CLI's notices and what you did to its agent, and after that only the
// events that came since. These tests count the rows of the log read, never time. (The chat
// rows, approvals, tasks and kept rows of the thread are still visited on each build.)
import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { ChatMessage } from '../../lib/gen/ChatMessage';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { store } from '../../lib/state/store.svelte';
import { mkAgent } from '../../office/demo/script';
import { buildSpine, logReadsKept, type Item, type SpineCost } from '../spine';

const solo = mkAgent('s1', 'Cleo', 'claude', 'solo', 1, 'idle');
const other = mkAgent('s2', 'Bo', 'codex', 'solo', 2, 'working');
const THREAD = 'agent:s1';
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

const row = (id: number, agent: string, kind: unknown): EventRow => ({ id, ts: 1000 + id * 10, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });

/** A log of `n` events, a fifth of them the thread's agent's (its turns, some failed, notices, mode changes), and `said` chat rows in its thread. */
function office(n: number, said = 1000): { events: EventRow[]; chats: ChatMessage[] } {
  const events: EventRow[] = [];
  for (let id = 1; id <= n; id++) {
    if (id % 5) {
      // Another agent's line (a cheap one to store: the store's cost is not what these count).
      events.push(row(id, 's2', { type: 'notice', text: `Other ${id}` }));
      continue;
    }
    const step = (id / 5) % 4;
    if (step === 0) events.push(row(id, 's1', { type: 'turn_started', input: `ask ${id}` }));
    else if (step === 1) events.push(row(id, 's1', { type: 'notice', text: `Hook said ${id}` }));
    else if (step === 2) events.push(row(id, 's1', { type: 'decision', id: `d${id}`, decision_type: 'agent_control', method: 'human', summary: `Cleo now works in the mode "Plan first" ${id}`, detail: {} }));
    else events.push(row(id, 's1', { type: 'turn_completed', status: id % 40 === 35 ? 'failed' : 'completed', result_text: null, duration_ms: null }));
  }
  const chats: ChatMessage[] = [];
  for (let i = 0; i < said; i++) chats.push({ id: `m${i}`, thread: THREAD, author: i % 2 ? 's1' : 'user', text: `message ${i}`, attachment: null, ts: 1000 + i });
  return { events, chats };
}

function load(n: number, said = 1000) {
  const { events, chats } = office(n, said);
  const snap = {
    as_of: n, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [solo, other], plans: [], tasks: [], approvals: [], chats,
    memories: [], triples: [], artifacts: [], events,
  };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
}

/** The log rows one more event of the thread's agent makes the next build read. */
function costOfOneMore(n: number): number {
  load(n, 100);
  buildSpine(THREAD);
  apply(row(n + 1, 's1', { type: 'notice', text: 'One more' }));
  const cost: SpineCost = { events: 0 };
  buildSpine(THREAD, cost);
  return cost.events;
}

const shape = (rows: Item[]) => rows.map((r) => `${r.kind} ${r.key}${r.kind === 'notice' || r.kind === 'control' ? ` ${r.notes.length}` : ''}`);

describe('the cost of one more event', () => {
  it('the log rows it reads do not grow with the log: 5,000 events cost what 1,000 cost', () => {
    expect(costOfOneMore(5000)).toBe(1);
    expect(costOfOneMore(1000)).toBe(1);
  });

  it('a first build reads the whole log once, for the three readers together', () => {
    load(2000, 100);
    const cost: SpineCost = { events: 0 };
    buildSpine(THREAD, cost);
    expect(cost.events).toBe(2000);
  });

  it('builds the same rows as a build from the start', () => {
    load(1000, 100);
    buildSpine(THREAD);
    apply(row(1001, 's1', { type: 'turn_started', input: 'more' }), row(1002, 's1', { type: 'notice', text: 'a' }), row(1003, 's1', { type: 'notice', text: 'b' }));
    buildSpine(THREAD);
    apply(row(1004, 's1', { type: 'error', message: 'boom', fatal: false }), row(1005, 's1', { type: 'turn_completed', status: 'failed', result_text: null, duration_ms: null }));
    const step = shape(buildSpine(THREAD));
    const { events, chats } = office(1000, 100);
    const all = [...events, ...store.events.slice(-5)];
    const snap = {
      as_of: 1005, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [solo, other], plans: [], tasks: [], approvals: [], chats,
      memories: [], triples: [], artifacts: [], events: all,
    };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
    expect(step).toEqual(shape(buildSpine(THREAD)));
    // The notice row of the last turn holds both of its notes, and the failed turn is one row.
    expect(step.filter((s) => s.startsWith('notice')).pop()).toBe('notice n:1002 2');
    expect(step.filter((s) => s.startsWith('trouble')).pop()).toBe('trouble e:1004');
  });

  it('a log that was trimmed is read again from its start', () => {
    load(5000, 100);
    buildSpine(THREAD);
    // Past the cap's margin the store keeps the newest 5,000.
    const more: EventRow[] = [];
    for (let id = 5001; id <= 6001; id++) more.push(row(id, 's2', { type: 'notice', text: 'other' }));
    apply(...more);
    const cost: SpineCost = { events: 0 };
    buildSpine(THREAD, cost);
    expect(cost.events).toBe(store.events.length);
  });

  it('a log that was trimmed or replaced is not kept by the read of another thread', () => {
    load(5000, 100);
    buildSpine(THREAD);
    buildSpine('agent:s2');
    expect(logReadsKept()).toBe(2);
    const more: EventRow[] = [];
    for (let id = 5001; id <= 6001; id++) more.push(row(id, 's2', { type: 'notice', text: 'other' }));
    apply(...more);
    buildSpine(THREAD);
    expect(logReadsKept()).toBe(1);
    load(100, 10);
    buildSpine('agent:s2');
    expect(logReadsKept()).toBe(1);
  });
});
