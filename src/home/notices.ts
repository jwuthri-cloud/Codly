// What an agent's CLI told about its runs (a hook's word, a goal, an MCP server that did not
// start), read from the event log and put together for the conversation: a hook that speaks
// on every tool call is one row for its turn, not a row for each call. Pure.
import type { EventRow } from '../lib/gen/EventRow';

/** One notice: the event behind it, when it was told, and its words. */
export type Note = { eventId: number; ts: number; text: string };

/** The note of a notice event, the same object each time (a row that did not change keeps its notes). */
const noteOf = new WeakMap<EventRow, Note>();

/**
 * The agent's notices as the rows that show them, oldest first (in a row: oldest first too).
 * Everything told between the start and the end of one turn is one row. A notice outside any
 * turn is a row of its own; the same words told again right after it join its row.
 * The log may begin inside a turn: what was told before the first turn end it holds, with no
 * turn start before it, was told in that turn.
 */
export function noticeRows(events: EventRow[], agentId: string): Note[][] {
  const rows: Note[][] = [];
  /** unknown: nothing in the log says yet whether a turn runs. */
  let turn: 'unknown' | 'open' | 'none' = 'unknown';
  /** The row of the turn that runs. */
  let inTurn: Note[] | null = null;
  /** The newest row outside a turn, while nothing came after it. */
  let alone: Note[] | null = null;
  for (const ev of events) {
    if (ev.agent_id !== agentId) continue;
    const k = ev.kind;
    switch (k.type) {
      case 'turn_started':
        turn = 'open';
        inTurn = alone = null;
        break;
      case 'turn_completed':
        if (turn === 'unknown' && rows.length > 1) rows.splice(0, rows.length, rows.flat());
        turn = 'none';
        inTurn = alone = null;
        break;
      case 'exited':
        turn = 'none';
        inTurn = null;
        break;
      case 'notice': {
        let note = noteOf.get(ev);
        if (!note) noteOf.set(ev, (note = { eventId: ev.id, ts: ev.ts, text: k.text }));
        if (turn === 'open') {
          if (!inTurn) rows.push((inTurn = []));
          inTurn.push(note);
        } else if (alone && alone[0].text === note.text) alone.push(note);
        else rows.push((alone = [note]));
        break;
      }
    }
  }
  return rows;
}

/**
 * What a row of several notices says next to its newest one: how often it was told when all
 * say the same, how many others the row holds when they differ. Null for a single notice.
 */
export function moreOf(notes: Note[]): string | null {
  if (notes.length < 2) return null;
  return notes.every((n) => n.text === notes[0].text) ? `${notes.length} times` : `and ${notes.length - 1} more`;
}
