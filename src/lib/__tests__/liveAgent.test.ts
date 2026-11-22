import { describe, expect, it } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import type { Agent } from '../gen/Agent';
import type { EventRow } from '../gen/EventRow';
import { store } from '../state/store.svelte';

// The core saves what an agent is doing right now (and its usage) without a row in the event log:
// that change arrives as an `agent_changed` with id 0.
const live = (agent: Agent): EventRow => ({ id: 0, ts: agent.updated_at, agent_id: agent.id, task_id: null, run_id: null, kind: { type: 'agent_changed', agent } });
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
// The store is one singleton: each test puts back what it set.
const reset = () => {
  store.agents = [];
  store.live = {};
};
const ada = (o: Partial<Agent>): Agent => ({ ...mkAgent('w1', 'Ada', 'claude', 'worker', 1, 'working'), ...o });

describe("an agent's live change", () => {
  it('shows what the agent is doing now', () => {
    store.agents = [ada({ updated_at: 10 })];
    const seen: string[] = [];
    const off = store.onEvent((ev) => seen.push(ev.kind.type));
    apply(live(ada({ activity: 'edit', activity_label: 'app.ts', updated_at: 11 })));
    expect(store.agents[0].activity_label).toBe('app.ts');
    expect(seen).toEqual(['agent_changed']);
    off();
    reset();
  });

  it('one that waited for the snapshot does not undo it', () => {
    // The snapshot already has the end of the turn; the tool call before it was still in the queue.
    store.agents = [ada({ status: 'idle', updated_at: 20 })];
    const seen: string[] = [];
    const off = store.onEvent((ev) => seen.push(ev.kind.type));
    apply(live(ada({ status: 'working', activity: 'edit', activity_label: 'app.ts', updated_at: 19 })));
    expect(store.agents[0].status).toBe('idle');
    expect(store.agents[0].activity_label).toBeNull();
    expect(seen).toEqual([]);
    off();
    reset();
  });

  it('does not bring back an agent that was let go', () => {
    store.agents = [];
    apply(live(ada({ activity: 'edit', updated_at: 30 })));
    expect(store.agents).toEqual([]);
    reset();
  });
});
