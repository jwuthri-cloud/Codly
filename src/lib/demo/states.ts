// The states a URL parameter starts the browser demo in (docs/DEMO.md lists them with example
// links). The scripted story does not reach these screens by itself: a merge that conflicts, a
// preview command that runs, required checks that are on… Each parameter is read once, when the
// mock backend starts; a value this file does not know starts nothing.
import type { IntakeKind } from '../gen/IntakeKind';

export type DemoStates = {
  /** `?trust=ask`: you were never asked about the repo's own CLI configuration. */
  trustAsk: boolean;
  /** `?merge=conflict`: your branch changed a file that each finished plan changed too. */
  mergeConflict: boolean;
  /** `?preview=running`: a command you started earlier runs in each worktree you look at. */
  previewRunning: boolean;
  /** `?checks=on`: the repo's required checks are turned on, so each task runs them. */
  checksOn: boolean;
  /** `?helper=slow`: an agent's helper (sub-agent) stays at work for about a minute. */
  slowHelper: boolean;
  /** `?mode=plan`: a 1:1 Claude agent in "plan first" shows you its plan. */
  planMode: boolean;
  /** `?intake=issue` or `?intake=pr`: that dialog is open, and your request is the first. */
  intake: IntakeKind | null;
};

export function demoStates(params: URLSearchParams): DemoStates {
  const intake = params.get('intake');
  return {
    trustAsk: params.get('trust') === 'ask',
    mergeConflict: params.get('merge') === 'conflict',
    previewRunning: params.get('preview') === 'running',
    checksOn: params.get('checks') === 'on',
    slowHelper: params.get('helper') === 'slow',
    planMode: params.get('mode') === 'plan',
    intake: intake === 'issue' ? 'issue' : intake === 'pr' ? 'pull_request' : null,
  };
}

/** Does a state need the demo repo open at once (as `?ws=1` does)? `?trust=ask` does not: it is asked when you open the repo. */
export function opensRepo(s: DemoStates): boolean {
  return s.mergeConflict || s.previewRunning || s.checksOn || s.slowHelper || s.planMode || s.intake !== null;
}
