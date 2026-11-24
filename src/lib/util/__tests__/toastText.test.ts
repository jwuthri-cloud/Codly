import { describe, expect, it } from 'vitest';
import { toastParts } from '../toastText';

describe('toastParts: a toast’s text as it is drawn', () => {
  it('draws what stands between backticks as code, without the backticks', () => {
    expect(toastParts('The check `pnpm test` failed on the merge result')).toEqual([
      { text: 'The check ', code: false },
      { text: 'pnpm test', code: true },
      { text: ' failed on the merge result', code: false },
    ]);
  });

  it('changes no letter: a text can start with a name whose case counts', () => {
    // The conflict toast of a plan merge (state/actions.ts) starts with the branch.
    expect(toastParts('main and this plan both changed a.ts. main is unchanged.')).toEqual([
      { text: 'main and this plan both changed a.ts. main is unchanged.', code: false },
    ]);
    expect(toastParts('codly: Lead needs you')).toEqual([{ text: 'codly: Lead needs you', code: false }]);
    expect(toastParts('makefile is outside this agent’s worktree')).toEqual([{ text: 'makefile is outside this agent’s worktree', code: false }]);
    expect(toastParts('`main` moved')).toEqual([
      { text: 'main', code: true },
      { text: ' moved', code: false },
    ]);
  });

  it('keeps a backtick that opens nothing', () => {
    expect(toastParts('a ` alone')).toEqual([{ text: 'a ` alone', code: false }]);
    expect(toastParts('run `a` then ` b')).toEqual([
      { text: 'run ', code: false },
      { text: 'a', code: true },
      { text: ' then ` b', code: false },
    ]);
  });

  it('is nothing for no text', () => {
    expect(toastParts('')).toEqual([]);
  });
});
