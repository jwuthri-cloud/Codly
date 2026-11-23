// The browser demo's answer to `agent_git` and `agent_diff`: an agent's worktree as the demo
// agent left it. Its changed files are the ones it wrote in its own conversation (its file
// changes outside any task, from the event log), so an agent that changed nothing lists no file
// and has an empty diff, and the diff has exactly the files the Git tab lists. Commits and
// branches are derived from its name, so each agent shows different (but stable) ones.
import type { Agent } from '../gen/Agent';
import type { AgentGit } from '../gen/AgentGit';
import type { EventRow } from '../gen/EventRow';

const MIN = 60_000;

/** A stable 7-letter hex name for the `n`th object of the agent's worktree. */
const sha = (agent: Agent, n: number) => Array.from({ length: 7 }, (_, i) => '0123456789abcdef'[(agent.name.charCodeAt(i % agent.name.length) * (n + 3) + i * 7) % 16]).join('');

/** The files the agent changed in its own worktree, in the order it first wrote them: its file
 *  changes outside any task (a task's work is in the task's worktree), relative to the worktree.
 *  A path outside it (another repository it was allowed to write) is not in its git status. */
export function ownFiles(agent: Agent, log: readonly EventRow[]): string[] {
  const root = agent.worktree?.replace(/\/+$/, '');
  if (!root) return [];
  const out = new Set<string>();
  for (const row of log) {
    if (row.agent_id !== agent.id || row.task_id || row.kind.type !== 'files_changed') continue;
    for (const p of row.kind.paths) {
      const rel = p.startsWith(`${root}/`) ? p.slice(root.length + 1) : p;
      if (!rel.startsWith('/') && !rel.split('/').includes('..')) out.add(rel);
    }
  }
  return [...out];
}

/** `log`: the demo's event log, where its changed files come from; a caller that reads only the
 *  branch (the demo's merge and pull request) leaves it out. */
export function mockAgentGit(agent: Agent | undefined, now: number, log: readonly EventRow[] = []): AgentGit {
  if (!agent?.worktree) return { worktree: null, branch: null, files: [], branches: [], log: [], note: 'This agent has no worktree yet.' };
  const slug = agent.worktree.split('/').pop() ?? agent.name.toLowerCase();
  const branch = `codly/${slug}`;
  return {
    worktree: agent.worktree,
    branch,
    // Not committed yet: what it wrote is in its worktree only.
    files: ownFiles(agent, log).map((path) => ({ code: ' M', path })),
    branches: [
      { name: branch, current: true, sha: sha(agent, 9), ts: now - 3 * 60 * MIN },
      { name: 'main', current: false, sha: sha(agent, 9), ts: now - 3 * 60 * MIN },
    ],
    log: [{ sha: sha(agent, 9), subject: 'Merge pull request #212 from acme/search-filters', author: 'Sam Rivera', ts: now - 3 * 60 * MIN }],
    note: null,
  };
}

/** How a line of a note starts in a file of this kind. */
const noteMark = (path: string) => (/\.(md|txt)$/i.test(path) ? '' : /\.(py|sh|ya?ml|toml)$/i.test(path) ? '# ' : '// ');

/** `agent_diff`: one small change per file the Git tab lists (`mockAgentGit`), nothing for an agent that changed nothing. */
export function mockAgentDiff(agent: Agent | undefined, log: readonly EventRow[]): string {
  if (!agent?.worktree) return '';
  return ownFiles(agent, log)
    .map((path, i) => {
      const mark = noteMark(path);
      return [
        `diff --git a/${path} b/${path}`,
        `index ${sha(agent, 2 * i)}..${sha(agent, 2 * i + 1)} 100644`,
        `--- a/${path}`,
        `+++ b/${path}`,
        '@@ -1,0 +2,2 @@',
        `+${mark}${agent.name}: a change from its 1:1 conversation.`,
        `+${mark}Not committed yet.`,
        '',
      ].join('\n');
    })
    .join('');
}
