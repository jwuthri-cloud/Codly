// Events → behaviour: the reducer folds real events into per-agent state, and the behaviour
// layer turns that state into where each character goes, its pose, prop and emote — with the
// honesty rules (excursion thresholds, minimum dwell, errands only for real instant events).
import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { ACT_MS, CHEER_MS, deriveIntent, Dwell, DWELL_MS, Errands, intentKey, LONG_IDLE_MS, placeKey, PLAY_AFTER_MS, THINK_HOLD_MS, type Intent } from '../behavior';
import { AGENTS, TASKS } from '../demo/script';
import type { Fx, OfficeModel } from '../model';
import { sentence } from '../narrate';
import { emptyModel, reduce, syncAgents, syncTasks } from '../reducer';

const T0 = 1_800_000_000_000;
let id = 0;
const row = (t: number, agent: string | null, kind: EventKind, task: string | null = null): EventRow => ({ id: ++id, ts: T0 + t, agent_id: agent, task_id: task, run_id: null, kind });
const started = (call: string, activity: string, target: Record<string, unknown> = {}, name = 'Tool'): EventKind =>
  ({ type: 'tool_started', call_id: call, name, activity, target: { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...target }, input: {}, confidence: 'observed' }) as EventKind;
const done = (call: string, extra: Record<string, unknown> = {}): EventKind =>
  ({ type: 'tool_completed', call_id: call, ok: true, output: null, exit_code: 0, duration_ms: 1, tests_passed: null, tests_failed: null, ...extra }) as EventKind;

function fresh(): OfficeModel {
  const m = emptyModel();
  syncAgents(
    m,
    AGENTS.map((a) => ({ ...a, status: a.role === 'lead' ? 'idle' : 'working' })),
    T0,
  );
  syncTasks(m, TASKS);
  return m;
}
const live = { replay: false };
const ada = (m: OfficeModel) => m.agents.get('a-ada')!;
const intent = (m: OfficeModel, t: number, errand = null) => deriveIntent(ada(m), m, T0 + t, errand);
const kinds = (fx: Fx[]) => fx.filter((f) => f.t !== 'ticker').map((f) => (f.t === 'errand' ? `errand:${f.kind}` : f.t));

