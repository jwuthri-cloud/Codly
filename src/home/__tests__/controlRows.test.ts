// What you did to an agent stays in its thread as a quiet row: a compaction, its mode, its
// model, a context reading. Before, '/context' and '/compact' answered with a toast that went
// away, and a model or mode switch left no row.
import { beforeEach, describe, expect, it } from 'vitest';
import type { Agent } from '../../lib/gen/Agent';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { contextReads } from '../../lib/state/control.svelte';
import { store } from '../../lib/state/store.svelte';
import { mkAgent } from '../../office/demo/script';
import { controlRows } from '../controlRows';
import { buildSpine } from '../spine';

const T = 1_760_000_080_000; // 40 s into a minute
const ada = { id: 'ada', name: 'Ada' };
const keeps = (w: string) => w === 'Ada';

let id = 0;
beforeEach(() => {
  id = 0;
});
const ev = (kind: unknown, ts: number, agent = 'ada'): EventRow => ({ id: ++id, ts, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const decision = (summary: string, ts: number, agent = 'ada') => ev({ type: 'decision', id: `d${id}`, decision_type: 'agent_control', method: 'human', summary, detail: { agent_id: agent } }, ts, agent);
const changed = (model: string | null, ts: number) => ev({ type: 'agent_changed', agent: { ...mkAgent('ada', 'Ada', 'claude', 'solo', 0, 'idle'), model } as Agent }, ts);
const started = (model: string, ts: number) => ev({ type: 'session_started', provider: 'claude', model, session_id: 's1' }, ts);
const turn = (ts: number) => ev({ type: 'turn_started', input: null }, ts);

/** The rows as their words. */
const words = (events: EventRow[], context: { ts: number; line: string } | null = null) => controlRows(events, ada, keeps, context).map((r) => r.map((n) => n.text));

describe('controlRows', () => {
  it('a compaction that is done says from how many tokens to how many; its start alone says nothing', () => {
    expect(words([ev({ type: 'compacting', pre_tokens: null, post_tokens: null }, T)])).toEqual([]);
    expect(words([ev({ type: 'compacting', pre_tokens: null, post_tokens: null }, T), ev({ type: 'compacting', pre_tokens: 182_000, post_tokens: 41_000 }, T + 120_000)])).toEqual([
      ['Ada’s conversation was compacted · 182k → 41k tokens'],
    ]);
    expect(words([ev({ type: 'compacting', pre_tokens: 182_000, post_tokens: null }, T)])).toEqual([['Ada’s conversation was compacted · 182k tokens before']]);
  });

  it('each change the core recorded for the agent is a sentence, as History says it', () => {
    expect(words([decision('Ada now works in the mode "Accept edits"', T)])).toEqual([['Ada now works in the mode "Accept edits"']]);
    expect(words([decision('asked Ada to compact its conversation', T)])).toEqual([['You asked Ada to compact its conversation']]);
    expect(words([decision('you turned postgres off for Ada\'s run', T)])).toEqual([['You turned postgres off for Ada\'s run']]);
    // Another agent's record, and a record of another kind, are not this thread's.
    expect(words([decision('Bo now works in the mode "Plan first"', T, 'bo'), ev({ type: 'decision', id: 'x', decision_type: 'review', method: 'human', summary: 'approved t1', detail: {} }, T)])).toEqual([]);
  });

  it('a model or effort change between two turns says what it works with now', () => {
    const log = [changed('opus', T - 600_000), turn(T - 500_000), changed('sonnet:high', T), changed(null, T + 120_000)];
    expect(words(log)).toEqual([['Ada now works with Sonnet · High effort'], ['Ada now works with its CLI’s default model']]);
  });

  it('the model id a run reports at its start is no change of yours', () => {
    const log = [changed('opus', T - 600_000), started('claude-opus-4-7', T - 590_000), changed('claude-opus-4-7', T - 590_000)];
    expect(words(log)).toEqual([]);
    // Once a turn ran, a change of the model is yours.
    expect(words([...log, turn(T - 500_000), changed('claude-sonnet-4-6', T)])).toEqual([['Ada now works with Sonnet 4.6']]);
  });

  it('the last context reading of this window is a row too, in its place', () => {
    expect(words([decision('Ada now works in the mode "Plan first"', T - 120_000)], { ts: T, line: '6k of 1M tokens (1%)' })).toEqual([
      ['Ada now works in the mode "Plan first"'],
      ['Context: 6k of 1M tokens (1%)'],
    ]);
  });

  it('notes of one kind in the same minute join into one row, oldest first; another kind or the next minute is a row of its own', () => {
    const log = [
      decision('Ada now works in the mode "Accept edits"', T), decision('you turned postgres off for Ada\'s run', T + 5_000), changed('opus', T - 900_000), turn(T - 800_000),
      changed('sonnet', T + 10_000), decision('steer note for Ada: keep it short', T + 30_000),
    ];
    expect(words(log)).toEqual([
      ['Ada now works in the mode "Accept edits"', 'You turned postgres off for Ada\'s run'],
      ['Ada now works with Sonnet'],
      ['Steer note for Ada: keep it short'],
    ]);
  });

  it('a mode switch keeps its place before a compaction of the same minute (round 8 review: it went behind "and 2 more")', () => {
    const log = [decision('Ada now works in the mode "Accept edits"', T), decision('asked Ada to compact its conversation', T + 5_000), ev({ type: 'compacting', pre_tokens: 90_000, post_tokens: 12_000 }, T + 10_000)];
    expect(words(log)).toEqual([['Ada now works in the mode "Accept edits"'], ['You asked Ada to compact its conversation', 'Ada’s conversation was compacted · 90k → 12k tokens']]);
  });

  it('a row keeps its notes while nothing changed, so the thread does not draw it again', () => {
    const log = [decision('Ada now works in the mode "Accept edits"', T)];
    expect(controlRows(log, ada, keeps, null)[0][0]).toBe(controlRows(log, ada, keeps, null)[0][0]);
  });
});

describe('in the thread', () => {
  beforeEach(() => {
    const solo = mkAgent('ada', 'Ada', 'claude', 'solo', 0, 'idle');
    const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [solo], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
    (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
    delete contextReads.ada;
  });
  const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

  it('a mode switch, the compaction you asked for and a context reading are quiet rows of the 1:1 thread', () => {
    apply(decision('Ada now works in the mode "Accept edits"', T), decision('asked Ada to compact its conversation', T + 120_000), ev({ type: 'compacting', pre_tokens: 90_000, post_tokens: 12_000 }, T + 130_000));
    contextReads.ada = { ts: T + 300_000, line: '12k of 200k tokens (6%)' };
    const rows = buildSpine('agent:ada').filter((r) => r.kind === 'control');
    expect(rows.map((r) => r.kind === 'control' && r.notes.map((n) => n.text))).toEqual([
      ['Ada now works in the mode "Accept edits"'],
      ['You asked Ada to compact its conversation', 'Ada’s conversation was compacted · 90k → 12k tokens'],
      ['Context: 12k of 200k tokens (6%)'],
    ]);
    expect(new Set(rows.map((r) => r.key)).size).toBe(3);
  });
});
