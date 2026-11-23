import { describe, expect, it } from 'vitest';
import { inlineParts } from '../inlineParts';

describe('inline text', () => {
  it('shows code spans and bold without their marks', () => {
    expect(inlineParts('**t1** adds `ThemeProvider` to the app')).toEqual([
      { kind: 'bold', t: 't1' },
      { kind: 'text', t: ' adds ' },
      { kind: 'code', t: 'ThemeProvider' },
      { kind: 'text', t: ' to the app' },
    ]);
  });

  it('keeps plain text, and marks that are not a pair, as written', () => {
    expect(inlineParts('2 * 3 ** 4 and a lone ` tick')).toEqual([{ kind: 'text', t: '2 * 3 ** 4 and a lone ` tick' }]);
    expect(inlineParts('')).toEqual([]);
  });

  it('leaves the stars of a command alone: bold has no space inside its marks and no letter or digit outside', () => {
    for (const command of ['x ** 2 + y ** 2', "python -c 'print(x ** 2 + y ** 2)'", 'x**2 + y**2', '2**3 + 4** 5', 'f(**a, **b)', 'ls src/**/*.ts test/**/*.ts'])
      expect(inlineParts(command)).toEqual([{ kind: 'text', t: command }]);
    // a code span after stars that are not a pair is still found
    expect(inlineParts('a**b `c` d** e')).toEqual([
      { kind: 'text', t: 'a**b ' },
      { kind: 'code', t: 'c' },
      { kind: 'text', t: ' d** e' },
    ]);
  });

  it('shows bold next to punctuation and in a text without spaces between words', () => {
    expect(inlineParts('(**note**) and **two words**.')).toEqual([
      { kind: 'text', t: '(' },
      { kind: 'bold', t: 'note' },
      { kind: 'text', t: ') and ' },
      { kind: 'bold', t: 'two words' },
      { kind: 'text', t: '.' },
    ]);
    expect(inlineParts('这是**重要**的')).toEqual([
      { kind: 'text', t: '这是' },
      { kind: 'bold', t: '重要' },
      { kind: 'text', t: '的' },
    ]);
  });

  it('never joins marks across lines, and keeps stars inside code', () => {
    expect(inlineParts('**a\nb**')).toEqual([{ kind: 'text', t: '**a\nb**' }]);
    expect(inlineParts('`a\nb`')).toEqual([{ kind: 'text', t: '`a\nb`' }]);
    expect(inlineParts('`a ** b`')).toEqual([{ kind: 'code', t: 'a ** b' }]);
  });
});
