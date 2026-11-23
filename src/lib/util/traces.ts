// The Traces tab: an agent's tool calls, built from the events Codly records. A call is a
// `tool_started` paired with its `tool_completed` by call id. Duration is what the CLI reported
// when it did (Observed); else the gap between the two events (Measured by Codly); a call with no
// completion yet is still running and has no duration. A Claude agent's call that showed you a
// plan is named as that, and a plan you did not accept is no failed call.
import type { ActivityKind } from '../gen/ActivityKind';
import type { EventRow } from '../gen/EventRow';
import type { Provider } from '../gen/Provider';
import { subagentTag, targetLabel } from './labels';
import { isPlanCall } from './planRequest';

export interface TraceCall {
  callId: string;
  name: string;
  activity: ActivityKind;
  /** What it touched: file, command or query, shortened. */
  label: string;
  /** "subagent Ada (explorer)" when one of the agent's subagents made the call; '' for its own. */
  subagent: string;
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
  /** The call showed you a plan and you did not accept it: `ok` is false, and nothing failed. */
  planSentBack: boolean;
}

export interface TraceSummary {
  calls: number;
  failed: number;
  running: number;
  /** Sum of the known durations. */
  totalMs: number;
}

/**
 * Chronological (oldest first). `events` may hold any agents' events, in the order Codly recorded
 * them. A call id names one call while that call runs, and no longer than its turn: a CLI that
 * uses the id again after the call ended, or in a later turn (the first call was stopped with its
 * turn and has no completion), started another call. Both are listed, as on the timeline.
 * `provider`: the agent's CLI (none when Codly does not know the agent): only a Claude agent's
 * call of the plan tool is its plan; for every other CLI a call of that name is shown by its name.
 */
export function buildTraces(events: readonly EventRow[], agentId: string, provider: Provider | null = null): TraceCall[] {
  const calls: TraceCall[] = [];
  /** The calls with no completion yet, by call id. */
  const open = new Map<string, TraceCall>();
  /** Those of them whose turn or process is over: a result may still come, a new start may not. */
  const left = new Set<TraceCall>();
  /** The calls that showed you a plan. */
  const plans = new Set<TraceCall>();
  for (const e of events) {
    if (e.agent_id !== agentId) continue;
    const k = e.kind;
    if (k.type === 'turn_completed' || k.type === 'exited') {
      for (const c of open.values()) left.add(c);
    } else if (k.type === 'tool_started') {
      const plan = isPlanCall(k.name, provider);
      const call: TraceCall = {
        callId: k.call_id, name: plan ? 'Its plan' : k.name, activity: k.activity, label: plan ? 'shown to you' : targetLabel(k.target, k.activity), subagent: subagentTag(k.input), start: e.ts,
        durationMs: null, source: null, ok: null, exitCode: null, testsPassed: null, testsFailed: null, planSentBack: false,
      };
      const running = open.get(k.call_id);
      // Announced again in the turn it runs in (the CLI knows more of its input by now): the same call.
      if (running && !left.has(running)) Object.assign(running, call, { start: running.start });
      else {
        calls.push(call);
        open.set(k.call_id, call);
        if (plan) plans.add(call);
      }
    } else if (k.type === 'tool_completed') {
      const c = open.get(k.call_id);
      if (!c) continue; // completion of a call older than the loaded history
      open.delete(k.call_id);
      const reported = k.duration_ms != null && k.duration_ms >= 0;
      c.durationMs = reported ? k.duration_ms : Math.max(0, e.ts - c.start);
      c.source = reported ? 'reported' : 'measured';
      c.ok = k.ok;
      c.exitCode = k.exit_code;
      c.testsPassed = k.tests_passed;
      c.testsFailed = k.tests_failed;
      c.planSentBack = !k.ok && plans.has(c);
    }
  }
  return calls.sort((a, b) => a.start - b.start);
}

export function summarize(calls: readonly TraceCall[]): TraceSummary {
  return {
    calls: calls.length,
    failed: calls.filter((c) => c.ok === false && !c.planSentBack).length,
    running: calls.filter((c) => c.ok === null).length,
    totalMs: calls.reduce((n, c) => n + (c.durationMs ?? 0), 0),
  };
}
