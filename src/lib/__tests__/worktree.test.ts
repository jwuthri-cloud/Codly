import { describe, expect, it } from 'vitest';
import { WorktreeDemo } from '../demo/worktree';

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
});
