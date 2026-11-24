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

  it('starts with a capital letter', () => {
    expect(toastParts('checks failed on the merge result')).toEqual([{ text: 'Checks failed on the merge result', code: false }]);
    expect(toastParts('couldn’t reach the core')).toEqual([{ text: 'Couldn’t reach the core', code: false }]);
    expect(toastParts('a')).toEqual([{ text: 'A', code: false }]);
  });

  it('leaves a first word as it is when it is not a plain word: a path, a file, code', () => {
    expect(toastParts('src/app.ts changed')).toEqual([{ text: 'src/app.ts changed', code: false }]);
    expect(toastParts('package.json changed')).toEqual([{ text: 'package.json changed', code: false }]);
    expect(toastParts('`main` moved')).toEqual([
      { text: 'main', code: true },
      { text: ' moved', code: false },
    ]);
  });

  it('keeps a backtick that opens nothing', () => {
    expect(toastParts('a ` alone')).toEqual([{ text: 'A ` alone', code: false }]);
    expect(toastParts('run `a` then ` b')).toEqual([
      { text: 'Run ', code: false },
      { text: 'a', code: true },
      { text: ' then ` b', code: false },
    ]);
  });

  it('is nothing for no text', () => {
    expect(toastParts('')).toEqual([]);
  });
});
