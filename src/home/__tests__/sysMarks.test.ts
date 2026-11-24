// The thread's quiet lines showed raw backticks, quote marks and "**t1**" (round 8 review, part B,
// minor 15): they show code, bold and curly quotes, as text only.
import { describe, expect, it } from 'vitest';
import { sysMarks } from '../sysMarks';

describe('sysMarks', () => {
  it('reads code, bold and quotes', () => {
    expect(sysMarks('Not merged: with "Dark mode toggle" merged into main, `npx playwright test settings-theme` failed.')).toEqual([
      { t: 'Not merged: with “Dark mode toggle” merged into main, ' },
      { t: 'npx playwright test settings-theme', code: true },
      { t: ' failed.' },
    ]);
    expect(sysMarks('**t1** is done')).toEqual([{ t: 't1', bold: true }, { t: ' is done' }]);
  });

  it('leaves code as it is, and a mark left open as plain words', () => {
    expect(sysMarks('`say "hi" **now**`')).toEqual([{ t: 'say "hi" **now**', code: true }]);
    expect(sysMarks('a ` b ** c')).toEqual([{ t: 'a ` b ** c' }]);
  });

  it('never makes markup of the words: a tag is words', () => {
    expect(sysMarks('<img src=x onerror=alert(1)> **<b>**')).toEqual([{ t: '<img src=x onerror=alert(1)> ' }, { t: '<b>', bold: true }]);
  });
});
