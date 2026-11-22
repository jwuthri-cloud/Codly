// While several agents stream, a batch of events costs the screen only what it changed: the
// store versions each agent's own state, the conversation keeps the rows that did not change,
// and a message being written is read by its own row.
import { beforeEach, describe, expect, it } from 'vitest';
import { buildSpine, keepRows, openStreams, streamText, thinkingText, type Item } from '../../home/spine';
import { mkAgent } from '../../office/demo/script';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import { FILE_CAP, store } from '../state/store.svelte';

const lead = { ...mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead'), run_id: 'r1' };
const worker = mkAgent('w1', 'Ada', 'claude', 'worker', 1, 'working');
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

// Every test starts from an empty office with the Lead and one worker.
let id = 0;
let clock = 1000;
beforeEach(() => {
  id = 0;
  clock = 1000;
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead, worker], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
});

/** A stored event (the clock moves 10 ms with each event). */
const ev = (agent: string | null, kind: unknown, o: Partial<EventRow> = {}): EventRow => ({ id: ++id, ts: (clock += 10), agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind, ...o });
/** An event that is in no log: a streamed word, a chunk of output. */
const live = (agent: string, kind: unknown): EventRow => ({ id: 0, ts: (clock += 10), agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const target = (o: Record<string, unknown>) => ({ path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...o });
const started = (agent: string, call: string, path: string, activity = 'read') => ev(agent, { type: 'tool_started', call_id: call, name: 'Tool', activity, target: target({ path }), input: {}, confidence: 'observed' });
const completed = (agent: string, call: string, output: string | null = null) => ev(agent, { type: 'tool_completed', call_id: call, ok: true, output, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null });
const read = (agent: string, call: string, path: string) => [started(agent, call, path), completed(agent, call)];
const said = (agent: string, item: string, text: string) => ev(agent, { type: 'message', item_id: item, text, phase: 'final' });
const word = (agent: string, item: string, text: string) => live(agent, { type: 'text_delta', item_id: item, text });
const thought = (agent: string, item: string, text: string) => live(agent, { type: 'reasoning_delta', item_id: item, text });
const output = (agent: string, call: string, chunk: string) => live(agent, { type: 'tool_output', call_id: call, chunk });
const turn = (agent: string, input: string | null = null) => ev(agent, { type: 'turn_started', input });
const turnEnd = (agent: string, status = 'completed') => ev(agent, { type: 'turn_completed', status, result_text: null, duration_ms: null });
const yours = (cid: string, text: string) => ev(null, { type: 'chat', message: { id: cid, thread: 'lead', author: 'user', text, attachment: null, ts: clock + 10 } });

const says = (rows: Item[]) => rows.filter((r): r is Item & { kind: 'say' } => r.kind === 'say');
const works = (rows: Item[]) => rows.filter((r): r is Item & { kind: 'work' } => r.kind === 'work');

describe('the store, per agent', () => {
  it('a batch moves the versions of the agents it is about, and of no other', () => {
    apply(word('w1', 'i1', 'word'));
    expect(store.liveRevBy.get('w1')).toBe(1);
    expect(store.revBy.get('w1')).toBe(0);
    expect(store.liveRevBy.get('l1')).toBe(0);
    apply(...read('l1', 'c1', 'a.ts'), word('w1', 'i1', ' more'));
    expect(store.revBy.get('l1')).toBe(1); // once per batch, not per event
    expect(store.liveRevBy.get('w1')).toBe(2);
    expect(store.liveRevBy.get('l1')).toBe(0);
  });

  // A panel that reads a version that is not there runs again on every agent's batch.
  it('an agent has both its versions from the moment it is known, and a new snapshot starts them again', () => {
    const versions = () => [store.revBy, store.liveRevBy].map((by) => Object.fromEntries(by));
    expect(versions()).toEqual([{ l1: 0, w1: 0 }, { l1: 0, w1: 0 }]);

    const hired = mkAgent('w2', 'Bo', 'codex', 'worker', 2, 'idle');
    apply(ev('w2', { type: 'agent_changed', agent: hired }), word('w1', 'i1', 'word'));
    expect(versions()).toEqual([{ l1: 0, w1: 0, w2: 1 }, { l1: 0, w1: 1, w2: 0 }]);
    store.upsertAgent(mkAgent('w3', 'Cy', 'gemini', 'worker', 3, 'idle'));
    expect(versions()).toEqual([{ l1: 0, w1: 0, w2: 1, w3: 0 }, { l1: 0, w1: 1, w2: 0, w3: 0 }]);

    // Back from a hidden period: the same office, with the agents it has now.
    const snap = { as_of: id, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead, hired], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
    (store as unknown as { mergeSnapshot(s: Snapshot, missed: EventRow[]): void }).mergeSnapshot(snap as unknown as Snapshot, []);
    expect(versions()).toEqual([{ l1: 0, w2: 0 }, { l1: 0, w2: 0 }]);
  });

  it('a list a panel holds is never changed by a later batch', () => {
    apply(...read('l1', 'c1', 'a.ts'));
    const { segments, files, messages } = store.live.l1;
    const held = [...segments];
    apply(started('l1', 'c2', 'b.ts'), said('l1', 'm1', 'Done.'), completed('l1', 'c2'), ...read('l1', 'c3', 'c.ts'));
    expect(segments).toEqual(held);
    expect(segments[0]).toBe(held[0]);
    expect(Object.keys(files)).toEqual(['a.ts']);
    expect(messages).toEqual([]);
    expect(store.live.l1.segments.map((s) => [s.callId, s.end != null])).toEqual([['c1', true], ['c2', true], ['c3', true]]);
    expect(Object.keys(store.live.l1.files)).toEqual(['a.ts', 'b.ts', 'c.ts']);
    expect(store.live.l1.messages.map((m) => m.itemId)).toEqual(['m1']);
  });

  it('a message sent again under the same id takes the place of the first', () => {
    apply(said('l1', 'm1', 'One'), said('l1', 'm2', 'Two'));
    apply(said('l1', 'm1', 'One, again'));
    expect(store.live.l1.messages.map((m) => [m.itemId, m.text])).toEqual([['m2', 'Two'], ['m1', 'One, again']]);
  });

  it('past the cap the oldest calls leave, and the calls still open are still found', () => {
    const calls: EventRow[] = [started('l1', 'first', 'a.ts')];
    for (let i = 0; i < 1500; i++) calls.push(...read('l1', `c${i}`, 'a.ts'));
    apply(...calls, started('l1', 'last', 'a.ts'));
    const l = store.live.l1;
    expect(l.segments).toHaveLength(1500);
    expect(l.segments[0].callId).toBe('c1'); // "first" and "c0" left
    expect(l.open).toEqual({ last: 1499 });
    apply(completed('l1', 'last'));
    expect(store.live.l1.segments[1499]).toMatchObject({ callId: 'last', ok: true });
    expect(store.live.l1.open).toEqual({});
  });

  it('keeps every file an agent changed, and of the files it only read the ones touched last', () => {
    const calls: EventRow[] = [started('w1', 'e', 'edited.ts', 'edit'), completed('w1', 'e')];
    const reads = Math.floor(FILE_CAP * 1.2) + 1;
    for (let i = 0; i < reads; i++) calls.push(...read('w1', `f${i}`, `src/f${i}.ts`));
    apply(...calls);
    const files = store.live.w1.files;
    expect(Object.keys(files)).toHaveLength(FILE_CAP);
    expect(files['edited.ts']?.modified).toBe(true);
    expect(files['src/f0.ts']).toBeUndefined();
    expect(files[`src/f${reads - 1}.ts`]?.read).toBe(true);
  });

  it('tells which search found a file from the start of that search’s output only', () => {
    const found = `early.ts\n${'x'.repeat(20_000)}\nlate.ts`;
    apply(ev('w1', { type: 'tool_started', call_id: 's', name: 'Grep', activity: 'search', target: target({ query: 'needle' }), input: {}, confidence: 'observed' }), completed('w1', 's', found));
    apply(...read('w1', 'r1', 'early.ts'), ...read('w1', 'r2', 'late.ts'));
    const why = (path: string) => store.live.w1.files[path].reasons.map((r) => r.text);
    expect(why('early.ts')).toEqual(['Read with Tool', 'Found via search “needle”']);
    expect(why('late.ts')).toEqual(['Read with Tool']);
  });

  it('a call cut off by the end of its turn leaves no live output behind', () => {
    apply(turn('l1'), started('l1', 'c1', 'a.ts', 'execute'), output('l1', 'c1', 'building…'));
    expect(store.liveOut.has('c1')).toBe(true);
    apply(turnEnd('l1', 'interrupted'));
    expect(store.liveOut.size).toBe(0);
    expect(store.live.l1.segments[0].end).not.toBeNull();
  });

  it('a process that exits takes what it was writing with it; an old run’s late exit takes nothing', () => {
    apply(word('l1', 'i1', 'Half a rep'), output('l1', 'c9', 'building…'), word('w1', 'i2', 'mine'));
    apply(ev('l1', { type: 'exited', code: 1 }, { run_id: 'r0' }));
    expect([...store.streams.keys()]).toEqual(['i1', 'i2']);
    apply(ev('l1', { type: 'exited', code: 1 }, { run_id: 'r1' }));
    expect([...store.streams.keys()]).toEqual(['i2']); // the worker's stays
    expect(store.liveOut.size).toBe(0);
    expect(says(buildSpine('lead'))).toEqual([]);
  });
});

describe('the conversation, row by row', () => {
  it('another agent’s work changes no row; its own agent’s tool call changes one', () => {
    apply(yours('u1', 'Fix it'), turn('l1', 'Fix it'), ...read('l1', 'c1', 'a.ts'), said('l1', 'm1', 'Done.'), turnEnd('l1'));
    const first = keepRows([], buildSpine('lead'));
    expect(first.map((r) => r.kind)).toEqual(['user', 'work', 'say']);

    apply(word('w1', 'x', 'word'), ...read('w1', 'k1', 'z.ts'), said('w1', 'x', 'word.'));
    expect(keepRows(first, buildSpine('lead'))).toBe(first); // the very same list: nothing to update

    apply(turn('l1'), ...read('l1', 'c2', 'b.ts'));
    const second = keepRows(first, buildSpine('lead'));
    expect(second.map((r) => r.kind)).toEqual(['user', 'work', 'say', 'work']);
    expect(second.slice(0, 3).every((row, i) => row === first[i])).toBe(true);

    apply(...read('l1', 'c3', 'c.ts'));
    const third = keepRows(second, buildSpine('lead'));
    expect(third.filter((row, i) => row !== second[i]).map((r) => r.key)).toEqual(['w:c2']);
    expect(works(third)[1].segs.map((s) => s.callId)).toEqual(['c2', 'c3']);
  });

  it('a message being written builds the list when it starts and when it ends; its row reads the words', () => {
    apply(word('l1', 'i1', 'Hel'));
    const open = openStreams('l1');
    const rows = keepRows([], buildSpine('lead'));
    const row = says(rows)[0];
    expect(row).toMatchObject({ key: 'say:i1', streaming: true, text: 'Hel' });

    apply(word('l1', 'i1', 'lo'), word('w1', 'i2', 'other'));
    expect(openStreams('l1')).toBe(open); // no new list
    expect(streamText(row)).toBe('Hello');
    // A list built for another reason keeps the row: its words are not what the row is made of.
    expect(keepRows(rows, buildSpine('lead'))).toBe(rows);

    apply(said('l1', 'i1', 'Hello.'));
    expect(openStreams('l1')).not.toBe(open);
    const done = says(keepRows(rows, buildSpine('lead')))[0];
    expect(done).toMatchObject({ key: 'say:i1', streaming: false, text: 'Hello.' });
    expect(streamText(done)).toBe('Hello.');
  });

  it('a thought being written is read by its work row', () => {
    apply(turn('l1'), ...read('l1', 'c1', 'a.ts'), thought('l1', 't1', 'Let me'));
    const open = openStreams('l1');
    const rows = keepRows([], buildSpine('lead'));
    const row = works(rows)[0];
    expect(row.thinking).toBe('Let me');
    apply(thought('l1', 't1', ' see'));
    expect(openStreams('l1')).toBe(open);
    expect(thinkingText(row)).toBe('Let me see');
    expect(keepRows(rows, buildSpine('lead'))).toBe(rows);
    // A row with no thought shows none, whatever the agent writes later.
    expect(thinkingText({ ...row, thinking: null })).toBeNull();
  });

  it('who speaks is shown once for messages in a row, with or without work between them', () => {
    apply(said('l1', 'm1', 'One'), ...read('l1', 'c1', 'a.ts'), said('l1', 'm2', 'Two'), yours('u1', 'And this?'), said('l1', 'm3', 'Three'));
    expect(says(buildSpine('lead')).map((r) => [r.text, r.header])).toEqual([['One', true], ['Two', false], ['Three', true]]);
  });
});
