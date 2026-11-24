import { describe, expect, it } from 'vitest';
import type { MergeState } from '../../gen/MergeState';
import { mergeOfferGone } from '../mergeOffer';

const state = (patch: Partial<MergeState> = {}): MergeState => ({
  target: 'main',
  branch: 'codly/plan-1',
  repo: '/code/app',
  conflicts: [],
  resolving: null,
  failed_checks: [],
  checks_not_run: null,
  behind: 0,
  ...patch,
});

describe('mergeOfferGone: a merge toast that offers what is no longer there goes', () => {
  it('goes once the plan is merged, whatever held it back before', () => {
    expect(mergeOfferGone('checks', true, state({ failed_checks: ['pnpm test'] }))).toBe(true);
    expect(mergeOfferGone('conflict', true, state({ conflicts: ['a.rs'] }))).toBe(true);
    expect(mergeOfferGone('branch', true, undefined)).toBe(true);
  });

  it('“Not merged into main · Merge anyway” goes once the checks no longer fail there', () => {
    expect(mergeOfferGone('checks', false, state({ failed_checks: ['pnpm test'] }))).toBe(false);
    expect(mergeOfferGone('checks', false, state({ checks_not_run: 'no worktree' }))).toBe(false);
    expect(mergeOfferGone('checks', false, state())).toBe(true);
  });

  it('“Have an agent resolve it” goes once the conflict is resolved', () => {
    expect(mergeOfferGone('conflict', false, state({ conflicts: ['a.rs'] }))).toBe(false);
    expect(mergeOfferGone('conflict', false, state())).toBe(true);
  });

  it('stays while nothing says otherwise: the merge state not known, or a question of branch', () => {
    expect(mergeOfferGone('checks', false, undefined)).toBe(false);
    expect(mergeOfferGone('conflict', false, undefined)).toBe(false);
    expect(mergeOfferGone('branch', false, state())).toBe(false);
  });
});
