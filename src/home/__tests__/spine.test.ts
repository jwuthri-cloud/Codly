// A row's key comes from the id of what the row shows, never from its words: an agent that
// says the same thing twice has two rows, and the keyed list of the conversation (which
// stops for good on two equal keys) always renders.
import { beforeEach, describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { store } from '../../lib/state/store.svelte';
import { mkAgent } from '../../office/demo/script';
import { buildSpine, keepRows, streamText, type Item } from '../spine';

const lead = { ...mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead'), run_id: 'r1' };
const solo = mkAgent('s1', 'Cleo', 'claude', 'solo', 1, 'working');
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

// Every test starts from an empty office with a Lead and one agent of your own.
let id = 0;
let clock = 1000;
beforeEach(() => {
  id = 0;
  clock = 1000;
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead, solo], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
});

/** A stored event (the clock moves 10 ms with each event, unless `ts` says when). */
const ev = (agent: string | null, kind: unknown, ts = (clock += 10)): EventRow => ({ id: ++id, ts, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
/** A streamed word: an event that is in no log. */
const word = (agent: string, item: string, text: string): EventRow => ({ id: 0, ts: (clock += 10), agent_id: agent, task_id: null, run_id: null, kind: { type: 'text_delta', item_id: item, text } });
const said = (agent: string, item: string, text: string, ts?: number) => ev(agent, { type: 'message', item_id: item, text, phase: 'final' }, ts);
/** A chat row: what you sent, or what the core posts to the thread right after an agent's message. */
const chat = (cid: string, thread: string, author: string, text: string, ts = (clock += 10)) =>
  ev(author === 'user' ? null : author, { type: 'chat', message: { id: cid, thread, author, text, attachment: null, ts } }, ts);
/** An agent's reply as the core sends it: the message, then its chat row with the same words. */
const reply = (agent: string, thread: string, item: string, text: string, ts?: number) => [said(agent, item, text, ts), chat(`c-${item}`, thread, agent, text, ts)];
const turn = (agent: string) => ev(agent, { type: 'turn_started', input: null });
const turnEnd = (agent: string) => ev(agent, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null });
const read = (agent: string, call: string, path: string) => [
  ev(agent, { type: 'tool_started', call_id: call, name: 'Read', activity: 'read', target: { path, paths: [], command: null, query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' }),
  ev(agent, { type: 'tool_completed', call_id: call, ok: true, output: null, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null }),
];

const says = (rows: Item[]) => rows.filter((r): r is Item & { kind: 'say' } => r.kind === 'say');
/** The keys of the rows, after a check that no two are equal. */
const keys = (rows: Item[]) => {
  const all = rows.map((r) => r.key);
  expect(new Set(all).size).toBe(all.length);
  return all;
};

describe('a reply the agent repeats', () => {
  it('is two rows with two keys in a request thread', () => {
    apply(chat('u1', 'lead', 'user', 'Is it done?'), ...reply('l1', 'lead', 'm1', 'Yes, all done.'), chat('u2', 'lead', 'user', 'Are you sure?'), ...reply('l1', 'lead', 'm2', 'Yes, all done.'));
    const rows = buildSpine('lead');
    expect(rows.map((r) => r.kind)).toEqual(['user', 'say', 'user', 'say']);
    expect(keys(rows)).toEqual(['c:u1', 'say:m1', 'c:u2', 'say:m2']);
    expect(says(rows).map((r) => r.text)).toEqual(['Yes, all done.', 'Yes, all done.']);
  });

  it('is two rows with two keys in a 1:1 thread', () => {
    apply(chat('u1', 'agent:s1', 'user', 'Hello'), ...reply('s1', 'agent:s1', 'm1', 'I am free. What do you need?'), chat('u2', 'agent:s1', 'user', 'And now?'), ...reply('s1', 'agent:s1', 'm2', 'I am free. What do you need?'));
    const rows = buildSpine('agent:s1');
    expect(rows.map((r) => r.kind)).toEqual(['user', 'say', 'user', 'say']);
    expect(keys(rows)).toEqual(['c:u1', 'say:m1', 'c:u2', 'say:m2']);
  });

  it('keeps one row of its own from its first streamed word to its chat row', () => {
    apply(...reply('l1', 'lead', 'm1', 'Done.'));
    let rows = keepRows([], buildSpine('lead'));
    const first = rows[0];

    apply(word('l1', 'm2', 'Do'));
    rows = keepRows(rows, buildSpine('lead'));
    expect(keys(rows)).toEqual(['say:m1', 'say:m2']);
    apply(word('l1', 'm2', 'ne.'));
    expect(streamText(says(rows)[1])).toBe('Done.'); // the very words of the first reply
    expect(keepRows(rows, buildSpine('lead'))).toBe(rows);

    // Said in full, its chat row still on its way: it is not taken for the first reply.
    apply(said('l1', 'm2', 'Done.'));
    rows = keepRows(rows, buildSpine('lead'));
    expect(says(rows).map((r) => [r.key, r.streaming, r.text])).toEqual([['say:m1', false, 'Done.'], ['say:m2', false, 'Done.']]);
    keys(rows);

    apply(chat('c-m2', 'lead', 'l1', 'Done.'));
    rows = keepRows(rows, buildSpine('lead'));
    expect(keys(rows)).toEqual(['say:m1', 'say:m2']);
    expect(rows[0]).toBe(first); // the first reply's row was there all along
  });

  it('is two rows when both were said in the same millisecond', () => {
    apply(...reply('l1', 'lead', 'm1', 'Ok.', 2000), ...reply('l1', 'lead', 'm2', 'Ok.', 2000));
    expect(keys(buildSpine('lead'))).toEqual(['say:m1', 'say:m2']);
  });

  it('is a row next to an equal message that has no chat row', () => {
    apply(said('l1', 'm0', 'On it.'), ...reply('l1', 'lead', 'm1', 'On it.'));
    expect(keys(buildSpine('lead'))).toEqual(['say:m0', 'say:m1']);
  });

  it('is a row next to an equal chat row whose message is not held any more', () => {
    apply(chat('a1', 'lead', 'l1', 'Done.'), ...reply('l1', 'lead', 'm2', 'Done.'));
    expect(keys(buildSpine('lead'))).toEqual(['c:a1', 'say:m2']);
  });
});

describe('a row key', () => {
  it('stays the only one of its kind when a CLI uses a call id again in a later turn', () => {
    apply(turn('l1'), ...read('l1', 'call-1', 'a.ts'), ...reply('l1', 'lead', 'm1', 'Read it.'), turnEnd('l1'), turn('l1'), ...read('l1', 'call-1', 'b.ts'));
    const rows = buildSpine('lead');
    expect(rows.map((r) => r.kind)).toEqual(['work', 'say', 'work']);
    expect(keys(rows)).toEqual(['w:call-1', 'say:m1', 'w:call-1#2']);
  });
});
