// The store and the app's event stream: `subscribe` installs the batch sink first and answers
// with a snapshot later, so batches arrive before, while and after a snapshot is taken, from a
// stream that may already be an old one. Each event is applied once, in order, on the state it
// belongs to.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import type { UiBatch } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import type { Workspace } from '../gen/Workspace';
import { store } from '../state/store.svelte';

/** One `subscribe` the store made: its batch sink, and the two ways the app answers it. */
type Sub = { batch: (b: UiBatch) => void; open: (s: Snapshot) => void; fail: (why: unknown) => void };
const core = vi.hoisted(() => ({ subs: [] as Sub[] }));
vi.mock('../api', async (original) => ({
  ...(await original<typeof import('../api')>()),
  getTransport: async () => ({
    subscribe: (batch: Sub['batch']) => new Promise<Snapshot>((open, fail) => core.subs.push({ batch, open, fail })),
  }),
}));

const lead = mkAgent('lead1', 'Lead', 'claude', 'lead', 0, 'working', 'lead');
let offices = 0;
/** A snapshot of a new office (the store then starts from nothing), or of `o.workspace`. */
const snap = (o: Partial<Snapshot>): Snapshot =>
  ({
    as_of: Math.max(0, ...(o.events ?? []).map((e) => e.id)), clis: [], accounts: [], workspace: { id: `w${++offices}`, name: 'w', repo_path: '/code/w' } as Workspace,
    settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [], ...o,
  }) as unknown as Snapshot;
/** The Lead's message `item`, stored as event `id`. */
const said = (id: number, item: string): EventRow => ({ id, ts: 1000 + id, agent_id: 'lead1', task_id: null, run_id: null, kind: { type: 'message', item_id: item, text: item, phase: 'final' } as EventKind });

/** Start a connect; resolves once the store has subscribed. `done`: the connect itself. */
const subscribe = async (): Promise<Sub & { done: Promise<void> }> => {
  const n = core.subs.length;
  const done = store.connect();
  await vi.waitFor(() => expect(core.subs).toHaveLength(n + 1));
  return { ...core.subs[n], done };
};
const messages = () => store.live.lead1.messages.map((m) => m.itemId);
const ids = () => store.events.map((e) => e.id);

beforeEach(() => {
  core.subs.length = 0;
});

describe('the event stream', () => {
  it('holds the events that come before the snapshot, then applies the new ones in order', async () => {
    const sub = await subscribe();
    const told: number[] = [];
    const off = store.onEvent((e) => told.push(e.id));
    sub.batch({ events: [said(2, 'm2'), said(3, 'm3')] });
    sub.batch({ events: [said(4, 'm4')] });
    expect(told).toEqual([]); // not applied to a state the snapshot is about to replace
    // The snapshot was taken after event 2: it holds that one already.
    sub.open(snap({ as_of: 2, events: [said(1, 'm1'), said(2, 'm2')] }));
    await sub.done;

    expect([store.ready, store.error]).toEqual([true, null]);
    expect(told).toEqual([3, 4]);
    expect(ids()).toEqual([1, 2, 3, 4]);
    expect(messages()).toEqual(['m1', 'm2', 'm3', 'm4']);
    // From here on a batch is applied when it arrives. One sent twice changes nothing.
    sub.batch({ events: [said(5, 'm5')] });
    sub.batch({ events: [said(5, 'm5')] });
    off();
    expect(told).toEqual([3, 4, 5]);
    expect(messages()).toEqual(['m1', 'm2', 'm3', 'm4', 'm5']);
  });

  it('takes a fresh snapshot when the app says events were skipped, and drops what the old stream still sends', async () => {
    const first = await subscribe();
    const office = snap({ events: [said(1, 'm1')] });
    first.open(office);
    await first.done;
    first.batch({ events: [said(2, 'm2')] });

    first.batch({ events: [], resync: true });
    await vi.waitFor(() => expect(core.subs).toHaveLength(2));
    const second = core.subs[1];
    first.batch({ events: [said(9, 'late')] }); // the old stream: no longer ours
    second.batch({ events: [said(5, 'm5')] }); // the new one: waits for its snapshot
    expect(messages()).toEqual(['m1', 'm2']);
    second.open(snap({ workspace: office.workspace, as_of: 4, events: [said(1, 'm1'), said(2, 'm2'), said(3, 'm3'), said(4, 'm4')] }));
    await vi.waitFor(() => expect(messages()).toEqual(['m1', 'm2', 'm3', 'm4', 'm5']));

    expect(ids()).toEqual([1, 2, 3, 4, 5]);
    expect(store.error).toBeNull();
  });

  it('never lets the answer of an old connect take the place of a newer one', async () => {
    // Two connects in a row (the window came back while the first one was still loading).
    const old = await subscribe();
    const current = await subscribe();
    current.open(snap({ events: [said(1, 'current')] }));
    await current.done;
    old.open(snap({ events: [said(1, 'old')] }));
    await old.done;
    expect(messages()).toEqual(['current']);

    // An old connect that fails late puts no error over a screen that works.
    const failing = await subscribe();
    const working = await subscribe();
    working.open(snap({ events: [said(1, 'working')] }));
    await working.done;
    failing.fail(new Error('the stream closed'));
    await failing.done;
    expect([store.error, messages()]).toEqual([null, ['working']]);
  });

  it('shows why a start failed, and a connect that works clears it', async () => {
    const failed = await subscribe();
    failed.fail(new Error('unable to open database file'));
    await failed.done;
    expect([store.ready, store.error]).toEqual([true, 'unable to open database file']); // the error screen, not the loading one

    // The app's commands reject with plain text.
    const again = await subscribe();
    again.fail('the core is still starting');
    await again.done;
    expect(store.error).toBe('the core is still starting');

    const ok = await subscribe();
    ok.open(snap({ events: [said(1, 'm1')] }));
    await ok.done;
    expect([store.ready, store.error, messages()]).toEqual([true, null, ['m1']]);
  });
});
