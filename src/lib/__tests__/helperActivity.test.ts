// What an agent does now (`store.now`, read by the agent sheet's "Now", its Why and its live
// output) when one of its helpers makes a call. The fourth and fifth browser reviews: after
// Eli's turn ended with a helper at work in the background, the sheet said "Now: Searching… for
// 0:56" with "Interrupt". A helper's call is never the agent's own (`seg.parent`).
import { beforeEach, describe, expect, it } from 'vitest';
import type { AgentStatus } from '../gen/AgentStatus';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import { store } from '../state/store.svelte';
import { mkAgent } from '../../office/demo/script';

const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
/** A fresh store with Eli, a Claude 1:1 agent, in `status`. */
const load = (status: AgentStatus) =>
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot({
    as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [{ ...mkAgent('e1', 'Eli', 'claude', 'solo', 3, status), run_id: 'r1' }],
    plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [],
  } as unknown as Snapshot);

let id = 0;
beforeEach(() => {
  id = 0;
});
const ev = (ts: number, kind: unknown): EventRow => ({ id: ++id, ts, agent_id: 'e1', task_id: null, run_id: 'r1', kind: kind as EventKind });
const target = { path: null, paths: [], command: null, query: 'docs/**/*.md', url: null, server: null, tool: null };
const call = (ts: number, callId: string, name: string, activity: string) => ev(ts, { type: 'tool_started', call_id: callId, name, activity, target, input: {}, confidence: 'observed' });
/** A turn that starts a helper in the background (`h1`), and the helper's Glob call (`g1`), linked to it as the Claude adapter links it. */
const helperAtWork = () => [
  ev(10, { type: 'turn_started', input: 'Look at the docs in the background' }),
  ev(11, { type: 'sub_agent', call_id: 'h1', description: 'Look at the docs', status: 'running' }),
  call(12, 'g1', 'Glob', 'search'),
  ev(12, { type: 'sub_agent_progress', call_id: 'h1', name: null, step: null, tool_call_id: 'g1', tool_uses: null, tokens: null, report: null }),
];

describe('what an agent does now while its helper works', () => {
  it('after its turn ended with the helper’s Glob call open, it does nothing of its own', () => {
    load('idle');
    apply(...helperAtWork(), ev(14, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 4 }));
    // (the helper's call stays open on purpose: the helper still works)
    expect(Object.keys(store.live.e1?.open ?? {})).toEqual(['g1']);
    expect(store.now('e1')).toBeNull();
  });

  it('after its turn ended, it rests also while the helper’s calls set it “working” or “thinking”', () => {
    // The Claude adapter sets the agent's state for every tool call, a helper's too.
    load('working');
    apply(...helperAtWork(), ev(14, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 4 }));
    expect(store.now('e1')).toBeNull();
    apply(ev(15, { type: 'status_changed', status: 'thinking' }));
    expect(store.agentMap.get('e1')?.status).toBe('thinking');
    expect(store.now('e1')).toBeNull();
  });

  it('while its turn runs and only the helper’s call is open, it waits for its helper', () => {
    load('working');
    apply(...helperAtWork());
    expect(store.now('e1')).toEqual({ activity: 'delegate', label: 'Look at the docs', since: 12, seg: null });
  });

  it('while its turn runs, its own call is what it does, also beside a newer call of its helper', () => {
    load('working');
    apply(ev(9, { type: 'turn_started', input: 'Test it' }), call(9, 'c1', 'Bash', 'execute'), ...helperAtWork().slice(1));
    const now = store.now('e1');
    expect(now?.seg?.callId).toBe('c1');
    expect(now?.activity).toBe('execute');
  });
});
