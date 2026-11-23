// The words of the review chip and of a task that waits on others.
import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../lib/gen/ApprovalSubject';
import { reviewLabel, taskTone, waitLine } from '../threadWords';

const ask = (kind: ApprovalSubject['kind']) => ({ subject: { kind } as ApprovalSubject });

describe('reviewLabel', () => {
  it('a question is answered, not reviewed', () => {
    expect(reviewLabel([ask('question')])).toBe('Answer the question');
    expect(reviewLabel([ask('question'), ask('question')])).toBe('Answer 2 questions');
  });
  it('approvals are reviewed; a mix is requests', () => {
    expect(reviewLabel([ask('command')])).toBe('Review the approval');
    expect(reviewLabel([ask('command'), ask('file_change')])).toBe('Review 2 approvals');
    expect(reviewLabel([ask('command'), ask('question')])).toBe('Review 2 requests');
  });
});

describe('waitLine', () => {
  it('names what it waits on', () => {
    expect(waitLine([{ key: 't1', status: 'running' }, { key: 't2', status: 'pending' }])).toEqual({ text: 'Waiting on t1, t2', stuck: false });
  });
  it('a cancelled task it depends on: it cannot start, and says why', () => {
    expect(waitLine([{ key: 't1', status: 'cancelled' }, { key: 't2', status: 'running' }])).toEqual({ text: 'Can’t start · t1 was cancelled', stuck: true });
    expect(waitLine([{ key: 't1', status: 'failed' }])?.text).toBe('Can’t start · t1 failed');
  });
  it('waits on nothing: no line', () => {
    expect(waitLine([])).toBeNull();
  });
});

describe('taskTone', () => {
  const base = { status: 'pending' as const, held: false, stuck: false, proposing: false, worker: null };
  it('a task that cannot start because one before it stopped is blocked, pending or ready', () => {
    expect([taskTone({ ...base, stuck: true }), taskTone({ ...base, status: 'ready', stuck: true })]).toEqual(['blocked', 'blocked']);
  });
  it('a cancelled task stays off, also after a cancelled one it waited on', () => {
    expect(taskTone({ ...base, status: 'cancelled', stuck: true })).toBe('off');
  });
  it('a held task is on hold, a worker that waits for you comes first, and a plan you decide on blocks nothing', () => {
    expect([taskTone({ ...base, status: 'ready', held: true, stuck: true }), taskTone({ ...base, status: 'running', worker: 'wait' }), taskTone({ ...base, stuck: true, proposing: true })]).toEqual(['idle', 'wait', 'idle']);
  });
});
