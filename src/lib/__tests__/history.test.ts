// The conversation history never gets shorter: a resync of the open office merges into what the
// store holds, and a thread loads what is older than the snapshot on request ("Load earlier").
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildSpine } from '../../home/spine';
import { mkAgent } from '../../office/demo/script';
import { api } from '../api';
import type { Approval } from '../gen/Approval';
import type { ChatMessage } from '../gen/ChatMessage';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import type { Task } from '../gen/Task';
import type { Workspace } from '../gen/Workspace';
import { LOG_CAP, store } from '../state/store.svelte';

// The store's transport: `subscribe` answers with the snapshot a test put here.
const core = vi.hoisted(() => ({ snapshot: null as unknown }));
vi.mock('../api', async (original) => ({
  ...(await original<typeof import('../api')>()),
  getTransport: async () => ({ subscribe: async () => core.snapshot }),
}));

const lead = mkAgent('lead1', 'Lead', 'claude', 'lead', 0, 'working', 'lead');
let offices = 0;
/** A snapshot of a new office (the store then starts from nothing), or of `o.workspace`. */
const snap = (o: Partial<Snapshot>): Snapshot =>
  ({
    as_of: Math.max(0, ...(o.events ?? []).map((e) => e.id)), clis: [], accounts: [], workspace: { id: `w${++offices}`, name: 'w', repo_path: '/code/w' } as Workspace,
    settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [], ...o,
  }) as unknown as Snapshot;
/** What `connect` and `resync` do: take the core's snapshot in. */
const sync = async (s: Snapshot): Promise<Snapshot> => {
  core.snapshot = s;
  await store.connect();
  expect(store.error).toBeNull();
  return s;
};
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

