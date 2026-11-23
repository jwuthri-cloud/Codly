// A Claude agent in plan mode asks to leave it with a request of its own ("ExitPlanMode") that
// carries the plan, and with a tool call of the same name. The approval card shows that request
// as the plan itself, and every sentence about the request or the call says "its plan": the
// name says nothing to a user, and a plan you sent back is not a call that failed.
// Only for a Claude agent: for every other CLI the name of a tool request is the title of the
// call, so a request with this name is shown as what it is, by its name and its input. An agent
// Codly does not know counts as not Claude.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import type { Provider } from '../gen/Provider';
import { permissionsAsked } from './permissionsAsked';
import { sandboxWants } from './sandbox';
import { basename, oneLine } from './sanitize';

/** The CLI of the agent that asks or calls; none when Codly does not know the agent. */
type Cli = Provider | null | undefined;

/** The tool name of that request (crates/codly-core/src/adapters/claude.rs, `EXIT_PLAN_MODE`). */
const EXIT_PLAN_MODE = 'ExitPlanMode';

/** Whether a tool call of this name, made by an agent of this CLI, is the one that shows you a plan. */
export const isPlanCall = (name: string, provider: Cli): boolean => provider === 'claude' && name === EXIT_PLAN_MODE;

/**
 * The plan a Claude agent shows you before it changes anything, when `s` is that request.
 * null: `s` is another request, it carries no plan text, or `provider` (the CLI of the agent
 * that asks) is not Claude.
 */
export function shownPlan(s: ApprovalSubject, provider: Cli): string | null {
  if (s.kind !== 'tool' || !isPlanCall(s.name, provider)) return null;
  const plan = (s.input as { plan?: unknown } | null)?.plan;
  return typeof plan === 'string' ? plan : null;
}

/** Whether `s` is the request that shows you a plan: the one its card shows as the plan. */
export const showsPlan = (s: ApprovalSubject, provider: Cli): boolean => shownPlan(s, provider) !== null;

/** What an agent that shows you its plan waits for, after "Waiting for you:". */
export const PLAN_ASK = 'read its plan';

/** A call of the plan tool, as one step of the agent's work: while it waits for you, and after. */
export const planStep = (open: boolean): string => (open ? 'Showing you its plan' : 'Showed you its plan');

/**
 * What a request asks for, after "<agent> wants to": "run pnpm test", "change theme.ts",
 * "reach example.com", "show you its plan", "use WebFetch". `max` cuts a command or a host to
 * that many characters, for a place with one short line.
 */
export function wantsTo(s: ApprovalSubject, provider: Cli, max?: number): string {
  switch (s.kind) {
    case 'command':
      return `run ${oneLine(s.command, max ?? 200)}`;
    case 'file_change':
      return s.paths.length === 1 ? `change ${basename(s.paths[0])}` : `change ${s.paths.length} files`;
    case 'tool':
      return showsPlan(s, provider) ? 'show you its plan' : (sandboxWants(s, provider, max) ?? `use ${oneLine(s.name, 60)}`);
    case 'question':
      return 'ask you a question';
    case 'permissions':
      return permissionsAsked(s.detail).wants;
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
