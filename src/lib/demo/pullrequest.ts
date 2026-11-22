// The browser demo's answer to the pull-request commands. There is no git and no gh behind it:
// a draft is written from the story's plan (or the agent's demo worktree), Create "opens" a pull
// request on acme/web, and Refresh shows its checks running, then done.
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { PrConfirm } from '../gen/PrConfirm';
import type { PrDraft } from '../gen/PrDraft';
import type { PrPushed } from '../gen/PrPushed';
import type { PrSource } from '../gen/PrSource';
import type { PrState } from '../gen/PrState';
import type { PullRequest } from '../gen/PullRequest';
import type { Task } from '../gen/Task';
import { mockAgentGit } from './agentgit';

/** What the demo knows: the story's plans, tasks and agents, and its clock. */
export type DemoWorld = { plans: Map<string, Plan>; tasks: Map<string, Task>; agents: Map<string, Agent>; now(): number };

const REPO_URL = 'https://github.com/acme/web';

/** `pr_state`: where the pull request "opened" for this source stands (null: there is none). */
function draft(source: PrSource, world: DemoWorld, pr_state: PrState | null): PrDraft {
  if (source.kind === 'plan') {
    const plan = world.plans.get(source.id);
    if (!plan?.integration_branch) throw new Error('the plan never ran, so it has no branch to push');
    const tasks = [...world.tasks.values()].filter((t) => t.plan_id === plan.id);
    const lines = tasks.map((t) => `- **${t.key}** ${t.title} (${t.status.replace(/_/g, ' ')})`);
    // In the story every finished task passed its checks.
    const checks = tasks.filter((t) => t.status === 'done').flatMap((t) => t.verify.map((cmd) => `- ${t.key}: \`${cmd}\` passed`));
    const body = `## Goal\n\n${plan.request || plan.summary}\n\n## Tasks\n\n${lines.join('\n')}\n\n## Checks\n\n${checks.length ? checks.join('\n') : 'No checks ran for this plan.'}\n`;
    return { remote: 'origin', branch: plan.integration_branch, base: plan.base_branch ?? 'main', title: plan.summary, body, gh_problem: null, warning: null, pr_state };
  }
  const agent = world.agents.get(source.id);
  const git = mockAgentGit(agent, world.now());
  if (!agent || !git.branch) throw new Error('this agent has no worktree yet, so there is nothing to push');
  const commits = git.log.filter((c) => c.author === agent.name);
  const body = `## Commits\n\n${commits.map((c) => `- ${c.subject} (${c.sha})`).join('\n')}\n\n## Checks\n\nNo checks ran in Codly: ${agent.name} worked alone, outside a plan.\n`;
  return { remote: 'origin', branch: git.branch, base: 'main', title: commits[0].subject, body, gh_problem: null, warning: null, pr_state };
}

export class PullRequestDemo {
  /** The pull requests "opened" in this session, by `kind:id` of their source. */
  private opened = new Map<string, PullRequest>();
  private number = 213;

  handle(cmd: string, a: Record<string, unknown>, world: DemoWorld): PullRequest | PrDraft | PrPushed | null {
    const source = a.source as PrSource;
    const key = `${source.kind}:${source.id}`;
    const confirm = a.confirm as PrConfirm;
    switch (cmd) {
      case 'pull_request':
        return this.opened.get(key) ?? null;
      case 'draft_pull_request':
        return draft(source, world, this.opened.get(key)?.state ?? null);
      case 'create_pull_request': {
        // As gh does: a branch that has its pull request gets that one back.
        const pr: PullRequest = this.opened.get(key) ?? { url: `${REPO_URL}/pull/${this.number++}`, ...confirm, created_at: world.now(), state: 'open', checks: null };
        this.opened.set(key, pr);
        return pr;
      }
      case 'push_pr_branch':
        return { remote: confirm.remote, branch: confirm.branch, compare_url: `${REPO_URL}/compare/${confirm.base}...${confirm.branch}?expand=1` };
      case 'refresh_pull_request': {
        const pr = this.opened.get(key);
        if (!pr) throw new Error('Codly has not opened a pull request from here yet');
        // The first read finds the checks running, the next one finds them done.
        const running = pr.checks === null;
        const read = { ...pr, checks: { passed: running ? 1 : 3, failed: 0, pending: running ? 2 : 0, skipped: 0, failing: [], read_at: world.now() } };
        this.opened.set(key, read);
        return read;
      }
      default:
        throw new Error(`Mock backend: unknown command “${cmd}”`);
    }
  }
}
