// What you said with an answer to a request, as the core puts it on record just before the
// answer (`said_with` in crates/codly-core/src/orchestrator/workspace.rs): the note of a "no",
// your answers to a question, and that a plan went ahead with your text. Null when the answer
// carried no words: then the demo, like the core, writes no `approval_said` row.
// A CLI other than Claude Code takes no note with its "no": the core sends the note of a deny
// to the agent as a message of its own (`answer_approval`, lead.rs `Said::DenyNote`).
import type { Agent } from '../gen/Agent';
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
  // Claude Code takes the note of a "no" with the answer. Another CLI gets the note of a deny
  // as a message (`denyNoteMessage`); the note of a "stop the turn" never reaches it.
  const text = message?.trim();
  const said = provider === 'claude' ? !allow : choice === 'deny';
  const note = said && text ? truncate(text, 2000) : null;
  const line = ap.subject.kind === 'question' && allow ? answerText(answers) : null;
  const answer = line ? truncate(line, 500) : null;
  // Only Claude Code's plan request takes your text as the plan.
  const plan = shownPlan(ap.subject, provider);
  const yours = answers && typeof answers === 'object' ? (answers as { plan?: unknown }).plan : null;
  const plan_changed = allow && plan !== null && typeof yours === 'string' && yours !== plan;
  return note || answer || plan_changed ? { note, answer, plan_changed } : null;
}

/**
 * The message the note of a deny becomes for an agent whose CLI takes no note with its answer
 * (every CLI but Claude Code), as the core writes it: in the agent's own thread as yours, or in
 * a Lead's request as Codly's line. Null for Claude Code, for another answer, or for no note.
 */
export function denyNoteMessage(agent: Pick<Agent, 'id' | 'provider' | 'role' | 'thread'>, choice: string, message: string | null): { thread: string; author: string; text: string } | null {
  const note = message?.trim();
  if (agent.provider === 'claude' || choice !== 'deny' || !note) return null;
  if (agent.role === 'lead') return { thread: agent.thread ?? 'lead', author: 'codly', text: `Your note with the deny: ${note}` };
  return { thread: `agent:${agent.id}`, author: 'user', text: `About the request I just denied: ${note}` };
}
