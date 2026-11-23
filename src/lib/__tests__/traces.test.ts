import { describe, expect, it } from 'vitest';
import type { EventRow } from '../gen/EventRow';
import type { EventKind } from '../gen/EventKind';
import { uniqueKeys } from '../util/keys';
import { buildTraces, summarize } from '../util/traces';

const T0 = 1_760_000_000_000;
const target = { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null };
let id = 0;
const ev = (agent: string, ts: number, kind: unknown): EventRow => ({ id: ++id, ts, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const started = (agent: string, ts: number, call_id: string, name = 'Bash', t = { ...target, command: 'pnpm test' }) =>
  ev(agent, ts, { type: 'tool_started', call_id, name, activity: 'test', target: t, input: {}, confidence: 'observed' });
const done = (agent: string, ts: number, call_id: string, o: Record<string, unknown> = {}) =>
  ev(agent, ts, { type: 'tool_completed', call_id, ok: true, output: null, exit_code: 0, duration_ms: null, tests_passed: null, tests_failed: null, ...o });

describe('buildTraces', () => {
  it('pairs a call with its completion and prefers the duration the CLI reported', () => {
    const t = buildTraces([started('a', T0, 'c1'), done('a', T0 + 5000, 'c1', { duration_ms: 4200, tests_passed: 12, tests_failed: 1, ok: false, exit_code: 1 })], 'a');
    expect(t).toHaveLength(1);
    expect(t[0]).toMatchObject({ name: 'Bash', durationMs: 4200, source: 'reported', ok: false, exitCode: 1, testsPassed: 12, testsFailed: 1 });
    expect(t[0].label).toContain('pnpm test');
  });

  it('measures the gap between the two events when the CLI reported no duration', () => {
    const t = buildTraces([started('a', T0, 'c1'), done('a', T0 + 1800, 'c1')], 'a');
    expect(t[0]).toMatchObject({ durationMs: 1800, source: 'measured', ok: true });
  });

  it('keeps a call without a completion as running, and ignores other agents and orphan completions', () => {
    const t = buildTraces([started('a', T0, 'c1'), started('b', T0 + 1, 'c2'), done('a', T0 + 9, 'ghost'), started('a', T0 - 500, 'c0')], 'a');
    expect(t.map((c) => c.callId)).toEqual(['c0', 'c1']); // chronological
    expect(t.every((c) => c.ok === null && c.durationMs === null && c.source === null)).toBe(true);
  });

  it('lists both calls when a CLI uses a call id again after the first call ended', () => {
    const t = buildTraces(
      [started('a', T0, 'call-1'), done('a', T0 + 1000, 'call-1'), started('a', T0 + 5000, 'call-1', 'Read'), done('a', T0 + 5200, 'call-1', { ok: false })],
      'a',
    );
    expect(t.map((c) => [c.callId, c.name, c.start, c.durationMs, c.ok])).toEqual([
      ['call-1', 'Bash', T0, 1000, true],
      ['call-1', 'Read', T0 + 5000, 200, false],
    ]);
    expect(summarize(t)).toMatchObject({ calls: 2, failed: 1, running: 0 });
    // The keys of the tab's rows: the first call keeps its own when the second one arrives.
    expect(uniqueKeys(t, (c) => c.callId)).toEqual(['call-1', 'call-1#2']);
  });

  it('keeps one call when the CLI announces it again while it runs', () => {
    const t = buildTraces([started('a', T0, 'c1', 'Bash', { ...target, command: 'pnpm' }), started('a', T0 + 300, 'c1'), done('a', T0 + 2000, 'c1')], 'a');
    expect(t).toHaveLength(1);
    // What it runs as last announced, from when it was first announced.
    expect(t[0]).toMatchObject({ start: T0, durationMs: 2000, ok: true });
    expect(t[0].label).toContain('pnpm test');
  });

  it('lists both calls when a call was stopped with its turn and a later turn uses its id again', () => {
    for (const end of [{ type: 'turn_completed', status: 'interrupted', result_text: null, duration_ms: null }, { type: 'exited', code: null }]) {
      const t = buildTraces([started('a', T0, 'c1'), ev('a', T0 + 400, end), started('a', T0 + 60_000, 'c1', 'Read'), done('a', T0 + 60_300, 'c1')], 'a');
      // The later call has its own start: its duration is not measured from the turn before.
      expect(t.map((c) => [c.name, c.start, c.durationMs, c.ok])).toEqual([
        ['Bash', T0, null, null],
        ['Read', T0 + 60_000, 300, true],
      ]);
      expect(summarize(t)).toMatchObject({ calls: 2, totalMs: 300 });
    }
  });

  it('still pairs a result that comes after the turn with the call it belongs to', () => {
    const t = buildTraces([started('a', T0, 'c1'), ev('a', T0 + 400, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null }), done('a', T0 + 900, 'c1')], 'a');
    expect(t).toHaveLength(1);
    expect(t[0]).toMatchObject({ start: T0, durationMs: 900, ok: true });
  });

  it("says which calls a subagent made, and leaves the agent's own unmarked", () => {
    const by = (input: unknown) => ev('a', T0, { type: 'tool_started', call_id: `c${id}`, name: 'commandExecution', activity: 'execute', target, input, confidence: 'observed' });
    const t = buildTraces([by({ command: 'ls' }), by({ command: 'ls', subagent: { thread: 't1', name: 'Ada (explorer)' } }), by({ command: 'ls', subagent: { thread: 't2', name: null } })], 'a');
    expect(t.map((c) => c.subagent)).toEqual(['', 'subagent Ada (explorer)', 'subagent']);
  });

  it('summarizes calls, failures, running and total time; empty for no events', () => {
    const t = buildTraces([started('a', T0, 'c1'), done('a', T0 + 1000, 'c1'), started('a', T0 + 2000, 'c2'), done('a', T0 + 2500, 'c2', { ok: false }), started('a', T0 + 3000, 'c3')], 'a');
    expect(summarize(t)).toEqual({ calls: 3, failed: 1, running: 1, totalMs: 1500 });
    expect(buildTraces([], 'a')).toEqual([]);
    expect(summarize([])).toEqual({ calls: 0, failed: 0, running: 0, totalMs: 0 });
  });

  it('a Claude agent’s call that showed a plan is named as that, and a plan sent back is not a failed call', () => {
    const plan = (ts: number, call_id: string) => started('a', ts, call_id, 'ExitPlanMode');
    const events = [plan(T0, 'p1'), done('a', T0 + 1000, 'p1', { ok: false }), plan(T0 + 2000, 'p2'), done('a', T0 + 3000, 'p2'), started('a', T0 + 4000, 'c1'), done('a', T0 + 5000, 'c1', { ok: false })];
    const t = buildTraces(events, 'a', 'claude');
    expect(t.map((c) => [c.name, c.label, c.ok, c.planSentBack])).toEqual([['Its plan', 'shown to you', false, true], ['Its plan', 'shown to you', true, false], ['Bash', 'pnpm test', false, false]]);
    expect(summarize(t)).toMatchObject({ calls: 3, failed: 1 });
    // Only for a Claude agent: from another CLI, or an agent Codly does not know, a call of that name is a call like any other.
    for (const other of [buildTraces(events, 'a', 'codex'), buildTraces(events, 'a')]) {
      expect(other.map((c) => [c.name, c.ok, c.planSentBack])).toEqual([['ExitPlanMode', false, false], ['ExitPlanMode', true, false], ['Bash', false, false]]);
      expect(summarize(other)).toMatchObject({ calls: 3, failed: 2 });
    }
  });
});
