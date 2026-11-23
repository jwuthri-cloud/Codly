import { describe, expect, it } from 'vitest';
import { WorkspaceTrustDemo } from '../demo/workspaceTrust';
import type { TrustFile } from '../gen/TrustFile';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';
import type { WorktreeTrust } from '../gen/WorktreeTrust';
import { MAX_SHOWN_FILES, asked, shouldAsk, shownFiles, trustChoices, trustStatus, withoutTrust } from '../util/workspaceTrust';

describe('the demo repo’s own CLI configuration', () => {
  it('is a question until you answer it, for the files shown', () => {
    const demo = new WorkspaceTrustDemo(true);
    const asked = demo.trust(null);
    expect(asked).toMatchObject({ state: 'ask', changed: false, problem: null, worktrees: [] });
    expect(asked.files.map((f) => f.path)).toEqual(['.claude/settings.json', '.mcp.json']);

    // Another set of files than the one shown is refused, as the core refuses it.
    expect(() => demo.trust({ trust: 'other-files' })).toThrow(/changed since you read it/);
    expect(demo.trust(null).state).toBe('ask');

    expect(demo.trust({ trust: asked.fingerprint }).state).toBe('trusted');
    expect(demo.trust({ trust: null }).state).toBe('without');
    expect(demo.trust(null).state).toBe('without');
  });

  it('asks again when the files change, whatever you chose before', () => {
    const demo = new WorkspaceTrustDemo();
    const first = demo.trust(null);
    expect(first.state).toBe('trusted');
    demo.files = [...demo.files, { path: '.claude/hooks/post.sh', cli: 'claude', what: [] }];

    const changed = demo.trust(null);
    expect(changed).toMatchObject({ state: 'ask', changed: true });
    expect(changed.fingerprint).not.toBe(first.fingerprint);
    // The files you trusted before are not these.
    expect(() => demo.trust({ trust: first.fingerprint })).toThrow(/changed since you read it/);
    expect(demo.trust({ trust: null })).toMatchObject({ state: 'without', changed: false });
  });
});

describe('what the question and Settings offer', () => {
  const base: WorkspaceTrust = {
    state: 'ask',
    changed: false,
    files: [
      { path: '.claude/settings.json', cli: 'claude', what: ['allows without asking: Bash(*)'] },
      { path: '.mcp.json', cli: 'claude', what: [] },
      { path: '.codex/config.toml', cli: 'codex', what: [] },
      { path: '.gemini/settings.json', cli: 'gemini', what: [] },
    ],
    instructions: [],
    fingerprint: 'abc',
    problem: null,
    worktrees: [],
  };
  const nothing: WorkspaceTrust = { ...base, state: 'nothing', files: [], fingerprint: null };
  /** A worktree where an agent added a command file. */
  const worktree: WorktreeTrust = {
    path: '/data/worktrees/app/solo/ana',
    files: [{ path: '.claude/commands/deploy.md', cli: 'claude', what: [] }],
    fingerprint: 'wt1',
    problem: null,
  };

  it('asks once for a set of files, and again when they change', () => {
    expect(shouldAsk(base, [])).toBe(true);
    // You closed the question on these files: not again until they change.
    expect(shouldAsk(base, ['abc'])).toBe(false);
    expect(shouldAsk({ ...base, fingerprint: 'def', changed: true }, ['abc'])).toBe(true);
    for (const state of ['trusted', 'without', 'nothing'] as const) expect(shouldAsk({ ...base, state }, [])).toBe(false);
  });

  it('asks about a worktree whose files are not the ones of your folder', () => {
    // Whatever you chose for the folder itself, also when it has no such files.
    for (const t of [{ ...base, state: 'trusted' as const }, nothing]) {
      const withWorktree = { ...t, worktrees: [worktree] };
      expect(asked(withWorktree)).toEqual(['wt1']);
      expect(shouldAsk(withWorktree, [])).toBe(true);
      expect(shouldAsk(withWorktree, ['wt1'])).toBe(false);
    }
    // Both at once: closing the question puts both off, and a new worktree set asks again.
    const both = { ...base, worktrees: [worktree] };
    expect(asked(both)).toEqual(['abc', 'wt1']);
    expect(shouldAsk(both, ['abc'])).toBe(true);
    expect(shouldAsk(both, ['abc', 'wt1'])).toBe(false);
    expect(shouldAsk({ ...both, worktrees: [{ ...worktree, fingerprint: 'wt2' }] }, ['abc', 'wt1'])).toBe(true);
    // You opened the repo without its files: the question asks nothing more (Settings shows the worktree).
    expect(shouldAsk({ ...both, state: 'without' }, [])).toBe(false);
  });

  it('lists what Codly read something in first, and counts what it does not show', () => {
    const plain = (n: number): TrustFile[] => Array.from({ length: n }, (_, i) => ({ path: `.claude/skills/s${i}.md`, cli: 'claude' as const, what: [] }));
    const hooks: TrustFile = { path: '.claude/settings.json', cli: 'claude', what: ['runs commands by itself (hooks): ./x.sh'] };
    expect(shownFiles(base.files)).toEqual({ shown: base.files, more: 0 });
    const many = shownFiles([...plain(MAX_SHOWN_FILES + 5), hooks]);
    expect(many.shown).toHaveLength(MAX_SHOWN_FILES);
    expect(many.shown[0]).toBe(hooks);
    expect(many.more).toBe(6);
  });

  it('offers only the choices that change something', () => {
    expect(trustChoices(base)).toEqual({ trust: 'abc', without: true });
    expect(trustChoices({ ...base, state: 'trusted' })).toEqual({ trust: null, without: true });
    expect(trustChoices({ ...base, state: 'without' })).toEqual({ trust: 'abc', without: false });
    expect(trustChoices(nothing)).toEqual({ trust: null, without: false });
    // Files Codly cannot read whole can only be opened without.
    expect(trustChoices({ ...base, problem: '.claude/x is not a regular file' })).toEqual({ trust: null, without: true });
  });

  it('says where the choice stands', () => {
    expect(trustStatus(nothing)).toMatch(/no configuration/);
    expect(trustStatus(base)).toMatch(/not chosen yet/);
    expect(trustStatus({ ...base, changed: true })).toMatch(/changed since your choice/);
    expect(trustStatus({ ...base, state: 'trusted' })).toMatch(/Agents load them/);
    expect(trustStatus({ ...base, state: 'without' })).toMatch(/start without them/);
  });

  it('says for each CLI with files here what happens without trust', () => {
    const lines = withoutTrust(base);
    expect(lines).toHaveLength(3); // one per CLI, not per file
    expect(lines[0]).toMatch(/^Claude Code starts with your own settings only/);
    expect(lines[1]).toMatch(/^Codex decides by itself/);
    expect(lines[2]).toBe('Gemini CLI agents do not start in this repo: Codly cannot keep Gemini CLI from loading these files.');
    expect(withoutTrust(nothing)).toEqual([]);
  });
});
