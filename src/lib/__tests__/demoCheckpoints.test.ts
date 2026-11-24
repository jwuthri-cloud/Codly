import { describe, expect, it, vi } from 'vitest';
import { DemoCheckpoints } from '../demo/checkpoints';
import { editedTurns } from '../demo/sim';
import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Task } from '../gen/Task';

const NOW = 1_760_000_000_000;
const MIN = 60_000;
const agent = (o: Partial<Agent> = {}) => ({ id: 'a1', name: 'Ada', role: 'solo', status: 'idle', worktree: '/demo/wt/ada', task_id: null, ...o }) as Agent;

/** The parts of the demo's Sim the checkpoints read and write, with a log of turns. */
function demo() {
  const log: EventRow[] = [];
  const emit = (agentId: string, task: string | null, ts: number, kind: EventKind) => log.push({ id: log.length + 1, ts, agent_id: agentId, task_id: task, run_id: null, kind });
  /** A turn of `agentId` that started with `input` and changed each list of `files` in one tool call. */
  const turn = (input: string | null, files: string[][], o: { agentId?: string; ts?: number; task?: string } = {}) => {
    const id = o.agentId ?? 'a1';
    const ts = o.ts ?? NOW;
    const task = o.task ?? null;
    emit(id, task, ts, { type: 'turn_started', input });
    for (const paths of files) emit(id, task, ts + 1, { type: 'files_changed', call_id: null, paths, source: 'tool_input' } as EventKind);
    emit(id, task, ts + 2, { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 2 });
  };
  const sim = { log, tasks: new Map([['t-id', { key: 'T-4' } as Task]]), chat: vi.fn(), decision: vi.fn() };
  return { sim, turn, points: new DemoCheckpoints(sim) };
}

describe('the turns the demo keeps points of', () => {
  it('are the agent’s own turns that changed files, oldest first, named as the core names them', () => {
    const { sim, turn } = demo();
    turn('[User] Add the form\n  to the page', [['src/form.ts'], ['src/form.ts', 'src/page.ts']], { ts: NOW - 9 * MIN });
    turn('Read the README', [], { ts: NOW - 8 * MIN });
    turn('Fix the label', [['src/form.ts']], { agentId: 'a2', ts: NOW - 7 * MIN });
    turn(null, [['src/page.ts']], { ts: NOW - 6 * MIN });
    turn('x'.repeat(130), [['a.ts']], { ts: NOW - 5 * MIN });
    const turns = editedTurns(sim, 'a1');
    expect(turns.map((t) => [t.label, t.files, t.ts])).toEqual([
      ['Add the form to the page', ['src/form.ts', 'src/page.ts'], NOW - 9 * MIN],
      ['a turn', ['src/page.ts'], NOW - 6 * MIN],
      [`${'x'.repeat(119)}…`, ['a.ts'], NOW - 5 * MIN],
    ]);
    expect(editedTurns(sim, 'a2').map((t) => t.label)).toEqual(['Fix the label']);
  });
});

