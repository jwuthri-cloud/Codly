// A helper (subagent) in the conversation: the call that started it is one step of a work row,
// and what the helper does shows under that step, with its kind, its progress and its report.
import { beforeEach, describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { store, type Seg, type SubAgent } from '../../lib/state/store.svelte';
import { mkAgent } from '../../office/demo/script';
import { helperLine, helperProgress, helperState, helperTitle, splitHelpers } from '../helpers';
import { buildSpine, type Item } from '../spine';
import { workSummary } from '../workSummary';

const lead = { ...mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead'), run_id: 'r1' };
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

let id = 0;
beforeEach(() => {
  id = 0;
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
});

const ev = (ts: number, kind: EventKind): EventRow => ({ id: ++id, ts, agent_id: 'l1', task_id: null, run_id: 'r1', kind });
const target = { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null };
const started = (ts: number, call: string, name: string, activity: 'delegate' | 'execute') =>
  ev(ts, { type: 'tool_started', call_id: call, name, activity, target: activity === 'execute' ? { ...target, command: 'echo hi' } : target, input: {}, confidence: 'observed' });
const completed = (ts: number, call: string, ok = true) =>
  ev(ts, { type: 'tool_completed', call_id: call, ok, output: null, exit_code: ok ? null : 1, duration_ms: null, tests_passed: null, tests_failed: null });
const helper = (ts: number, call: string, status: string) => ev(ts, { type: 'sub_agent', call_id: call, description: 'Run echo hi via Bash', status });
const SAID_NOTHING = { name: null, step: null, tool_call_id: null, tool_uses: null, tokens: null, report: null };
type Said = { name: string; step: string; tool_call_id: string; tool_uses: number; tokens: number; report: string };
const progress = (ts: number, call: string, said: Partial<Said>) => ev(ts, { type: 'sub_agent_progress', call_id: call, ...SAID_NOTHING, ...said });

const works = () => buildSpine('lead').filter((r): r is Item & { kind: 'work' } => r.kind === 'work');
const calls = (segs: Seg[]) => segs.map((s) => s.callId);

/** What the Claude adapter emits for a helper that works in the background (recording `subagent`). */
function launch() {
  apply(
    ev(10, { type: 'turn_started', input: 'Have a subagent run `echo hi`' }),
    helper(11, 'toolu-1', 'running'),
    progress(11, 'toolu-1', { name: 'general-purpose' }),
    started(11, 'toolu-1', 'Agent', 'delegate'),
    completed(12, 'toolu-1'), // the call only launched it
    ev(13, { type: 'message', item_id: 'm1', text: 'I launched the subagent.', phase: 'final' }),
    ev(14, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null }),
    // the helper works on after the turn
    started(15, 'toolu-2', 'Bash', 'execute'),
    progress(15, 'toolu-1', { tool_call_id: 'toolu-2' }),
    progress(16, 'toolu-1', { step: 'Running Echo hi to stdout', tool_uses: 1, tokens: 5932 }),
  );
}

describe('a helper in the conversation', () => {
  it('its calls show under the call that started it, also after the agent has spoken', () => {
    launch();
    const rows = works();
    expect(rows).toHaveLength(1); // not a second row after the message
    expect(calls(rows[0].segs)).toEqual(['toolu-1', 'toolu-2']);
    expect(buildSpine('lead').map((r) => r.kind)).toEqual(['work', 'say']);

    const { own, under } = splitHelpers(rows[0].segs, rows[0].helpers);
    expect(calls(own)).toEqual(['toolu-1']);
    expect(calls(under.get('toolu-1') ?? [])).toEqual(['toolu-2']);
    // The line of the row counts the agent's own steps, not the helper's.
    expect(workSummary(own)).toBe('Briefed a helper');
  });

  it('says its kind, what it does now and how far it is, then what it reported', () => {
    launch();
    let [row] = works();
    let [h] = row.helpers;
    expect(h).toMatchObject({ callId: 'toolu-1', description: 'Run echo hi via Bash', name: 'general-purpose', step: 'Running Echo hi to stdout', toolUses: 1, tokens: 5932, report: null });
    expect(helperState(h, row.segs[0])).toBe('running'); // its call returned, the helper did not
    expect(helperTitle(h, 'running')).toBe('Helper at work');
    expect(helperProgress(h)).toBe('1 step · 5.9k tokens');
    // The agent sheet's line under the helper: what it does now, and how far it is.
    expect(helperLine(h)).toBe('Running Echo hi to stdout · 1 step · 5.9k tokens');
    expect(helperLine({ ...h, step: null, toolUses: null, tokens: null })).toBe('');

    apply(completed(17, 'toolu-2'), progress(18, 'toolu-1', { report: 'hi', tool_uses: 1, tokens: 6604 }), helper(18, 'toolu-1', 'completed'));
    [row] = works();
    [h] = row.helpers;
    expect(h).toMatchObject({ status: 'completed', name: 'general-purpose', report: 'hi', tokens: 6604, startedAt: 11, ts: 18 });
    expect(helperState(h, row.segs[0])).toBe('done');
    expect(helperTitle(h, 'done')).toBe('Helper finished');
    expect(helperProgress(h)).toBe('1 step · 6.6k tokens');
    expect(helperLine(h)).toBe('1 step · 6.6k tokens'); // over: its last step is no longer what it does
  });

  it('a helper whose process is gone no longer counts as at work', () => {
    launch();
    apply(ev(20, { type: 'exited', code: 1 }));
    const [row] = works();
    const [h] = row.helpers;
    expect(h.status).toBe('stopped');
    expect(helperState(h, row.segs[0])).toBe('failed');
    expect(helperTitle(h, 'failed')).toBe('Helper stopped');
  });
});

/** The other recorded order (recording `subagent-stdin-closed`): the helper's call starts before the turn ends. */
function launchEarly() {
  apply(
    ev(10, { type: 'turn_started', input: 'Have a subagent run the tests' }),
    helper(11, 'toolu-1', 'running'),
    started(11, 'toolu-1', 'Agent', 'delegate'),
    completed(12, 'toolu-1'),
    started(13, 'toolu-2', 'Bash', 'execute'),
    progress(13, 'toolu-1', { tool_call_id: 'toolu-2' }),
    started(13, 'toolu-3', 'Bash', 'execute'), // a call of the agent's own, cut off by the end of the turn
    ev(14, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null }),
  );
}
const callOf = (call: string) => works().flatMap((r) => r.segs).find((s) => s.callId === call);

