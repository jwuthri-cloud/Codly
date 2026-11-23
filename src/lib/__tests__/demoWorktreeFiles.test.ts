// The browser demo's Git tab and "All changes" of an agent: the files are what the demo agent
// wrote in its own conversation, the diff has exactly the files the Git tab lists, and an agent
// that changed nothing lists no file and has an empty diff.
import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import { mockAgentDiff, mockAgentGit, ownFiles } from '../demo/agentgit';
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

  it('an agent with no worktree has neither', () => {
    const none = { ...eli, worktree: null } as Agent;
    expect(mockAgentGit(none, 0, [wrote('s1', null, 'a.ts')]).worktree).toBeNull();
    expect(mockAgentDiff(none, [wrote('s1', null, 'a.ts')])).toBe('');
  });
});
