import { describe, expect, it } from 'vitest';
import type { Plan } from '../../gen/Plan';
import type { Task } from '../../gen/Task';
import { keyList, stuckPlan, waitingOn } from '../plan';

const plan = (o: Partial<Plan> = {}): Plan => ({
  id: 'p1', workspace_id: 'ws', thread: 'lead', request: '', summary: 'Sign-in', assumptions: [], open_questions: [], status: 'running',
  base_commit: null, base_branch: null, integration_branch: null, integration_worktree: null, merged_sha: null, kept: false, created_at: 0, updated_at: 0, ...o,
});

const task = (key: string, status: Task['status'], o: Partial<Task> = {}): Task => ({
  id: key, plan_id: 'p1', key, title: key, goal: '', acceptance: [], write_scope: [], verify: [], depends_on: [], status,
  assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: false, branch: null, worktree: null, attempt: 0,
  status_reason: null, summary: null, created_at: 0, updated_at: 0, ...o,
});

const keys = (ts: Task[]) => ts.map((t) => t.key);

describe('stuckPlan', () => {
  it('names what stopped the plan, what waits behind it and what is merged', () => {
    const tasks = [task('t1', 'done'), task('t2', 'blocked', { status_reason: 'blocked: the gate failed' }), task('t3', 'pending', { depends_on: ['t2'] }), task('t4', 'pending', { depends_on: ['t3'] })];
    const stuck = stuckPlan(plan(), tasks);
    expect(stuck && { stopped: keys(stuck.stopped), waiting: keys(stuck.waiting), merged: stuck.merged }).toEqual({ stopped: ['t2'], waiting: ['t3', 't4'], merged: 1 });
  });

  it('counts a failed task, and a cancelled one that others wait for', () => {
    expect(keys(stuckPlan(plan(), [task('t1', 'failed')])!.stopped)).toEqual(['t1']);
    expect(keys(stuckPlan(plan(), [task('t1', 'cancelled'), task('t2', 'pending', { depends_on: ['t1'] })])!.stopped)).toEqual(['t1']);
  });

  it('is null while the plan still goes on', () => {
    const stopped = task('t1', 'blocked', { status_reason: 'blocked: the gate failed' });
    for (const status of ['ready', 'running', 'verifying', 'reviewing', 'changes_requested', 'merging'] as const) {
      expect(stuckPlan(plan(), [stopped, task('t2', status)])).toBeNull();
    }
    // A task whose dependencies are all merged starts at the next pass.
    expect(stuckPlan(plan(), [stopped, task('t2', 'done'), task('t3', 'pending', { depends_on: ['t2'] })])).toBeNull();
    // Every task done or cancelled: the plan finishes by itself. A plan that does not run is never stuck.
    expect(stuckPlan(plan(), [task('t1', 'done'), task('t2', 'cancelled')])).toBeNull();
    expect(stuckPlan(plan({ status: 'awaiting_approval' }), [stopped])).toBeNull();
  });

  it('leaves a plan that only waits for your answer to its Ask me card', () => {
    const asks = task('t1', 'blocked', { status_reason: 'waiting for you: Which provider?' });
    expect(stuckPlan(plan(), [asks, task('t2', 'pending', { depends_on: ['t1'] })])).toBeNull();
    expect(keys(stuckPlan(plan(), [asks, task('t3', 'failed')])!.stopped)).toEqual(['t1', 't3']);
  });
});

describe('waitingOn', () => {
  const tasks = [
    task('t1', 'failed'), task('t2', 'pending', { depends_on: ['t1'] }), task('t3', 'pending', { depends_on: ['t2'] }),
    task('t4', 'pending'), task('t5', 'pending', { depends_on: ['t4', 't3'] }), task('t6', 'cancelled', { depends_on: ['t1'] }),
  ];

  it('finds the tasks behind a task, through other tasks, that are not over', () => {
    expect(keys(waitingOn(tasks, 't1'))).toEqual(['t2', 't3', 't5']);
    expect(keys(waitingOn(tasks, 't4'))).toEqual(['t5']);
    expect(waitingOn(tasks, 't5')).toEqual([]);
  });
});

describe('keyList', () => {
  it('reads as a sentence', () => {
    expect([keyList([]), keyList([task('t1', 'done')]), keyList([task('t1', 'done'), task('t2', 'done'), task('t3', 'done')])]).toEqual(['', 't1', 't1, t2 and t3']);
  });
});