describe("a helper's call that is open when the turn ends", () => {
  it('stays open, and its result says how it went', () => {
    launchEarly();
    expect(callOf('toolu-2')).toMatchObject({ parent: 'toolu-1', end: null, ok: null });
    expect(callOf('toolu-3')).toMatchObject({ end: 14, ok: null }); // the agent's own call ends with its turn
    expect(store.now('l1')?.seg?.callId).toBe('toolu-2');

    apply(completed(40, 'toolu-2', false));
    expect(callOf('toolu-2')).toMatchObject({ end: 40, ok: false, exitCode: 1 });
    expect(store.now('l1')).toBeNull();
  });

  it('ends with its helper when no result came', () => {
    launchEarly();
    apply(helper(30, 'toolu-1', 'stopped'));
    expect(callOf('toolu-2')).toMatchObject({ end: 30, ok: null });
    // A result that comes after that changes nothing.
    apply(completed(40, 'toolu-2', false));
    expect(callOf('toolu-2')).toMatchObject({ end: 30, ok: null });
  });

  it('ends with the turn when its helper is over with that turn', () => {
    // A helper whose `Agent` call waited for it: the adapter says "stopped" right after the turn's end.
    launchEarly();
    apply(helper(14, 'toolu-1', 'stopped'));
    expect(callOf('toolu-2')).toMatchObject({ end: 14, ok: null });
  });

  it('ends when the process is gone', () => {
    launchEarly();
    apply(ev(20, { type: 'exited', code: 1 }));
    expect(callOf('toolu-2')).toMatchObject({ end: 20, ok: null });
  });
});

describe('a plan shown to you', () => {
  it('is said as that in the line of its work row, whatever kind of step the call is', () => {
    const shown = (ts: number, call: string, activity: 'plan' | 'mcp') =>
      ev(ts, { type: 'tool_started', call_id: call, name: 'ExitPlanMode', activity, target: { ...target, tool: 'ExitPlanMode' }, input: { plan: '1. Add the line' }, confidence: 'observed' });
    apply(ev(10, { type: 'turn_started', input: 'Plan it' }), shown(11, 'p1', 'plan'));
    expect(workSummary(works()[0].segs)).toBe('Showed you its plan');
    apply(completed(12, 'p1', false), shown(13, 'p2', 'mcp'), started(14, 'c1', 'Bash', 'execute'));
    expect(workSummary(works()[0].segs)).toBe('Ran echo hi · showed you its plan 2 times');
  });
});

