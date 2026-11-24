// With a pull request open, "View the pull request" is the main button until its checks are read and pass.
import { describe, expect, it } from 'vitest';
import type { PrChecks } from '../../lib/gen/PrChecks';
import { afterLocalMerge, pullRequestFirst, pullRequestTip } from '../mergeChoice';

const checks = (c: Partial<PrChecks>): PrChecks => ({ passed: 0, failed: 0, pending: 0, skipped: 0, failing: [], read_at: 1, ...c });

describe('pullRequestFirst', () => {
  it('no pull request: Merge is the main button', () => {
    expect(pullRequestFirst(null)).toBe(false);
    expect(pullRequestFirst({ state: 'merged', checks: null })).toBe(false);
  });
  it('checks not read yet: the pull request first', () => {
    expect(pullRequestFirst({ state: 'open', checks: null })).toBe(true);
    expect(pullRequestTip({ checks: null })).toMatch(/not read yet/);
  });
  it('checks still run, or one failed: the pull request first', () => {
    expect(pullRequestFirst({ state: 'open', checks: checks({ passed: 2, pending: 1 }) })).toBe(true);
    expect(pullRequestFirst({ state: 'open', checks: checks({ passed: 2, failed: 1 }) })).toBe(true);
    expect(pullRequestTip({ checks: checks({ failed: 1 }) })).toMatch(/failed/);
  });
  it('checks read and passed: Merge is the main button', () => {
    expect(pullRequestFirst({ state: 'open', checks: checks({ passed: 3, skipped: 1 }) })).toBe(false);
  });
});

// The review's defect 13: after a merge here, the pull request's row said nothing of it.
describe('afterLocalMerge', () => {
  it('says where the pull request stands as Codly last read it', () => {
    expect(afterLocalMerge('open', 'main')).toBe('You merged the plan into main here. As last read, the pull request is still open on GitHub.');
    expect(afterLocalMerge('merged', 'main')).toMatch(/merged on GitHub too/);
    expect(afterLocalMerge('closed', 'dev')).toBe('You merged the plan into dev here. The pull request is closed on GitHub.');
  });
});
