// The scripted demo session. Two Leads serve two parallel requests; four workers across
// providers read, search, edit, create files, run tests (fail, then pass), ask for an
// approval, post notes, ask their Lead, write memory and knowledge facts, generate an image,
// get blocked, hit a rate limit, get reviewed and merge. Every event is shaped exactly like
// the core's (crates/codly-core/src/model.rs).

import type { ActivityKind } from '../../lib/gen/ActivityKind';
import type { Agent } from '../../lib/gen/Agent';
import type { AgentStatus } from '../../lib/gen/AgentStatus';
import type { Approval } from '../../lib/gen/Approval';
import type { EventKind } from '../../lib/gen/EventKind';
import type { Plan } from '../../lib/gen/Plan';
import type { Provider } from '../../lib/gen/Provider';
import type { Role } from '../../lib/gen/Role';
import type { Task } from '../../lib/gen/Task';
import type { TaskStatus } from '../../lib/gen/TaskStatus';

export interface Step {
  t: number;
  agent: string | null;
  task: string | null;
  kind: EventKind;
}

const WT = '/Users/demo/.codly/worktrees/atlas';

export function mkAgent(id: string, name: string, provider: Provider, role: Role, color: number, status: AgentStatus, thread: string | null = null): Agent {
  return {
    id,
    thread,
    name,
    provider,
    model: null,
    role,
    status,
    activity: null,
    activity_label: null,
    task_id: null,
    run_id: null,
    session_id: null,
    worktree: null,
    billing: 'subscription',
    account: null,
    color,
    look_seed: id,
    member: null,
    title: null,
    consultant: false,
    archived: false,
    usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: null, context_window: null, cost_usd: null },
    created_at: 0,
    updated_at: 0,
  };
}

export const AGENTS: Agent[] = [
  mkAgent('a-mira', 'Mira', 'claude', 'lead', 6, 'idle', 'lead'),
  mkAgent('a-noor', 'Noor', 'codex', 'lead', 1, 'idle', 'lead:r2'),
  mkAgent('a-ada', 'Ada', 'claude', 'worker', 0, 'offline'),
  mkAgent('a-bo', 'Bo', 'codex', 'worker', 4, 'offline'),
  mkAgent('a-cy', 'Cy', 'gemini', 'worker', 2, 'offline'),
  mkAgent('a-eli', 'Eli', 'opencode', 'worker', 3, 'offline'),
  mkAgent('a-dee', 'Dee', 'opencode', 'reviewer', 7, 'offline'),
];

function mkTask(id: string, plan: string, key: string, title: string, assignee: string, created: number): Task {
  return {
    id,
    plan_id: plan,
    key,
    title,
    goal: title,
    acceptance: [],
    write_scope: [],
    verify: ['pnpm test'],
    depends_on: [],
    status: 'pending',
    assignee,
    agent_hint: null,
    account_hint: null,
    consultant: null,
    fresh_session: false,
    branch: `codly/atlas/${key}`,
    worktree: `${WT}/${key}`,
    attempt: 1,
    status_reason: null,
    summary: null,
    created_at: created,
    updated_at: 0,
  };
}

export const TASKS: Task[] = [
  mkTask('t-1', 'p1', 't1', 'Rotate refresh tokens on every use', 'a-ada', 1),
  mkTask('t-2', 'p1', 't2', 'Auth guide for the new flow', 'a-cy', 2),
  mkTask('t-3', 'p2', 't3', 'Dark-mode color tokens', 'a-bo', 3),
  mkTask('t-4', 'p2', 't4', 'Theme toggle in settings', 'a-eli', 4),
];

function plan(id: string, thread: string, request: string, status: Plan['status']): Plan {
  return { id, workspace_id: 'w1', thread, request, summary: request, assumptions: [], open_questions: [], status, base_commit: null, integration_branch: 'codly/atlas/integration', integration_worktree: null, merged_sha: null, kept: false, created_at: 0, updated_at: 0 };
}

let call = 0;

