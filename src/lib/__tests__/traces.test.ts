import { describe, expect, it } from 'vitest';
import type { EventRow } from '../gen/EventRow';
import type { EventKind } from '../gen/EventKind';
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

  it('summarizes calls, failures, running and total time; empty for no events', () => {
    const t = buildTraces([started('a', T0, 'c1'), done('a', T0 + 1000, 'c1'), started('a', T0 + 2000, 'c2'), done('a', T0 + 2500, 'c2', { ok: false }), started('a', T0 + 3000, 'c3')], 'a');
    expect(summarize(t)).toEqual({ calls: 3, failed: 1, running: 1, totalMs: 1500 });
    expect(buildTraces([], 'a')).toEqual([]);
    expect(summarize([])).toEqual({ calls: 0, failed: 0, running: 0, totalMs: 0 });
  });
});