describe('DemoCheckpoints', () => {
  it('lists the agent’s own turns, newest first, each with the files that differ from it now; later turns join', () => {
    const { turn, points } = demo();
    turn('Add the form', [['a.ts', 'b.ts']], { ts: NOW - 9 * MIN });
    turn('Fix the label', [['b.ts']], { ts: NOW - 5 * MIN });
    const view = points.list(agent(), NOW);
    expect(view).toMatchObject({ off: null, task: null, left_out: [] });
    expect(view.checkpoints).toEqual([
      { n: 2, kind: 'turn', ts: NOW - 5 * MIN, label: 'Fix the label', changed: 1, files: ['b.ts'] },
      { n: 1, kind: 'turn', ts: NOW - 9 * MIN, label: 'Add the form', changed: 2, files: ['a.ts', 'b.ts'] },
    ]);
    turn('Add a test', [['c.ts']], { ts: NOW - MIN });
    expect(points.list(agent(), NOW).checkpoints.map((c) => [c.n, c.label, c.changed])).toEqual([
      [3, 'Add a test', 1],
      [2, 'Fix the label', 2],
      [1, 'Add the form', 3],
    ]);
  });

  it('says why an agent without a worktree has none, and restores nothing there', () => {
    const { points } = demo();
    const lead = agent({ name: 'Lead', worktree: null });
    expect(points.list(lead, NOW)).toEqual({ off: 'Lead has no worktree now, so there are no checkpoints.', checkpoints: [], task: null, left_out: [] });
    expect(() => points.restore(lead, 1, NOW)).toThrow('no worktree');
    expect(points.list(undefined, NOW).checkpoints).toEqual([]);
  });

  it('a restore keeps the way back as the newest point, and says so in the conversation and in History', () => {
    const { sim, turn, points } = demo();
    turn('Add the form', [['a.ts', 'b.ts']], { ts: NOW - 9 * MIN });
    turn('Fix the label', [['b.ts']], { ts: NOW - 5 * MIN });
    expect(points.restore(agent(), 2, NOW)).toEqual({ files: ['b.ts'], undo: 3 });
    expect(points.list(agent(), NOW).checkpoints.map((c) => [c.n, c.kind, c.label, c.changed])).toEqual([
      [3, 'restore', 'Fix the label', 1],
      [2, 'turn', 'Fix the label', 0],
      [1, 'turn', 'Add the form', 2],
    ]);
    expect(sim.decision).toHaveBeenLastCalledWith('checkpoint', 'human', 'you put 1 file of Ada back to before “Fix the label”', { agent_id: 'a1', checkpoint: 2, undo: 3, files: ['b.ts'] });
    expect(sim.chat).toHaveBeenLastCalledWith('agent:a1', 'codly', 'Put 1 file of Ada back to before “Fix the label”. Files only: Ada still remembers the later turns. To undo, restore the newest point.');
    // The files are as at that point now: nothing to restore there. The undo puts them back.
    expect(() => points.restore(agent(), 2, NOW)).toThrow('already as they were');
    expect(points.restore(agent(), 3, NOW + 1)).toEqual({ files: ['b.ts'], undo: 4 });
    expect(sim.decision).toHaveBeenLastCalledWith('checkpoint', 'human', 'you put 1 file of Ada back to before an earlier restore', { agent_id: 'a1', checkpoint: 3, undo: 4, files: ['b.ts'] });
    expect(points.list(agent(), NOW).checkpoints.find((c) => c.n === 2)).toMatchObject({ changed: 1, files: ['b.ts'] });
  });

  it('one file goes back alone, and the counts of the other points follow', () => {
    const { sim, turn, points } = demo();
    turn('Add the form', [['a.ts', 'b.ts']], { ts: NOW - 9 * MIN });
    turn('Fix the label', [['b.ts']], { ts: NOW - 5 * MIN });
    expect(points.restore(agent(), 1, NOW, 'a.ts')).toEqual({ files: ['a.ts'], undo: 3 });
    expect(sim.chat).toHaveBeenLastCalledWith('agent:a1', 'codly', 'Put a.ts of Ada back to before “Add the form”. The other files stay as they are, and Ada still remembers the later turns. To undo, restore the newest point.');
    const after = points.list(agent(), NOW).checkpoints;
    expect(after[0]).toMatchObject({ n: 3, kind: 'restore', changed: 1, files: ['a.ts'] });
    expect(after.find((c) => c.n === 1)).toMatchObject({ changed: 1, files: ['b.ts'] });
    // a.ts was as "Add the form" left it before "Fix the label": it differs from that point now.
    expect(after.find((c) => c.n === 2)).toMatchObject({ changed: 2, files: ['a.ts', 'b.ts'] });
    expect(() => points.restore(agent(), 1, NOW, 'a.ts')).toThrow('a.ts is already as it was at that point');
    expect(() => points.restore(agent(), 1, NOW, 'README.md')).toThrow('README.md is already as it was at that point');
    expect(() => points.restore(agent({ status: 'working' }), 1, NOW, 'b.ts')).toThrow('Ada is still working');
  });

  it('a 1:1 agent on a task: the turns of the task’s worktree, and the list says which task', () => {
    const { turn, points } = demo();
    turn('Own work', [['a.ts']], { ts: NOW - 9 * MIN });
    turn('Task T-4: Dark mode', [['theme.ts']], { ts: NOW - 5 * MIN, task: 't-id' });
    const onTask = agent({ task_id: 't-id' });
    const view = points.list(onTask, NOW);
    expect(view.task).toBe('T-4');
    expect(view.checkpoints.map((c) => c.label)).toEqual(['Task T-4: Dark mode']);
    expect(points.lastTurn(onTask, NOW)).toMatchObject({ label: 'Task T-4: Dark mode', task: 'T-4' });
    // Its own worktree again: its own turns.
    expect(points.list(agent(), NOW).checkpoints.map((c) => c.label)).toEqual(['Own work']);
    // A team member's worktree is the task's: the list names no other one.
    expect(points.list(agent({ role: 'worker', task_id: 't-id' }), NOW).task).toBeNull();
  });

  it('the last turn is the newest point taken before a turn, with what differs from it now', () => {
    const { turn, points } = demo();
    turn('Add the form', [['a.ts', 'b.ts']], { ts: NOW - 9 * MIN });
    turn('Fix the label', [['b.ts']], { ts: NOW - 5 * MIN });
    const last = points.lastTurn(agent(), NOW)!;
    expect(last).toMatchObject({ label: 'Fix the label', ts: NOW - 5 * MIN, task: null });
    expect(last.diff).toContain('diff --git a/b.ts b/b.ts');
    expect(last.diff).not.toContain('a.ts');
    // Its files went back: the turn is still the last one, and nothing of it is left.
    points.restore(agent(), 2, NOW);
    expect(points.lastTurn(agent(), NOW)).toMatchObject({ label: 'Fix the label', diff: '' });
    expect(points.lastTurn(agent({ worktree: null }), NOW)).toBeNull();
    expect(points.lastTurn(undefined, NOW)).toBeNull();
  });

  it('an agent that changed nothing yet has no point and no last turn; a point that is gone cannot be restored', () => {
    const { turn, points } = demo();
    turn('Read the README', []);
    expect(points.list(agent(), NOW)).toEqual({ off: null, checkpoints: [], task: null, left_out: [] });
    expect(points.lastTurn(agent(), NOW)).toBeNull();
    expect(() => points.restore(agent(), 1, NOW)).toThrow('that point is gone');
    expect(() => points.restore(agent({ status: 'working' }), 1, NOW)).toThrow('Ada is still working; restore files when its turn ends');
  });
});
