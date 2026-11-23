// How a request reads once it no longer waits: the closed card in the conversation and the
// lines of an agent's history. A permission is approved or denied. A question is answered or
// skipped, and says what you answered. A "no" says the note you sent with it, and a plan says
// when it went ahead with your changes: your words, as the core recorded them (state/answers).
import type { Approval } from '../gen/Approval';
import type { ApprovalStatus } from '../gen/ApprovalStatus';
import type { Provider } from '../gen/Provider';
import { permissionsAsked } from './permissionsAsked';
import { yourWords, type Words } from '../state/answers';
import { showsPlan } from './planRequest';
import { oneLine } from './sanitize';

type Asked = Pick<Approval, 'subject'>;
/** A request as the store holds it: what you said with its answer is on record under its id. */
type Held = Asked & Partial<Pick<Approval, 'id'>>;

/**
 * "Approved", "Denied", "Stopped", "Closed"; for a question "Answered" and "Skipped". Empty while
 * it waits. "Closed": the request went away unanswered with the agent's run (you killed the
 * agent, it exited, Codly restarted). `asked` null: a request that is not known any more.
 */
export function outcomeWord(status: ApprovalStatus, asked: Asked | null): string {
  const question = asked?.subject.kind === 'question';
  switch (status) {
    case 'approved': return question ? 'Answered' : 'Approved';
    case 'denied': return question ? 'Skipped' : 'Denied';
    case 'cancelled': return 'Stopped';
    case 'expired': return 'Closed';
    case 'pending': return '';
  }
}

/**
 * Who settled it: "by you", "by Codly’s rules", "by Codly" (it closed the request when the
 * agent's run ended), "by its CLI" (the agent's CLI took the request back), "by <who>". Empty
 * when nobody is on record.
 */
export function outcomeBy(decidedBy: string | null): string {
  switch (decidedBy) {
    case 'user': return 'by you';
    case 'policy': return 'by Codly’s rules';
    case 'codly': return 'by Codly';
    case 'provider': return 'by its CLI';
    default: return decidedBy ? `by ${decidedBy}` : '';
  }
}

/** The answers a question card sends, on one line: "Keep it"; "Red; Small, Fast" for several
 *  questions (a question with several picks lists them). Null when nothing was picked or typed.
 *  The core writes the same line on record (`answer_line` in orchestrator/workspace.rs). */
export function answerText(answers: unknown): string | null {
  if (!answers || typeof answers !== 'object' || Array.isArray(answers)) return null;
  const said = Object.values(answers)
    .map((v) => (Array.isArray(v) ? v.filter((x) => typeof x === 'string' && x).join(', ') : typeof v === 'string' ? v.trim() : ''))
    .filter(Boolean);
  return said.length ? said.join('; ') : null;
}

/** The first question of a question card, as the agent wrote it; null when the card has none to show.
 *  A question that comes in its request's envelope (an MCP server's elicitation through Codex:
 *  `{ serverName, mode, message, … }`) is its `message`. */
export function firstQuestion(asked: Asked): string | null {
  if (asked.subject.kind !== 'question') return null;
  const raw = asked.subject.questions as unknown;
  const list = Array.isArray(raw) ? raw : raw && typeof raw === 'object' ? (raw as { questions?: unknown }).questions : null;
  if (!Array.isArray(list)) {
    const message = raw && typeof raw === 'object' ? (raw as { message?: unknown }).message : null;
    return typeof message === 'string' && message.trim() ? message : null;
  }
  const first = Array.isArray(list) ? (list[0] as { question?: unknown; prompt?: unknown } | null) : null;
  const text = first && typeof first === 'object' ? (first.question ?? first.prompt) : null;
  return typeof text === 'string' && text ? text : null;
}

/** An agent's history: what it asked you. */
export function askedLine(asked: Asked, provider: Provider | null = null): string {
  const s = asked.subject;
  if (s.kind === 'question') {
    const q = firstQuestion(asked);
    return q ? `Asked you: ${oneLine(q, 80)}` : 'Asked you a question';
  }
  if (showsPlan(s, provider)) return 'Asked you to read its plan';
  if (s.kind === 'permissions') {
    const asks = permissionsAsked(s.detail);
    return asks.lines.length ? `Asked to ${oneLine(asks.wants, 60)}` : 'Asked for more permissions';
  }
  return `Asked for approval${s.kind === 'command' ? `: ${oneLine(s.command, 60)}` : ''}`;
}

/**
 * What you said with your answer to a request, beside the answer to a question: the note of a
 * "no", and that you changed a plan before you accepted it. Null when nothing is on record
 * for it, or when it was not you who settled it (`by`): only a row that reads "by you" shows
 * your words.
 */
export function wordsOf(asked: Held | null, by: string | null): Words | null {
  return by === 'user' && asked?.id ? (yourWords.get(asked.id) ?? null) : null;
}

/**
 * `outcomeWord`, and for a Claude agent's plan what you did with it: "Plan accepted", "Plan
 * accepted with your changes" (`planChanged`: your text went back as the plan), "Plan sent back".
 */
export function settledWord(status: ApprovalStatus, asked: Asked | null, provider: Provider | null, planChanged: boolean): string {
  if (asked && showsPlan(asked.subject, provider)) {
    if (status === 'approved') return planChanged ? 'Plan accepted with your changes' : 'Plan accepted';
    if (status === 'denied') return 'Plan sent back';
  }
  return outcomeWord(status, asked);
}

/** An agent's history: how that request ended ("Approved by you", "Denied by you: use pnpm",
 *  "Closed by Codly"). `asked` is the request when it is still known (the line of a question or
 *  of a plan says so, and your note is found by its id), `answer` what you answered to a question. */
export function settledLine(status: ApprovalStatus, by: string, asked: Held | null, answer: string | null, provider: Provider | null = null): string {
  const said = wordsOf(asked, by);
  const line = `${settledWord(status, asked, provider, !!said?.planChanged)} ${outcomeBy(by)}`.trim();
  if (asked?.subject.kind === 'question' && status === 'approved' && answer) return `${line}: ${oneLine(answer, 80)}`;
  return said?.note ? `${line}: ${oneLine(said.note, 80)}` : line;
}
