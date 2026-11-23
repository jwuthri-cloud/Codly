// A Claude agent in the mode "plan first" shows its plan with a request of its own, and with a
// tool call of the same name ("ExitPlanMode"). The name says nothing to a user: what shows that
// request or that call says "its plan", and a plan you sent back is not a call that failed.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import { sandboxWants } from './sandbox';
import { basename, oneLine } from './sanitize';

/** The tool name of that request (crates/codly-core/src/adapters/claude.rs, `EXIT_PLAN_MODE`). */
export const PLAN_TOOL = 'ExitPlanMode';

/** A tool call of that name, as one step of the agent's work: while it waits for you, and after. */
export const planStep = (open: boolean): string => (open ? 'Showing you its plan' : 'Showed you its plan');

/** Whether `s` is the request that shows you a plan. */
export const showsPlan = (s: ApprovalSubject): boolean => s.kind === 'tool' && s.name === PLAN_TOOL;

/**
 * What a request asks for, after "<agent> wants to": "run pnpm test", "change theme.ts",
 * "reach example.com", "show you its plan", "use WebFetch". `max` cuts a command or a host to
 * that many characters, for a place with one short line.
 */
export function wantsTo(s: ApprovalSubject, max?: number): string {
  switch (s.kind) {
    case 'command':
      return `run ${oneLine(s.command, max ?? 200)}`;
    case 'file_change':
      return s.paths.length === 1 ? `change ${basename(s.paths[0])}` : `change ${s.paths.length} files`;
    case 'tool':
      return showsPlan(s) ? 'show you its plan' : (sandboxWants(s, max) ?? `use ${oneLine(s.name, 40)}`);
    case 'question':
      return 'ask you a question';
    case 'permissions':
      return 'get more permissions';
  }
}

/**
 * How a call of the plan tool ended, for a line of an agent's history: "You accepted the plan",
 * "You sent the plan back". `answer` is how you answered the request of that call, when it is
 * still known: a plan that was not accepted for another reason (its turn was stopped) says only that.
 */
export function planCallEnd(ok: boolean, answer: 'approved' | 'denied' | 'cancelled' | 'expired' | 'pending' | null): string {
  if (ok) return 'You accepted the plan';
  return answer === null || answer === 'denied' ? 'You sent the plan back' : 'The plan was not accepted';
}
