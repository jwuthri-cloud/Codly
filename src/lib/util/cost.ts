// How spend reads: one agent's reported cost (its sheet, the chip under the composer) and the
// totals of a plan or a day (plan cards, Settings → Autonomy & Budgets).
import type { Agent } from '../gen/Agent';
import type { Spent } from '../gen/Spent';
import { tokens, usd } from './format';

type Paying = Pick<Agent, 'usage' | 'provider' | 'billing'>;

/** An agent's session cost, and who pays it. */
export function costLine(agent: Paying): string {
  const cost = agent.usage.cost_usd;
  if (cost == null) return agent.provider === 'codex' ? 'Tokens only — Codex reports no cost' : 'No cost reported';
  if (agent.billing === 'subscription') return `≈ ${usd(cost)} at list price · included in your plan`;
  if (agent.billing === 'api_key') return `${usd(cost)} · billed to your API key`;
  if (agent.billing === 'third_party') return `${usd(cost)} · billed by the provider`;
  return `≈ ${usd(cost)}`;
}

/** The same cost in a few characters: "$0.42" when it is billed as it reads, "≈ $0.42" when it is an estimate. Null: none reported. */
export function costShort(agent: Paying): string | null {
  const cost = agent.usage.cost_usd;
  if (cost == null) return null;
  return agent.billing === 'api_key' || agent.billing === 'third_party' ? usd(cost) : `≈ ${usd(cost)}`;
}

/** How full an agent's context was in its last turn, 0..1. Null until its CLI reports both numbers. */
export function contextFraction(usage: Agent['usage']): number | null {
  return usage.context_used != null && usage.context_window ? usage.context_used / usage.context_window : null;
}

/** A total: "1.2M tokens · about $3.40". The cost only when a CLI reported one; it is an estimate for a subscription. */
export function spentLine(s: Spent): string {
  return s.cost_usd > 0 ? `${tokens(s.tokens)} tokens · about ${usd(s.cost_usd)}` : `${tokens(s.tokens)} tokens`;
}

/** What a plan's total counts, for its tooltip. */
export const PLAN_SPENT_TIP =
  'What this plan’s tasks used so far, over every worker and reviewer run: work tokens (cache reads don’t count) and the cost the CLIs report. The Lead’s own turns are not in it.';