export function buildScript(): Step[] {
  call = 0;
  const steps: Step[] = [];
  const at = (t: number, agent: string | null, kind: EventKind, task: string | null = null) => steps.push({ t, agent, task, kind });
  const status = (t: number, agent: string, s: AgentStatus) => at(t, agent, { type: 'status_changed', status: s });
  const tool = (
    t0: number,
    t1: number,
    agent: string,
    activity: ActivityKind,
    name: string,
    target: Partial<{ path: string; paths: string[]; command: string; query: string; url: string; tool: string; server: string }>,
    done: Partial<{ ok: boolean; tests_passed: number; tests_failed: number }> = {},
  ) => {
    const id = `c${++call}`;
    at(t0, agent, {
      type: 'tool_started',
      call_id: id,
      name,
      activity,
      target: { path: target.path ?? null, paths: target.paths ?? [], command: target.command ?? null, query: target.query ?? null, url: target.url ?? null, server: target.server ?? null, tool: target.tool ?? null },
      input: {},
      confidence: 'observed',
    });
    at(t1, agent, { type: 'tool_completed', call_id: id, ok: done.ok ?? true, output: null, exit_code: done.ok === false ? 1 : 0, duration_ms: Math.round((t1 - t0) * 1000), tests_passed: done.tests_passed ?? null, tests_failed: done.tests_failed ?? null });
    return id;
  };
  const task = (t: number, id: string, s: TaskStatus) => {
    const base = TASKS.find((x) => x.id === id)!;
    at(t, null, { type: 'task_changed', task: { ...base, status: s, updated_at: 0 } }, id);
  };
  const assign = (t: number, agentId: string, taskId: string | null, s: AgentStatus) => {
    const base = AGENTS.find((a) => a.id === agentId)!;
    at(t, agentId, { type: 'agent_changed', agent: { ...base, task_id: taskId, status: s } });
  };
  const chat = (t: number, author: string, thread: string, text: string) =>
    at(t, author === 'user' ? null : author, { type: 'chat', message: { id: `m${t}`, thread, author, text, attachment: null, ts: 0 } });
  const memory = (t: number, agent: string, kind: 'note' | 'decision' | 'fact' | 'lesson', text: string, taskId: string | null) =>
    at(t, agent, { type: 'memory_added', memory: { id: `mem-${t}`, workspace_id: 'w1', scope: kind === 'note' ? 'task' : 'project', scope_id: taskId, kind, text, title: '', tags: [], refs: [], author: agent, source_event: null, importance: 0.7, pinned: false, superseded_by: null, created_at: 0, accessed_at: 0 } }, taskId);
  const todo = (t: number, agent: string, items: [string, 'pending' | 'in_progress' | 'completed'][]) => at(t, agent, { type: 'plan_updated', steps: items.map(([text, s]) => ({ text, status: s })) });

  const MIRA = 'a-mira';
  const NOOR = 'a-noor';
  const ADA = 'a-ada';
  const BO = 'a-bo';
  const CY = 'a-cy';
  const ELI = 'a-eli';
  const DEE = 'a-dee';
  const web = 'apps/web/src';

  // ── request 1 → Mira (Claude Code) ──
  chat(0, 'user', 'lead', 'Rotate refresh tokens on every use and document the new auth flow.');
  at(0.2, MIRA, { type: 'turn_started', input: null });
  status(0.3, MIRA, 'thinking');
  tool(1.6, 2.5, MIRA, 'read', 'Read', { path: 'README.md' });
  tool(2.7, 5.0, MIRA, 'search', 'Grep', { query: 'refresh_token', path: `${web}/lib/auth` });
  at(5.0, MIRA, { type: 'retrieval', query: 'refresh token rotation', confidence: 'high', citations: [] });
  tool(5.3, 6.4, MIRA, 'read', 'Read', { path: 'docs/adr/0003-session-cookies.md' });
  todo(6.6, MIRA, [
    ['Map the auth flow', 'completed'],
    ['Split into tasks', 'in_progress'],
    ['Brief Ada and Cy', 'pending'],
    ['Review and merge', 'pending'],
  ]);
  tool(6.8, 9.0, MIRA, 'plan', 'mcp__codly__codly_submit_plan', { tool: 'codly_submit_plan', server: 'codly' });
  at(9.0, MIRA, { type: 'plan_changed', plan: plan('p1', 'lead', 'Rotate refresh tokens and document the flow', 'running') });
  task(9.1, 't-1', 'ready');
  task(9.15, 't-2', 'ready');
  chat(9.3, MIRA, 'lead', 'Plan: Ada rotates the tokens, Cy writes the guide.');
  todo(9.4, MIRA, [
    ['Map the auth flow', 'completed'],
    ['Split into tasks', 'completed'],
    ['Brief Ada and Cy', 'completed'],
    ['Review and merge', 'pending'],
  ]);
  at(9.6, MIRA, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 9400 });
  status(9.7, MIRA, 'idle');

  // ── request 2 → Noor (Codex) ──
  chat(1.0, 'user', 'lead:r2', 'Ship a dark mode with a toggle in settings.');
  at(1.2, NOOR, { type: 'turn_started', input: null });
  status(1.3, NOOR, 'thinking');
  tool(2.6, 3.6, NOOR, 'read', 'read_file', { path: `${web}/styles/tokens.css` });
  tool(4.0, 7.0, NOOR, 'fetch', 'web_search', { url: 'https://developer.mozilla.org/docs/Web/CSS/@media/prefers-color-scheme', query: 'prefers-color-scheme' });
  tool(7.4, 9.8, NOOR, 'plan', 'codly_submit_plan', { tool: 'codly_submit_plan', server: 'codly' });
  at(9.8, NOOR, { type: 'plan_changed', plan: plan('p2', 'lead:r2', 'Ship a dark mode with a toggle', 'running') });
  todo(9.85, NOOR, [
    ['Audit color tokens', 'completed'],
    ['Plan tokens + toggle', 'completed'],
    ['Watch contrast', 'pending'],
  ]);
  task(9.9, 't-3', 'ready');
  task(9.95, 't-4', 'ready');
  at(10.3, NOOR, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 9000 });
  status(10.4, NOOR, 'idle');

  // ── the crew arrives: each worker takes its card to its desk ──
  task(10.6, 't-1', 'running');
  assign(10.7, ADA, 't-1', 'starting');
  task(11.4, 't-3', 'running');
  assign(11.5, BO, 't-3', 'starting');
  task(12.2, 't-2', 'running');
  assign(12.3, CY, 't-2', 'starting');
  task(13.0, 't-4', 'running');
  assign(13.1, ELI, 't-4', 'starting');

  // ── Ada (Claude Code): refresh-token rotation ──
  status(21.0, ADA, 'working');
  tool(21.2, 22.8, ADA, 'read', 'Read', { path: `${WT}/t1/${web}/lib/auth/session.ts` });
  tool(23.0, 23.6, ADA, 'read', 'Read', { path: `${WT}/t1/${web}/lib/auth/token.ts` });
  tool(23.8, 26.0, ADA, 'search', 'Grep', { query: 'rotate', path: `${WT}/t1/crates/server/src/auth` });
  tool(26.4, 29.4, ADA, 'edit', 'Edit', { path: `${WT}/t1/${web}/lib/auth/session.ts` });
  at(29.4, ADA, { type: 'files_changed', call_id: null, paths: [`${web}/lib/auth/session.ts`], source: 'provider' });
  tool(29.8, 32.0, ADA, 'create', 'Write', { path: `${WT}/t1/${web}/lib/auth/refresh.ts` });
  tool(32.3, 33.0, ADA, 'note', 'mcp__codly__codly_post_note', { tool: 'codly_post_note', server: 'codly' });
  memory(33.0, ADA, 'note', 'Refresh tokens now rotate on every use; old ones are revoked at once.', 't-1');
  tool(33.4, 38.4, ADA, 'test', 'Bash', { command: 'pnpm test -- auth' }, { ok: false, tests_passed: 31, tests_failed: 2 });
  status(38.6, ADA, 'thinking');
  status(40.0, ADA, 'working');
  tool(40.2, 42.6, ADA, 'edit', 'Edit', { path: `${WT}/t1/${web}/lib/auth/refresh.ts` });
  tool(42.9, 47.4, ADA, 'test', 'Bash', { command: 'pnpm test -- auth' }, { ok: true, tests_passed: 33, tests_failed: 0 });
  // An approval: a destructive command.
  const rm = `c${++call}`;
  at(48.6, ADA, { type: 'tool_started', call_id: rm, name: 'Bash', activity: 'execute', target: { path: null, paths: [], command: 'rm -rf apps/web/dist', query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' });
  const approval: Approval = {
    id: 'ap-1',
    agent_id: ADA,
    run_id: 'r-ada',
    task_id: 't-1',
    call_id: rm,
    subject: { kind: 'command', command: 'rm -rf apps/web/dist', cwd: `${WT}/t1` },
    reason: 'Clean stale build output before the production build',
    choices: ['allow_once', 'deny'],
    risks: ['deletes files'],
    status: 'pending',
    decided_by: null,
    created_at: 0,
  };
  at(48.7, ADA, { type: 'approval_requested', approval });
  status(48.8, ADA, 'waiting_approval');
  at(55.5, ADA, { type: 'approval_resolved', approval_id: 'ap-1', status: 'approved', by: 'user' });
  status(55.6, ADA, 'working');
  at(55.9, ADA, { type: 'tool_completed', call_id: rm, ok: true, output: null, exit_code: 0, duration_ms: 300, tests_passed: null, tests_failed: null });
  tool(56.2, 60.6, ADA, 'execute', 'Bash', { command: 'pnpm build' });
  tool(61.0, 62.4, ADA, 'memory', 'mcp__codly__codly_memory_write', { tool: 'codly_memory_write', server: 'codly' });
  memory(62.4, ADA, 'decision', 'Refresh tokens rotate on every use and live in an httpOnly cookie.', 't-1');
  at(63.0, ADA, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 42000 });
  status(63.1, ADA, 'done');
  at(63.5, ADA, { type: 'committed', sha: 'a1b2c3d', files: [`${web}/lib/auth/session.ts`, `${web}/lib/auth/refresh.ts`], message: 'codly(t1): rotate refresh tokens' }, 't-1');
  task(64.0, 't-1', 'verifying');
  at(66.4, ADA, { type: 'verified', command: 'pnpm test', ok: true, exit_code: 0, tail: ' Test Files  6 passed (6)\n      Tests  33 passed (33)\n' }, 't-1');
  task(66.8, 't-1', 'reviewing');
  assign(67.0, DEE, 't-1', 'starting');
  status(70.0, DEE, 'working');
  tool(74.0, 76.0, DEE, 'read', 'Read', { path: `${web}/lib/auth/session.ts` });
  tool(76.3, 78.2, DEE, 'read', 'Read', { path: `${web}/lib/auth/refresh.ts` });
  tool(78.5, 80.0, DEE, 'review', 'mcp__codly__codly_submit_review', { tool: 'codly_submit_review', server: 'codly' });
  at(80.2, DEE, { type: 'reviewed', verdict: 'approve', summary: 'Rotation is atomic; reuse detection is tested.' }, 't-1');
  status(80.4, DEE, 'done');
  task(80.8, 't-1', 'merging');
  at(81.6, ADA, { type: 'merged', sha: 'd4e5f6a', branch: 'codly/atlas/integration' }, 't-1');
  task(81.8, 't-1', 'done');

  // ── Bo (Codex): color tokens, a helper, an image, a blocked lint ──
  status(21.8, BO, 'working');
  tool(22.0, 23.6, BO, 'read', 'read_file', { path: `${web}/styles/tokens.css` });
  tool(23.8, 25.0, BO, 'read', 'read_file', { path: 'packages/ui/src/theme/palette.ts' });
  at(25.2, BO, { type: 'sub_agent', call_id: 'sa-1', description: 'Audit contrast of every token pair', status: 'started' });
  tool(25.6, 28.4, BO, 'edit', 'apply_patch', { path: `${web}/styles/tokens.css` });
  tool(28.8, 31.0, BO, 'edit', 'apply_patch', { path: 'packages/ui/src/theme/palette.ts' });
  at(32.4, BO, { type: 'sub_agent', call_id: 'sa-1', description: 'Audit contrast of every token pair', status: 'completed' });
  tool(32.8, 36.0, BO, 'create', 'imageGeneration', { path: `${WT}/t3/docs/assets/theme-preview.png` });
  at(36.2, BO, {
    type: 'artifact_added',
    artifact: { id: 'art-1', kind: 'image', mime: 'image/png', path: 'docs/assets/theme-preview.png', abs_path: `${WT}/t3/docs/assets/theme-preview.png`, title: 'Theme preview', bytes: 182_000, agent_id: BO, task_id: 't-3', caption: 'Light and dark tokens side by side', created_at: 0 },
  }, 't-3');
  tool(36.8, 39.0, BO, 'execute', 'commandExecution', { command: 'pnpm lint' }, { ok: false });
  at(39.2, BO, { type: 'error', message: 'ESLint config not found in packages/config', fatal: true });
  status(39.3, BO, 'blocked');
  tool(39.6, 40.2, BO, 'message', 'codly_ask_lead', { tool: 'codly_ask_lead', server: 'codly' });
  chat(42.0, NOOR, 'lead:r2', 'Bo: use the shared config in packages/config.');
  status(46.0, BO, 'working');
  tool(46.2, 47.4, BO, 'read', 'read_file', { path: 'packages/config/eslint.config.js' });
  tool(47.8, 49.6, BO, 'edit', 'apply_patch', { path: 'packages/config/eslint.config.js' });
  tool(50.0, 53.0, BO, 'execute', 'commandExecution', { command: 'pnpm lint' });
  at(53.4, BO, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 31000 });
  status(53.5, BO, 'done');
  at(54.0, BO, { type: 'committed', sha: 'b7c8d9e', files: [`${web}/styles/tokens.css`, 'packages/ui/src/theme/palette.ts', 'packages/config/eslint.config.js'], message: 'codly(t3): dark-mode tokens' }, 't-3');
  task(54.4, 't-3', 'verifying');
  at(56.6, BO, { type: 'verified', command: 'pnpm test', ok: true, exit_code: 0, tail: '      Tests  118 passed (118)\n' }, 't-3');
  task(57.0, 't-3', 'merging');
  at(58.0, BO, { type: 'merged', sha: 'c0ffee1', branch: 'codly/atlas/integration' }, 't-3');
  task(58.2, 't-3', 'done');

  // ── Cy (Gemini CLI): the guide, a question for the Lead, a rate limit ──
  status(22.6, CY, 'working');
  tool(22.8, 24.4, CY, 'read', 'read_file', { path: 'docs/guides/auth.md' });
  tool(24.8, 28.6, CY, 'fetch', 'web_fetch', { url: 'https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html' });
  tool(29.0, 33.0, CY, 'message', 'mcp__codly__codly_ask_lead', { tool: 'codly_ask_lead', server: 'codly' });
  chat(33.2, MIRA, 'lead', 'Document only the new flow; link the ADR for history.');
  at(34.0, CY, { type: 'rate_limit', window: '5h', used_fraction: 1, resets_at: null, status: 'rejected' });
  status(34.1, CY, 'rate_limited');
  status(46.0, CY, 'working');
  at(46.0, CY, { type: 'rate_limit', window: '5h', used_fraction: 0.2, resets_at: null, status: 'allowed' });
  tool(46.4, 50.6, CY, 'edit', 'replace', { path: `${WT}/t2/docs/guides/auth.md` });
  tool(51.0, 53.2, CY, 'search', 'search_file_content', { query: 'httpOnly', path: 'docs' });
  tool(53.5, 54.2, CY, 'note', 'codly_post_note', { tool: 'codly_post_note', server: 'codly' });
  memory(54.2, CY, 'note', 'The guide links ADR 0003 instead of repeating its history.', 't-2');
  at(55.0, CY, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 24000 });
  status(55.1, CY, 'done');
  at(55.6, CY, { type: 'committed', sha: 'e1f2a3b', files: ['docs/guides/auth.md'], message: 'codly(t2): auth guide' }, 't-2');
  task(56.0, 't-2', 'verifying');
  at(57.4, CY, { type: 'verified', command: 'pnpm test', ok: true, exit_code: 0, tail: '      Tests  33 passed (33)\n' }, 't-2');
  task(57.8, 't-2', 'merging');
  at(59.6, CY, { type: 'merged', sha: 'f00dbab', branch: 'codly/atlas/integration' }, 't-2');
  task(59.8, 't-2', 'done');

  // ── Eli (OpenCode): the settings toggle, a Linear call, knowledge facts ──
  status(23.4, ELI, 'working');
  tool(23.6, 25.4, ELI, 'read', 'read', { path: `${web}/components/settings/SettingsPage.svelte` });
  tool(25.8, 28.0, ELI, 'mcp', 'linear_create_issue', { server: 'linear', tool: 'create_issue' });
  tool(28.4, 31.6, ELI, 'create', 'write', { path: `${web}/components/settings/ThemeToggle.svelte` });
  tool(32.0, 34.4, ELI, 'edit', 'edit', { path: `${web}/components/settings/SettingsPage.svelte` });
  tool(34.8, 37.6, ELI, 'test', 'bash', { command: 'pnpm vitest settings' }, { ok: true, tests_passed: 12, tests_failed: 0 });
  tool(38.0, 38.6, ELI, 'memory', 'codly_kg_add', { tool: 'codly_kg_add', server: 'codly' });
  at(38.6, ELI, { type: 'triple_added', triple: { id: 'tr-1', workspace_id: 'w1', subject: 'file:ThemeToggle.svelte', predicate: 'reads', object: 'concept:color-scheme', source: 'agent', source_event: null, confidence: 0.9, valid_from: 0, valid_to: null } });
  at(38.8, ELI, { type: 'triple_added', triple: { id: 'tr-2', workspace_id: 'w1', subject: 'concept:theme', predicate: 'persists-in', object: 'concept:localStorage', source: 'agent', source_event: null, confidence: 0.9, valid_from: 0, valid_to: null } });
  at(40.0, ELI, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 17000 });
  status(40.1, ELI, 'done');
  at(40.6, ELI, { type: 'committed', sha: '9a8b7c6', files: [`${web}/components/settings/ThemeToggle.svelte`, `${web}/components/settings/SettingsPage.svelte`], message: 'codly(t4): theme toggle' }, 't-4');
  task(41.0, 't-4', 'verifying');
  at(43.0, ELI, { type: 'verified', command: 'pnpm test', ok: true, exit_code: 0, tail: '      Tests  130 passed (130)\n' }, 't-4');
  task(43.4, 't-4', 'merging');
  at(60.6, ELI, { type: 'merged', sha: 'bead123', branch: 'codly/atlas/integration' }, 't-4');
  task(60.8, 't-4', 'done');

  // ── wrap-up ──
  at(61.4, NOOR, { type: 'turn_started', input: null });
  status(61.5, NOOR, 'thinking');
  chat(63.0, NOOR, 'lead:r2', 'Dark mode merged into codly/atlas/integration.');
  at(63.4, NOOR, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 2000 });
  status(63.5, NOOR, 'idle');
  at(82.4, MIRA, { type: 'turn_started', input: null });
  status(82.5, MIRA, 'thinking');
  chat(84.0, MIRA, 'lead', 'Rotation and the guide are merged.');
  at(84.4, MIRA, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 1900 });
  status(84.5, MIRA, 'idle');
  for (const a of [BO, CY, ELI]) status(64, a, 'idle');
  for (const a of [ADA, DEE]) status(85, a, 'idle');

  return steps.sort((a, b) => a.t - b.t);
}

