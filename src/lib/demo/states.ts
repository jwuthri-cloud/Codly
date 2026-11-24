// The states a URL parameter starts the browser demo in (docs/DEMO.md lists them with example
// links). The scripted story does not reach these screens by itself: a merge that conflicts, a
// preview command that runs, required checks that are on… Each parameter is read once, when the
// mock backend starts; a value this file does not know starts nothing.
import type { IntakeKind } from '../gen/IntakeKind';
import { LONG_DEFAULT } from './longThread';

/** The kinds of request `?approval=…` makes a 1:1 agent ask (story.ts `seedAsk`). */
const ASK_KINDS = ['file', 'plan', 'network', 'permissions', 'question-long', 'questions-two'] as const;
export type AskKind = (typeof ASK_KINDS)[number];

export type DemoStates = {
  /** `?trust=ask`: you were never asked about the repo's own CLI configuration. */
  trustAsk: boolean;
  /** `?merge=conflict`: your branch changed a file that each finished plan changed too. */
  mergeConflict: boolean;
  /** `?merge=checks`: your branch moved, and a check of each finished plan fails on the merge result. */
  mergeChecks: boolean;
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
  /** `?approval=file` (or `plan`, `network`, `permissions`, `question-long`, `questions-two`): a 1:1 agent asks that kind of request. */
  approval: AskKind | null;
  /** `?hour=13`: the hour of the day (from 0, below 24) the demo's clock starts at. */
  hour: number | null;
  /** `?resume=ask`: work was under way when Codly last closed, and waits for your Resume. */
  resumeAsk: boolean;
  /** `?background=on`: a 1:1 Claude agent left a command and a helper at work in the background. */
  background: boolean;
  /** `?turn=failed`: the first turn of the first 1:1 agent you talk to fails. */
  turnFailed: boolean;
  /** `?pr=open`: a finished plan has an open pull request whose checks are not read yet. */
  prOpen: boolean;
  /** `?search=fail`: every code search fails. */
  searchFails: boolean;
  /** `?requests=two`: both scripted requests are there when the page loads. */
  twoRequests: boolean;
  /** `?cards=3`: the request is a plan of four tasks, and three of its workers ask you within seconds of each other. */
  threeCards: boolean;
  /** `?long=2000`: a 1:1 agent with that many turns behind it streams one long answer (demo/longThread.ts); no number: 2000. */
  longThread: number | null;
};

export function demoStates(params: URLSearchParams): DemoStates {
  const intake = params.get('intake');
  // `Number('')` is 0: a parameter with no value is not midnight.
  const hour = params.get('hour') ? Number(params.get('hour')) : NaN;
  const long = params.has('long') ? Math.floor(Number(params.get('long'))) : NaN;
  return {
    trustAsk: params.get('trust') === 'ask',
    mergeConflict: params.get('merge') === 'conflict',
    mergeChecks: params.get('merge') === 'checks',
    previewRunning: params.get('preview') === 'running',
    checksOn: params.get('checks') === 'on',
    slowHelper: params.get('helper') === 'slow',
    planMode: params.get('mode') === 'plan',
    intake: intake === 'issue' ? 'issue' : intake === 'pr' ? 'pull_request' : null,
    approval: ASK_KINDS.find((k) => k === params.get('approval')) ?? null,
    hour: hour >= 0 && hour < 24 ? hour : null,
    resumeAsk: params.get('resume') === 'ask',
    background: params.get('background') === 'on',
    turnFailed: params.get('turn') === 'failed',
    prOpen: params.get('pr') === 'open',
    searchFails: params.get('search') === 'fail',
    twoRequests: params.get('requests') === 'two',
    threeCards: params.get('cards') === '3',
    longThread: params.has('long') ? (long > 0 ? long : LONG_DEFAULT) : null,
  };
}

/**
 * Does a state need the demo repo open at once (as `?ws=1` does)? `?trust=ask` does not: it is
 * asked when you open the repo. `?hour=` does not either: it only moves the clock.
 */
export function opensRepo(s: DemoStates): boolean {
  return (
    s.mergeConflict || s.mergeChecks || s.previewRunning || s.checksOn || s.slowHelper || s.planMode || s.intake !== null || s.approval !== null || s.resumeAsk || s.background ||
    s.turnFailed || s.prOpen || s.searchFails || s.twoRequests || s.threeCards || s.longThread !== null
  );
}
