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

  it('never joins marks across lines, and keeps stars inside code', () => {
    expect(inlineParts('**a\nb**')).toEqual([{ kind: 'text', t: '**a\nb**' }]);
    expect(inlineParts('`a\nb`')).toEqual([{ kind: 'text', t: '`a\nb`' }]);
    expect(inlineParts('`a ** b`')).toEqual([{ kind: 'code', t: 'a ** b' }]);
  });
});
