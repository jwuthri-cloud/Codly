// The browser demo's Git tab and "All changes" of an agent: the files are what the demo agent
// wrote in its own conversation, the diff has exactly the files the Git tab lists, and an agent
// that changed nothing lists no file and has an empty diff (and its pull request still drafts).
import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import { mockAgentDiff, mockAgentGit, ownFiles } from '../demo/agentgit';
import { PullRequestDemo } from '../demo/pullrequest';
import type { PrDraft } from '../gen/PrDraft';
import { parseDiff } from '../util/diff';

const eli = { id: 's1', name: 'Eli', role: 'solo', worktree: '/demo/.codly/solo/eli' } as Agent;
let id = 0;
const row = (agent: string, task: string | null, kind: EventKind): EventRow => ({ id: ++id, ts: id, agent_id: agent, task_id: task, run_id: null, kind });
const wrote = (agent: string, task: string | null, ...paths: string[]) => row(agent, task, { type: 'files_changed', call_id: null, paths, source: 'tool_input' });

describe('the demo agent’s worktree', () => {
  it('an agent that changed nothing lists no file and has an empty diff', () => {
    const log = [wrote('w1', 't1', 'src/a.ts')];
    expect(mockAgentGit(eli, 0, log).files).toEqual([]);
    expect(mockAgentDiff(eli, log)).toBe('');
  });

  it('lists what it wrote in its own conversation once each, not a task’s work nor a file outside its worktree', () => {
    const log = [
      wrote('s1', null, 'README.md'),
      wrote('s1', null, `${eli.worktree}/src/theme.ts`, 'README.md'),
      wrote('s1', 't1', 'src/task.ts'),
      wrote('s1', null, '../acme-docs/CHANGELOG.md', '/etc/hosts'),
      wrote('w1', null, 'src/other.ts'),
    ];
    expect(ownFiles(eli, log)).toEqual(['README.md', 'src/theme.ts']);
    expect(mockAgentGit(eli, 0, log).files).toEqual([
      { code: ' M', path: 'README.md' },
      { code: ' M', path: 'src/theme.ts' },
    ]);
  });

  it('the diff has exactly the files the Git tab lists', () => {
    const log = [wrote('s1', null, 'README.md', 'src/theme.ts', 'scripts/build.sh')];
    const files = parseDiff(mockAgentDiff(eli, log));
    expect(files.map((f) => f.path)).toEqual(mockAgentGit(eli, 0, log).files.map((f) => f.path));
    expect(files.every((f) => f.added === 2 && f.removed === 0)).toBe(true);
  });

  it('a worker on a task lists what it wrote for that task since Codly last committed it, not its other work', () => {
    const fay = { id: 'w1', name: 'Fay', role: 'worker', task_id: 't2', worktree: '/demo/.codly/worktrees/t2-fay' } as Agent;
    const log = [
      wrote('w1', 't1', 'src/old.ts'),
      wrote('w1', null, 'notes.md'),
      wrote('w1', 't2', 'src/first.ts'),
      row('w1', 't2', { type: 'committed', sha: 'abc', files: ['src/first.ts'], message: 't2: first' }),
      wrote('w1', 't2', 'src/second.ts', 'src/third.ts'),
      wrote('w2', 't2', 'src/someone-else.ts'),
    ];
    expect(mockAgentGit(fay, 0, log).files.map((f) => f.path)).toEqual(['src/second.ts', 'src/third.ts']);
    // The files the task knows show their text; the diff has exactly the files the Git tab lists.
    const known = [{ path: 'src/second.ts', before: 'a\n', after: 'a\nb\n' }];
    const diff = mockAgentDiff(fay, log, { key: 't2', files: known });
    const files = parseDiff(diff);
    expect(files.map((f) => [f.path, f.added, f.removed])).toEqual([['src/second.ts', 1, 0], ['src/third.ts', 2, 0]]);
    // A file the task does not know is a change for the task, not one of a 1:1 conversation.
    expect(diff).toContain('+// Fay: a change for task t2.');
    expect(diff).not.toContain('1:1 conversation');
    // Off its task, it is back to its own changes outside any task.
    expect(ownFiles({ ...fay, task_id: null }, log)).toEqual(['notes.md']);
  });

  it('a 1:1 agent’s edit shows the lines its call wrote, the ones its card showed', () => {
    const edit = (call: string, input: unknown) => row('s1', null, { type: 'tool_started', call_id: call, name: 'Edit', activity: 'edit', target: { path: 'README.md', paths: [], command: null, query: null, url: null, server: null, tool: null }, input: input as never, confidence: 'observed' });
    const log = [
      edit('c1', { file_path: 'README.md', old_string: '- npm test', new_string: '- npm test\n\n## Notes\n\n- Theming' }),
      row('s1', null, { type: 'files_changed', call_id: 'c1', paths: ['README.md'], source: 'tool_input' }),
    ];
    const [file] = parseDiff(mockAgentDiff(eli, log));
    expect(file.hunks[0].lines.map((l) => [l.kind, l.text])).toEqual([['ctx', '- npm test'], ['add', ''], ['add', '## Notes'], ['add', ''], ['add', '- Theming']]);
  });

  it('an agent with no worktree has neither', () => {
    const none = { ...eli, worktree: null } as Agent;
    expect(mockAgentGit(none, 0, [wrote('s1', null, 'a.ts')]).worktree).toBeNull();
    expect(mockAgentDiff(none, [wrote('s1', null, 'a.ts')])).toBe('');
  });

  it('the pull request of an agent with no commit of its own is drafted, with no Commits part', () => {
    const world = { plans: new Map(), tasks: new Map(), agents: new Map([[eli.id, eli]]), chats: [], now: () => 1000 };
    const draft = new PullRequestDemo().handle('draft_pull_request', { source: { kind: 'agent', id: eli.id } }, world) as PrDraft;
    expect(draft).toMatchObject({ branch: 'codly/eli', base: 'main', title: "Eli's work" });
    expect(draft.body).toBe('## Checks\n\nNo checks ran in Codly: Eli worked alone, outside a plan.\n');
  });
});
