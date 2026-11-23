// How a request reads once it no longer waits: the closed card in the conversation and the
// lines of an agent's history. A permission is approved or denied. A question is answered or
// skipped, and says what you answered.
import type { Approval } from '../gen/Approval';
import type { ApprovalStatus } from '../gen/ApprovalStatus';
import { oneLine } from './sanitize';

type Asked = Pick<Approval, 'subject'>;

/** "Approved", "Denied", "Stopped", "Expired"; for a question "Answered" and "Skipped". Empty while it waits. */
export function outcomeWord(status: ApprovalStatus, asked: Asked): string {
  const question = asked.subject.kind === 'question';
  switch (status) {
    case 'approved': return question ? 'Answered' : 'Approved';
    case 'denied': return question ? 'Skipped' : 'Denied';
    case 'cancelled': return 'Stopped';
    case 'expired': return 'Expired';
    case 'pending': return '';
  }
}

/** Who settled it: "by you", "by Codly’s rules", "by <who>". Empty when nobody is on record. */
export function outcomeBy(decidedBy: string | null): string {
  return decidedBy === 'user' ? 'by you' : decidedBy === 'policy' ? 'by Codly’s rules' : decidedBy ? `by ${decidedBy}` : '';
}

/** The answers a question card sends, on one line: "Keep it"; "Red · Small, Fast" for several
 *  questions (a question with several picks lists them). Null when nothing was picked or typed. */
export function answerText(answers: unknown): string | null {
  if (!answers || typeof answers !== 'object' || Array.isArray(answers)) return null;
  const said = Object.values(answers)
    .map((v) => (Array.isArray(v) ? v.filter((x) => typeof x === 'string' && x).join(', ') : typeof v === 'string' ? v.trim() : ''))
    .filter(Boolean);
  return said.length ? said.join(' · ') : null;
}

/** The first question of a question card, as the agent wrote it; null when the card has none to show. */
export function firstQuestion(asked: Asked): string | null {
  if (asked.subject.kind !== 'question') return null;
  const raw = asked.subject.questions as unknown;
  const list = Array.isArray(raw) ? raw : raw && typeof raw === 'object' ? (raw as { questions?: unknown }).questions : null;
  const first = Array.isArray(list) ? (list[0] as { question?: unknown; prompt?: unknown } | null) : null;
  const text = first && typeof first === 'object' ? (first.question ?? first.prompt) : null;
  return typeof text === 'string' && text ? text : null;
}

/** An agent's history: what it asked you. */
export function askedLine(asked: Asked): string {
  const s = asked.subject;
  if (s.kind === 'question') {
    const q = firstQuestion(asked);
    return q ? `Asked you: ${oneLine(q, 80)}` : 'Asked you a question';
  }
  return `Asked for approval${s.kind === 'command' ? `: ${oneLine(s.command, 60)}` : ''}`;
}

/** An agent's history: how that request ended. `asked` is the request when it is still known
 *  (the line of a question says so), `answer` what you answered when this window sent it. */
export function settledLine(status: ApprovalStatus, by: string, asked: Asked | null, answer: string | null): string {
  if (asked?.subject.kind !== 'question') return `Approval ${status} by ${by === 'user' ? 'you' : by}`;
  const line = `${outcomeWord(status, asked)} ${outcomeBy(by)}`.trim();
  return status === 'approved' && answer ? `${line}: ${oneLine(answer, 80)}` : line;
}