/** A generated "theme preview" image for the artifact frame. */
export function themePreviewImage(): string {
  const c = document.createElement('canvas');
  c.width = 480;
  c.height = 300;
  const g = c.getContext('2d')!;
  const halves: [string, string, string][] = [
    ['#f6f3ec', '#1d1f24', '#3b82f6'],
    ['#0e121b', '#e8edf7', '#7aa2ff'],
  ];
  halves.forEach(([bg, fg, ac], i) => {
    const x = i * 240;
    g.fillStyle = bg;
    g.fillRect(x, 0, 240, 300);
    g.fillStyle = fg;
    g.globalAlpha = 0.9;
    g.fillRect(x + 28, 40, 140, 16);
    g.globalAlpha = 0.35;
    g.fillRect(x + 28, 70, 180, 10);
    g.fillRect(x + 28, 88, 160, 10);
    g.fillRect(x + 28, 106, 120, 10);
    g.globalAlpha = 1;
    g.fillStyle = ac;
    g.beginPath();
    g.roundRect(x + 28, 140, 100, 34, 10);
    g.fill();
    g.fillStyle = i ? '#1a2131' : '#e9e4da';
    g.beginPath();
    g.roundRect(x + 28, 196, 184, 72, 14);
    g.fill();
    g.fillStyle = ac;
    g.beginPath();
    g.arc(x + 196, 54, 14, 0, Math.PI * 2);
    g.fill();
  });
  return c.toDataURL('image/png');
}
