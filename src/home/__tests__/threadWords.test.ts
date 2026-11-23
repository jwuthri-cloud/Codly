// The words of the review chip and of a task that waits on others.
import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../lib/gen/ApprovalSubject';
import { reviewLabel, waitLine } from '../threadWords';

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