// Stored events: the id says when (ts = 1000 + id). The Lead's own, unless said otherwise.
const at = (id: number, kind: unknown, o: Partial<EventRow> = {}): EventRow => ({ id, ts: 1000 + id, agent_id: 'lead1', task_id: null, run_id: null, kind: kind as EventKind, ...o });
const said = (id: number, item: string) => at(id, { type: 'message', item_id: item, text: item, phase: 'final' });
const thought = (id: number) => at(id, { type: 'reasoning', item_id: `r${id}`, text: 'hm' });
const started = (id: number, call: string) =>
  at(id, { type: 'tool_started', call_id: call, name: 'Read', activity: 'read', target: { path: 'a.ts', paths: [], command: null, query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' });
const completed = (id: number, call: string) => at(id, { type: 'tool_completed', call_id: call, ok: true, exit_code: null, tests_passed: null, tests_failed: null, output: null });
const message = (id: string, thread: string, ts: number): ChatMessage => ({ id, thread, author: 'user', text: id, attachment: null, ts });
const chat = (id: number, thread: string) => at(id, { type: 'chat', message: message(`c${id}`, thread, 1000 + id) }, { agent_id: null });
// A worker's approval and the check of its task t1 (a task with no plan: the "lead" request's).
const approval = (id: string, status: Approval['status'] = 'pending') =>
  ({ id, agent_id: 'worker', run_id: 'r', task_id: 't1', call_id: null, subject: { kind: 'command', command: 'ls', cwd: null }, reason: null, choices: ['allow_once', 'deny'], risks: [], status, decided_by: null, created_at: 1000 }) as Approval;
const asked = (id: number, a: Approval) => at(id, { type: 'approval_requested', approval: a }, { agent_id: 'worker', task_id: 't1' });
const answered = (id: number, a: string, status: Approval['status']) => at(id, { type: 'approval_resolved', approval_id: a, status, by: 'user' }, { agent_id: 'worker', task_id: 't1' });
const checked = (id: number) => at(id, { type: 'verified', command: 'pnpm test', ok: true, exit_code: 0, tail: '' }, { agent_id: null, task_id: 't1' });
const t1 = { id: 't1', plan_id: null, status: 'done', assignee: 'worker' } as Task;

const messages = () => store.live.lead1.messages.map((m) => m.itemId);
const approvals = () => store.approvals.map((a) => [a.id, a.status]);

afterEach(() => vi.restoreAllMocks());

describe('a resync', () => {
  it('of the open office keeps what is held, adds what was missed, and does not build the office again', async () => {
    const first = await sync(snap({ events: [started(1, 'c1'), completed(2, 'c1'), said(3, 'm1')] }));
    const epoch = store.epoch;
    const told: number[] = [];
    const off = store.onEvent((e) => told.push(e.id));
    apply(asked(4, approval('ap1')), answered(5, 'ap1', 'approved'), checked(6));
    // Back after a hidden period: the snapshot holds its newest events only, and no answered approval.
    await sync(snap({ workspace: first.workspace, events: [checked(6), said(7, 'm2'), started(8, 'c2')] }));
    off();

    expect(store.epoch).toBe(epoch);
    expect(told).toEqual([4, 5, 6, 7, 8]); // the missed ones reach the listeners (the office) like live ones
    expect(store.events.map((e) => e.id)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(store.live.lead1.segments.map((s) => [s.callId, s.end])).toEqual([['c1', 1002], ['c2', null]]);
    expect(messages()).toEqual(['m1', 'm2']);
    expect(approvals()).toEqual([['ap1', 'approved']]);
    expect(store.taskMarks.t1.map((m) => m.eventId)).toEqual([6]);
  });

  it('reads the events between what is held and the snapshot when the snapshot starts after them', async () => {
    const first = await sync(snap({ events: [said(1, 'm1'), said(2, 'm2')] }));
    const epoch = store.epoch;
    const between = vi.spyOn(api, 'events').mockResolvedValue([said(4, 'm4'), said(3, 'm3'), said(2, 'm2'), said(1, 'm1')]);
    await sync(snap({ workspace: first.workspace, events: [said(5, 'm5'), said(6, 'm6')] }));
    expect(between).toHaveBeenCalledWith(null, 5, LOG_CAP);
    expect(messages()).toEqual(['m1', 'm2', 'm3', 'm4', 'm5', 'm6']);
    expect(store.epoch).toBe(epoch);
  });

  it('starts from the snapshot when too much was missed, or when it is another office’s', async () => {
    const first = await sync(snap({ events: [said(1, 'm1')] }));
    const epoch = store.epoch;
    // A full page of missed events and still none of ours.
    vi.spyOn(api, 'events').mockResolvedValue(Array.from({ length: LOG_CAP }, (_, i) => thought(LOG_CAP + 10 - i)));
    await sync(snap({ workspace: first.workspace, events: [said(9000, 'm2')] }));
    expect(messages()).toEqual(['m2']);
    expect(store.epoch).toBe(epoch + 1);

    await sync(snap({ events: [said(9001, 'm3')] }));
    expect(messages()).toEqual(['m3']);
    expect(store.epoch).toBe(epoch + 2);
  });

  it('keeps messages older than the snapshot’s, and lets go of a closed request’s', async () => {
    const first = await sync(snap({ chats: [message('a1', 'lead', 1), message('a2', 'lead', 2), message('x1', 'lead:x', 1), message('x2', 'lead:x', 2), message('x3', 'lead:x', 3)] }));
    // "lead" was closed (its messages moved to its archive). The snapshot has the newest two of "lead:x".
    await sync(snap({ workspace: first.workspace, chats: [message('x3', 'lead:x', 3), message('x4', 'lead:x', 4)] }));
    expect(store.chats.map((c) => c.id)).toEqual(['x1', 'x2', 'x3', 'x4']);
  });
});

describe('a snapshot', () => {
  it('shows the approvals answered in its events, not only the ones that wait', async () => {
    await sync(snap({ approvals: [approval('ap2')], events: [asked(1, approval('ap1')), answered(2, 'ap1', 'denied'), asked(3, approval('ap2'))] }));
    expect(approvals()).toEqual([['ap2', 'pending'], ['ap1', 'denied']]);
  });
});

describe('load earlier', () => {
  it('puts a thread’s older work, messages, approvals and finished tasks above what is shown', async () => {
    const first = await sync(snap({ tasks: [t1], events: [completed(10, 'c0'), said(11, 'm2'), started(12, 'c2')] }));
    expect(store.live.lead1.segments.map((s) => s.callId)).toEqual(['c2']); // c0 started before the snapshot's events
    // The core's page, newest first. The approval comes as it ended.
    const page = vi.spyOn(api, 'threadEvents').mockResolvedValue([asked(9, approval('ap0', 'denied')), checked(8), chat(7, 'lead'), said(6, 'm1'), started(5, 'c0')]);
    await store.peekEarlier('lead');
    expect(page).toHaveBeenLastCalledWith('lead', 'lead1', 10, 1);
    expect(store.earlier.lead).toEqual({ more: true, loading: false });

    await store.loadEarlier('lead');
    expect(page).toHaveBeenLastCalledWith('lead', 'lead1', 10, 500);
    // The call that started on the older page ends where the newer events say.
    expect(store.live.lead1.segments.map((s) => [s.callId, s.end])).toEqual([['c0', 1010], ['c2', null]]);
    expect(messages()).toEqual(['m1', 'm2']);
    expect(store.chats.map((c) => c.id)).toEqual(['c7']);
    expect(approvals()).toEqual([['ap0', 'denied']]);
    expect(buildSpine('lead').map((i) => i.kind)).toEqual(['approval', 'work', 'say', 'user', 'task-done', 'say', 'work']);
    expect(store.earlier.lead).toEqual({ more: false, loading: false }); // a short page: that was all
    expect(store.events.map((e) => e.id)).toEqual([10, 11, 12]); // the log (timeline, inspector) is not this thread's

    // What comes live afterwards lands on it, no cap drops its rows again, and a resync keeps it.
    apply(completed(13, 'c2'), ...Array.from({ length: 61 }, (_, i) => thought(14 + i)));
    expect(store.live.lead1.segments[1].end).toBe(1013);
    expect(store.live.lead1.reasoning).toHaveLength(61);
    await sync(snap({ workspace: first.workspace, tasks: [t1], chats: [message('c7', 'lead', 1007)], events: [thought(74)] }));
    expect(messages()).toEqual(['m1', 'm2']);
    expect(store.chats.map((c) => c.id)).toEqual(['c7']);
    expect(approvals()).toEqual([['ap0', 'denied']]);
  });

  it('reaches back past what is on screen when the log was trimmed, so the thread never gets shorter', async () => {
    await sync(snap({ events: [said(100, 'old'), said(2000, 'new')] }));
    store.events = store.events.slice(-1); // a long session: the log keeps its newest events only
    const full = Array.from({ length: 500 }, (_, i) => thought(1999 - i));
    const page = vi.spyOn(api, 'threadEvents').mockResolvedValueOnce(full).mockResolvedValueOnce([said(100, 'old'), said(50, 'older')]);
    await store.loadEarlier('lead');
    expect(page.mock.calls.map((c) => c[2])).toEqual([2000, 1500]);
    expect(messages()).toEqual(['older', 'old', 'new']);
  });

  it('is offered again for rows a cap took off the screen', async () => {
    await sync(snap({ events: [said(10, 'm')] }));
    vi.spyOn(api, 'threadEvents').mockResolvedValue([]);
    await store.peekEarlier('lead');
    expect(store.earlier.lead).toEqual({ more: false, loading: false });
    apply(...Array.from({ length: 61 }, (_, i) => thought(11 + i)));
    expect(store.live.lead1.reasoning).toHaveLength(60);
    expect(store.earlier.lead).toBeUndefined(); // the thread asks again
    await store.peekEarlier('lead');
    expect(store.earlier.lead).toEqual({ more: true, loading: false });
    await store.loadEarlier('lead');
    expect(store.live.lead1.reasoning).toHaveLength(61);
    expect(store.earlier.lead).toEqual({ more: false, loading: false });
  });
});