describe('events → behaviour', () => {
  it('reading happens at the desk, with a paper in hand and the document on screen', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'read', { path: '/wt/t1/src/lib/auth/session.ts' })), live);
    const { intent: i } = intent(m, 100);
    expect(placeKey(i.place)).toBe('desk');
    expect([i.pose, i.prop, i.screen, i.icon]).toEqual(['read', 'paper', 'doc', 'read']);
    expect(sentence(ada(m), T0 + 100).text).toBe('Reading session.ts…');
  });

  it('editing types at the desk with code on the monitor; creating sparkles', () => {
    const m = fresh();
    const fx = reduce(m, row(0, 'a-ada', started('c1', 'create', { path: 'src/new.ts' })), live);
    expect(kinds(fx)).toEqual(['sparkle']);
    const { intent: i } = intent(m, 50);
    expect([placeKey(i.place), i.pose, i.screen]).toEqual(['desk', 'type', 'code']);
  });

  it('tests: stay at the desk until the excursion threshold, then walk to the test monitor', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'test', { command: 'pnpm test' })), live);
    const early = intent(m, 200);
    expect(placeKey(early.intent.place)).toBe('desk');
    expect(early.intent.screen).toBe('terminal');
    expect(early.nextAt).toBe(T0 + 400);
    const later = intent(m, 450);
    expect(placeKey(later.intent.place)).toBe('st:server.monitor');
    expect(later.intent.pose).toBe('watch');
  });

  it('a test that ends before the threshold never leaves the desk (no phantom trip)', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'test', { command: 'pnpm test' })), live);
    reduce(m, row(300, 'a-ada', done('c1', { tests_passed: 4, tests_failed: 0 })), live);
    const i = intent(m, 320).intent;
    expect(i.place).toEqual({ kind: 'desk', stand: true });
    expect(i.pose).toBe('celebrate');
  });

  it('tests that fail slump at the monitor; tests that pass celebrate, then back to work', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'test', { command: 'pnpm test' })), live);
    const fail = reduce(m, row(5000, 'a-ada', done('c1', { ok: false, tests_passed: 31, tests_failed: 2 })), live);
    expect(kinds(fail)).toEqual(['tests']);
    let i = intent(m, 5100).intent;
    expect([placeKey(i.place), i.pose]).toEqual(['st:server.monitor', 'slump']);
    expect(sentence(ada(m), T0 + 5100).text).toBe('2 tests failing');
    reduce(m, row(9000, 'a-ada', started('c2', 'test', { command: 'pnpm test' })), live);
    reduce(m, row(14000, 'a-ada', done('c2', { tests_passed: 33, tests_failed: 0 })), live);
    i = intent(m, 14100).intent;
    expect([placeKey(i.place), i.pose]).toEqual(['st:server.monitor', 'celebrate']);
    expect(sentence(ada(m), T0 + 14100).text).toBe('Tests pass ✓ 33');
    expect(m.testHistory).toEqual([false, true]);
    expect(placeKey(intent(m, 14000 + CHEER_MS + 10).intent.place)).toBe('desk');
  });

  it('long commands go to the server racks; searches to the library; memory searches to the archive', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'execute', { command: 'pnpm build' })), live);
    expect(placeKey(intent(m, 1000).intent.place)).toBe('desk');
    expect(placeKey(intent(m, 1600).intent.place)).toBe('st:server.rack');
    reduce(m, row(2000, 'a-ada', done('c1')), live);
    reduce(m, row(2100, 'a-ada', started('c2', 'search', { query: 'rotate' })), live);
    expect(placeKey(intent(m, 2200).intent.place)).toBe('desk');
    expect(placeKey(intent(m, 3400).intent.place)).toBe('st:library.shelf');
    reduce(m, row(3500, 'a-ada', done('c2')), live);
    reduce(m, row(3600, 'a-ada', started('c3', 'search', { tool: 'codly_memory_search' }, 'mcp__codly__codly_memory_search')), live);
    expect(placeKey(intent(m, 5000).intent.place)).toBe('st:archive');
  });

  it('plans and notes go to the whiteboard, reviews to the meeting table, ask-lead to the Lead', () => {
    const m = fresh();
    m.plans.set('p1', { thread: 'lead', request: 'r', status: 'running', at: 0 });
    reduce(m, row(0, 'a-ada', started('c1', 'plan')), live);
    expect(intent(m, 500).intent).toMatchObject({ pose: 'write', prop: 'marker' });
    expect(placeKey(intent(m, 500).intent.place)).toBe('st:meeting.board');
    reduce(m, row(600, 'a-ada', done('c1')), live);
    ada(m).taskId = 't-1';
    const fx = reduce(m, row(700, 'a-ada', started('c2', 'message', { tool: 'codly_ask_lead' }, 'mcp__codly__codly_ask_lead')), live);
    expect(fx.find((f) => f.t === 'plane')).toMatchObject({ agentId: 'a-ada', to: 'a-mira' });
    expect(placeKey(intent(m, 1200).intent.place)).toBe('lead:a-mira');
    reduce(m, row(1300, 'a-ada', done('c2')), live);
    reduce(m, row(1400, 'a-ada', started('c3', 'review')), live);
    expect(placeKey(intent(m, 1900).intent.place)).toBe('st:meeting.table');
  });

  it('an approval raises a hand at the desk with the amber beacon; resolving lowers it', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'execute', { command: 'rm -rf dist' })), live);
    reduce(
      m,
      row(100, 'a-ada', {
        type: 'approval_requested',
        approval: { id: 'ap', agent_id: 'a-ada', run_id: 'r', task_id: null, call_id: 'c1', subject: { kind: 'command', command: 'rm -rf dist', cwd: null }, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 0 },
      }),
      live,
    );
    const i = intent(m, 2000).intent;
    expect(i.place).toEqual({ kind: 'desk', stand: true });
    expect([i.pose, i.emote, i.screen]).toEqual(['raise', 'alert', 'alert']);
    expect(sentence(ada(m), T0 + 2000)).toMatchObject({ pre: 'May I run', code: 'rm -rf dist', tone: 'warn' });
    reduce(m, row(3000, 'a-ada', { type: 'approval_resolved', approval_id: 'ap', status: 'approved', by: 'user' }), live);
    expect(intent(m, 3100).intent.emote).toBeNull();
  });

  it('blocked slumps under a rain cloud; rate limited sleeps (zZ); long idle falls asleep at 10 min', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', { type: 'error', message: 'ESLint config not found', fatal: true }), live);
    expect(intent(m, 10).intent).toMatchObject({ pose: 'slump', emote: 'rain', screen: 'error' });
    reduce(m, row(100, 'a-ada', { type: 'rate_limit', window: '5h', used_fraction: 1, resets_at: null, status: 'rejected' }), live);
    expect(intent(m, 110).intent).toMatchObject({ pose: 'sleep', emote: 'zz' });
    const m2 = fresh();
    reduce(m2, row(0, 'a-ada', { type: 'status_changed', status: 'idle' }), live);
    const before = intent(m2, LONG_IDLE_MS - 1);
    expect(before.intent.place).toEqual({ kind: 'station', id: expect.stringMatching(/^games\./) });
    expect(before.nextAt).toBe(T0 + LONG_IDLE_MS);
    expect(intent(m2, LONG_IDLE_MS + 1).intent).toMatchObject({ pose: 'sleep', emote: 'zz' });
  });

  it('idle workers play a game after a minute (always the same one), sleep at the desk after 10; Leads never play', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', { type: 'status_changed', status: 'idle' }), live);
    const before = intent(m, PLAY_AFTER_MS - 1);
    expect([placeKey(before.intent.place), before.intent.pose]).toEqual(['desk', 'sit']);
    expect(before.nextAt).toBe(T0 + PLAY_AFTER_MS);
    const play = intent(m, PLAY_AFTER_MS);
    const key = placeKey(play.intent.place);
    expect(['st:games.console', 'st:games.foosball', 'st:games.pool']).toContain(key);
    expect(['gamepad', 'play']).toContain(play.intent.pose);
    expect([play.intent.icon, play.intent.screen]).toEqual(['idle', 'off']);
    expect(play.nextAt).toBe(T0 + LONG_IDLE_MS);
    for (const t of [PLAY_AFTER_MS + 5000, LONG_IDLE_MS / 2, LONG_IDLE_MS - 1]) expect(placeKey(intent(m, t).intent.place)).toBe(key);
    const sleep = intent(m, LONG_IDLE_MS).intent;
    expect([placeKey(sleep.place), sleep.pose, sleep.emote]).toEqual(['desk', 'sleep', 'zz']);
    // Each game is somebody's.
    const games = new Set<string>();
    for (const a of m.agents.values()) {
      if (a.role === 'lead') continue;
      a.status = 'idle';
      a.lastEventAt = T0;
      games.add(placeKey(deriveIntent(a, m, T0 + PLAY_AFTER_MS, null).intent.place));
    }
    expect(games.size).toBeGreaterThan(1);
    const lead = m.agents.get('a-mira')!;
    expect(lead.role).toBe('lead');
    lead.status = 'idle';
    lead.lastEventAt = T0;
    for (const t of [PLAY_AFTER_MS, LONG_IDLE_MS - 1]) expect(placeKey(deriveIntent(lead, m, T0 + t, null).intent.place)).toBe('desk');
  });

  it('a tool call or an approval brings a player straight back to the desk', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', { type: 'status_changed', status: 'idle' }), live);
    expect(placeKey(intent(m, PLAY_AFTER_MS + 10).intent.place)).toMatch(/^st:games\./);
    reduce(m, row(PLAY_AFTER_MS + 20, 'a-ada', started('c1', 'edit', { path: 'a.ts' })), live);
    expect([placeKey(intent(m, PLAY_AFTER_MS + 30).intent.place), intent(m, PLAY_AFTER_MS + 30).intent.pose]).toEqual(['desk', 'type']);

    const m2 = fresh();
    reduce(m2, row(0, 'a-ada', { type: 'status_changed', status: 'idle' }), live);
    expect(placeKey(intent(m2, PLAY_AFTER_MS + 10).intent.place)).toMatch(/^st:games\./);
    reduce(
      m2,
      row(PLAY_AFTER_MS + 20, 'a-ada', {
        type: 'approval_requested',
        approval: { id: 'ap', agent_id: 'a-ada', run_id: 'r', task_id: null, call_id: 'c1', subject: { kind: 'command', command: 'ls', cwd: null }, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 0 },
      }),
      live,
    );
    expect(intent(m2, PLAY_AFTER_MS + 30).intent.place).toEqual({ kind: 'desk', stand: true });
  });

  it('a short think between two calls holds the last pose (no flicker); a long one shows the thought cloud', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'read', { path: 'a.ts' })), live);
    reduce(m, row(500, 'a-ada', done('c1')), live);
    const held = intent(m, 700).intent;
    expect([held.pose, held.icon, held.emote]).toEqual(['read', 'thinking', null]);
    expect(intent(m, 500 + THINK_HOLD_MS + 1).intent).toMatchObject({ pose: 'think', emote: 'think' });
  });

  it('task cards: running → the worker fetches its card; review → it brings it back', () => {
    const m = fresh();
    const go = reduce(m, row(0, null, { type: 'task_changed', task: { ...TASKS[0], status: 'running' } }, 't-1'), live);
    expect(kinds(go)).toEqual(['card', 'errand:card-take']);
    const back = reduce(m, row(100, null, { type: 'task_changed', task: { ...TASKS[0], status: 'reviewing' } }, 't-1'), live);
    expect(kinds(back)).toEqual(['card', 'errand:card-return']);
    const fin = reduce(m, row(200, null, { type: 'task_changed', task: { ...TASKS[0], status: 'done' } }, 't-1'), live);
    expect(kinds(fin)).toEqual(['card']);
    expect(m.tasks.get('t-1')!.column).toBe('done');
  });

  it('notes become sticky notes; memories light a drawer; a memory right after its tool needs no extra trip', () => {
    const m = fresh();
    const mem = (t: number, kind: 'note' | 'decision', text: string) =>
      row(t, 'a-ada', { type: 'memory_added', memory: { id: `m${t}`, workspace_id: null, scope: 'task', scope_id: null, kind, text, author: 'a-ada', source_event: null, importance: 0.5, pinned: false, superseded_by: null, created_at: 0, accessed_at: 0 } });
    expect(kinds(reduce(m, mem(0, 'note', 'Tokens rotate on use'), live))).toEqual(['sticky', 'errand:note']);
    expect(m.notes.map((n) => n.text)).toEqual(['Tokens rotate on use']);
    reduce(m, row(1000, 'a-ada', started('c1', 'memory', { tool: 'codly_memory_write' })), live);
    reduce(m, row(1500, 'a-ada', done('c1')), live);
    expect(kinds(reduce(m, mem(1600, 'decision', 'httpOnly cookies'), live))).toEqual(['drawer']);
    expect(m.memories).toHaveLength(1);
  });

  it('commits and merges are mail; replay folds state without effects', () => {
    const m = fresh();
    expect(kinds(reduce(m, row(0, 'a-ada', { type: 'committed', sha: 'abc', files: ['a.ts'], message: 'm' }, 't-1'), live))).toEqual(['mail', 'errand:commit']);
    expect(kinds(reduce(m, row(10, 'a-ada', { type: 'merged', sha: 'def', branch: 'codly/x/integration' }, 't-1'), live))).toEqual(['merge', 'errand:merge']);
    const r = emptyModel();
    syncAgents(r, AGENTS, T0);
    const fx = reduce(r, row(0, 'a-ada', { type: 'committed', sha: 'abc', files: ['a.ts'], message: 'm' }, 't-1'), { replay: true });
    expect(kinds(fx)).toEqual([]);
    expect(r.commits).toHaveLength(1);
  });

  it('labels are sanitized: hostile file names cannot inject controls or bidi tricks', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'edit', { path: 'src/‮gnp.ts\u0007' })), live);
    expect(sentence(ada(m), T0 + 10).code).toBe('gnp.ts');
  });
});

