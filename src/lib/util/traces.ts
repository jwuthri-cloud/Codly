// The Traces tab: an agent's tool calls, built from the events Codly records. A call is a
// `tool_started` paired with its `tool_completed` by call id. Duration is what the CLI reported
// when it did (Observed); else the gap between the two events (Measured by Codly); a call with no
// completion yet is still running and has no duration.
import type { ActivityKind } from '../gen/ActivityKind';
import type { EventRow } from '../gen/EventRow';
import { targetLabel } from './labels';

export interface TraceCall {
  callId: string;
  name: string;
  activity: ActivityKind;
  /** What it touched: file, command or query, shortened. */
  label: string;
  start: number;
  /** null while running. */
  durationMs: number | null;
  /** Where the duration comes from. */
  source: 'reported' | 'measured' | null;
  /** null while running. */
  ok: boolean | null;
  exitCode: number | null;
  testsPassed: number | null;
  testsFailed: number | null;
}

export interface TraceSummary {
  calls: number;
  failed: number;
  running: number;
  /** Sum of the known durations. */
  totalMs: number;
}

/** Chronological (oldest first). `events` may hold any agents' events, in any order of arrival. */
export function buildTraces(events: readonly EventRow[], agentId: string): TraceCall[] {
  const calls = new Map<string, TraceCall>();
  for (const e of events) {
    if (e.agent_id !== agentId) continue;
    const k = e.kind;
    if (k.type === 'tool_started') {
      calls.set(k.call_id, {
        callId: k.call_id, name: k.name, activity: k.activity, label: targetLabel(k.target, k.activity), start: e.ts,
        durationMs: null, source: null, ok: null, exitCode: null, testsPassed: null, testsFailed: null,
      });
    } else if (k.type === 'tool_completed') {
      const c = calls.get(k.call_id);
      if (!c) continue; // completion of a call older than the loaded history
      const reported = k.duration_ms != null && k.duration_ms >= 0;
      c.durationMs = reported ? k.duration_ms : Math.max(0, e.ts - c.start);
      c.source = reported ? 'reported' : 'measured';
      c.ok = k.ok;
      c.exitCode = k.exit_code;
      c.testsPassed = k.tests_passed;
      c.testsFailed = k.tests_failed;
    }
  }
  return [...calls.values()].sort((a, b) => a.start - b.start);
}

export function summarize(calls: readonly TraceCall[]): TraceSummary {
  return {
    calls: calls.length,
    failed: calls.filter((c) => c.ok === false).length,
    running: calls.filter((c) => c.ok === null).length,
    totalMs: calls.reduce((n, c) => n + (c.durationMs ?? 0), 0),
  };
}
