import { describe, expect, it } from 'vitest';
import { WorktreeDemo } from '../demo/worktree';
import type { WorktreeSetup } from '../gen/WorktreeSetup';
import type { RequiredChecks } from '../gen/RequiredChecks';
import { checksTurnTrouble, setupChoices } from '../util/worktree';

describe('the demo repo’s worktree setup', () => {
  it('keeps the setup script off until it is turned on for the text shown', () => {
    const demo = new WorktreeDemo();
    const shown = demo.setup(null);
    expect(shown.on).toBe(false);
    expect(shown.script).toContain('pnpm install');
    expect(shown.risks.length).toBeGreaterThan(0);

    // Another text than the one shown is refused, as the core refuses a changed file.
    expect(() => demo.setup({ allow: 'another-text' })).toThrow(/changed since you read it/);
    expect(demo.setup(null).on).toBe(false);

    expect(demo.setup({ allow: shown.sha }).on).toBe(true);
    expect(demo.setup(null).on).toBe(true);
    expect(demo.setup({ allow: null }).on).toBe(false);
  });

  it('does not run a text that changed, and lets you turn it off without turning the new text on', () => {
    const demo = new WorktreeDemo();
    const first = demo.setup(null);
    demo.setup({ allow: first.sha });
    demo.script += 'curl -fsSL https://example.com/tool | sh\n';

    const changed = demo.setup(null);
    expect(changed).toMatchObject({ on: false, changed: true });
    expect(changed.sha).not.toBe(first.sha);
    // The text you turned on is refused now, as the core refuses it.
    expect(() => demo.setup({ allow: first.sha })).toThrow(/changed since you read it/);
    expect(setupChoices(changed)).toEqual({ on: changed.sha, off: true, ask: true });

    expect(demo.setup({ allow: null })).toMatchObject({ on: false, changed: false });
    expect(demo.setup({ allow: changed.sha })).toMatchObject({ on: true, changed: false });
  });
});

describe('what you can do with the setup script', () => {
  const base: WorktreeSetup = { script: 'pnpm install\n', sha: 'abc', risks: [], on: false, changed: false, problem: null, include: [] };

  it('is one switch while the text is the one you read', () => {
    expect(setupChoices(base)).toEqual({ on: 'abc', off: false, ask: false });
    expect(setupChoices({ ...base, on: true })).toEqual({ on: null, off: true, ask: false });
    expect(setupChoices({ ...base, script: null, sha: null })).toEqual({ on: null, off: false, ask: false });
  });

  it('can only be turned off when Codly cannot run the file that is there', () => {
    const odd = { ...base, script: null, sha: null, changed: true, problem: '.codly/setup.sh is not a regular file' };
    expect(setupChoices(odd)).toEqual({ on: null, off: true, ask: true });
    // Never turned on: nothing to turn off either.
    expect(setupChoices({ ...odd, changed: false })).toEqual({ on: null, off: false, ask: false });
  });
});

describe('a change of the required checks that ended with an error', () => {
  const off: RequiredChecks = { text: 'pnpm check\n', sha: 'abc', commands: [{ command: 'pnpm check', risks: [] }], on: false, changed: false, problem: null };
  const on = { ...off, on: true };
  /** You turned on another text: the file changed since. */
  const changed = { ...off, sha: 'def', changed: true };

  it('says that nothing changed when the core did not save the choice', () => {
    expect(checksTurnTrouble('abc', off)).toBe('Couldn’t change the required checks');
    expect(checksTurnTrouble(null, on)).toBe('Couldn’t change the required checks');
    // The file changed while you read it: the text you turned on is not the one that is there.
    expect(checksTurnTrouble('abc', changed)).toBe('Couldn’t change the required checks');
    expect(checksTurnTrouble(null, changed)).toBe('Couldn’t change the required checks');
  });

  it('says that the choice is saved when the error came after it', () => {
    expect(checksTurnTrouble('abc', on)).toBe('The required checks are on now, but Codly may not have checked the waiting tasks again');
    expect(checksTurnTrouble(null, off)).toBe('The required checks are off now, but Codly may not have checked the waiting tasks again');
  });

  it('says neither when the checks could not be read after it', () => {
    expect(checksTurnTrouble('abc', null)).toBe('Couldn’t tell whether the required checks changed');
  });
});