describe('dwell and errands', () => {
  it('holds a pose for at least 600 ms; the latest request wins when it ends', () => {
    const d = new Dwell<Intent>(DWELL_MS, intentKey);
    const mk = (pose: Intent['pose']): Intent => ({ place: { kind: 'desk' }, pose, prop: null, emote: null, screen: 'idle', icon: pose, errand: null });
    expect(d.request('a', mk('read'), 0).pose).toBe('read');
    expect(d.request('a', mk('type'), 100).pose).toBe('read');
    expect(d.request('a', mk('think'), 200).pose).toBe('read');
    expect(d.nextAt()).toBe(DWELL_MS);
    expect(d.flush(DWELL_MS)).toEqual(['a']);
    expect(d.current('a')!.pose).toBe('think');
    expect(d.request('a', mk('type'), DWELL_MS + DWELL_MS).pose).toBe('type');
  });

  it('runs an errand on arrival, finishes it after the action, and drops stale ones', () => {
    const log: string[] = [];
    const e = new Errands((id, er, phase) => log.push(`${id}:${er.kind}:${phase}`));
    e.push('a', 'commit', 't1', 0);
    e.push('a', 'commit', 't1', 10); // the same trip
    expect(e.size).toBe(1);
    const cur = e.current('a', 100, true)!;
    expect(cur.kind).toBe('commit');
    e.arrived('a', 'st:mail', 3000, () => 'st:mail');
    expect(log).toEqual(['a:commit:act']);
    expect(e.current('a', 3000 + ACT_MS - 1, true)).not.toBeNull();
    expect(e.current('a', 3000 + ACT_MS, true)).toBeNull();
    expect(log).toEqual(['a:commit:act', 'a:commit:done']);
    // Not allowed (busy, blocked…): it waits, then expires instead of running late.
    e.push('b', 'memory', null, 0);
    expect(e.current('b', 1000, false)).toBeNull();
    expect(e.current('b', 20_000, false)).toBeNull();
    expect(log.at(-1)).toBe('b:memory:drop');
  });

  it('a live station activity pre-empts an errand (the truth wins over the story)', () => {
    const m = fresh();
    reduce(m, row(0, 'a-ada', started('c1', 'test', { command: 'pnpm test' })), live);
    const e = { kind: 'commit' as const, taskId: null, at: T0, arrivedAt: null };
    const i = deriveIntent(ada(m), m, T0 + 600, e).intent;
    expect(i.errand).toBeNull();
    expect(placeKey(i.place)).toBe('st:server.monitor');
  });
});
