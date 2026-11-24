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
 * The agent's troubles, read one event at a time (`add`, oldest first): the conversation reads
 * only the events that came since its last read. A failed turn takes the last error of that turn
 * as its reason (one line, not two): that trouble changes in place when the end comes.
 * An interrupted end is in `out` also when you stopped that turn yourself: `troublesOf` leaves
 * those out, as you can take a stop back at any time.
 */
export class TroubleReader {
  readonly out: Trouble[] = [];
  private turn: { start: number; input: string | null; error: Trouble | null } | null = null;
  constructor(private readonly agentId: string) {}

  add(ev: EventRow) {
    if (ev.agent_id !== this.agentId) return;
    const k = ev.kind;
    const turn = this.turn;
    const within = () => (turn ? { start: turn.start, input: turn.input } : null);
    switch (k.type) {
      case 'turn_started':
        this.turn = { start: ev.ts, input: k.input, error: null };
        break;
      case 'error': {
        const t: Trouble = { eventId: ev.id, ts: ev.ts, what: k.fatal ? 'stopped' : 'problem', reason: k.message, turn: within() };
        this.out.push(t);
        if (turn) turn.error = t;
        break;
      }
      case 'turn_completed': {
        const end = (what: TroubleKind) => this.out.push({ eventId: ev.id, ts: ev.ts, what, reason: null, turn: within() });
        if (k.status === 'failed') {
          // "stopped" already says more than "failed".
          if (turn?.error?.what === 'problem') turn.error.what = 'failed';
          else if (!turn?.error) end('failed');
        } else if (k.status === 'max_turns') end('limit');
        else if (k.status === 'interrupted') end('interrupted');
        this.turn = null;
        break;
      }
      case 'exited':
        this.turn = null;
        break;
    }
  }
}

/** Whether a trouble is reported: not the end of a turn you stopped yourself (`byYou(turnStart)`). */
export const reported = (t: Trouble, byYou: (turnStart: number) => boolean): boolean => t.what !== 'interrupted' || !(t.turn && byYou(t.turn.start));

/**
 * The agent's troubles, oldest first. `byYou(turnStart)`: you stopped that turn yourself, so its
 * "interrupted" end is not reported.
 */
export function troublesOf(events: EventRow[], agentId: string, byYou: (turnStart: number) => boolean): Trouble[] {
  const r = new TroubleReader(agentId);
  for (const ev of events) r.add(ev);
  return r.out.filter((t) => reported(t, byYou));
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

/** How long after a Kill the end of a turn it cut can come (the CLI's shutdown ladder). */
export const KILL_GRACE = 15_000;

/**
 * Whether this trouble is the end of a turn you killed (`kills`: when, on the wall clock), so
 * the line "You killed …" already says it. A turn that started before the kill, or an
 * interruption with no turn on record that came within `KILL_GRACE` of it.
 */
export function endedByKill(t: Pick<Trouble, 'what' | 'ts' | 'turn'>, kills: number[]): boolean {
  if (t.what !== 'interrupted') return false;
  return kills.some((k) => k <= t.ts + 1000 && (t.turn ? k >= t.turn.start : t.ts - k <= KILL_GRACE));
}
