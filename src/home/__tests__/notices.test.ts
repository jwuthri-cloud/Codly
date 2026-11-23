// What an agent's CLI told about its runs is one row for a turn, however often it spoke: a hook
// that says a word on every tool call does not fill the conversation.
import { beforeEach, describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { store } from '../../lib/state/store.svelte';
import { mkAgent } from '../../office/demo/script';
import { moreOf, noticeRows } from '../notices';
import { buildSpine, keepRows, type Item } from '../spine';

let id = 0;
let clock = 1000;
beforeEach(() => {
  id = 0;
  clock = 1000;
});

/** A stored event of the Lead `l1` (`agent`: another agent's), 10 ms after the one before. */
const ev = (kind: unknown, agent = 'l1'): EventRow => ({ id: ++id, ts: (clock += 10), agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const turn = (agent?: string) => ev({ type: 'turn_started', input: null }, agent);
const turnEnd = (agent?: string) => ev({ type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null }, agent);
const notice = (text: string, agent?: string) => ev({ type: 'notice', text }, agent);
/** The rows as their words. */
const words = (events: EventRow[]) => noticeRows(events, 'l1').map((row) => row.map((n) => n.text));

describe('noticeRows', () => {
  it('everything told in one turn is one row, oldest first', () => {
    const begun = turn();
    const hook = Array.from({ length: 30 }, (_, i) => notice(`hook: checked call ${i + 1}`));
    const rows = noticeRows([begun, ...hook, turnEnd()], 'l1');
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveLength(30);
    expect(rows[0][0]).toEqual({ eventId: hook[0].id, ts: hook[0].ts, text: 'hook: checked call 1' });
    expect(rows[0][29].text).toBe('hook: checked call 30');
    expect(moreOf(rows[0])).toBe('and 29 more');
  });

  it('each turn has its own row, and a turn that still runs has one too', () => {
    expect(words([turn(), notice('a'), notice('b'), turnEnd(), turn(), notice('c'), turnEnd(), turn(), notice('d'), notice('e')])).toEqual([['a', 'b'], ['c'], ['d', 'e']]);
  });

  it('a notice outside any turn is a row of its own', () => {
    expect(words([turn(), turnEnd(), notice('MCP server linear did not start'), notice('Goal: ship it'), turn(), notice('in the turn'), turnEnd(), notice('after')])).toEqual([
      ['MCP server linear did not start'],
      ['Goal: ship it'],
      ['in the turn'],
      ['after'],
    ]);
  });

  it('the same words told again outside a turn are one row with a count', () => {
    const rows = noticeRows([turn(), turnEnd(), notice('Goal: ship it'), notice('Goal: ship it'), notice('Goal: ship it'), notice('other'), notice('Goal: ship it')], 'l1');
    expect(rows.map((r) => r.map((n) => n.text))).toEqual([['Goal: ship it', 'Goal: ship it', 'Goal: ship it'], ['other'], ['Goal: ship it']]);
    expect(rows.map(moreOf)).toEqual(['3 times', null, null]);
  });

  it('the same words before and after a turn are two rows: the turn lies between them', () => {
    expect(words([notice('MCP server linear did not start'), turn(), turnEnd(), notice('MCP server linear did not start')])).toEqual([['MCP server linear did not start'], ['MCP server linear did not start']]);
  });

  it('a process that ended closes its turn', () => {
    expect(words([turn(), notice('a'), ev({ type: 'exited', code: 1 }), notice('b'), notice('c')])).toEqual([['a'], ['b'], ['c']]);
  });

  it('a log that begins inside a turn: what was told before that turn ended is one row', () => {
    expect(words([notice('a'), notice('b'), notice('c'), turnEnd(), notice('d')])).toEqual([['a', 'b', 'c'], ['d']]);
    // Nothing says a turn ran: they stay rows of their own.
    expect(words([notice('a'), notice('b'), turn(), notice('c')])).toEqual([['a'], ['b'], ['c']]);
  });

  it("another agent's turns and notices are not this agent's", () => {
    expect(words([notice('mine, outside'), turn('w1'), notice('theirs', 'w1'), notice('mine, still outside'), turnEnd('w1')])).toEqual([['mine, outside'], ['mine, still outside']]);
  });

  it('says how many there are: a count for the same words, "more" for different ones', () => {
    const [same] = noticeRows([turn(), notice('a'), notice('a'), turnEnd()], 'l1');
    const [mixed] = noticeRows([turn(), notice('a'), notice('b'), notice('a'), turnEnd()], 'l1');
    const [one] = noticeRows([notice('a')], 'l1');
    expect([moreOf(same), moreOf(mixed), moreOf(one)]).toEqual(['2 times', 'and 2 more', null]);
  });
});

describe('in the conversation', () => {
  const lead = { ...mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead'), run_id: 'r1' };
  const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
  beforeEach(() => {
    const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
  });

  const read = (call: string) => [
    ev({ type: 'tool_started', call_id: call, name: 'Read', activity: 'read', target: { path: `${call}.ts`, paths: [], command: null, query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' }),
    ev({ type: 'tool_completed', call_id: call, ok: true, output: null, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null }),
  ];
  const said = (item: string, text: string) => {
    const message = ev({ type: 'message', item_id: item, text, phase: 'final' });
    return [message, ev({ type: 'chat', message: { id: `c-${item}`, thread: 'lead', author: 'l1', text, attachment: null, ts: clock + 10, item_id: item } })];
  };
  const notices = (rows: Item[]) => rows.filter((r): r is Item & { kind: 'notice' } => r.kind === 'notice');

  it('a hook that speaks on each of 30 tool calls is one row, after the work of its turn', () => {
    const begun = turn();
    const calls = Array.from({ length: 30 }, (_, i) => [...read(`call-${i + 1}`), notice(`hook: checked call ${i + 1}`)]).flat();
    apply(begun, ...calls, ...said('m1', 'Read them all.'), turnEnd());
    const rows = buildSpine('lead');
    expect(rows.map((r) => r.kind)).toEqual(['work', 'notice', 'say']);
    const [row] = notices(rows);
    expect(row.notes).toHaveLength(30);
    // It sits where the newest one was told, and its key is the first one's: it stays one row.
    expect([row.key, row.ts]).toEqual([`n:${row.notes[0].eventId}`, row.notes[29].ts]);
  });

  it('keeps its row while nothing new is told, and its key when more is told', () => {
    apply(turn(), ...read('call-1'), notice('hook: checked call 1'));
    let rows = keepRows([], buildSpine('lead'));
    const first = notices(rows)[0];
    expect(keepRows(rows, buildSpine('lead'))).toBe(rows);

    apply(...read('call-2'), notice('hook: checked call 2'));
    rows = keepRows(rows, buildSpine('lead'));
    expect(rows.map((r) => r.kind)).toEqual(['work', 'notice']);
    const now = notices(rows)[0];
    expect(now.key).toBe(first.key);
    expect(now.notes.map((n) => n.text)).toEqual(['hook: checked call 1', 'hook: checked call 2']);
    expect(now.notes[0]).toBe(first.notes[0]); // the same note object: only what changed is drawn again
  });

  it('a second turn has a row of its own, after its own work', () => {
    apply(turn(), ...read('call-1'), notice('first turn'), ...said('m1', 'One.'), turnEnd(), turn(), ...read('call-2'), notice('second turn'), ...said('m2', 'Two.'), turnEnd());
    const rows = buildSpine('lead');
    expect(rows.map((r) => r.kind)).toEqual(['work', 'notice', 'say', 'work', 'notice', 'say']);
    expect(notices(rows).map((r) => r.notes.map((n) => n.text))).toEqual([['first turn'], ['second turn']]);
  });
});
