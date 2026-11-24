// Which button of a finished plan's card is the main one while a pull request for it is open. Pure.
import type { PrState } from '../lib/gen/PrState';
import type { PullRequest } from '../lib/gen/PullRequest';

/**
 * Look at the pull request before you merge here: it is open and its checks are not read yet,
 * still run, or failed. "Merge into <branch>" is the main button only with no open pull request,
 * or when its checks are read and none runs or failed.
 */
export function pullRequestFirst(pr: Pick<PullRequest, 'state' | 'checks'> | null): boolean {
  if (!pr || pr.state !== 'open') return false;
  return !pr.checks || pr.checks.pending > 0 || pr.checks.failed > 0;
}

/**
 * What became of the pull request after you merged its plan into `branch` on this Mac, as far as
 * Codly last read it: a local merge pushes nothing, so GitHub may still show it open.
 */
export function afterLocalMerge(state: PrState, branch: string): string {
  switch (state) {
    case 'open': return `You merged the plan into ${branch} here. As last read, the pull request is still open on GitHub.`;
    case 'merged': return `You merged the plan into ${branch} here, and the pull request is merged on GitHub too.`;
    case 'closed': return `You merged the plan into ${branch} here. The pull request is closed on GitHub.`;
  }
}

/** Why the pull request comes first, for the tooltip of its button. */
export function pullRequestTip(pr: Pick<PullRequest, 'checks'>): string {
  if (!pr.checks) return 'Its checks are not read yet: look at them before you merge here';
  if (pr.checks.failed) return 'A check failed: look at it before you merge here';
  return 'Its checks still run: look at them before you merge here';
}
