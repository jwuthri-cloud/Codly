// Action chips come only from real events: an open tool call, a compaction, a status.
import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { chipFor } from '../bubble';
import { AGENTS, TASKS } from '../demo/script';
import type { OfficeModel } from '../model';
import { emptyModel, reduce, syncAgents, syncTasks } from '../reducer';

const T0 = 1_800_000_000_000;
let id = 0;
const row = (t: number, agent: string | null, kind: EventKind): EventRow => ({ id: ++id, ts: T0 + t, agent_id: agent, task_id: null, run_id: null, kind });
const started = (call: string, activity: string, name: string, target: Record<string, unknown> = {}): EventKind =>
  ({ type: 'tool_started', call_id: call, name, activity, target: { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...target }, input: {}, confidence: 'observed' }) as EventKind;
const completed = (call: string): EventKind =>
  ({ type: 'tool_completed', call_id: call, ok: true, output: null, exit_code: 0, duration_ms: 1, tests_passed: null, tests_failed: null }) as EventKind;

function fresh(): OfficeModel {
  const m = emptyModel();
  syncAgents(
    m,
    AGENTS.map((a) => ({ ...a, status: 'idle' })),
    T0,
  );
  syncTasks(m, TASKS);
  return m;
}
const ada = (m: OfficeModel) => m.agents.get('a-ada')!;

describe('action chips', () => {
  it('say idle when nothing is happening', () => {
    expect(chipFor(ada(fresh()))).toEqual({ text: 'idle', tone: 'muted' });
  });

  it('name the live tool while it runs, and drop it when the call completes', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'execute', 'Bash', { command: 'ls' })), { replay: false });
    expect(chipFor(ada(m))?.text).toBe('using Bash');
    reduce(m, row(10, 'a-ada', completed('c1')), { replay: false });
    expect(chipFor(ada(m))?.text).not.toBe('using Bash');
  });

  it('says what a Codex item type does instead of its raw name', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'edit', 'fileChange', { path: 'src/a.ts' })), { replay: false });
    expect(chipFor(ada(m))?.text).toBe('using file edit');
  });

  it('names the MCP server for an MCP call', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'mcp', 'mcp__time__now', { server: 'time', tool: 'now' })), { replay: false });
    expect(chipFor(ada(m))?.text).toBe('using time');
  });

  it('shows compaction from the real flag, and an offline agent shows nothing', () => {
    const m = fresh();
    ada(m).compactingSince = T0;
    expect(chipFor(ada(m))?.text).toBe('compacting context');
    ada(m).present = false;
    expect(chipFor(ada(m))).toBeNull();
  });

  it('a pending approval wins over everything', () => {
    const m = fresh();
    ada(m).approval = { id: 'x', text: 'rm', since: T0 };
    expect(chipFor(ada(m))).toEqual({ text: 'needs you', tone: 'warn' });
  });
});
