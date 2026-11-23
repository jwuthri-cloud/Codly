// The Changes room with 1:1 agents and plans that still run: which chip a diff is under, which
// agents are listed, a 1:1 agent's review key and editor place, and where Send back takes each
// comment of a plan's diff (the task at work that wrote its file, or a new request).
import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Task } from '../gen/Task';
import { reviewKey } from '../state/review.svelte';
import { agentBarWords, agentEditorPlace, AGENTS_GROUP, agentsWithChanges, diffGroup, fileAnchor, fileOwners, splitByTask, splitWords, type ReviewComment } from '../util/review';

const agent = (o: Partial<Agent>) => ({ id: 's1', name: 'Eli', role: 'solo', worktree: '/wt/eli', archived: false, ...o }) as Agent;
const task = (o: Partial<Task>) => ({ id: 't1', key: 't1', status: 'running', assignee: 'w1', plan_id: 'p1', ...o }) as Task;
const note = (path: string, text = 'Look again.'): ReviewComment => ({ ...fileAnchor(path), text });

describe('the chip a diff is under', () => {
  const planOf = (id: string) => ({ t1: 'p1', t2: 'p2', t3: null })[id as 't1' | 't2' | 't3'];
  const settled = new Set(['p2']);
  const shown = (pid: string) => pid === 'p1';

  it('a 1:1 agent is under the agents chip; a plan, a task, a merged plan and a loose task as before', () => {
    expect(diffGroup({ kind: 'agent', id: 's1' }, planOf, settled, shown)).toBe(AGENTS_GROUP);
    expect(diffGroup({ kind: 'plan', id: 'p1' }, planOf, settled, shown)).toBe('p1');
    expect(diffGroup({ kind: 'task', id: 't1' }, planOf, settled, shown)).toBe('p1');
    expect(diffGroup({ kind: 'task', id: 't2' }, planOf, settled, shown)).toBe('merged');
    expect(diffGroup({ kind: 'task', id: 't3' }, planOf, settled, shown)).toBe('loose');
    expect(diffGroup(null, planOf, settled, shown)).toBeNull();
  });

  it('an agent diff has a review key of its own', () => {
    expect(reviewKey({ kind: 'agent', id: 's1' })).toBe('agent:s1');
    expect(reviewKey({ kind: 'agent', id: 's1' })).not.toBe(reviewKey({ kind: 'task', id: 's1' }));
  });
});

describe('the 1:1 agents the Changes room lists', () => {
  const eli = agent({});
  const ana = agent({ id: 's2', name: 'Ana' });
  const lead = agent({ id: 'l1', name: 'Lead', role: 'lead', worktree: null });
  const worker = agent({ id: 'w1', name: 'Ada', role: 'worker', worktree: '/wt/ada' });
  const fresh = agent({ id: 's3', name: 'Bo', worktree: null });
  const gone = agent({ id: 's4', name: 'Cy', archived: true });
  const all = [eli, ana, lead, worker, fresh, gone];

  it('each 1:1 agent with a worktree that changed files, and no other agent', () => {
    expect(agentsWithChanges(all, () => true, null)).toEqual([eli, ana]);
    expect(agentsWithChanges(all, (a) => a.id === 's2', null)).toEqual([ana]);
  });

  it('the one you opened from its sheet, also before it changed a file', () => {
    expect(agentsWithChanges(all, () => false, 's1')).toEqual([eli]);
    expect(agentsWithChanges(all, () => false, 's3')).toEqual([]);
  });

  it('a file opens in the agent’s own worktree, or in your project folder for an agent that works there', () => {
    expect(agentEditorPlace(eli, '/repo')).toEqual({ agentId: 's1', tip: 'Opens the file in Eli’s worktree' });
    expect(agentEditorPlace(agent({ worktree: '/repo' }), '/repo')).toEqual({ agentId: 's1', tip: 'Opens the file in your project folder' });
    expect(agentEditorPlace(fresh, '/repo').agentId).toBeNull();
    expect(agentEditorPlace(null, '/repo').agentId).toBeNull();
  });

  it('the review bar says where Send takes the comments', () => {
    expect(agentBarWords(1, 'Eli')).toBe('Send gives it to Eli as one message in your 1:1 chat; if Eli is working, it reads it when its turn ends.');
    expect(agentBarWords(3, 'Eli')).toContain('Send gives them to Eli');
  });
});

describe('Send back on a plan that still runs', () => {
  const t1 = task({});
  const t2 = task({ id: 't2', key: 't2', status: 'reviewing', assignee: 'w2' });
  const t3 = task({ id: 't3', key: 't3', status: 'done', assignee: 'w3' });
  const t4 = task({ id: 't4', key: 't4', status: 'cancelled', assignee: 'w4' });
  const files: Record<string, string[]> = { t1: ['a.ts', 'shared.ts'], t2: ['b.ts', 'shared.ts'], t3: ['c.ts'], t4: ['d.ts'] };
  const owners = fileOwners([t1, t2, t3, t4], (t) => files[t.id]);

  it('a file belongs to the first task that has it', () => {
    expect(owners.get('shared.ts')).toBe(t1);
    expect(owners.get('b.ts')).toBe(t2);
    expect(owners.get('nobody.ts')).toBeUndefined();
  });

  it('the comments of a task at work go to it; those of a done or cancelled task, or of no task, to a new request', () => {
    const list = [note('b.ts'), note('a.ts'), note('c.ts'), note('shared.ts'), note('d.ts'), note('nobody.ts')];
    const { tasks, rest } = splitByTask(list, owners);
    expect(tasks.map((p) => [p.task.key, p.comments.map((c) => c.path)])).toEqual([
      ['t2', ['b.ts']],
      ['t1', ['a.ts', 'shared.ts']],
    ]);
    expect(rest.map((c) => c.path)).toEqual(['c.ts', 'd.ts', 'nobody.ts']);
  });

  it('the split in words, before and after the press', () => {
    const parts = [
      { n: 2, who: 'Ada', key: 't1' },
      { n: 1, who: 'Bo', key: 't2' },
    ];
    expect(splitWords(parts, 1, true)).toBe('2 comments to Ada (t1), 1 to Bo (t2); 1 is in a new request');
    expect(splitWords(parts, 2, false)).toBe('2 comments to Ada (t1), 1 to Bo (t2); 2 go to a new request');
    expect(splitWords([{ n: 1, who: 'Bo', key: 't2' }], 0, true)).toBe('1 comment to Bo (t2)');
  });
});
