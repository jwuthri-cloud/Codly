// The browser demo's answer to `agent_git` and `agent_diff`: an agent's worktree as the demo
// agent left it. Its changed files are the ones it wrote there (from the event log): a 1:1
// agent's are its file changes outside any task, and a worker's, while it is on a task, are its
// changes for that task since Codly last committed them (the worktree it works in is the
// task's). So an agent that changed nothing lists no file and has an empty diff, and the diff has
// exactly the files the Git tab lists. Commits and branches are derived from its name, so each
// agent shows different (but stable) ones.
import type { Agent } from '../gen/Agent';
import type { AgentGit } from '../gen/AgentGit';
import type { EventRow } from '../gen/EventRow';
import { unifiedDiff, type RepoFile } from './repo';

const MIN = 60_000;

/** A stable 7-letter hex name for the `n`th object of the agent's worktree. */
const sha = (agent: Agent, n: number) => Array.from({ length: 7 }, (_, i) => '0123456789abcdef'[(agent.name.charCodeAt(i % agent.name.length) * (n + 3) + i * 7) % 16]).join('');

/** The rows of the log that count for the agent's worktree now: those after Codly last committed the task it is on. */
function sinceCommit(agent: Agent, log: readonly EventRow[]): readonly EventRow[] {
  if (!agent.task_id) return log;
  for (let i = log.length - 1; i >= 0; i--) if (log[i].task_id === agent.task_id && log[i].kind.type === 'committed') return log.slice(i + 1);
  return log;
}

/** The files the agent changed in its worktree, in the order it first wrote them, relative to
 *  the worktree: a 1:1 agent's changes outside any task, a worker's for the task it is on (not
 *  committed yet). A path outside it (another repository it was allowed to write) is not in
 *  its git status. */
export function ownFiles(agent: Agent, log: readonly EventRow[]): string[] {
  const root = agent.worktree?.replace(/\/+$/, '');
  if (!root) return [];
  const task = agent.task_id ?? null;
  const out = new Set<string>();
  for (const row of sinceCommit(agent, log)) {
    if (row.agent_id !== agent.id || row.task_id !== task || row.kind.type !== 'files_changed') continue;
    for (const p of row.kind.paths) {
      const rel = p.startsWith(`${root}/`) ? p.slice(root.length + 1) : p;
      if (!rel.startsWith('/') && !rel.split('/').includes('..')) out.add(rel);
    }
  }
  return [...out];
}

/** `log`: the demo's event log, where its changed files come from; a caller that needs no file
 *  (the demo's merge, and its pull request, which reads the branch and the log) leaves it out.
 *  The agent has no commit of its own: what it wrote is not committed yet. */
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

/**
 * The lines of the agent's last Claude `Edit` (or `Write`) of `path`, from the call's input as
 * Claude Code sends it (`old_string` and `new_string`, or `content`): the lines both strings
 * start with stay, the rest is taken out and put in. Null when no call of it carries the text.
 */
function editLines(agent: Agent, log: readonly EventRow[], path: string): string[] | null {
  const inputs = new Map<string, unknown>();
  let last: unknown = null;
  for (const row of log) {
    if (row.agent_id !== agent.id) continue;
    if (row.kind.type === 'tool_started' && row.kind.call_id) inputs.set(row.kind.call_id, row.kind.input);
    if (row.kind.type === 'files_changed' && row.kind.call_id && row.kind.paths.includes(path)) last = inputs.get(row.kind.call_id) ?? null;
  }
  const input = last && typeof last === 'object' && !Array.isArray(last) ? (last as Record<string, unknown>) : null;
  if (typeof input?.content === 'string') return input.content.replace(/\n$/, '').split('\n').map((l) => `+${l}`);
  if (typeof input?.old_string !== 'string' || typeof input.new_string !== 'string') return null;
  const before = input.old_string.split('\n');
  const after = input.new_string.split('\n');
  let same = 0;
  while (same < before.length && same < after.length && before[same] === after[same]) same++;
  return [...before.slice(0, same).map((l) => ` ${l}`), ...before.slice(same).map((l) => `-${l}`), ...after.slice(same).map((l) => `+${l}`)];
}

/**
 * `agent_diff`: one change per file the Git tab lists (`mockAgentGit`), nothing for an agent
 * that changed nothing. `task`: the task the agent is on, with its files as the task's diff has
 * them; a file it edited with a call that carries the text shows that text (the change you saw
 * on its card); any other file a short note of what it is a change for.
 */
export function mockAgentDiff(agent: Agent | undefined, log: readonly EventRow[], task: { key: string; files: readonly RepoFile[] } | null = null): string {
  if (!agent?.worktree) return '';
  const what = task ? `a change for task ${task.key}` : 'a change from its 1:1 conversation';
  return ownFiles(agent, log)
    .map((path, i) => {
      const file = task?.files.find((f) => f.path === path);
      if (file) return `${unifiedDiff(file)}\n`;
      const edit = editLines(agent, log, path);
      const mark = noteMark(path);
      const lines = edit ?? [`+${mark}${agent.name}: ${what}.`, `+${mark}Not committed yet.`];
      const removed = lines.filter((l) => !l.startsWith('+')).length;
      const added = lines.filter((l) => !l.startsWith('-')).length;
      return [
        `diff --git a/${path} b/${path}`,
        `index ${sha(agent, 2 * i)}..${sha(agent, 2 * i + 1)} 100644`,
        `--- a/${path}`,
        `+++ b/${path}`,
        edit ? `@@ -1,${removed} +1,${added} @@` : '@@ -1,0 +2,2 @@',
        ...lines,
        '',
      ].join('\n');
    })
    .join('');
}
