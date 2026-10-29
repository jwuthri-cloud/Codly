import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Task } from '../gen/Task';
import { narrateFrom, type NarrationInput } from '../narrate';
import type { AgentLive, Seg } from '../state/store.svelte';
import { repoMap } from '../demo/repomap';
import { emptyCareer } from '../util/career';

const T0 = 1_760_000_000_000;

const agent = (o: Partial<Agent> = {}): Agent => ({
  id: 'a1', name: 'Ada', provider: 'claude', model: 'claude-sonnet-5-5', role: 'worker', status: 'working', activity: null, activity_label: null,
  task_id: 't1', run_id: 'r1', session_id: null, worktree: '/wt/t1-ada', billing: 'subscription', color: 7, look_seed: 'ada', thread: null, account: null, member: null, title: null, consultant: false, archived: false,
  usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: null, context_window: null, cost_usd: null },
  career: emptyCareer(), created_at: T0, updated_at: T0, ...o,
});

const task = (o: Partial<Task> = {}): Task => ({
  id: 't1', plan_id: 'p1', key: 't1', title: 'Theme provider', goal: '', acceptance: [], write_scope: [], verify: [], depends_on: [], status: 'running',
  assignee: 'a1', agent_hint: null, account_hint: null, consultant: null, fresh_session: false, branch: null, worktree: null, attempt: 1, status_reason: null, summary: null, created_at: T0, updated_at: T0, ...o,
});

const seg = (o: Partial<Seg>): Seg => ({
  callId: 'c1', agentId: 'a1', taskId: 't1', activity: 'read', name: 'Read', label: '', target: { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null },
  input: {}, confidence: 'observed', start: T0, end: null, ok: null, exitCode: null, testsPassed: null, testsFailed: null, eventId: 1, ...o,
});

const live = (segments: Seg[], o: Partial<AgentLive> = {}): AgentLive => {
  const open: Record<string, number> = {};
  segments.forEach((s, i) => s.end == null && (open[s.callId] = i));
  return {
    segments, open, todo: [], todoTs: 0, files: {}, messages: [], reasoning: [], retrievals: [], searches: [], session: null, turn: null, turns: [],
    statusSince: T0, lastOutput: null, errors: [], subagents: [], firstTs: T0, lastTs: T0, ...o,
  };
};

const input = (o: Partial<NarrationInput> = {}): NarrationInput => ({
  agent: agent(), now: T0 + 41_000, approvals: [], live: live([]), streaming: null, task: task(), lastDone: null, rate: null, ...o,
});

describe('narration', () => {
  it('says what file is being read', () => {
    const n = narrateFrom(input({ live: live([seg({ target: { path: 'src/server/auth.ts', paths: [], command: null, query: null, url: null, server: null, tool: null } })]) }));
    expect(n.text).toBe('Reading `auth.ts`');
    expect(n.tone).toBe('live');
  });

  it('adds a timer to commands and keeps a stable key without it', () => {
    const n = narrateFrom(input({ live: live([seg({ activity: 'test', name: 'Bash', target: { path: null, paths: [], command: 'pnpm test', query: null, url: null, server: null, tool: null } })]) }));
    expect(n.text).toBe('Running `pnpm test` · 0:41');
    expect(n.key).toBe('Running `pnpm test`');
  });

  it('says exactly what an approval asks', () => {
    const ap: Approval = {
      id: 'ap1', agent_id: 'a1', run_id: 'r1', task_id: 't1', call_id: null, subject: { kind: 'command', command: 'rm -rf dist', cwd: '/wt' },
      reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: T0 + 30_000,
    };
    const n = narrateFrom(input({ agent: agent({ status: 'waiting_approval' }), approvals: [ap] }));
    expect(n.key).toBe('Waiting for you: `rm -rf dist`');
    expect(n.tone).toBe('wait');
  });

  it('reports the diff of the last finished task', () => {
    const n = narrateFrom(input({
      agent: agent({ status: 'idle', task_id: null }), task: null,
      lastDone: { task: task({ status: 'done' }), stat: { files: 2, added: 48, removed: 6 }, files: 2 },
    }));
    expect(n.text).toBe('Done — +48 −6 in 2 files');
    expect(n.tone).toBe('done');
  });

  it('shows test counts right after a run', () => {
    const n = narrateFrom(input({
      agent: agent({ status: 'thinking' }),
      live: live([seg({ activity: 'test', end: T0 + 5000, ok: false, testsPassed: 2, testsFailed: 1, target: { path: null, paths: [], command: 'npx playwright test', query: null, url: null, server: null, tool: null } })]),
    }));
    expect(n.text).toBe('Thinking · last run: 1 failed, 2 passed');
  });

  it('names Codly tools in plain words', () => {
    const n = narrateFrom(input({ live: live([seg({ activity: 'mcp', name: 'mcp__codly__codly_ask_lead', target: { path: null, paths: [], command: null, query: null, url: null, server: 'codly', tool: 'codly_ask_lead' } })]) }));
    expect(n.text).toBe('Asking the Lead');
  });

  it('measures todo progress', () => {
    const n = narrateFrom(input({ live: live([], { todo: [{ text: 'a', status: 'completed' }, { text: 'b', status: 'in_progress' }, { text: 'c', status: 'pending' }, { text: 'd', status: 'pending' }] }) }));
    expect(n.progress).toBe(0.25);
  });
});

describe('demo repo map', () => {
  it('is a believable ~300-file repo whose new files appear once committed', () => {
    const before = repoMap(new Set());
    const after = repoMap(new Set(['t1']));
    expect(before.length).toBeGreaterThan(260);
    expect(before.length).toBeLessThan(360);
    expect(before.some((f) => f.path === 'src/theme/ThemeProvider.tsx')).toBe(false);
    expect(after.some((f) => f.path === 'src/theme/ThemeProvider.tsx')).toBe(true);
    expect(before.find((f) => f.path === 'src/app/layout.tsx')?.lang).toBe('typescript');
  });
});
