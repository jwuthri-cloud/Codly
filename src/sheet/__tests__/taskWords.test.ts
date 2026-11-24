// The task sheet's head and the agent's Git tab. The fourth browser review (defects 19 and 20):
// "Cancelled — Cancelled by you", a task after a cancelled one read "Waiting on t1, t2", and
// "Merge into main" stood on an agent with nothing to merge.
import { describe, expect, it } from 'vitest';
import type { AgentGit } from '../../lib/gen/AgentGit';
import type { Task } from '../../lib/gen/Task';
import { hasSomethingToMerge, taskStatusLine } from '../taskWords';

const dep = (key: string, status: Task['status']) => ({ key, status });

describe('the status line of a task', () => {
  it('a reason that only repeats the status is said once', () => {
    expect(taskStatusLine({ status: 'cancelled', status_reason: 'cancelled by you' }, [])).toBe('Cancelled by you');
    expect(taskStatusLine({ status: 'cancelled', status_reason: 'Cancelled' }, [])).toBe('Cancelled');
    expect(taskStatusLine({ status: 'blocked', status_reason: 'blocked by you.' }, [])).toBe('Blocked by you');
  });

  it('another reason follows the status', () => {
    expect(taskStatusLine({ status: 'blocked', status_reason: 'stopped by you' }, [])).toBe('Blocked — stopped by you');
    expect(taskStatusLine({ status: 'running', status_reason: null }, [])).toBe('Running');
  });

  it('a task that waits on a cancelled one says it cannot start, as the thread does', () => {
    const deps = [dep('t1', 'cancelled'), dep('t2', 'running')];
    expect(taskStatusLine({ status: 'pending', status_reason: 'Waiting on t1, t2' }, deps)).toBe('Backlog — Can’t start · t1 was cancelled');
    expect(taskStatusLine({ status: 'pending', status_reason: 'Waiting on t1, t2' }, [dep('t1', 'done'), dep('t2', 'running')])).toBe('Backlog — Waiting on t2');
    // A task that waits on nothing any more keeps the core's reason.
    expect(taskStatusLine({ status: 'ready', status_reason: 'Ready: a free agent picks it up' }, [dep('t1', 'done')])).toBe('Ready: a free agent picks it up');
  });
});

describe('“Merge into main” in an agent’s Git tab', () => {
  const view = (o: Partial<AgentGit>): Pick<AgentGit, 'branch' | 'files' | 'branches'> => ({
    branch: 'codly/eli',
    files: [],
    branches: [{ name: 'main', current: false, sha: 'abc1234', ts: 1 }, { name: 'codly/eli', current: true, sha: 'abc1234', ts: 1 }],
    ...o,
  });

  it('is not offered when the branch is where main is and nothing changed', () => {
    expect(hasSomethingToMerge(view({}), 'main')).toBe(false);
  });

  it('is offered for a commit of its own, or a change in the worktree', () => {
    expect(hasSomethingToMerge(view({ branches: [{ name: 'main', current: false, sha: 'abc1234', ts: 1 }, { name: 'codly/eli', current: true, sha: 'def5678', ts: 2 }] }), 'main')).toBe(true);
    expect(hasSomethingToMerge(view({ files: [{ path: 'a.ts', code: ' M' }] }), 'main')).toBe(true);
  });

  it('is offered when the branch it merges into is not known, and never without a branch', () => {
    expect(hasSomethingToMerge(view({}), 'your branch')).toBe(true);
    expect(hasSomethingToMerge(view({ branch: null }), 'main')).toBe(false);
  });
});
