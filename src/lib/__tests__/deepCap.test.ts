// A thread loaded far back keeps its agent's events apart from the capped log. That store has a
// cap of its own: past it the oldest go, and "Load earlier" reads them again, one page.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import { DEEP_CAP, store } from '../state/store.svelte';
import { mkAgent } from '../../office/demo/script';

const bo = mkAgent('a1', 'Bo', 'claude', 'solo', 0, 'idle');
const apply = (evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
const held = () => (store as unknown as { deep: Map<string, EventRow[]> }).deep.get('a1') ?? [];
const said = (id: number): EventRow => ({ id, ts: id, agent_id: 'a1', task_id: null, run_id: null, kind: { type: 'notice', text: `told ${id}` } as EventKind });

afterEach(() => vi.restoreAllMocks());

describe('the events held for a thread loaded back', () => {
  it('keep the newest DEEP_CAP; the thread can load the rest again, one page from there', async () => {
    const snap = {
      as_of: 1000, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [bo], plans: [], tasks: [], approvals: [], chats: [],
      memories: [], triples: [], artifacts: [], events: [said(999), said(1000)],
    };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
    const pages = vi.spyOn(api, 'threadEvents').mockResolvedValue([said(998)]);
    await store.loadEarlier('agent:a1');
    expect(held().map((e) => e.id)).toEqual([998, 999, 1000]);
    expect(store.earlier['agent:a1']).toEqual({ more: false, loading: false });

    const more: EventRow[] = [];
    for (let id = 1001; id <= 1000 + DEEP_CAP * 1.2 + 1; id++) more.push(said(id));
    apply(more);
    const kept = held();
    expect(kept).toHaveLength(DEEP_CAP);
    expect(kept[kept.length - 1].id).toBe(more[more.length - 1].id);
    // The agent's state is built from what is held.
    expect(store.live.a1.firstTs).toBe(kept[0].ts);
    expect(store.earlier['agent:a1']).toEqual({ more: true, loading: false });

    // One full page from where it holds now, and no more: its state starts there.
    pages.mockClear();
    const first = kept[0].id;
    pages.mockResolvedValue(Array.from({ length: 500 }, (_, i) => said(first - 1 - i)));
    await store.loadEarlier('agent:a1');
    expect(pages).toHaveBeenCalledTimes(1);
    expect(pages.mock.calls[0][2]).toBe(first);
    expect(held()[0].id).toBe(first - 500);
    expect(store.earlier['agent:a1']).toEqual({ more: true, loading: false });
  });
});
