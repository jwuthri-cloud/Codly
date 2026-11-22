// What went wrong in an agent's turns, read from the event log, so the conversation can say it
// where it happened: an error the agent reported, a turn that failed, was interrupted or hit its
// step limit. Pure: `troublesOf` takes the log and who to leave out (turns you stopped yourself).
import type { EventRow } from '../lib/gen/EventRow';

/**
 * problem: the agent reported an error and carried on · stopped: its process or account gave up ·
 * failed: the turn ended in failure · interrupted: something other than you cut the turn ·
 * limit: the turn used all the steps it may take.
 */
export type TroubleKind = 'problem' | 'stopped' | 'failed' | 'interrupted' | 'limit';
export type Trouble = {
  /** The event it comes from: the row's key, and where Details scrolls to. */
  eventId: number;
  ts: number;
  what: TroubleKind;
  /** The agent's own words (an error message), when it gave any. */
  reason: string | null;
  /** The turn it happened in: when it started and the prompt Codly sent (null: between turns). */
  turn: { start: number; input: string | null } | null;
};

/**
 * The agent's troubles, oldest first. A failed turn takes the last error of that turn as its
 * reason (one line, not two). `byYou(turnStart)`: you stopped that turn yourself, so its
 * "interrupted" end is not reported.
 */
export function troublesOf(events: EventRow[], agentId: string, byYou: (turnStart: number) => boolean): Trouble[] {
  const out: Trouble[] = [];
  let turn: { start: number; input: string | null; error: Trouble | null } | null = null;
  const within = () => (turn ? { start: turn.start, input: turn.input } : null);
  for (const ev of events) {
    if (ev.agent_id !== agentId) continue;
    const k = ev.kind;
    switch (k.type) {
      case 'turn_started':
        turn = { start: ev.ts, input: k.input, error: null };
        break;
      case 'error': {
        const t: Trouble = { eventId: ev.id, ts: ev.ts, what: k.fatal ? 'stopped' : 'problem', reason: k.message, turn: within() };
        out.push(t);
        if (turn) turn.error = t;
        break;
      }
      case 'turn_completed': {
        const end = (what: TroubleKind) => out.push({ eventId: ev.id, ts: ev.ts, what, reason: null, turn: within() });
        if (k.status === 'failed') {
          // "stopped" already says more than "failed".
          if (turn?.error?.what === 'problem') turn.error.what = 'failed';
          else if (!turn?.error) end('failed');
        } else if (k.status === 'max_turns') end('limit');
        else if (k.status === 'interrupted' && !(turn && byYou(turn.start))) end('interrupted');
        turn = null;
        break;
      }
      case 'exited':
        turn = null;
        break;
    }
  }
  return out;
}

/** The line's headline, in plain words. */
export function troubleTitle(what: TroubleKind, name: string): string {
  switch (what) {
    case 'problem':
      return `${name} reported a problem`;
    case 'stopped':
      return `${name} stopped`;
    case 'failed':
      return `${name}’s turn failed`;
    case 'interrupted':
      return `${name}’s turn was interrupted`;
    case 'limit':
      return `${name} used all the steps one turn may take`;
  }
}

/** Whether sending the message again can help: the turn (or the agent) gave up on it. */
export const canRetry = (what: TroubleKind): boolean => what === 'failed' || what === 'stopped';

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();

/**
 * The message of yours that a turn was working on: the newest one sent before the turn started
 * whose words are in its prompt (Codly wraps your words, it never rewrites them). Null when the
 * turn was not started by a message of yours.
 */
export function messageOf(turn: Trouble['turn'], yours: { ts: number; text: string }[]): string | null {
  if (!turn?.input) return null;
  const prompt = norm(turn.input);
  let best: { ts: number; text: string } | null = null;
  for (const m of yours) if (m.ts <= turn.start && (!best || m.ts > best.ts) && prompt.includes(norm(m.text))) best = m;
  return best?.text ?? null;
}
