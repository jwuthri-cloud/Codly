// The browser demo's answer to the pull-request commands. There is no git and no gh behind it:
// a draft is written from the story's plan (or the agent's demo worktree), Create "opens" a pull
// request on acme/web, and Refresh shows its checks running, then done. A plan whose request
// started from an issue ends its description with "Closes #N" (the core's `intake::link_line`).
// With `?pr=open` each finished plan has an open pull request already, its checks not read yet.
import type { Agent } from '../gen/Agent';
import type { ChatMessage } from '../gen/ChatMessage';
import type { IntakeSource } from '../gen/IntakeSource';
import type { Plan } from '../gen/Plan';
import type { PrConfirm } from '../gen/PrConfirm';
import type { PrDraft } from '../gen/PrDraft';
import type { PrPushed } from '../gen/PrPushed';
import type { PrSource } from '../gen/PrSource';
import type { PrState } from '../gen/PrState';
import type { PullRequest } from '../gen/PullRequest';
import type { Task } from '../gen/Task';
import { mockAgentGit } from './agentgit';

/** What the demo knows: the story's plans, tasks, agents and messages, and its clock. */
export type DemoWorld = { plans: Map<string, Plan>; tasks: Map<string, Task>; agents: Map<string, Agent>; chats: ChatMessage[]; now(): number };

const REPO_URL = 'https://github.com/acme/web';
/** Where the demo's pull requests are opened. */
const PR_REPO = { host: 'github.com', owner: 'acme', repo: 'web' };

function intakeOf(message: ChatMessage): IntakeSource | null {
  const a = message.attachment;
  return a && typeof a === 'object' && !Array.isArray(a) && a.intake ? (a.intake as unknown as IntakeSource) : null;
}

/**
 * The issue or pull request a plan's request started from (the core's `plan_intake`): the newest
 * "From issue…" / "From pull request…" message of the plan's thread that came before the plan.
 * None once an earlier plan made after that message is done: its work was delivered.
 */
function planIntake(plan: Plan, world: DemoWorld): IntakeSource | null {
  const asked = world.chats.filter((c) => c.thread === plan.thread && c.author === 'user' && c.ts <= plan.created_at && intakeOf(c)).pop();
  if (!asked) return null;
  const delivered = [...world.plans.values()].some((p) => p.id !== plan.id && p.thread === plan.thread && p.status === 'done' && p.created_at >= asked.ts && p.created_at < plan.created_at);
  return delivered ? null : intakeOf(asked);
}

/** The last line of the description: GitHub's closing keyword for an issue of the same host, else a plain link. */
function linkLine(source: IntakeSource): string {
  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  const sameHost = same(source.host, PR_REPO.host);
  const reference = !sameHost ? source.url : same(source.owner, PR_REPO.owner) && same(source.repo, PR_REPO.repo) ? `#${source.number}` : `${source.owner}/${source.repo}#${source.number}`;
  return source.kind === 'issue' && sameHost ? `Closes ${reference}` : `Related to ${reference}`;
}

/** `pr_state`: where the pull request "opened" for this source stands (null: there is none). */
function draft(source: PrSource, world: DemoWorld, pr_state: PrState | null): PrDraft {
  if (source.kind === 'plan') {
    const plan = world.plans.get(source.id);
    if (!plan?.integration_branch) throw new Error('the plan never ran, so it has no branch to push');
    const tasks = [...world.tasks.values()].filter((t) => t.plan_id === plan.id);
    const lines = tasks.map((t) => `- **${t.key}** ${t.title} (${t.status.replace(/_/g, ' ')})`);
    // In the story every finished task passed its checks.
    const checks = tasks.filter((t) => t.status === 'done').flatMap((t) => t.verify.map((cmd) => `- ${t.key}: \`${cmd}\` passed`));
    const started = planIntake(plan, world);
    const body = `## Goal\n\n${plan.request || plan.summary}\n\n## Tasks\n\n${lines.join('\n')}\n\n## Checks\n\n${checks.length ? checks.join('\n') : 'No checks ran for this plan.'}\n${started ? `\n${linkLine(started)}\n` : ''}`;
    return { remote: 'origin', branch: plan.integration_branch, base: plan.base_branch ?? 'main', title: plan.summary, body, gh_problem: null, warning: null, pr_state };
  }
  const agent = world.agents.get(source.id);
  const git = mockAgentGit(agent, world.now());
  if (!agent || !git.branch) throw new Error('this agent has no worktree yet, so there is nothing to push');
  // As the core writes it: a branch with no commit of the agent's yet has no Commits part.
  const commits = git.log.filter((c) => c.author === agent.name);
  const listed = commits.length ? `## Commits\n\n${commits.map((c) => `- ${c.subject} (${c.sha})`).join('\n')}\n\n` : '';
  const body = `${listed}## Checks\n\nNo checks ran in Codly: ${agent.name} worked alone, outside a plan.\n`;
  return { remote: 'origin', branch: git.branch, base: 'main', title: commits[0]?.subject ?? `${agent.name}'s work`, body, gh_problem: null, warning: null, pr_state };
}

export class PullRequestDemo {
  /** The pull requests "opened" in this session, by `kind:id` of their source. */
  private opened = new Map<string, PullRequest>();
  private number = 213;
  /** `finished`: `?pr=open`, a finished plan has its pull request open from the start. */
  constructor(private finished = false) {}

  /** The pull request opened for this source, if any. With `?pr=open` a finished plan has one, opened when it finished. */
  private of(source: PrSource, world: DemoWorld): PullRequest | undefined {
    const key = `${source.kind}:${source.id}`;
    const plan = source.kind === 'plan' ? world.plans.get(source.id) : undefined;
    if (this.finished && !this.opened.has(key) && plan?.status === 'done' && plan.integration_branch) {
      const pr: PullRequest = { url: `${REPO_URL}/pull/${this.number++}`, remote: 'origin', branch: plan.integration_branch, base: plan.base_branch ?? 'main', created_at: plan.updated_at, state: 'open', checks: null };
      this.opened.set(key, pr);
    }
    return this.opened.get(key);
  }

  handle(cmd: string, a: Record<string, unknown>, world: DemoWorld): PullRequest | PrDraft | PrPushed | null {
    const source = a.source as PrSource;
    const key = `${source.kind}:${source.id}`;
    const confirm = a.confirm as PrConfirm;
    switch (cmd) {
      case 'pull_request':
        return this.of(source, world) ?? null;
      case 'draft_pull_request':
        return draft(source, world, this.of(source, world)?.state ?? null);
      case 'create_pull_request': {
        // As gh does: a branch that has its pull request gets that one back.
        const pr: PullRequest = this.of(source, world) ?? { url: `${REPO_URL}/pull/${this.number++}`, ...confirm, created_at: world.now(), state: 'open', checks: null };
        this.opened.set(key, pr);
        return pr;
      }
      case 'push_pr_branch':
        return { remote: confirm.remote, branch: confirm.branch, compare_url: `${REPO_URL}/compare/${confirm.base}...${confirm.branch}?expand=1` };
      case 'refresh_pull_request': {
        const pr = this.of(source, world);
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
