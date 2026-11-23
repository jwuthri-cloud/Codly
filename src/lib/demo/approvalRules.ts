// Mock of "Always allow in this repo" for `pnpm dev` and the site demo. In the app the core
// decides what a rule may cover (orchestrator/approvals.rs `rule_offer`) and its broker follows
// the saved rules (approval_rules.rs). The demo follows the same rules, with one difference: it
// has no classifier of its own, so it reads the risk labels the story gives a request.
// - Only a command can be saved, from an agent that may write and has a worktree, outside "Ask
//   first", and never one that changes your own account (`npm login`).
// - Every risk of the request must be one a rule may cover: a package download or a git write
//   (the command, or the words it starts with), and other network use (the exact command only).
//   A delete, a push or a path outside the worktree is never covered.
// - Saving a rule answers the requests that wait and offer the same rule; a later request that
//   offers it is allowed without asking you (story.ts `approval`).
import type { Approval } from '../gen/Approval';
import type { ApprovalRule } from '../gen/ApprovalRule';
import type { ApprovalRuleKind } from '../gen/ApprovalRuleKind';
import type { ApprovalRuleOffer } from '../gen/ApprovalRuleOffer';
import { uid, type Sim } from './sim';
import type { Story } from './story';

/** The core's own risk labels for what a rule may cover. */
export const PACKAGE_INSTALL = 'network (package install)';
export const NPX_DOWNLOAD = 'network (npx)';
export const GIT_WRITE = 'git write (Codly commits and merges for you)';
const BY_PREFIX = [PACKAGE_INSTALL, NPX_DOWNLOAD, GIT_WRITE];
const BY_EXACT = [...BY_PREFIX, 'network'];

/** Programs whose first argument is a subcommand: their prefix is `program subcommand`. */
const SUBCOMMAND_TOOLS = [
  'git', 'npm', 'pnpm', 'yarn', 'bun', 'cargo', 'go', 'uv', 'pip', 'pip3', 'poetry', 'bundle', 'gem', 'composer', 'make', 'just', 'task',
  'gradle', 'mvn', 'dotnet', 'swift', 'deno', 'mix',
];
/** git subcommands a prefix may cover: what they change stays in the agent's worktree and on its branch. */
const GIT_PREFIXES = ['add', 'commit', 'checkout', 'switch', 'restore', 'merge', 'cherry-pick', 'revert', 'mv', 'apply'];
/** Programs that run every target named on the line: a prefix never covers a second target. */
const MANY_TARGETS = ['make', 'just', 'task', 'gradle', 'mvn'];
/** Subcommands that change your own account or settings, or publish under your name: never saved. */
const OWN_ACCOUNT = ['config', 'login', 'logout', 'publish'];

const plainName = (w: string | undefined): w is string => !!w && /^[A-Za-z][A-Za-z0-9_:-]*$/.test(w);

/** Do these words only say that something runs, so the next word names what (`npm run build`)? */
function namesWhatRuns(words: string[]): boolean {
  const [prog, sub, third] = words;
  if (words.length === 2 && ['npm', 'pnpm', 'yarn', 'bun'].includes(prog)) return ['run', 'run-script', 'exec', 'dlx', 'x', 'create', 'init'].includes(sub);
  if (words.length === 2 && prog === 'uv') return ['run', 'pip', 'tool'].includes(sub);
  if (words.length === 2) return ['poetry run', 'bundle exec', 'deno task', 'deno run', 'go run'].includes(`${prog} ${sub}`);
  return words.length === 3 && `${prog} ${sub} ${third}` === 'uv tool run';
}

/**
 * The prefix a command can be saved under (the core's `rule_prefix`): `npm install` for
 * `npm install -D zod`, `npm run build` for `npm run build -- --watch`. Only for one plain
 * command: nothing the shell chains, substitutes, expands or redirects, and no `VAR=x` or path
 * in front of the program.
 */
