// Which button of a finished plan's card is the main one while a pull request for it is open. Pure.
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

/** Why the pull request comes first, for the tooltip of its button. */
export function pullRequestTip(pr: Pick<PullRequest, 'checks'>): string {
  if (!pr.checks) return 'Its checks are not read yet: look at them before you merge here';
  if (pr.checks.failed) return 'A check failed: look at it before you merge here';
  return 'Its checks still run: look at them before you merge here';
}
