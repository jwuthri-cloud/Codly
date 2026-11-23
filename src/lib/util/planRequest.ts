// A Claude agent in plan mode asks to leave it with a request of its own ("ExitPlanMode") that
// carries the plan. The approval card shows that request as the plan itself. Only for a Claude
// agent: for every other CLI the name of a tool request is the title of the call, so a request
// with this name is shown as what it is, by its name and its input. An agent Codly does not
// know counts as not Claude.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import type { Provider } from '../gen/Provider';

/** The tool name of that request. */
const EXIT_PLAN_MODE = 'ExitPlanMode';

/**
 * The plan a Claude agent shows you before it changes anything, when `s` is that request.
 * null: `s` is another request, it carries no plan text, or `provider` (the CLI of the agent
 * that asks) is not Claude.
 */
export function shownPlan(s: ApprovalSubject, provider: Provider | null | undefined): string | null {
  if (provider !== 'claude' || s.kind !== 'tool' || s.name !== EXIT_PLAN_MODE) return null;
  const plan = (s.input as { plan?: unknown } | null)?.plan;
  return typeof plan === 'string' ? plan : null;
}
