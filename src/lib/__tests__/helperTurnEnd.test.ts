// Which helpers (subagents) are still at work after a turn that was interrupted or that failed.
// A Claude agent's adapter says it: a `sub_agent` row "stopped" for each helper that ends with
// the turn (the ones whose `Agent` call still waited), and nothing for a helper in the
// background, which works on. For another CLI no such row is known, so the end of the turn
// stops its helpers. The end of the process stops every helper of every CLI.
import { beforeEach, describe, expect, it } from 'vitest';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import { store } from '../state/store.svelte';
import { mkAgent } from '../../office/demo/script';

const claude = { ...mkAgent('c1', 'Bo', 'claude', 'solo', 0, 'working'), run_id: 'r1' };
const codex = { ...mkAgent('x1', 'Ada', 'codex', 'solo', 1, 'working'), run_id: 'r2' };
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
const snapshot = (events: EventRow[]) =>
  ({ as_of: events.length, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [claude, codex], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events }) as unknown as Snapshot;
const load = (events: EventRow[] = []) => (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snapshot(events));

let id = 0;
beforeEach(() => {
  id = 0;
  load();
});

const ev = (agent: string, ts: number, kind: EventKind): EventRow => ({ id: ++id, ts, agent_id: agent, task_id: null, run_id: agent === 'c1' ? 'r1' : 'r2', kind });
const helper = (agent: string, ts: number, call: string, status: string) => ev(agent, ts, { type: 'sub_agent', call_id: call, description: `Run the tests (${call})`, status });
const turnEnd = (agent: string, ts: number, status: 'completed' | 'interrupted' | 'failed') => ev(agent, ts, { type: 'turn_completed', status, result_text: null, duration_ms: null });
/** A turn with two helpers at work: `fg` (its call still waits) and `bg` (in the background). */
const turn = (agent: string) => [ev(agent, 10, { type: 'turn_started', input: 'Have two subagents run the tests' }), helper(agent, 11, 'fg', 'running'), helper(agent, 12, 'bg', 'running')];
const helpers = (agent: string) => (store.live[agent]?.subagents ?? []).map((s) => [s.callId, s.status, s.ts]);

describe('the helpers of a Claude agent after a turn that did not end normally', () => {
  it.each(['interrupted', 'failed'] as const)('still run after a turn that was %s, when the adapter said nothing about them', (status) => {
    apply(...turn('c1'), turnEnd('c1', 14, status));
    expect(helpers('c1')).toEqual([['fg', 'running', 11], ['bg', 'running', 12]]);
  });

  it('the ones the adapter says "stopped" for are stopped; the one in the background works on', () => {
    // The adapter's order: the turn's end, then one row for each helper that ends with it.
    apply(...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped'));
    expect(helpers('c1')).toEqual([['bg', 'running', 12], ['fg', 'stopped', 14]]);
    // The helper in the background ends at its own row.
    apply(helper('c1', 30, 'bg', 'completed'));
    expect(helpers('c1')).toEqual([['fg', 'stopped', 14], ['bg', 'completed', 30]]);
  });

  it('reads the same from a snapshot as from live events', () => {
    load([...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped')]);
    expect(helpers('c1')).toEqual([['bg', 'running', 12], ['fg', 'stopped', 14]]);
  });
});

describe('the helpers of an agent of another CLI', () => {
  it.each(['interrupted', 'failed'] as const)('are stopped with a turn that was %s', (status) => {
    apply(...turn('x1'), turnEnd('x1', 14, status));
    expect(helpers('x1')).toEqual([['fg', 'stopped', 14], ['bg', 'stopped', 14]]);
  });

  it('go on after a turn that ended normally', () => {
    apply(...turn('x1'), turnEnd('x1', 14, 'completed'));
    expect(helpers('x1')).toEqual([['fg', 'running', 11], ['bg', 'running', 12]]);
  });
});

describe('the end of the process', () => {
  it.each(['c1', 'x1'])('stops every helper of %s', (agent) => {
    apply(...turn(agent), turnEnd(agent, 14, 'completed'), ev(agent, 20, { type: 'exited', code: 0 }));
    expect(helpers(agent)).toEqual([['fg', 'stopped', 20], ['bg', 'stopped', 20]]);
  });

  it('stops the Claude helper that worked on after an interrupted turn', () => {
    apply(...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped'), ev('c1', 20, { type: 'exited', code: 0 }));
    expect(helpers('c1')).toEqual([['bg', 'stopped', 20], ['fg', 'stopped', 14]]);
  });
});
