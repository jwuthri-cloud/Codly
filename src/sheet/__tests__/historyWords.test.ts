import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { TaskStatus } from '../../lib/gen/TaskStatus';
import type { Approval } from '../../lib/gen/Approval';
import type { JsonValue } from '../../lib/gen/serde_json/JsonValue';
import { backgroundLine, closedLine, compactionRowLine, decisionLine, entityWord, foldActs, helperRowLine, historyFacts, historySentence, quietStatus, reconnectAsked, repeatsReply, restartLine, savedRuleOf, stoppedWithHelper, taskChangedLine, toolDoneLine, toolEndLine, toolStartLine, tripleLine, withoutRepeatedTaskRows, type Names } from '../historyWords';

let eid = 0;
const ev = (agent: string, kind: unknown): EventRow => ({ id: ++eid, ts: eid, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const call = (agent: string, call_id: string, name: string) => ev(agent, { type: 'tool_started', call_id, name, activity: 'plan', target: null, input: {}, confidence: 'observed' });
const message = (agent: string, text: string) => ev(agent, { type: 'message', item_id: `m${eid}`, text, phase: 'final' });
const chat = (agent: string, author: string, text: string) => ev(agent, { type: 'chat', message: { id: `c${eid}`, thread: 'lead', author, text, attachment: null, ts: eid } });

describe('what a history row knows about the agent’s other events', () => {
  const events = [call('eli', 'p1', 'ExitPlanMode'), call('eli', 'c1', 'Bash'), call('ada', 'p9', 'ExitPlanMode'), message('eli', 'Done: the section is in.'), message('ada', 'Mine.')];
  const facts = historyFacts(events, 'eli', 'claude');

  it('which of a Claude agent’s calls showed a plan', () => {
    expect([...facts.planCalls]).toEqual(['p1']);
  });

  it('for another CLI, or an agent Codly does not know, a call of that name is a call like any other', () => {
    expect([...historyFacts(events, 'eli', 'codex').planCalls]).toEqual([]);
    expect([...historyFacts(events, 'eli').planCalls]).toEqual([]);
  });

  it('a reply is listed once: its row in the conversation only repeats it', () => {
    expect(repeatsReply(chat('eli', 'eli', 'Done: the section is in.'), 'eli', facts)).toBe(true);
    // Your message to it, another agent's words, and a message of its own it never wrote as a reply stay.
    expect(repeatsReply(chat('eli', 'user', 'Done: the section is in.'), 'eli', facts)).toBe(false);
    expect(repeatsReply(chat('eli', 'eli', 'Mine.'), 'eli', facts)).toBe(false);
    expect(repeatsReply(message('eli', 'Done: the section is in.'), 'eli', facts)).toBe(false);
  });
});

const AGENTS: Record<string, string> = { 'a-01a2061a': 'Ada' };
const TASKS: Record<string, string> = { 'task-1': 't1' };
const names: Names = { agent: (id) => AGENTS[id], task: (id) => TASKS[id] };

describe('history rows built from ids', () => {
  it('names the agent and the task of a fact, never their ids', () => {
    expect(tripleLine({ subject: 'agent:a-01a2061a', predicate: 'created', object: 'file:src/theme/ThemeProvider.tsx' }, names)).toBe('Ada created src/theme/ThemeProvider.tsx');
    expect(tripleLine({ subject: 'task:task-1', predicate: 'assigned_to', object: 'agent:a-01a2061a' }, names)).toBe('t1 assigned to Ada');
  });

  it('an agent or a task that is gone is named in words', () => {
    expect(entityWord('agent:a-gone', names)).toBe('an agent');
    expect(entityWord('task:task-gone', names)).toBe('a task');
  });

  it('keeps a file path, a concept and a plain value as written', () => {
    expect(entityWord('file:C:/repo/a.ts', names)).toBe('C:/repo/a.ts');
    expect(entityWord('concept:dark mode', names)).toBe('dark mode');
    expect(entityWord('running', names)).toBe('running');
  });

  it('says what a task changed to', () => {
    expect(taskChangedLine({ key: 't2', status: 'reviewing' })).toBe('Task t2 is now In review');
    expect(taskChangedLine({ key: 't2', status: 'done' })).toBe('Task t2 is now Done');
  });
});

describe('the rows of a task in a history', () => {
  /** A history row: [what it is, the status of its task]. A task's row comes for every update of the task. */
  type Row = { n: number; kind: EventKind };
  const taskRow = (n: number, id: string, status: TaskStatus): Row => ({ n, kind: { type: 'task_changed', task: { id, key: id, status } } as unknown as EventKind });
  const other = (n: number): Row => ({ n, kind: { type: 'notice', text: 'x' } });
  const shown = (oldestFirst: Row[]) => withoutRepeatedTaskRows([...oldestFirst].reverse()).map((r) => r.n);

  it('a row shows when the status is not the one of the task\'s row before it', () => {
    // t1: ready (assigned), ready (branch set), running, running (summary), ready again.
    const rows = [taskRow(1, 't1', 'ready'), taskRow(2, 't1', 'ready'), other(3), taskRow(4, 't1', 'running'), taskRow(5, 't1', 'running'), taskRow(6, 't1', 'ready')];
    expect(shown(rows)).toEqual([6, 4, 3, 1]);
  });

  it('each task is compared with its own rows, and the oldest row of a task always shows', () => {
    const rows = [taskRow(1, 't1', 'ready'), taskRow(2, 't2', 'ready'), taskRow(3, 't1', 'ready'), taskRow(4, 't2', 'running'), taskRow(5, 't2', 'running')];
    expect(shown(rows)).toEqual([4, 2, 1]);
    expect(shown([taskRow(9, 't3', 'done')])).toEqual([9]);
    expect(shown([])).toEqual([]);
  });

  it('keeps every other row, in its order', () => {
    expect(shown([other(1), other(2), taskRow(3, 't1', 'ready'), other(4)])).toEqual([4, 3, 2, 1]);
  });
});

describe('a change of what an agent runs in the background', () => {
  const task = (kind: string, description: string) => ({ id: description, kind, description });
  const ended = (kind: string, description: string, status: string) => ({ ...task(kind, description), status });

  it('says what runs when nothing ended', () => {
    expect(backgroundLine({ tasks: [task('command', 'npm run dev'), task('helper', 'Run the tests')], ended: [] })).toBe('Runs in the background: npm run dev, Run the tests');
    expect(backgroundLine({ tasks: [], ended: [] })).toBe('Nothing runs in the background');
  });

  it('says what ended, in Claude Code\'s word, and what still runs', () => {
    expect(backgroundLine({ tasks: [], ended: [ended('command', 'sleep 8; echo woke', 'completed')] })).toBe('Background command completed: sleep 8; echo woke');
    expect(backgroundLine({ tasks: [task('command', 'npm test')], ended: [ended('command', 'npm run dev', 'stopped')] })).toBe('Background command stopped: npm run dev · still runs: npm test');
    expect(backgroundLine({ tasks: [], ended: [ended('command', 'npm test', 'ended'), ended('task', 'x', 'completed')] })).toBe('Background command ended: npm test; Background task completed: x');
  });

  // Before, the end of a background helper was two rows with two words: "Subagent completed: Run
  // the tests" (its own row) and "Helper completed: Run the tests" (the list's row).
  it('leaves the end of a helper to the helper\'s own row: a list that ends a command and a helper names the command', () => {
    const k = { tasks: [task('command', 'npm run dev')], ended: [ended('command', 'npm test', 'completed'), ended('helper', 'Run the tests', 'stopped')] };
    expect(backgroundLine(k)).toBe('Background command completed: npm test · still runs: npm run dev');
  });

  it('a list that ends only a helper has nothing left to say: it is no row', () => {
    expect(backgroundLine({ tasks: [], ended: [ended('helper', 'Run the tests', 'stopped')] })).toBeNull();
    expect(backgroundLine({ tasks: [task('command', 'npm run dev')], ended: [ended('helper', 'Run the tests', 'completed'), ended('helper', 'Read the docs', 'ended')] })).toBeNull();
    // What ran before (the list before it named it) is not new either.
    expect(backgroundLine({ tasks: [task('command', 'npm run dev')], ended: [ended('helper', 'Run the tests', 'completed')] }, new Set(['npm run dev', 'Run the tests']))).toBeNull();
  });

  // Before, such a row was dropped, and nothing said that the new command started.
  it('a list that ends a helper and starts a command in the same row says the new command', () => {
    const k = { tasks: [task('command', 'npm run dev'), task('command', 'npm test')], ended: [ended('helper', 'Run the tests', 'completed')] };
    expect(backgroundLine(k, new Set(['npm run dev', 'Run the tests']))).toBe('Runs in the background: npm test');
  });

  it('knows what the list before named, in the same run', () => {
    let id = 0;
    const list = (run: string, ids: string[]): EventRow => ({ id: ++id, ts: id, agent_id: 'eli', task_id: null, run_id: run, kind: { type: 'background_tasks', tasks: ids.map((i) => task('command', i)), ended: [] } });
    const rows = [list('r1', ['a']), list('r1', ['a', 'b']), list('r2', ['c'])];
    const before = historyFacts(rows, 'eli').backgroundBefore;
    expect([before.get(1), before.get(2), before.get(3)]).toEqual([null, new Set(['a']), null]);
  });
});

describe('the row of a helper', () => {
  it('calls it a helper, with its CLI\'s word for where it stands', () => {
    expect(helperRowLine({ description: 'Run the tests', status: 'completed' })).toBe('Helper completed: Run the tests');
    expect(helperRowLine({ description: 'Run the\ntests', status: 'stopped' })).toBe('Helper stopped: Run the tests');
    expect(helperRowLine({ description: 'Run the tests', status: 'running' })).toBe('Helper running: Run the tests');
  });
});

// Before, every `compacting` row read "Compacted context", also the one that only says that a
// compaction began.
describe('the row of a compaction', () => {
  it('says "Compacted context" only for a row with token numbers, as the row under "Compact now" does', () => {
    expect(compactionRowLine({ pre_tokens: 180_000, post_tokens: 40_000 }, 'claude')).toBe('Compacted context 180k → 40k');
    expect(compactionRowLine({ pre_tokens: null, post_tokens: 800 }, 'claude')).toBe('Compacted context');
    expect(compactionRowLine({ pre_tokens: 180_000, post_tokens: 40_000 }, 'codex')).toBe('Compacted context 180k → 40k');
  });

  it('a Claude row with no numbers says that a compaction started', () => {
    expect(compactionRowLine({ pre_tokens: null, post_tokens: null }, 'claude')).toBe('Compaction started');
  });

  // Claude Code's boundary line has the size before and comes when the compaction is done.
  it('a Claude row with the size before only says that the context was compacted', () => {
    expect(compactionRowLine({ pre_tokens: 180_000, post_tokens: null }, 'claude')).toBe('Compacted context, 180k tokens before');
    expect(compactionRowLine({ pre_tokens: 180_000, post_tokens: null }, 'codex')).toBe('Context compaction');
  });

  it('a row with no numbers from another CLI can be the start or the end: it says neither', () => {
    for (const cli of ['codex', 'gemini', 'opencode', null] as const) {
      expect(compactionRowLine({ pre_tokens: null, post_tokens: null }, cli)).toBe('Context compaction');
    }
  });
});

describe('one act of yours is one row', () => {
  let n = 0;
  /** A history row at `ts` (ms), of task `task` when it has one. */
  const row = (ts: number, kind: unknown, task: string | null = null): EventRow => ({ id: ++n, ts, agent_id: 'eli', task_id: task, run_id: 'r1', kind: kind as EventKind });
  const COMMENTS = 'Review comments (3):\n\n1. a.ts:4\n   Use the v2 client\n\n2. b.ts:9\n   Keep the test\n\n3. c.ts:1\n   Rename it';
  const sentBack = (ts: number, task = 'task-1', key = 't1') =>
    row(ts, { type: 'decision', id: `d${n}`, decision_type: 'review', method: 'human', summary: `you sent ${key} back with review comments; its worker reads them now`, detail: { plan_id: 'p1', comments: COMMENTS } }, task);
  const status = (ts: number, task: string, s: TaskStatus) => row(ts, { type: 'task_changed', task: { id: task, key: task, status: s } });
  const yourChat = (ts: number, text: string) => row(ts, { type: 'chat', message: { id: `c${n}`, thread: 'agent:eli', author: 'user', text, attachment: null, ts } });
  /** Newest first, as History lists them; the numbers of the rows that show. */
  const shown = (oldestFirst: EventRow[], planCalls: string[] = []) => foldActs([...oldestFirst].reverse(), new Set(planCalls)).rows.map((r) => r.id);

  it('a send-back is one row: its status and your comments are part of it', () => {
    const rows = [yourChat(1000, COMMENTS), status(1010, 'task-1', 'changes_requested'), sentBack(1020)];
    expect(shown(rows)).toEqual([rows[2].id]);
    const line = decisionLine(rows[2].kind as Extract<EventKind, { type: 'decision' }>, () => false);
    expect(line).toBe('You sent t1 back with 3 review comments; its worker reads them now');
  });

  it('the worker’s turn that takes your comments is part of it, also when it starts a moment after', () => {
    const turn = row(1500, { type: 'turn_started', input: `[Codly] The user reviewed your work on task t1 and sent it back with the comments below.\n\n${COMMENTS}` });
    const rows = [yourChat(1000, COMMENTS), status(1010, 'task-1', 'changes_requested'), sentBack(1020), turn];
    expect(shown(rows)).toEqual([rows[2].id]);
  });

  it('rows of another task between them stay, and so do rows from before the act', () => {
    const early = status(-5000, 'task-1', 'reviewing');
    const other = status(1005, 'task-2', 'running');
    const yourOther = yourChat(1008, 'and the docs too');
    const rows = [early, yourChat(1000, COMMENTS), other, yourOther, status(1010, 'task-1', 'changes_requested'), sentBack(1020)];
    expect(shown(rows)).toEqual([rows[5].id, yourOther.id, other.id, early.id]);
  });

  const approval = (id: string, subject: unknown, call: string | null = null) => ({ id, agent_id: 'eli', run_id: 'r1', task_id: null, call_id: call, subject, reason: null, choices: [], risks: [], status: 'pending', decided_by: null, created_at: 0 }) as unknown as Approval;
  const asked = (ts: number, a: Approval) => row(ts, { type: 'approval_requested', approval: a });
  const answered = (ts: number, id: string, s: string) => row(ts, { type: 'approval_resolved', approval_id: id, status: s, by: 'user' });

  it('a plan sent back keeps its answer’s row: the request and its call go once it is closed', () => {
    const plan = approval('ap1', { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add it' } }, 'p1');
    const rows = [row(10, { type: 'tool_started', call_id: 'p1', name: 'ExitPlanMode', activity: 'plan', target: null, input: {}, confidence: 'observed' }), asked(11, plan), answered(40, 'ap1', 'denied'), row(41, { type: 'tool_completed', call_id: 'p1', ok: false, exit_code: null, tests_passed: null, tests_failed: null, duration_ms: 30 })];
    expect(shown(rows, ['p1'])).toEqual([rows[2].id]);
    // While it waits, it is its own row.
    expect(shown(rows.slice(0, 2), ['p1'])).toEqual([rows[1].id, rows[0].id]);
  });

  it('a closed request is one row that says what was asked and how it ended; a record of the answer goes too', () => {
    const edit = approval('ap2', { kind: 'file_change', paths: ['README.md'], diff: null });
    const record = row(31, { type: 'decision', id: 'd9', decision_type: 'approval', method: 'human', summary: 'You denied the request', detail: { approval: 'ap2' } });
    const rows = [asked(10, edit), answered(30, 'ap2', 'denied'), record];
    const folded = foldActs([...rows].reverse(), new Set());
    expect(folded.rows.map((r) => r.id)).toEqual([rows[1].id]);
    // (the answer first: a long request is cut at the end of the row)
    expect(closedLine(folded.asked.get('ap2') ?? null, 'Denied by you', null)).toBe('Denied by you · asked to change README.md');
    // A plan's answer names it already.
    expect(closedLine({ subject: { kind: 'tool', name: 'ExitPlanMode', input: { plan: 'x' } } }, 'Plan sent back by you', 'claude')).toBe('Plan sent back by you');
  });

  // The core's records of why Codly asked and of the rule that answered (orchestrator/ingest.rs)
  // say what the answer's row does not; the record of a rule you saved repeats the answer.
  it('keeps why Codly asked and which rule answered; drops the record of a rule you saved with the answer', () => {
    const cmd = approval('ap5', { kind: 'command', command: 'rm x', cwd: null });
    const policy = (ts: number, summary: string, rule: unknown, method = 'rule') =>
      row(ts, { type: 'decision', id: `d${n}`, decision_type: 'approval_policy', method, summary, detail: { approval_id: 'ap5', mode: 'ask_risky', rule, subject: cmd.subject, read_only: false } });
    const why = policy(11, 'asking you: it deletes files', 'it deletes files');
    const saved = policy(29, 'you saved a rule: always allow rm x in this repo', { id: 'r1', kind: 'exact', command: 'rm x' }, 'human');
    const rows = [asked(10, cmd), why, saved, answered(30, 'ap5', 'approved')];
    expect(shown(rows)).toEqual([rows[3].id, why.id]);
    const auto = approval('ap6', { kind: 'command', command: 'ls', cwd: null });
    const allowed = row(21, { type: 'decision', id: 'd20', decision_type: 'approval_policy', method: 'rule', summary: 'auto-allowed read-only command', detail: { approval_id: 'ap6', mode: 'ask_risky', rule: 'read-only command', subject: auto.subject, read_only: true } });
    const autoRows = [asked(20, auto), row(21, { type: 'approval_resolved', approval_id: 'ap6', status: 'approved', by: 'policy' }), allowed];
    expect(shown(autoRows)).toEqual([allowed.id, autoRows[1].id]);
  });

  it('finds the rule you saved with an answer in the core’s record of it', () => {
    const decisions: { type: string; detail: JsonValue }[] = [
      { type: 'approval_policy', detail: { rule: { id: 'r1', kind: 'prefix', command: 'npm install' }, approval_id: 'ap3' } },
      { type: 'approval_policy', detail: { rule: { id: 'r2', kind: 'exact', command: 'ls' } } },
    ];
    expect(savedRuleOf(decisions, 'ap3')).toEqual({ kind: 'prefix', command: 'npm install' });
    expect(savedRuleOf(decisions, 'ap4')).toBeNull();
  });
});

describe('every History row is a sentence', () => {
  const keeps = (w: string) => ['t1', 'Ada', 'eli'].includes(w);
  it('a request Codly sent for you says that you asked', () => {
    expect(historySentence('asked Lead to reconnect its MCP server linear', 'human', keeps)).toBe('You asked Lead to reconnect its MCP server linear');
    expect(historySentence('asked Eli to compact its conversation', 'human', keeps)).toBe('You asked Eli to compact its conversation');
    expect(historySentence('asked Ada to stop what it left running in the background: npm run dev', 'human', keeps)).toBe('You asked Ada to stop what it left running in the background: npm run dev');
    expect(historySentence('asked Ada to stop', 'rule', keeps)).toBe('Codly asked Ada to stop');
  });

  it('another starts with a capital, unless it starts with a name or a task key', () => {
    expect(historySentence("you turned postgres off for Lead's run", 'human', keeps)).toBe("You turned postgres off for Lead's run");
    expect(historySentence('tools blocked for Ada', 'human', keeps)).toBe('Tools blocked for Ada');
    expect(historySentence('t1 merged into main', 'rule', keeps)).toBe('t1 merged into main');
    expect(historySentence("eli's plan is in", 'rule', keeps)).toBe("eli's plan is in");
  });
});

describe('the row of a tool call', () => {
  const start = (name: string, target: unknown = null, input: unknown = {}) => ({ name, activity: 'mcp', target, input }) as unknown as Parameters<typeof toolStartLine>[0];
  const mcp = (server: string, tool: string) => ({ path: null, paths: [], command: null, query: null, url: null, server, tool });

  it('names a Codly tool in words only, and another MCP tool by its server and its name', () => {
    expect(toolStartLine(start('mcp__codly__codly_submit_plan', mcp('codly', 'codly_submit_plan')), 'claude')).toBe('Submitting the plan');
    expect(toolStartLine(start('mcp__codly__codly_new_thing', mcp('codly', 'codly_new_thing')), 'claude')).toBe('Using Codly’s new thing tool');
    expect(toolStartLine(start('mcp__linear__create_issue', mcp('linear', 'create_issue')), 'claude')).toBe('Using linear: create_issue');
    // A name the adapter gave no target for.
    expect(toolStartLine(start('mcp__linear__create_issue'), 'claude')).toBe('Using linear: create_issue');
  });

  it('no row shows mcp__', () => {
    for (const k of [start('mcp__codly__codly_kg_add', mcp('codly', 'codly_kg_add')), start('mcp__docs__search', mcp('docs', 'search')), start('mcp__x__y')]) {
      expect(toolStartLine(k, 'claude')).not.toContain('mcp__');
    }
  });

  it('a CLI’s own tool keeps what it does and its name; a helper’s call says so', () => {
    const bash = { ...start('Bash', { path: null, paths: [], command: 'npm test', query: null, url: null, server: null, tool: null }, { subagent: { thread: 't', name: 'Ada' } }), activity: 'execute' } as Parameters<typeof toolStartLine>[0];
    expect(toolStartLine(bash, 'codex')).toBe('Running npm test · Bash · helper Ada');
  });
});

describe('a call of a helper you stopped', () => {
  let n = 0;
  const row = (kind: unknown): EventRow => ({ id: ++n, ts: n, agent_id: 'eli', task_id: null, run_id: 'r1', kind: kind as EventKind });
  const events = [
    row({ type: 'tool_started', call_id: 'h1', name: 'Agent', activity: 'other', target: null, input: {}, confidence: 'observed' }),
    row({ type: 'sub_agent', call_id: 'h1', description: 'Look at the docs', status: 'running' }),
    row({ type: 'tool_started', call_id: 'g1', name: 'Glob', activity: 'search', target: null, input: {}, confidence: 'observed' }),
    row({ type: 'sub_agent_progress', call_id: 'h1', name: null, step: null, tool_call_id: 'g1', tool_uses: null, tokens: null, report: null }),
    row({ type: 'sub_agent', call_id: 'h1', description: 'Look at the docs', status: 'stopped' }),
    row({ type: 'sub_agent', call_id: 'h2', description: 'Run the tests', status: 'failed' }),
  ];
  const facts = historyFacts(events, 'eli', 'claude');

  it('is no failed call: the helper’s own call goes, a call it made stopped with it', () => {
    expect(stoppedWithHelper('h1', facts)).toBe('helper');
    expect(stoppedWithHelper('g1', facts)).toBe('own');
    // A helper that failed by itself, and the agent's own calls, are as they were.
    expect(stoppedWithHelper('h2', facts)).toBeNull();
    expect(stoppedWithHelper('c9', facts)).toBeNull();
  });
});

// The fifth browser review (defect 20): a tool call was two rows ("Reading package.json · Read",
// then "Finished · 0.5 s"), "Status → idle" rows came between them, and a Restart read as "You
// asked Lead to reconnect…".
describe('History is one row per act', () => {
  let n = 0;
  const row = (kind: unknown, run = 'r1'): EventRow => ({ id: ++n, ts: n, agent_id: 'eli', task_id: null, run_id: run, kind: kind as EventKind });
  const target = (o: Record<string, unknown>) => ({ path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...o });
  const started = (call_id: string, name: string, activity: string, t: Record<string, unknown>, input: unknown = {}) =>
    row({ type: 'tool_started', call_id, name, activity, target: target(t), input, confidence: 'observed' });
  const ended = (call_id: string, o: Record<string, unknown> = {}) =>
    row({ type: 'tool_completed', call_id, ok: true, exit_code: null, tests_passed: null, tests_failed: null, duration_ms: 500, ...o });
  const startOf = (e: EventRow) => e.kind as Extract<EventKind, { type: 'tool_started' }>;
  const endOf = (e: EventRow) => e.kind as Extract<EventKind, { type: 'tool_completed' }>;

  it('a tool call is the row of its start, which says what it did and how it ended', () => {
    const read = started('c1', 'Read', 'read', { path: 'web/package.json' });
    const done = ended('c1');
    const folded = foldActs([done, read], new Set());
    expect(folded.rows.map((r) => r.id)).toEqual([read.id]);
    expect(folded.at.get(done.id)).toBe(read.id);
    expect(toolStartLine(startOf(read), 'claude')).toBe('Reading package.json · Read');
    expect(toolEndLine(startOf(read), endOf(done), 'claude')).toBe('Read package.json · 0.5 s');
    // A call that runs is still its own row.
    expect(foldActs([read], new Set()).rows).toEqual([read]);
  });

  it('a call that failed says so, with its exit code and its tests', () => {
    const test = startOf(started('c2', 'Bash', 'test', { command: 'npm test' }));
    expect(toolEndLine(test, endOf(ended('c2', { ok: false, exit_code: 1, tests_passed: 4, tests_failed: 2, duration_ms: 2100 })), 'claude')).toBe('Ran npm test · failed · exit 1 · 4 passed, 2 failed · 2 s');
    // An MCP tool is named in words; a helper's call says so.
    expect(toolDoneLine(startOf(started('c3', 'mcp__linear__create_issue', 'mcp', { server: 'linear', tool: 'create_issue' })), 'claude')).toBe('Used linear: create_issue');
    expect(toolDoneLine(startOf(started('c4', 'Bash', 'execute', { command: 'ls' }, { subagent: { thread: 't', name: 'Ada' } })), 'codex')).toBe('Ran ls · helper Ada');
    // The end of a call whose start is not shown says only how it ended.
    expect(toolEndLine(null, endOf(ended('c5', { ok: false })), 'claude')).toBe('Failed · 0.5 s');
  });

  it('knows how each call ended, and what it was', () => {
    const read = started('c6', 'Read', 'read', { path: 'a.ts' });
    const done = ended('c6');
    const facts = historyFacts([read, done], 'eli', 'claude');
    expect(facts.ends.get('c6')).toBe(done.kind);
    expect(facts.starts.get('c6')).toBe(read.kind);
  });

  it('a state its turn and its calls already say is no row', () => {
    for (const s of ['idle', 'working', 'thinking', 'starting']) expect(quietStatus(s)).toBe(true);
    for (const s of ['waiting_approval', 'blocked', 'rate_limited', 'failed', 'offline']) expect(quietStatus(s)).toBe(false);
  });

  it('a reconnect of a connected server is a restart, and says so', () => {
    // The core writes the record of a reconnect with no run id (controls.rs `control_decision`).
    const reconnect = (server: string): EventRow => ({ ...row({ type: 'decision', id: `d${n}`, decision_type: 'control', method: 'human', summary: `asked Lead to reconnect its MCP server ${server}`, detail: null }), run_id: null });
    const server = (name: string, status: string, run = 'r1'): EventRow => ({ ...row({ type: 'mcp_server', name, status, error: null }), run_id: run });
    const restart = reconnect('docs');
    const retry = reconnect('linear');
    const facts = historyFacts([server('docs', 'connected'), server('linear', 'failed'), restart, retry], 'eli', 'claude');
    expect([...facts.restarts]).toEqual([restart.id]);
    expect(restartLine((restart.kind as Extract<EventKind, { type: 'decision' }>).summary)).toBe('You restarted docs for Lead’s run');
    expect(reconnectAsked('asked Eli 2 to reconnect its MCP server my server')).toEqual({ agent: 'Eli 2', server: 'my server' });
    expect(restartLine('you turned docs off for Lead’s run')).toBeNull();
    // A server of an earlier run is gone with that run: a newer run that lists other servers does not have it.
    expect([...historyFacts([server('docs', 'connected'), server('linear', 'connected', 'r2'), reconnect('docs')], 'eli', 'claude').restarts]).toEqual([]);
    // Its status in the newer run is what counts.
    const again = reconnect('docs');
    expect([...historyFacts([server('docs', 'failed'), server('docs', 'connected', 'r2'), again], 'eli', 'claude').restarts]).toEqual([again.id]);
  });

  // Package claude-part5-truth words the record of the switch for the folder: History shows it as a sentence.
  it('the record of a switch reads as a sentence, in its old and its new words', () => {
    const keeps = () => false;
    expect(historySentence("you turned docs off for Lead's run", 'human', keeps)).toBe("You turned docs off for Lead's run");
    expect(historySentence('you turned docs off for Claude Code in this folder (Lead’s run and every later one)', 'human', keeps)).toBe('You turned docs off for Claude Code in this folder (Lead’s run and every later one)');
  });
});

// Round 7's review: a jump from Home to a History row found nothing when the fold dropped it.
describe('a jump to a row that History folded', () => {
  let n = 0;
  const row = (ts: number, kind: unknown, task: string | null = null): EventRow => ({ id: ++n, ts, agent_id: 'eli', task_id: task, run_id: 'r1', kind: kind as EventKind });
  const COMMENTS = 'Review comments (1):\n\n1. a.ts:4\n   Use the v2 client';

  it('lands on the row that stands for it: the three rows of a send-back on its decision', () => {
    const chat = row(1000, { type: 'chat', message: { id: 'c1', thread: 'agent:eli', author: 'user', text: COMMENTS, attachment: null, ts: 1000 } });
    const status = row(1010, { type: 'task_changed', task: { id: 'task-1', key: 't1', status: 'changes_requested' } });
    const sent = row(1020, { type: 'decision', id: 'd1', decision_type: 'review', method: 'human', summary: 'you sent t1 back with review comments; its worker reads them now', detail: { comments: COMMENTS } }, 'task-1');
    const folded = foldActs([sent, status, chat], new Set());
    expect(folded.rows).toEqual([sent]);
    // The jump to the first of the three lands on the one that is kept.
    expect([chat, status].map((r) => folded.at.get(r.id))).toEqual([sent.id, sent.id]);
    expect(folded.at.has(sent.id)).toBe(false);
  });

  it('a closed request and its record land on the answer’s row', () => {
    const approval = { id: 'ap1', agent_id: 'eli', run_id: 'r1', task_id: null, call_id: null, subject: { kind: 'command', command: 'rm x', cwd: null }, reason: null, choices: [], risks: [], status: 'pending', decided_by: null, created_at: 0 } as unknown as Approval;
    const asked = row(10, { type: 'approval_requested', approval });
    const answered = row(30, { type: 'approval_resolved', approval_id: 'ap1', status: 'approved', by: 'user' });
    const record = row(31, { type: 'decision', id: 'd2', decision_type: 'approval', method: 'human', summary: 'You allowed it', detail: { approval: 'ap1' } });
    const folded = foldActs([record, answered, asked], new Set());
    expect(folded.rows).toEqual([answered]);
    expect([asked, record].map((r) => folded.at.get(r.id))).toEqual([answered.id, answered.id]);
  });
});
