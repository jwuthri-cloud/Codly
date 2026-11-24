import { describe, expect, it } from 'vitest';
import { cutTip } from '../cutTip';

const fits = { scrollWidth: 300, clientWidth: 300, scrollHeight: 40, clientHeight: 40 };

describe('cutTip: a fact that is cut tells its whole text', () => {
  it('is nothing for a text that shows whole', () => {
    expect(cutTip(fits, 'proposed 10:42')).toBeNull();
  });

  it('is the whole text for one line with an ellipsis', () => {
    expect(cutTip({ ...fits, scrollWidth: 412 }, 'codly/plan-7f3a-add-the-note')).toBe('codly/plan-7f3a-add-the-note');
  });

  it('is the whole text for a sentence whose last line does not show', () => {
    expect(cutTip({ ...fits, scrollHeight: 60 }, 'Diffs from each task’s worktree and the plan’s integration branch')).toBe('Diffs from each task’s worktree and the plan’s integration branch');
  });

  it('is nothing for a box with no text', () => {
    expect(cutTip({ ...fits, scrollWidth: 412 }, '')).toBeNull();
  });
});