export function rulePrefix(command: string): string | null {
  if (/[;&|<>`$()\n]/.test(command)) return null;
  const [prog, sub, ...rest] = command.trim().split(/\s+/);
  if (!SUBCOMMAND_TOOLS.includes(prog) || !plainName(sub)) return null;
  // `git checkout -B x` moves a branch that exists, `-f` drops work that is not committed.
  const forced = ['checkout', 'switch'].includes(sub) && rest.some((a) => (a.startsWith('--') ? /^--(force|discard)/.test(a) : a.startsWith('-') && /[BCf]/.test(a)));
  if (prog === 'git' && (!GIT_PREFIXES.includes(sub) || forced)) return null;
  if ((MANY_TARGETS.includes(prog) && rest.some((w) => !w.startsWith('-'))) || (prog === 'mix' && sub === 'do')) return null;
  const prefix = [prog, sub];
  for (let i = 0; namesWhatRuns(prefix); i++) {
    if (!plainName(rest[i])) return null;
    prefix.push(rest[i]);
  }
  return prefix.join(' ');
}

/** Does the line run a subcommand that changes your own account (`cargo login x`, `npm config set …`)? */
function ownAccount(command: string): boolean {
  return command.split(/&&|\|\||[;|&\n]/).some((part) => {
    const [prog, ...args] = part.trim().split(/\s+/);
    const sub = args.find((a) => !a.startsWith('-'));
    return SUBCOMMAND_TOOLS.includes(prog) && !!sub && OWN_ACCOUNT.includes(sub);
  });
}

/** How a rule reads on a card and in the decision log. */
export function describeRule(rule: ApprovalRule): string {
  return rule.kind === 'exact' ? `\`${rule.command}\`` : `commands that start with \`${rule.command}\``;
}

export class MockApprovalRules {
  private rules: ApprovalRule[] = [];

  constructor(private sim: Sim, private story: Story) {}

  /** The permissions of the request the agent works for: its plan's request, or its own 1:1 chat. */
  private mode(ap: Approval) {
    const sim = this.sim;
    const plan = ap.task_id ? sim.tasks.get(ap.task_id)?.plan_id : null;
    const thread = plan ? sim.plans.get(plan)?.thread : `agent:${ap.agent_id}`;
    return (thread ? sim.requestPrefs[thread]?.approval_mode : null) ?? sim.settings.approval_mode;
  }

  /** What a waiting request can be saved as: nothing for what Codly always asks about. */
  private offerOf(ap: Approval | undefined): ApprovalRuleOffer {
    const none = { exact: null, prefix: null };
    if (!ap || ap.status !== 'pending' || ap.subject.kind !== 'command') return none;
    const command = ap.subject.command.trim();
    const agent = this.sim.agents.get(ap.agent_id);
    const mayWrite = !!agent?.worktree && agent.role !== 'lead' && agent.role !== 'reviewer';
    const oneTime = !ap.choices.length || ap.choices.includes('allow_once');
    if (!command || !mayWrite || !oneTime || this.mode(ap) === 'strict') return none;
    if (!ap.risks.every((r) => BY_EXACT.includes(r)) || ownAccount(command)) return none;
    return { exact: command, prefix: ap.risks.every((r) => BY_PREFIX.includes(r)) ? rulePrefix(command) : null };
  }

  offer(approvalId: string): ApprovalRuleOffer {
    return this.offerOf(this.sim.approvals.get(approvalId));
  }

  /** The rule you saved that answers this request in your place (null: Codly asks you). */
  covering(ap: Approval): ApprovalRule | null {
    const offer = this.offerOf(ap);
    return this.rules.find((r) => offer[r.kind] === r.command) ?? null;
  }

  add(approvalId: string, kind: ApprovalRuleKind): ApprovalRule {
    const sim = this.sim;
    const command = this.offer(approvalId)[kind];
    if (!command) throw new Error('Codly always asks about this kind of request, so it cannot be saved as a rule.');
    let rule = this.rules.find((r) => r.kind === kind && r.command === command);
    if (!rule) {
      rule = { id: uid('rule-'), workspace_id: sim.workspace?.id ?? '', kind, command, created_at: sim.now() };
      this.rules = [...this.rules, rule];
    }
    sim.decision('approval_policy', 'human', `you saved a rule: always allow ${describeRule(rule)} in this repo`, { rule: rule.id, approval_id: approvalId });
    // The request on the card, and the same one from other agents: answered together.
    for (const ap of [...sim.approvals.values()]) if (this.offer(ap.id)[kind] === command) this.story.resolveApproval(ap.id, 'allow_once', null);
    return rule;
  }

  list(): ApprovalRule[] {
    return this.rules;
  }

  remove(ruleId: string) {
    const rule = this.rules.find((r) => r.id === ruleId);
    if (!rule) throw new Error('That rule was already removed.');
    this.rules = this.rules.filter((r) => r !== rule);
    this.sim.decision('approval_policy', 'human', `you removed a rule: Codly asks again about ${describeRule(rule)}`, { rule: rule.id });
  }
}
