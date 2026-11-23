// The office keeps its own list of an agent's helpers (the companion at the desk, the "+1"
// badge, the "briefing a helper" line). It must say what the sheet says, so it follows the
// store's rule (src/lib/__tests__/helperTurnEnd.test.ts): after a turn that was interrupted or
// that failed, a Claude agent's adapter says which helpers ended with the turn (a `sub_agent`
// row "stopped" for each) and a helper in the background works on; for another CLI the end of
// the turn ends its helpers. The end of the process ends every helper of every CLI.
import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { chipFor } from '../bubble';
import { mkAgent } from '../demo/script';
import type { OfficeModel } from '../model';
import { emptyModel, reduce, syncAgents } from '../reducer';

const T0 = Date.UTC(2026, 10, 23, 12, 0, 0);

function office(): OfficeModel {
  const m = emptyModel();
  syncAgents(m, [{ ...mkAgent('c1', 'Bo', 'claude', 'worker', 0, 'working'), run_id: 'r1' }, { ...mkAgent('x1', 'Ada', 'codex', 'worker', 1, 'working'), run_id: 'r2' }], T0);
  return m;
}

const runOf: Record<string, string> = { c1: 'r1', x1: 'r2' };
const ev = (agent: string, ts: number, kind: EventKind): EventRow => ({ id: 0, ts: T0 + ts, agent_id: agent, task_id: null, run_id: runOf[agent] ?? null, kind });
const helper = (agent: string, ts: number, call: string, status: string) => ev(agent, ts, { type: 'sub_agent', call_id: call, description: `Run the tests (${call})`, status });
const turnEnd = (agent: string, ts: number, status: 'completed' | 'interrupted' | 'failed') => ev(agent, ts, { type: 'turn_completed', status, result_text: null, duration_ms: null });
/** A turn with two helpers at work: `fg` (its call still waits) and `bg` (in the background). */
const turn = (agent: string) => [ev(agent, 10, { type: 'turn_started', input: 'Have two subagents run the tests' }), helper(agent, 11, 'fg', 'running'), helper(agent, 12, 'bg', 'running')];
const fold = (m: OfficeModel, evs: EventRow[], replay = false) => evs.forEach((e) => reduce(m, e, { replay }));
const helpers = (m: OfficeModel, agent: string) => [...m.agents.get(agent)!.subagents.keys()];

describe('the office: the helpers of a Claude agent after a turn that did not end normally', () => {
  it.each(['interrupted', 'failed'] as const)('are still there after a turn that was %s, when the adapter said nothing about them', (status) => {
    const m = office();
    fold(m, [...turn('c1'), turnEnd('c1', 14, status)]);
    expect(helpers(m, 'c1')).toEqual(['fg', 'bg']);
  });

  it('the ones the adapter says "stopped" for are gone; the one in the background stays until its own row', () => {
    const m = office();
    // The adapter's order: the turn's end, then one row for each helper that ends with it.
    fold(m, [...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped')]);
    expect(helpers(m, 'c1')).toEqual(['bg']);
    expect(chipFor(m.agents.get('c1')!)?.text).toBe('briefing a helper');
    fold(m, [helper('c1', 30, 'bg', 'completed')]);
    expect(helpers(m, 'c1')).toEqual([]);
  });

  it('reads the same when the events are replayed (the window was loaded again)', () => {
    const m = office();
    fold(m, [...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped')], true);
    expect(helpers(m, 'c1')).toEqual(['bg']);
  });
});

describe('the office: the helpers of an agent of another CLI', () => {
  it.each(['interrupted', 'failed'] as const)('are gone with a turn that was %s', (status) => {
    const m = office();
    fold(m, [...turn('x1'), turnEnd('x1', 14, status)]);
    expect(helpers(m, 'x1')).toEqual([]);
  });

  it('stay after a turn that ended normally', () => {
    const m = office();
    fold(m, [...turn('x1'), turnEnd('x1', 14, 'completed')]);
    expect(helpers(m, 'x1')).toEqual(['fg', 'bg']);
  });

  it('are gone with such a turn when the office does not know the CLI of the agent', () => {
    const m = office();
    fold(m, [...turn('new'), turnEnd('new', 14, 'interrupted')]);
    expect(helpers(m, 'new')).toEqual([]);
  });
});

describe('the office: the end of the process', () => {
  it.each(['c1', 'x1'])('ends every helper of %s', (agent) => {
    const m = office();
    fold(m, [...turn(agent), turnEnd(agent, 14, 'completed'), ev(agent, 20, { type: 'exited', code: 0 })]);
    expect(helpers(m, agent)).toEqual([]);
  });

  it('ends the Claude helper that worked on after an interrupted turn', () => {
    const m = office();
    fold(m, [...turn('c1'), turnEnd('c1', 14, 'interrupted'), helper('c1', 14, 'fg', 'stopped'), ev('c1', 20, { type: 'exited', code: 0 })]);
    expect(helpers(m, 'c1')).toEqual([]);
  });

  it('ends the helpers of a process the roster no longer names (a replay after the run is over)', () => {
    const m = emptyModel();
    syncAgents(m, [mkAgent('c1', 'Bo', 'claude', 'worker', 0, 'idle')], T0);
    fold(m, [...turn('c1'), turnEnd('c1', 14, 'completed'), ev('c1', 20, { type: 'exited', code: 0 })], true);
    expect(helpers(m, 'c1')).toEqual([]);
  });
});
