// What you said with an answer (a note, an answer, a changed plan) is a row of the record
// (`approval_said`): the store reads it live, from a snapshot after a restart, and from an older
// page of a thread, so the closed card and the history show it each time.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import { api } from '../api';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import type { Task } from '../gen/Task';
import type { Workspace } from '../gen/Workspace';
import { resolveApproval } from '../state/actions';
import { yourAnswers, yourWords } from '../state/answers';
import { store } from '../state/store.svelte';

// The store's transport: `subscribe` answers with the snapshot a test put here.
const core = vi.hoisted(() => ({ snapshot: null as unknown }));
vi.mock('../api', async (original) => ({
  ...(await original<typeof import('../api')>()),
  getTransport: async () => ({ subscribe: async () => core.snapshot }),
}));

const lead = mkAgent('lead1', 'Lead', 'claude', 'lead', 0, 'working', 'lead');
let offices = 0;
/** A snapshot of a new office: the store then starts from nothing, as after a restart. */
const snap = (o: Partial<Snapshot>): Snapshot =>
  ({
    as_of: Math.max(0, ...(o.events ?? []).map((e) => e.id)), clis: [], accounts: [], workspace: { id: `said-w${++offices}`, name: 'w', repo_path: '/code/w' } as Workspace,
    settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [], ...o,
  }) as unknown as Snapshot;
const sync = async (s: Snapshot) => {
  core.snapshot = s;
  await store.connect();
  expect(store.error).toBeNull();
};
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

// A worker's requests on task t1 (a task with no plan: the "lead" request's).
const at = (id: number, kind: unknown): EventRow => ({ id, ts: 1000 + id, agent_id: 'worker', task_id: 't1', run_id: 'r', kind: kind as EventKind });
const approval = (id: string, subject: Approval['subject'], status: Approval['status']) =>
  ({ id, agent_id: 'worker', run_id: 'r', task_id: 't1', call_id: null, subject, reason: null, choices: ['allow_once', 'deny'], risks: [], status, decided_by: status === 'pending' ? null : 'user', created_at: 1000 }) as Approval;
const command: Approval['subject'] = { kind: 'command', command: 'npm install', cwd: null };
const question: Approval['subject'] = { kind: 'question', questions: [{ question: 'Colour?' }] };
const plan: Approval['subject'] = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } };
const asked = (id: number, a: Approval) => at(id, { type: 'approval_requested', approval: a });
const said = (id: number, a: string, o: { note?: string; answer?: string; plan_changed?: boolean }) =>
  at(id, { type: 'approval_said', approval_id: a, note: o.note ?? null, answer: o.answer ?? null, plan_changed: o.plan_changed ?? false });
const answered = (id: number, a: string, status: Approval['status']) => at(id, { type: 'approval_resolved', approval_id: a, status, by: 'user' });
const t1 = { id: 't1', plan_id: null, status: 'done', assignee: 'worker' } as Task;

afterEach(() => {
  vi.restoreAllMocks();
  yourAnswers.clear();
  yourWords.clear();
});

describe('what you said with an answer', () => {
  it('is read again from the record after a restart: the note, the answer and the changed plan', async () => {
    await sync(snap({
      tasks: [t1],
      events: [
        asked(1, approval('ap-note', command, 'denied')), said(2, 'ap-note', { note: 'use pnpm' }), answered(3, 'ap-note', 'denied'),
        asked(4, approval('ap-q', question, 'approved')), said(5, 'ap-q', { answer: 'Red' }), answered(6, 'ap-q', 'approved'),
        asked(7, approval('ap-plan', plan, 'approved')), said(8, 'ap-plan', { plan_changed: true }), answered(9, 'ap-plan', 'approved'),
        // Answered without words: one row, and nothing to show.
        asked(10, approval('ap-plain', command, 'approved')), answered(11, 'ap-plain', 'approved'),
      ],
    }));
    expect([...yourWords]).toEqual([['ap-note', { note: 'use pnpm', planChanged: false }], ['ap-plan', { note: null, planChanged: true }]]);
    expect([...yourAnswers]).toEqual([['ap-q', 'Red']]);
    // The closed cards are there to show it on.
    expect(store.approvals.map((a) => [a.id, a.status])).toEqual([['ap-note', 'denied'], ['ap-q', 'approved'], ['ap-plan', 'approved'], ['ap-plain', 'approved']]);
  });

  it('is read from the live rows, which come just before the answer', async () => {
    await sync(snap({ tasks: [t1], approvals: [approval('ap1', command, 'pending')] }));
    apply(said(1, 'ap1', { note: 'use pnpm' }));
    expect(yourWords.get('ap1')).toEqual({ note: 'use pnpm', planChanged: false });
    apply(answered(2, 'ap1', 'denied'));
    expect(store.approvals.map((a) => [a.id, a.status, a.decided_by])).toEqual([['ap1', 'denied', 'user']]);
    expect(yourAnswers.size).toBe(0);
  });

  it('is read from an older page of the thread ("Load earlier")', async () => {
    await sync(snap({ tasks: [t1], events: [{ ...at(10, { type: 'reasoning', item_id: 'r10', text: 'hm' }), agent_id: 'lead1', task_id: null }] }));
    // The core's page, newest first. The approval comes as it ended.
    vi.spyOn(api, 'threadEvents').mockResolvedValue([answered(9, 'ap0', 'denied'), said(8, 'ap0', { note: 'not on main' }), asked(7, approval('ap0', command, 'denied'))]);
    await store.loadEarlier('lead');
    expect(store.approvals.map((a) => [a.id, a.status])).toEqual([['ap0', 'denied']]);
    expect([...yourWords]).toEqual([['ap0', { note: 'not on main', planChanged: false }]]);
  });
});

describe('what this window keeps of the answer it sent', () => {
  it('leaves the line the core recorded while the answer was on its way', async () => {
    await sync(snap({ tasks: [t1], approvals: [approval('ap-q', question, 'pending')] }));
    // The core's row comes before the call returns: its words are the record.
    vi.spyOn(api, 'resolveApproval').mockImplementation(async () => apply(said(1, 'ap-q', { answer: 'Red, the brand red' })));
    expect(await resolveApproval('ap-q', 'allow_once', null, { 'Colour?': 'Red' })).toBe(true);
    expect([...yourAnswers]).toEqual([['ap-q', 'Red, the brand red']]);
  });

  it('keeps the answer to a question until the core’s row comes', async () => {
    await sync(snap({ tasks: [t1], approvals: [approval('ap-q', question, 'pending')] }));
    vi.spyOn(api, 'resolveApproval').mockResolvedValue();
    await resolveApproval('ap-q', 'allow_once', null, { 'Colour?': 'Red' });
    expect([...yourAnswers]).toEqual([['ap-q', 'Red']]);
  });

  it('stores no plan text as an answer when you accept a plan with your changes', async () => {
    await sync(snap({ tasks: [t1], approvals: [approval('ap-plan', plan, 'pending')] }));
    vi.spyOn(api, 'resolveApproval').mockResolvedValue();
    await resolveApproval('ap-plan', 'allow_once', null, { plan: '1. Add the line\n2. Test it' });
    expect(yourAnswers.size).toBe(0);
  });
});
