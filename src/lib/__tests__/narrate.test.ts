import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Task } from '../gen/Task';
import { narrateFrom, type NarrationInput } from '../narrate';
import type { AgentLive, Seg, SubAgent } from '../state/store.svelte';
import { repoMap } from '../demo/repomap';
import { emptyCareer } from '../util/career';

const T0 = 1_760_000_000_000;

const agent = (o: Partial<Agent> = {}): Agent => ({
  id: 'a1', name: 'Ada', provider: 'claude', model: 'claude-sonnet-5-5', role: 'worker', status: 'working', activity: null, activity_label: null,
  task_id: 't1', run_id: 'r1', session_id: null, worktree: '/wt/t1-ada', billing: 'subscription', color: 7, look_seed: 'ada', thread: null, account: null, member: null, title: null, consultant: false, kept: false, archived: false,
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
    statusSince: T0, lastOutput: null, errors: [], subagents: [], mcp: [], background: [], firstTs: T0, lastTs: T0, ...o,
  };
};

const input = (o: Partial<NarrationInput> = {}): NarrationInput => ({
  agent: agent(), now: T0 + 41_000, approvals: [], live: live([]), streaming: null, task: task(), lastDone: null, rate: null, inTerminal: false, ...o,
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
    // A command in Claude Code's sandbox wants a host: the host, not the name of the request's tool.
    const tool = (name: string, asked: Record<string, string>, provider: Agent['provider'] = 'claude') =>
      narrateFrom(input({ agent: agent({ status: 'waiting_approval', provider }), approvals: [{ ...ap, subject: { kind: 'tool', name, input: asked } }] })).key;
    expect(tool('SandboxNetworkAccess', { host: 'example.com' })).toBe('Waiting for you: reach `example.com`');
    expect(tool('SandboxNetworkAccess', {})).toBe('Waiting for you: use the network');
    expect(tool('WebFetch', { url: 'https://example.com' })).toBe('Waiting for you: use `WebFetch`');
    // A Claude agent's plan to read is said as that, not by the name of the request's tool.
    expect(tool('ExitPlanMode', { plan: '1. Add the line' })).toBe('Waiting for you: read its plan');
    // Those requests are Claude Code's own: from another CLI the same name is a tool like any other.
    expect(tool('SandboxNetworkAccess', { host: 'example.com' }, 'codex')).toBe('Waiting for you: use `SandboxNetworkAccess`');
    expect(tool('SandboxNetworkAccess', { host: 'example.com' }, 'gemini')).toBe('Waiting for you: use `SandboxNetworkAccess`');
    expect(tool('ExitPlanMode', { plan: '1. Add the line' }, 'codex')).toBe('Waiting for you: use `ExitPlanMode`');
  });

  it('a task waiting for you narrates as wait', () => {
    const n = narrateFrom(input({ agent: agent({ status: 'idle' }), task: task({ status: 'blocked', status_reason: 'waiting for you: Which provider?' }) }));
    expect(n.tone).toBe('wait');
    // Any other blocked reason stays red.
    expect(narrateFrom(input({ agent: agent({ status: 'idle' }), task: task({ status: 'blocked', status_reason: 'verify failed' }) })).tone).toBe('blocked');
  });

  it('says an agent is in your terminal, whatever its row says', () => {
    // After a restart of Codly its row says it stopped; a task of its own may be blocked.
    for (const a of [agent({ status: 'offline', task_id: null }), agent({ status: 'idle' })]) {
      const n = narrateFrom(input({ agent: a, task: task({ status: 'blocked', status_reason: 'stopped by you' }), inTerminal: true }));
      expect([n.text, n.tone]).toEqual(['In your terminal', 'idle']);
    }
    expect(narrateFrom(input({ agent: agent({ status: 'offline' }) })).text).toBe('Stopped');
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

// The fourth and fifth browser reviews (defect 8, defect 11): after Eli's turn ended with a helper
// at work in the background, the header, the line above the composer and "Now" said "Searching
// for docs/**/*.md" with a pulsing dot and "Interrupt". A helper's call is the helper's work.
describe('an agent whose helper works', () => {
  const helper = (callId: string, description: string, status = 'running'): SubAgent => ({ callId, description, status, startedAt: T0, ts: T0, name: null, step: null, toolUses: null, tokens: null, report: null });
  const glob = seg({ callId: 'g1', activity: 'search', name: 'Glob', target: { path: null, paths: [], command: null, query: 'docs/**/*.md', url: null, server: null, tool: null }, parent: 'h1' });
  const solo = (status: Agent['status']) => agent({ role: 'solo', status, task_id: null });

  it('rests once its turn ended with a helper’s Glob call open, and says the helper works in the background', () => {
    const n = narrateFrom(input({ agent: solo('idle'), task: null, live: live([glob], { subagents: [helper('h1', 'Look at the docs')] }) }));
    expect(n.text).toBe('Ready to chat · 1 helper at work in the background');
    expect(n.tone).toBe('idle');
  });

  it('counts the helpers and the commands of its live run in the background', () => {
    const cmd = (id: string, runId: string, status = 'running') => ({ id, kind: 'command', description: 'npm run dev', status, runId, startedAt: T0, endedAt: null });
    const l = live([], { subagents: [helper('h1', 'a'), helper('h2', 'b'), helper('h3', 'c', 'completed')], background: [cmd('t1', 'r1'), cmd('t2', 'r0'), cmd('t3', 'r1', 'stopped')] });
    expect(narrateFrom(input({ agent: solo('idle'), task: null, live: l })).text).toBe('Ready to chat · 2 helpers and 1 command at work in the background');
    expect(narrateFrom(input({ agent: solo('idle'), task: null, live: live([]) })).text).toBe('Ready to chat');
  });

  it('waits for its helper while its turn runs and only the helper’s call is open', () => {
    const n = narrateFrom(input({ agent: solo('working'), task: null, live: live([glob], { subagents: [helper('h1', 'Look at the docs')] }) }));
    expect(n.text).toBe('Waiting for its helper “Look at the docs”');
    expect(n.tone).toBe('live');
  });

  it('says its own call while its turn runs, not its helper’s newer one', () => {
    const own = seg({ callId: 'c1', activity: 'execute', name: 'Bash', start: T0 - 5000, target: { path: null, paths: [], command: 'pnpm test', query: null, url: null, server: null, tool: null } });
    const n = narrateFrom(input({ agent: solo('working'), task: null, live: live([own, glob], { subagents: [helper('h1', 'Look at the docs')] }) }));
    expect(n.key).toBe('Running `pnpm test`');
  });
});

// The fifth browser review (defect 19): `?turn=failed` said "Ready to chat" for an agent whose
// last turn failed.
describe('an agent whose last turn failed', () => {
  const failed = (o: Partial<AgentLive> = {}) => live([], { turn: { startedAt: T0, input: 'Fix it', endedAt: T0 + 3000, status: 'failed' }, ...o });

  it('says that its last turn failed, with the reason its CLI gave', () => {
    const n = narrateFrom(input({ agent: agent({ role: 'solo', status: 'idle', task_id: null }), task: null, live: failed({ errors: [{ message: 'Anthropic’s servers are overloaded: wait a moment, then try again', fatal: false, ts: T0 + 2900 }] }) }));
    expect(n.text).toMatch(/^Its last turn failed: Anthropic’s servers are overloaded/);
    expect(n.tone).toBe('blocked');
  });

  it('an error from before the turn is not its reason; a turn that ended well is ready again', () => {
    const solo = agent({ role: 'solo', status: 'idle', task_id: null });
    expect(narrateFrom(input({ agent: solo, task: null, live: failed({ errors: [{ message: 'old', fatal: false, ts: T0 - 1 }] }) })).text).toBe('Its last turn failed');
    expect(narrateFrom(input({ agent: solo, task: null, live: live([], { turn: { startedAt: T0, input: 'x', endedAt: T0 + 1, status: 'completed' } }) })).text).toBe('Ready to chat');
  });
});
