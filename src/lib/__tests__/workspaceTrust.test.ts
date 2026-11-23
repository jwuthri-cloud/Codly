import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { WorkspaceTrustDemo } from '../demo/workspaceTrust';
import type { EventRow } from '../gen/EventRow';
import type { TrustFile } from '../gen/TrustFile';
import type { Workspace } from '../gen/Workspace';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';
import type { WorktreeTrust } from '../gen/WorktreeTrust';
import { store } from '../state/store.svelte';
import { workspaceTrust } from '../state/workspaceTrust.svelte';
import { CODEX_OWN_SERVERS, MAX_SHOWN_FILES, asked, shouldAsk, shownFiles, trustChoices, trustStatus, startsOwnCodexServers, withoutTrust } from '../util/workspaceTrust';

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
    const lines = withoutTrust(base, false);
    expect(lines).toHaveLength(3); // one per CLI, not per file
    expect(lines[0]).toMatch(/^Claude Code starts with your own settings only/);
    expect(lines[1]).toMatch(/^Codex decides by itself/);
    expect(lines[2]).toBe('Gemini CLI agents do not start in this repo: Codly cannot keep Gemini CLI from loading these files.');
    expect(withoutTrust(nothing, false)).toEqual([]);
  });

  it('says what does not wait for trust: a Codex agent’s own MCP servers', () => {
    // A Claude agent loses your own servers without trust; a Codex agent keeps them. Said last.
    const lines = withoutTrust(base, true);
    expect(lines).toHaveLength(4);
    expect(lines[3]).toBe(CODEX_OWN_SERVERS);
    expect(CODEX_OWN_SERVERS).toMatch(/in every folder, trusted or not: they are yours, not this repo’s/);
    // Also for a repo that has no Codex files of its own.
    const claudeOnly = { ...base, files: base.files.filter((f) => f.cli === 'claude') };
    expect(withoutTrust(claudeOnly, true)).toHaveLength(2);
    // Nothing to decide: nothing to say.
    expect(withoutTrust(nothing, true)).toEqual([]);

    // Only when Codex agents do start them: the switch is on and Codex is installed.
    const clis = [
      { provider: 'claude' as const, path: '/bin/claude' },
      { provider: 'codex' as const, path: '/bin/codex' },
    ];
    expect(startsOwnCodexServers({ use_my_claude_setup: true }, clis)).toBe(true);
    expect(startsOwnCodexServers({ use_my_claude_setup: false }, clis)).toBe(false);
    expect(startsOwnCodexServers({ use_my_claude_setup: true }, [clis[0], { provider: 'codex', path: null }])).toBe(false);
    expect(startsOwnCodexServers({ use_my_claude_setup: true }, [clis[0]])).toBe(false);
  });
});

describe('the records that ask for the question', () => {
  const office: Workspace = { id: 'w1', repo_path: '/tmp/w1', name: 'w1', default_branch: 'main', current_branch: 'main', head: null, dirty: false, created_at: 0 };
  const settled: WorkspaceTrust = { state: 'nothing', changed: false, files: [], instructions: [], fingerprint: null, problem: null, worktrees: [] };
  /** A workspace-trust record: the core's (`rule`: an agent started without files you have
   *  not decided about) or your own choice (`human`). */
  const record = (id: number, method = 'rule') =>
    ({ id, ts: id, agent_id: null, task_id: null, run_id: null, workspace_id: 'w1', kind: { type: 'decision', id: `d${id}`, decision_type: 'workspace_trust', method, summary: 'x', detail: {} } }) as unknown as EventRow;
  const turn = () => new Promise((r) => setTimeout(r));

  afterEach(() => {
    vi.restoreAllMocks();
    store.workspace = null;
  });

  it('a burst of records reads the repo once, and once more after it: not once per record', async () => {
    const listeners: ((ev: EventRow) => void)[] = [];
    vi.spyOn(store, 'onEvent').mockImplementation((cb) => {
      listeners.push(cb);
      return () => {};
    });
    const answers: ((t: WorkspaceTrust) => void)[] = [];
    const read = vi.spyOn(api, 'workspaceTrust').mockImplementation(() => new Promise<WorkspaceTrust>((resolve) => answers.push(resolve)));
    store.workspace = office;
    workspaceTrust.start();
    expect(listeners).toHaveLength(1);

    // A window that was hidden gets every record it missed at once.
    for (let id = 1; id <= 5; id++) listeners[0](record(id));
    expect(read).toHaveBeenCalledTimes(1);
    answers.shift()!(settled);
    await turn();
    // One more read, started after the last record: it sees what all of them were about.
    expect(read).toHaveBeenCalledTimes(2);
    answers.shift()!(settled);
    await turn();
    expect(read).toHaveBeenCalledTimes(2);
    expect(workspaceTrust.value).toEqual(settled);

    // Your own choice asks for nothing; a later record of the core reads again.
    listeners[0](record(6, 'human'));
    expect(read).toHaveBeenCalledTimes(2);
    listeners[0](record(7));
    expect(read).toHaveBeenCalledTimes(3);
    answers.shift()!(settled);
    await turn();
  });
});
