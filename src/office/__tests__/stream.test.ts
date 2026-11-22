// Streamed chunks come many times a second; the engine runs its director pass only for the ones
// that can change what the office shows. `streamOnly` decides, and these tests hold it to the
// honesty contract: whenever it says "skip", the office shows exactly the same after the event.
import { describe, expect, it } from 'vitest';
import type { AgentStatus } from '../../lib/gen/AgentStatus';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { deriveIntent, intentKey, LONG_IDLE_MS } from '../behavior';
import { chipFor } from '../bubble';
import { mkAgent } from '../demo/script';
import type { Fx, OfficeModel } from '../model';
import { sentence } from '../narrate';
import { emptyModel, lasting, reduce, streamOnly, syncAgents } from '../reducer';

const T0 = Date.UTC(2026, 9, 25, 12, 0, 0);

function officeWith(status: AgentStatus): OfficeModel {
  const m = emptyModel();
  syncAgents(m, [mkAgent('a1', 'Ada', 'claude', 'worker', 1, status)], T0);
  return m;
}

const ev = (ts: number, kind: EventKind, agent = 'a1'): EventRow => ({ id: 0, ts, agent_id: agent, task_id: null, run_id: null, kind });
const text = (ts: number, agent?: string) => ev(ts, { type: 'text_delta', item_id: 'm1', text: 'hi' }, agent);
const thought = (ts: number, agent?: string) => ev(ts, { type: 'reasoning_delta', item_id: 'r1', text: 'hm' }, agent);
const output = (ts: number, agent?: string) => ev(ts, { type: 'tool_output', call_id: 'c1', chunk: 'ok\n' }, agent);
const usage = (ts: number, agent?: string) => ev(ts, { type: 'usage_updated', usage: mkAgent('x', 'X', 'claude', 'worker', 0, 'idle').usage }, agent);

/** Everything the office derives for an agent: where it is and what it does, its sentence, its chip. */
function shows(m: OfficeModel, now: number): string {
  const a = m.agents.get('a1')!;
  return [intentKey(deriveIntent(a, m, now, null).intent), sentence(a, now).text, chipFor(a)?.text ?? ''].join(' | ');
}

/** Fold `e`; when `streamOnly` said skip, the office must show the same before and after. */
function fold(m: OfficeModel, e: EventRow, now: number): boolean {
  const skip = streamOnly(m, e);
  const before = shows(m, now);
  const fxs = reduce(m, e, { replay: false });
  if (skip) {
    expect(shows(m, now)).toBe(before);
    expect(fxs).toEqual([]);
  }
  return skip;
}

describe('streamOnly', () => {
  it('the first chunk of a reply shows (the agent starts writing); the rest of the stream is skipped', () => {
    const m = officeWith('working');
    const now = T0 + 5000;
    const before = shows(m, now);
    expect(fold(m, text(T0 + 1000), now)).toBe(false);
    expect(shows(m, now)).not.toBe(before);
    for (let i = 2; i < 40; i++) expect(fold(m, text(T0 + i * 100), now)).toBe(true);
  });

  it('the first chunk of a thought shows; the rest is skipped', () => {
    const m = officeWith('working');
    const now = T0 + 5000;
    expect(fold(m, thought(T0 + 1000), now)).toBe(false);
    for (let i = 2; i < 40; i++) expect(fold(m, thought(T0 + i * 100), now)).toBe(true);
  });

  it('an agent already thinking: its reasoning stream is skipped from the start', () => {
    const m = officeWith('thinking');
    expect(fold(m, thought(T0 + 1000), T0 + 5000)).toBe(true);
  });

  it('tool output from a working agent is skipped', () => {
    const m = officeWith('working');
    for (let i = 1; i < 40; i++) expect(fold(m, output(T0 + i * 100), T0 + 5000)).toBe(true);
  });

  it('tool output that wakes a sleeping agent is not skipped', () => {
    const m = officeWith('idle');
    const now = T0 + LONG_IDLE_MS + 1000;
    const asleep = shows(m, now);
    expect(asleep).toContain('sleep');
    expect(fold(m, output(now), now)).toBe(false);
    expect(shows(m, now)).not.toBe(asleep);
  });

  it('usage updates are skipped', () => {
    for (const status of ['working', 'idle', 'done'] as const) {
      const m = officeWith(status);
      expect(fold(m, usage(T0 + 1000), T0 + 5000)).toBe(true);
    }
  });

  it('a chunk from an agent the office has not seen is never skipped (it walks in)', () => {
    for (const make of [text, thought, output, usage]) {
      const m = officeWith('working');
      expect(streamOnly(m, make(T0 + 1000, 'a2'))).toBe(false);
    }
  });

  it('anything that is not a streamed chunk is never skipped', () => {
    const m = officeWith('working');
    expect(streamOnly(m, ev(T0 + 1000, { type: 'status_changed', status: 'idle' }))).toBe(false);
    expect(streamOnly(m, ev(T0 + 1000, { type: 'message', item_id: 'm1', text: 'done', phase: 'final' }))).toBe(false);
    expect(streamOnly(m, ev(T0 + 1000, { type: 'reasoning', item_id: 'r1', text: 'hm' }))).toBe(false);
  });
});

describe('lasting', () => {
  it('effects that leave something behind are kept while nobody can see the office', () => {
    const kept: Fx[] = [
      { t: 'ticker', agentId: 'a1', text: 'Ada · Merged', code: null, tone: 'success', at: T0, eventId: 1 },
      { t: 'mail', agentId: 'a1', at: T0 },
      { t: 'merge', agentId: 'a1', taskId: null, at: T0 },
      { t: 'artifact', artifactId: 'art-1', at: T0 },
    ];
    for (const f of kept) expect(lasting(f)).toBe(true);
  });

  it('effects that only flash, or walk an agent somewhere, are dropped', () => {
    const dropped: Fx[] = [
      { t: 'errand', agentId: 'a1', kind: 'commit', taskId: null, at: T0 },
      { t: 'sparkle', agentId: 'a1', at: T0 },
      { t: 'plane', agentId: 'a1', to: null, at: T0 },
      { t: 'tests', agentId: 'a1', ok: true, at: T0 },
      { t: 'shelf', at: T0 },
      { t: 'sticky', noteId: 'n1', agentId: 'a1', at: T0 },
      { t: 'trash', agentId: 'a1', at: T0 },
    ];
    for (const f of dropped) expect(lasting(f)).toBe(false);
  });
});