describe('a helper whose turn was interrupted or failed', () => {
  const atWork = () => (store.live.l1?.subagents ?? []).filter((s) => s.status === 'running').map((s) => s.callId);
  /** As `launchEarly`, but the turn does not end normally. */
  function cut(status: 'interrupted' | 'failed') {
    apply(
      ev(10, { type: 'turn_started', input: 'Have a subagent run the tests' }),
      helper(11, 'toolu-1', 'running'),
      started(11, 'toolu-1', 'Agent', 'delegate'),
      started(13, 'toolu-2', 'Bash', 'execute'),
      progress(13, 'toolu-1', { tool_call_id: 'toolu-2' }),
      ev(14, { type: 'turn_completed', status, result_text: null, duration_ms: null }),
    );
  }

  it.each(['interrupted', 'failed'] as const)('is stopped with a turn that was %s, and so is its open call', (status) => {
    cut(status);
    expect(atWork()).toEqual([]);
    expect(callOf('toolu-2')).toMatchObject({ end: 14, ok: null });
    expect(store.now('l1')).toBeNull();
    const [row] = works();
    expect(row.helpers).toMatchObject([{ callId: 'toolu-1', status: 'stopped', ts: 14 }]);
    expect(helperTitle(row.helpers[0], helperState(row.helpers[0], row.segs[0]))).toBe('Helper stopped');
  });

  it('a retry shows one helper at work, not the stopped one too', () => {
    cut('interrupted');
    apply(
      ev(20, { type: 'turn_started', input: 'Have a subagent run the tests' }),
      helper(21, 'toolu-9', 'running'),
      started(21, 'toolu-9', 'Agent', 'delegate'),
    );
    expect(atWork()).toEqual(['toolu-9']);
  });

  it('a helper still at work when its turn ends normally goes on', () => {
    launchEarly();
    expect(atWork()).toEqual(['toolu-1']);
  });
});

describe('a Codex helper', () => {
  // What the Codex adapter emits when the subagent's own turn starts before the `spawnAgent`
  // call names its thread: nothing for that turn's start, then the helper when the call ends,
  // under the call's id. A helper shows at the call whose id it carries.
  it('shows at the call that started it, also when its thread started first', () => {
    const codexHelper = (ts: number, status: string) => ev(ts, { type: 'sub_agent', call_id: 'k1', description: 'Find the flaky test', status });
    apply(
      ev(10, { type: 'turn_started', input: 'Have a subagent find the flaky test' }),
      started(11, 'k1', 'collabAgentToolCall', 'delegate'),
      completed(13, 'k1'),
      codexHelper(13, 'running'),
    );
    let [row] = works();
    expect(calls(row.segs)).toEqual(['k1']);
    expect(row.helpers).toMatchObject([{ callId: 'k1', description: 'Find the flaky test', status: 'running' }]);
    expect(helperState(row.helpers[0], row.segs[0])).toBe('running'); // its call returned, the helper did not

    apply(codexHelper(20, 'completed'));
    [row] = works();
    expect(row.helpers).toHaveLength(1);
    expect(helperState(row.helpers[0], row.segs[0])).toBe('done');
  });
});

describe('splitHelpers', () => {
  const seg = (callId: string, parent?: string): Seg => ({
    callId, agentId: 'l1', taskId: null, activity: 'execute', name: 'Bash', label: '', target, input: {}, confidence: 'observed',
    start: 1, end: 2, ok: true, exitCode: 0, testsPassed: null, testsFailed: null, eventId: 1, ...(parent ? { parent } : {}),
  });
  const sub = (callId: string, status = 'running'): SubAgent => ({ callId, description: '', status, startedAt: 1, ts: 1, name: null, step: null, toolUses: null, tokens: null, report: null });

  it("a helper's helper shows under the first one", () => {
    const { own, under } = splitHelpers([seg('a'), seg('b', 'a'), seg('c', 'b'), seg('d')], [sub('a'), sub('b')]);
    expect(calls(own)).toEqual(['a', 'd']);
    expect(calls(under.get('a') ?? [])).toEqual(['b', 'c']);
  });

  it('a call whose helper is not in the row stays among the steps', () => {
    expect(calls(splitHelpers([seg('c', 'gone')], []).own)).toEqual(['c']);
    // The helper is known, but the call that started it is in no row any more.
    expect(calls(splitHelpers([seg('c', 'a')], [sub('a')]).own)).toEqual(['c']);
  });

  it("another CLI's status word follows the call that started the helper", () => {
    expect(helperState(sub('a', 'started'), { ...seg('a'), end: null, ok: null })).toBe('running');
    expect(helperState(sub('a', 'started'), seg('a'))).toBe('done');
    expect(helperState(sub('a', 'started'), { ...seg('a'), ok: false })).toBe('failed');
    expect(helperTitle(sub('a', 'failed'), 'failed')).toBe('Helper failed');
  });
});
