import { describe, expect, it } from 'vitest';
import { press, wants } from '../palette';

describe('palette confirmation', () => {
  const merge = { id: 'plan:merge:p1', confirm: 'Merge “Dark mode” into main?' };
  const go = { id: 'go:plan' };

  it('a row that cannot be taken back runs on the second Enter only', () => {
    const first = press(merge, null, false);
    expect(first).toEqual({ run: false, armed: 'plan:merge:p1' });
    expect(press(merge, first.armed, false)).toEqual({ run: true, armed: null });
  });

  it('a held Enter or a double-click never confirms: only a new press does', () => {
    const first = press(merge, null, false);
    expect(press(merge, first.armed, true)).toEqual({ run: false, armed: 'plan:merge:p1' });
    expect(press(merge, null, true)).toEqual({ run: false, armed: null });
    expect(press(merge, first.armed, false)).toEqual({ run: true, armed: null });
  });

  it('arming one row never confirms another', () => {
    const deny = { id: 'ap:no:a1', confirm: 'Deny?' };
    expect(press(deny, 'plan:merge:p1', false)).toEqual({ run: false, armed: 'ap:no:a1' });
  });

  it('an ordinary row runs at once and lets an armed row go', () => {
    expect(press(go, null, false)).toEqual({ run: true, armed: null });
    expect(press(go, 'plan:merge:p1', false)).toEqual({ run: true, armed: null });
  });

  it('says in full what an approval asks for', () => {
    const long = `rm -rf ${'very/long/path/'.repeat(8)}dist`;
    expect(wants({ kind: 'command', command: long, cwd: '/repo' })).toBe(`run ${long}\nin /repo`);
    expect(wants({ kind: 'command', command: 'ls', cwd: null })).toBe('run ls');
    expect(wants({ kind: 'file_change', paths: ['a.ts', 'b.ts'], diff: null })).toBe('change a.ts, b.ts');
    expect(wants({ kind: 'tool', name: 'WebFetch', input: null })).toBe('use the tool WebFetch');
  });
});
