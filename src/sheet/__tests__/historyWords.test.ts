import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { TaskStatus } from '../../lib/gen/TaskStatus';
import { backgroundLine, entityWord, historyFacts, repeatsReply, taskChangedLine, tripleLine, withoutRepeatedTaskRows, type Names } from '../historyWords';

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
    expect(backgroundLine({ tasks: [], ended: [ended('helper', 'Run the tests', 'ended'), ended('task', 'x', 'completed')] })).toBe('Helper ended: Run the tests; Background task completed: x');
  });
});
