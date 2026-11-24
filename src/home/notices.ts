// What an agent's CLI told about its runs (a hook's word, a goal, an MCP server that did not
// start), read from the event log and put together for the conversation: a hook that speaks
// on every tool call is one row for its turn, not a row for each call. Pure.
import type { EventRow } from '../lib/gen/EventRow';

/** One notice: the event behind it, when it was told, and its words. */
export type Note = { eventId: number; ts: number; text: string };

/** The note of a notice event, the same object each time (a row that did not change keeps its notes). */
const noteOf = new WeakMap<EventRow, Note>();

/**
 * The agent's notices as the rows that show them, oldest first (in a row: oldest first too),
 * read one event at a time (`add`, oldest first): the conversation reads only the events that
 * came since its last read. A row that gets a note is a new list, so a row that did not change
 * keeps its list.
 * Everything told between the start and the end of one turn is one row. A notice outside any
 * turn is a row of its own; the same words told again right after it join its row.
 * The log may begin inside a turn whose start it does not hold any more (a long turn): until
 * the log says that a turn started or ended, what is told is one row too, as in a turn. Nothing
 * between those notices says that they are of different turns, and the row lists them all.
 */
export class NoticeReader {
  readonly rows: Note[][] = [];
  /** The log says that no turn runs: a turn or the process ended, and no turn started since.
   *  At the log's start it does not say so: a turn may run whose start left the log. */
  private outside = false;
  /** Where in `rows` the row of the turn that runs (or that may run) is (-1: none yet). */
  private inTurn = -1;
  /** Where in `rows` the newest row outside a turn is, while nothing came after it (-1: none). */
  private alone = -1;
  constructor(private readonly agentId: string) {}

  add(ev: EventRow) {
    if (ev.agent_id !== this.agentId) return;
    const k = ev.kind;
    switch (k.type) {
      case 'turn_started':
        this.outside = false;
        this.inTurn = this.alone = -1;
        break;
      case 'turn_completed':
        this.outside = true;
        this.inTurn = this.alone = -1;
        break;
      case 'exited':
        this.outside = true;
        this.inTurn = -1;
        break;
      case 'notice': {
        let note = noteOf.get(ev);
        if (!note) noteOf.set(ev, (note = { eventId: ev.id, ts: ev.ts, text: k.text }));
        if (!this.outside) {
          if (this.inTurn < 0) this.inTurn = this.rows.push([note]) - 1;
          else this.join(this.inTurn, note);
        } else if (this.alone >= 0 && this.rows[this.alone][0].text === note.text) this.join(this.alone, note);
        else this.alone = this.rows.push([note]) - 1;
        break;
      }
    }
  }

  private join(at: number, note: Note) {
    this.rows[at] = [...this.rows[at], note];
  }
}

/** The agent's notices as the rows that show them (see `NoticeReader`). */
export function noticeRows(events: EventRow[], agentId: string): Note[][] {
  const r = new NoticeReader(agentId);
  for (const ev of events) r.add(ev);
  return r.rows;
}

/**
 * A notice that says something went wrong or changed under the agent: an MCP server that did
 * not start, a hook that blocked, failed, stopped, warned or asked to stop, a turn run on another
 * model, a goal that is blocked or stopped at a limit. Each is the core's own sentence, matched
 * from its start (adapters/codex.rs `mcp_start_failed`, `hook_row`, `rerouted_text`,
 * `goal_text`): what a goal or a hook says is free text, and its words do not count ("Goal: fix
 * the failed test" is a plain note). The others (a goal, a hook's context, the run's start) are
 * plain notes.
 */
const WORSE = [
  /^MCP server .+ did not start\b/,
  /^Codex hook \S+(?: \([^)]*\))? (?:blocked|failed|stopped|reported an error|gave a warning|asked to stop)(?::|$)/,
  /^Codex ran this turn on .+ instead of /,
  /^Goal (?:blocked|stopped at)\b/,
];
const worse = (n: Note) => WORSE.some((re) => re.test(n.text));

/** The notice a row shows: its newest of the worst kind (a failure before a plain note). */
export function headline(notes: Note[]): Note {
  for (let i = notes.length - 1; i >= 0; i--) if (worse(notes[i])) return notes[i];
  return notes[notes.length - 1];
}

/**
 * What a row of several notices says next to its headline: how often it was told when all
 * say the same, how many others the row holds when they differ. Null for a single notice.
 */
export function moreOf(notes: Note[]): string | null {
  if (notes.length < 2) return null;
  return notes.every((n) => n.text === notes[0].text) ? `${notes.length} times` : `and ${notes.length - 1} more`;
}
