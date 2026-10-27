// The browser demo's answer to `agent_git`: a believable worktree for an agent, derived from
// its name and task so each teammate shows different (but stable) files and commits.
import type { Agent } from '../gen/Agent';
import type { AgentGit } from '../gen/AgentGit';

const MIN = 60_000;

export function mockAgentGit(agent: Agent | undefined, now: number): AgentGit {
  if (!agent?.worktree) return { worktree: null, branch: null, files: [], branches: [], log: [], note: 'This agent has no worktree yet.' };
  const slug = agent.worktree.split('/').pop() ?? agent.name.toLowerCase();
  const branch = `codly/${slug}`;
  const h = (n: number) => Array.from({ length: 7 }, (_, i) => '0123456789abcdef'[(agent.name.charCodeAt(i % agent.name.length) * (n + 3) + i * 7) % 16]).join('');
  return {
    worktree: agent.worktree,
    branch,
    files: [
      { code: ' M', path: 'src/theme/ThemeProvider.tsx' },
      { code: 'A ', path: 'src/theme/useTheme.ts' },
      { code: '??', path: 'src/theme/theme.test.tsx' },
    ],
    branches: [
      { name: branch, current: true, sha: h(0), ts: now - 4 * MIN },
      { name: 'main', current: false, sha: h(9), ts: now - 3 * 60 * MIN },
    ],
    log: [
      { sha: h(0), subject: 'Wire the theme toggle into the header', author: agent.name, ts: now - 4 * MIN },
      { sha: h(1), subject: 'Add useTheme hook with system preference', author: agent.name, ts: now - 19 * MIN },
      { sha: h(2), subject: 'Scaffold ThemeProvider', author: agent.name, ts: now - 47 * MIN },
      { sha: h(9), subject: 'Merge pull request #212 from acme/search-filters', author: 'Sam Rivera', ts: now - 3 * 60 * MIN },
    ],
    note: null,
  };
}
