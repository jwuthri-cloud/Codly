// What you said with an answer to a request, as the core puts it on record just before the
// answer (`said_with` in crates/codly-core/src/orchestrator/workspace.rs): the note of a "no",
// your answers to a question, and that a plan went ahead with your text. Null when the answer
// carried no words: then the demo, like the core, writes no `approval_said` row.
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { Provider } from '../gen/Provider';
import { answerText } from '../util/outcome';
import { shownPlan } from '../util/planRequest';
import { truncate } from '../util/sanitize';

type Said = Omit<Extract<EventKind, { type: 'approval_said' }>, 'type' | 'approval_id'>;

/** `provider`: the CLI of the agent that asked. `choice`: what you answered. */
export function saidWith(ap: Approval, provider: Provider | undefined, choice: string, message: string | null, answers: unknown): Said | null {
  const allow = choice !== 'deny' && choice !== 'cancel';
  // Only Claude Code hands the note of a "no" to the agent: for another CLI it was never said.
  const text = message?.trim();
  const note = provider === 'claude' && !allow && text ? truncate(text, 2000) : null;
  const line = ap.subject.kind === 'question' && allow ? answerText(answers) : null;
  const answer = line ? truncate(line, 500) : null;
  // Only Claude Code's plan request takes your text as the plan.
  const plan = shownPlan(ap.subject, provider);
  const yours = answers && typeof answers === 'object' ? (answers as { plan?: unknown }).plan : null;
  const plan_changed = allow && plan !== null && typeof yours === 'string' && yours !== plan;
  return note || answer || plan_changed ? { note, answer, plan_changed } : null;
}
