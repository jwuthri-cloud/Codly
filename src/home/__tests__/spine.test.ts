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
import type { Approval } from '../../lib/gen/Approval';
import { noteStop } from '../../lib/state/stops';

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
/** A chat row: what you sent, or what the core posts to the thread right after an agent's
 *  message (`item`: the id of that message, which the core writes on the row). */
const chat = (cid: string, thread: string, author: string, text: string, ts = (clock += 10), item?: string) =>
  ev(author === 'user' ? null : author, { type: 'chat', message: { id: cid, thread, author, text, attachment: null, ts, ...(item ? { item_id: item } : {}) } }, ts);
/** An agent's reply as the core sends it: the message, then its chat row with the same words and the message's id. */
const reply = (agent: string, thread: string, item: string, text: string, ts?: number) => [said(agent, item, text, ts), chat(`c-${item}`, thread, agent, text, ts, item)];
/** A reply whose chat row was stored before rows had the message's id: the row has the words only. */
const oldReply = (agent: string, thread: string, item: string, text: string, ts?: number) => [said(agent, item, text, ts), chat(`c-${item}`, thread, agent, text, ts)];
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

    apply(chat('c-m2', 'lead', 'l1', 'Done.', undefined, 'm2'));
    rows = keepRows(rows, buildSpine('lead'));
    expect(keys(rows)).toEqual(['say:m1', 'say:m2']);
    expect(rows[0]).toBe(first); // the first reply's row was there all along
  });

  it('is two rows when both were said in the same millisecond', () => {
    apply(...reply('l1', 'lead', 'm1', 'Ok.', 2000), ...reply('l1', 'lead', 'm2', 'Ok.', 2000));
    expect(keys(buildSpine('lead'))).toEqual(['say:m1', 'say:m2']);
  });

  it('keeps its key when it comes in the millisecond of the first reply’s chat row', () => {
    // The second message is there, its chat row is not yet. The first chat row still shows the
    // first message: matched by words and time it would take the second one, the newest with
    // its words, and the two keys would change places until the second chat row comes.
    apply(...reply('l1', 'lead', 'm1', 'Ok.', 2000), said('l1', 'm2', 'Ok.', 2000));
    expect(keys(buildSpine('lead'))).toEqual(['say:m1', 'say:m2']);
    apply(chat('c-m2', 'lead', 'l1', 'Ok.', 2000, 'm2'));
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

describe('a chat row shows the message whose id it carries', () => {
  it('whatever their words are: the words are not compared', () => {
    apply(said('l1', 'm1', 'Yes, all done.'), said('l1', 'm2', 'Yes.'), chat('c1', 'lead', 'l1', 'Yes.', undefined, 'm1'));
    const rows = says(buildSpine('lead'));
    // The row with the id of m1 is the row of m1, though m2 has its words. m2 has no chat row
    // yet, so it shows where it was said: before the chat row.
    expect(rows.map((r) => [r.key, r.itemId])).toEqual([['say:m2', 'm2'], ['say:m1', 'm1']]);
  });

  it('once, when that message is not held any more', () => {
    apply(chat('c1', 'lead', 'l1', 'Done.', undefined, 'gone'));
    expect(says(buildSpine('lead')).map((r) => [r.key, r.itemId, r.text])).toEqual([['c:c1', null, 'Done.']]);
  });

  it('and leaves a later message alone when its CLI uses the id again in another session', () => {
    // The store holds one message for an id: the second "msg-1" takes the place of the first.
    apply(...reply('l1', 'lead', 'msg-1', 'First.'), said('l1', 'msg-1', 'Second.'));
    let rows = says(buildSpine('lead'));
    // The new reply shows before its chat row has come: the old row does not take it.
    expect(rows.map((r) => [r.key, r.text])).toEqual([['c:c-msg-1', 'First.'], ['say:msg-1', 'Second.']]);
    apply(chat('c2', 'lead', 'l1', 'Second.', undefined, 'msg-1'));
    rows = says(buildSpine('lead'));
    expect(rows.map((r) => [r.key, r.text])).toEqual([['c:c-msg-1', 'First.'], ['say:msg-1', 'Second.']]);
  });

  it('and a turn result the core posted with no message shows no message', () => {
    apply(...reply('l1', 'lead', 'm1', 'Done.'), chat('c2', 'lead', 'l1', 'Done.'));
    expect(says(buildSpine('lead')).map((r) => [r.key, r.itemId])).toEqual([['say:m1', 'm1'], ['c:c2', null]]);
  });
});

describe('a chat row stored before rows had the id', () => {
  it('shows the message with its words that was said before it', () => {
    apply(chat('u1', 'lead', 'user', 'Is it done?'), ...oldReply('l1', 'lead', 'm1', 'Yes, all done.'));
    expect(keys(buildSpine('lead'))).toEqual(['c:u1', 'say:m1']);
  });

  it('is a row of its own for each reply with the same words', () => {
    apply(chat('u1', 'lead', 'user', 'Is it done?'), ...oldReply('l1', 'lead', 'm1', 'Yes, all done.'), chat('u2', 'lead', 'user', 'Are you sure?'), ...oldReply('l1', 'lead', 'm2', 'Yes, all done.'));
    expect(keys(buildSpine('lead'))).toEqual(['c:u1', 'say:m1', 'c:u2', 'say:m2']);
  });

  it('is two rows when both replies were said in the same millisecond', () => {
    apply(...oldReply('l1', 'lead', 'm1', 'Ok.', 2000), ...oldReply('l1', 'lead', 'm2', 'Ok.', 2000));
    expect(keys(buildSpine('lead'))).toEqual(['say:m1', 'say:m2']);
  });

  it('is a row next to an equal message that has no chat row', () => {
    apply(said('l1', 'm0', 'On it.'), ...oldReply('l1', 'lead', 'm1', 'On it.'));
    expect(keys(buildSpine('lead'))).toEqual(['say:m0', 'say:m1']);
  });

  it('never takes the message of a row that names it', () => {
    // The old row's own message is not held; a newer one with its words has a row that names it.
    apply(chat('a1', 'lead', 'l1', 'Done.'), ...reply('l1', 'lead', 'm2', 'Done.'), chat('a3', 'lead', 'l1', 'Done.'));
    expect(keys(buildSpine('lead'))).toEqual(['c:a1', 'say:m2', 'c:a3']);
    // An old reply and a new one with the same words: each row has its own message.
    apply(...oldReply('l1', 'lead', 'm4', 'Fixed.'), ...reply('l1', 'lead', 'm5', 'Fixed.'));
    expect(keys(buildSpine('lead')).slice(3)).toEqual(['say:m4', 'say:m5']);
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

describe('a turn you stopped while a card waited', () => {
  const card = (status: Approval['status'], created_at: number): Approval =>
    ({ id: `ap-${created_at}`, agent_id: 's1', run_id: 'r', task_id: null, call_id: null, subject: { kind: 'command', command: 'ls', cwd: null }, reason: null, choices: [], risks: [], status, decided_by: 'provider', created_at }) as Approval;
  const lines = (rows: Item[]) => rows.flatMap((r) => (r.kind === 'system' ? [r.text] : []));

  it('says that you stopped it, after the card', () => {
    apply(turn('s1'));
    noteStop('s1');
    store.approvals = [card('cancelled', clock + 1)];
    apply(turnEnd('s1'));
    const rows = buildSpine('agent:s1');
    expect(lines(rows)).toEqual(['You stopped Cleo while it waited for your answer']);
    expect(rows.map((r) => r.kind)).toEqual(['approval', 'system']);
  });

  it('says nothing for a turn you did not stop, or one with no card that waited', () => {
    clock = 5000; // the stops you made are remembered by turn start: not the turn of the test above
    apply(turn('s1'));
    store.approvals = [card('cancelled', clock + 1)];
    apply(turnEnd('s1'), turn('s1'));
    noteStop('s1');
    apply(turnEnd('s1'));
    expect(lines(buildSpine('agent:s1'))).toEqual([]);
  });
});

describe('the finished card of a plan', () => {
  const planChat = (cid: string, text: string, ts: number) => ev(null, { type: 'chat', message: { id: cid, thread: 'lead', author: 'codly', text, attachment: { plan_id: 'p1' }, ts } }, ts);
  const noticeAt = (text: string, ts: number) => ev('l1', { type: 'notice', text }, ts);
  const order = (rows: Item[]) => rows.map((r) => (r.kind === 'system' ? `system:${r.card?.type}` : r.kind === 'notice' ? `notice:${r.notes[0].text}` : r.kind));
  beforeEach(() => {
    const plan = { id: 'p1', thread: 'lead', status: 'done', summary: 'Dark mode' };
    const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead, solo], plans: [plan], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
  });

  it('is below a notice row that began before the done message, though its last note is newer', () => {
    apply(turn('l1'), turnEnd('l1'), planChat('c-ready', 'Plan ready', 1100), noticeAt('Goal: ship it', 1200), planChat('c-done', 'Plan done', 1300), noticeAt('Goal: ship it', 1400));
    const rows = buildSpine('lead');
    expect(order(rows)).toEqual(['system:plan', 'notice:Goal: ship it', 'system:plan-done']);
    expect(rows[1].ts).toBe(1400);
  });

  it('is above a row that begins after the done message', () => {
    apply(turn('l1'), turnEnd('l1'), planChat('c-ready', 'Plan ready', 1100), noticeAt('Goal: ship it', 1200), planChat('c-done', 'Plan done', 1300), noticeAt('Goal: ship it', 1400), noticeAt('MCP server linear did not start', 1500));
    expect(order(buildSpine('lead'))).toEqual(['system:plan', 'notice:Goal: ship it', 'system:plan-done', 'notice:MCP server linear did not start']);
  });
});
