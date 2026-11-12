import { describe, expect, it, vi } from 'vitest';
import { buildSpine } from '../../home/spine';
import { mkAgent } from '../../office/demo/script';
import { emptyModel, reduce, syncAgents } from '../../office/reducer';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Office } from '../gen/Office';
import type { Snapshot } from '../gen/Snapshot';
import type { Task } from '../gen/Task';
import type { Workspace } from '../gen/Workspace';
import { installNotifications } from '../state/notify';
import { offices } from '../state/offices.svelte';
import { STEER_GRACE, waitingOf, type Sent, type Turn } from '../state/queue';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';

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

// The store is one singleton: each test puts back what it set.
let eid = 100;
const row = (kind: unknown, o: Partial<EventRow> = {}): EventRow => ({ id: ++eid, ts: eid, agent_id: null, task_id: null, run_id: null, kind: kind as EventKind, ...o });
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
const office = (id: string) => ({ id, name: id, repo_path: `/code/${id}` }) as Workspace;

describe('offices', () => {
  it('the_outbox_is_per_office', () => {
    store.setWorkspace(office('a'));
    const id = store.queueOutgoing('lead', 'x', 1);
    store.setWorkspace(office('b'));
    expect(store.outbox).toHaveLength(0);
    store.setWorkspace(office('a'));
    expect(store.outbox.map((o) => o.id)).toEqual([id]);
    store.dropOutgoing(id);
    store.setWorkspace(null);
  });

  it('a task that was already failed does not toast again', () => {
    const off = installNotifications();
    const t1 = { id: 't1', key: 't1', title: 'T', status: 'failed', status_reason: null, assignee: 'a1' } as Task;
    store.upsertTask(t1);
    // Bookkeeping on a failed task (its member left the team): still failed, nothing new to say.
    apply(row({ type: 'task_changed', task: { ...t1, assignee: null } }));
    expect(toasts.list).toHaveLength(0);
    apply(row({ type: 'task_changed', task: { ...t1, id: 't2', key: 't2', status: 'running' } }));
    apply(row({ type: 'task_changed', task: { ...t1, id: 't2', key: 't2' } }));
    expect(toasts.list.map((t) => t.title)).toEqual(['t2 failed']);
    off();
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.tasks = [];
  });

  it("another office's approval raises a toast that names the office, and goes when it is answered", () => {
    vi.useFakeTimers();
    const refresh = vi.spyOn(offices, 'refresh').mockResolvedValue();
    const off = installNotifications();
    store.setWorkspace(office('a'));
    offices.others = [{ id: 'b', name: 'billing-api', repo_path: '/code/b', crew: 1, working: 1, needs_you: 0, home: [] } satisfies Office];
    const ap = { id: 'ap1', agent_id: 'w9', task_id: null, subject: { kind: 'command', command: 'rm -rf dist', cwd: null }, status: 'pending' } as Approval;
    apply(row({ type: 'approval_requested', approval: ap }, { workspace_id: 'b', agent_id: 'w9' }));
    expect(toasts.list.map((t) => t.title)).toEqual(['billing-api: An agent needs your approval']);
    expect(store.approvals).toEqual([]); // not this office's: only pointed at
    vi.advanceTimersByTime(1000);
    expect(refresh).toHaveBeenCalled(); // the tab's count follows without waiting for the poll
    // One toast per office, however much waits there.
    apply(row({ type: 'approval_requested', approval: { ...ap, id: 'ap2' } }, { workspace_id: 'b', agent_id: 'w9' }));
    expect(toasts.list.map((t) => t.title)).toEqual(['billing-api: 2 waiting for you']);
    apply(row({ type: 'approval_resolved', approval_id: 'ap2', status: 'approved', by: 'user' }, { workspace_id: 'b' }));
    expect(toasts.list.map((t) => t.title)).toEqual(['billing-api: An agent needs your approval']);
    apply(row({ type: 'approval_resolved', approval_id: 'ap1', status: 'approved', by: 'user' }, { workspace_id: 'b' }));
    expect(toasts.list).toHaveLength(0);
    off();
    vi.runAllTimers();
    vi.useRealTimers();
    refresh.mockRestore();
    offices.others = [];
    store.setWorkspace(null);
  });
});

describe('a run that is not the agent’s', () => {
  it("a dead run's late status does not change the agent", () => {
    // The start timed out (run r1 failed); the retry is starting on r2 when the dead CLI comes up.
    const w = { ...mkAgent('w1', 'Ada', 'claude', 'worker', 1, 'starting'), run_id: 'r2' };
    const usage = { ...w.usage, input_tokens: 9 };
    const late = [
      row({ type: 'session_started', provider: 'claude', model: 'old', cwd: '/old', cli_version: null, session_id: null, billing: 'unknown', tools: [] }, { agent_id: 'w1', run_id: 'r1' }),
      row({ type: 'usage_updated', usage }, { agent_id: 'w1', run_id: 'r1' }),
      row({ type: 'status_changed', status: 'offline' }, { agent_id: 'w1', run_id: 'r1' }),
      row({ type: 'exited', code: 1 }, { agent_id: 'w1', run_id: 'r1' }),
    ];

    store.agents = [w];
    apply(...late);
    expect(store.agents[0].status).toBe('starting');
    expect(store.agents[0].usage.input_tokens).toBe(0);
    expect(store.live.w1?.session ?? null).toBeNull();
    apply(row({ type: 'status_changed', status: 'idle' }, { agent_id: 'w1', run_id: 'r2' }));
    expect(store.agents[0].status).toBe('idle'); // its own run still drives it
    store.agents = [];
    store.live = {};

    // The office view folds the same events.
    const m = emptyModel();
    syncAgents(m, [w], 1);
    const a = m.agents.get('w1')!;
    reduce(m, row({ type: 'tool_started', call_id: 'c1', name: 'Read', activity: 'read', target: { path: 'a.ts', paths: [], command: null, query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' }, { agent_id: 'w1', run_id: 'r2' }), { replay: false });
    for (const ev of late) reduce(m, ev, { replay: false });
    expect(a.status).toBe('working');
    expect(a.present).toBe(true);
    expect(a.tools.size).toBe(1);
  });

  it('a delta for a message already in the snapshot makes no second item', () => {
    const lead = { ...mkAgent('l1', 'Lead', 'claude', 'lead', 0, 'working', 'lead'), run_id: 'r1' };
    const said = row({ type: 'message', item_id: 'm1', text: 'Done.', phase: 'final' }, { agent_id: 'l1', run_id: 'r1' });
    const snap = { as_of: said.id, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [said] };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
    // Buffered while the snapshot was on its way: the tail of that message's stream.
    apply(row({ type: 'text_delta', item_id: 'm1', text: 'ne.' }, { id: 0, agent_id: 'l1', run_id: 'r1' }));
    expect(buildSpine('lead').filter((i) => i.key === 'say:m1')).toHaveLength(1);
    expect(store.now('l1')?.activity).not.toBe('replying');
    store.agents = [];
    store.live = {};
    store.streams = new Map();
  });

  it('a plan merged before a reload still shows as merged', () => {
    // After a reload (or a switch of office and back) the merge command's event is not in the
    // snapshot's events: the plan itself carries it.
    const plan = { id: 'p1', status: 'done', merged_sha: 'abc1234def', kept: false };
    const snap = { as_of: 1, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [], plans: [plan], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
    expect(store.mergeOf('p1')?.sha).toBe('abc1234def');
    store.plans = [];
  });
});
