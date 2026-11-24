// What you did to the agent of a thread, as quiet rows in the thread: a compaction that is done,
// each change the core recorded for the agent ("agent_control": a mode switch, a compaction you
// asked for, an MCP server turned on or off, a steer note, a stop of background work), a change
// of its model or thinking effort, and the last context reading of this window. Notes of one kind
// that follow each other in one minute join into one row; one of another kind keeps its own row,
// at its place in time. Pure.
import type { Agent } from '../lib/gen/Agent';
import type { EventRow } from '../lib/gen/EventRow';
import { tokens } from '../lib/util/format';
import { prettyModel } from '../lib/util/labels';
import { effortLabel } from '../lib/util/models';
import { splitModel } from '../lib/util/team';
import { historySentence } from '../sheet/historyWords';
import type { Note } from './notices';

/** The last context reading of an agent in this window: when, and the words its line shows. */
export type ContextRead = { ts: number; line: string };

/** A note of an event, the same object each time (a row that did not change keeps its notes). */
const noteOf = new WeakMap<object, Note>();
function note(of: object, eventId: number, ts: number, text: string): Note {
  let n = noteOf.get(of);
  if (!n || n.text !== text) noteOf.set(of, (n = { eventId, ts, text }));
  return n;
}

/** "Opus 4.7 · High effort", "its CLI’s default model". */
function modelWords(model: string | null): string {
  const { name, effort } = splitModel(model);
  const shown = name ? prettyModel(name) : 'its CLI’s default model';
  return effort ? `${shown} · ${effortLabel(effort)} effort` : shown;
}

/**
 * A compaction that is done, in words, or null. Only Claude Code says that one is done: its
 * boundary line carries how big the conversation was before (and after). Its start carries no
 * number, and another CLI's event carries none either and can be a start or an end (Codex sends
 * both, and nothing tells them apart): those show nothing here. A compaction you asked for has
 * its own row from the core's record.
 */
function compacted(name: string, k: { pre_tokens: number | null; post_tokens: number | null }): string | null {
  const { pre_tokens: pre, post_tokens: post } = k;
  if (pre == null && post == null) return null;
  const base = `${name}’s conversation was compacted`;
  if (pre != null && post != null) return `${base} · ${tokens(pre)} → ${tokens(post)} tokens`;
  return pre != null ? `${base} · ${tokens(pre)} tokens before` : `${base} · ${tokens(post)} tokens now`;
}

/**
 * The control rows of `agent`, oldest first (in a row: oldest first too), from the log
 * (`events`: oldest first) and the last context reading of this window (`context`). `keeps`:
 * whether a word is a name or a task key, which a sentence does not capitalise.
 *
 * A model change is read from the agent's own rows in the log (`agent_changed`): a row whose model
 * differs from the one before. Not the one a run's start makes: the core then writes the model
 * id its CLI reported in place of the name you picked ("opus" becomes "claude-opus-4-7"), and
 * nothing changed for you.
 */
export function controlRows(events: readonly EventRow[], agent: Pick<Agent, 'id' | 'name'>, keeps: (word: string) => boolean, context: ContextRead | null): Note[][] {
  const r = new ControlReader(agent, keeps);
  for (const ev of events) r.add(ev);
  return r.rows(context);
}

/** What a note is about: only notes of the same kind join one row. */
type Kind = 'compaction' | 'control' | 'model' | 'context';
const kindOf = new WeakMap<Note, Kind>();

/**
 * The control rows of one agent (see `controlRows`), read one event at a time (`add`, oldest
 * first): the conversation reads only the events that came since its last read.
 */
export class ControlReader {
  /** The notes the log gave so far, in the log's order. */
  private notes: Note[] = [];
  private model: string | null | undefined;
  /** The model the agent's newest run said at its start, while no turn has started since. */
  private started: string | null = null;
  constructor(
    private readonly agent: Pick<Agent, 'id' | 'name'>,
    private readonly keeps: (word: string) => boolean,
  ) {}

  private push(of: object, eventId: number, ts: number, text: string, kind: Kind) {
    const n = note(of, eventId, ts, text);
    kindOf.set(n, kind);
    this.notes.push(n);
  }

  add(ev: EventRow) {
    if (ev.agent_id !== this.agent.id) return;
    const k = ev.kind;
    const name = this.agent.name;
    switch (k.type) {
      case 'session_started':
        this.started = k.model;
        break;
      case 'turn_started':
        this.started = null;
        break;
      case 'compacting': {
        const text = compacted(name, k);
        if (text) this.push(ev, ev.id, ev.ts, text, 'compaction');
        break;
      }
      case 'decision':
        // A compaction you asked for and the compaction that came of it are one act.
        if (k.decision_type === 'agent_control') this.push(ev, ev.id, ev.ts, historySentence(k.summary, k.method, this.keeps), k.summary.startsWith('asked ') && k.summary.endsWith(' to compact its conversation') ? 'compaction' : 'control');
        break;
      case 'agent_changed': {
        const now = k.agent.model;
        const fromStart = this.started != null && splitModel(now).name === this.started;
        if (this.model !== undefined && now !== this.model && !fromStart) this.push(ev, ev.id, ev.ts, `${name} now works with ${modelWords(now)}`, 'model');
        this.model = now;
        break;
      }
    }
  }

  /**
   * The rows, with the last context reading of this window (`context`) in its place. Notes of
   * the same kind that follow each other in one clock minute are one row; a note of another
   * kind between them, or a new minute, starts another, so each row stays where it happened.
   */
  rows(context: ContextRead | null): Note[][] {
    const notes = this.notes.slice();
    if (context) {
      const n = note(context, 0, context.ts, `Context: ${context.line}`);
      kindOf.set(n, 'context');
      notes.push(n);
    }
    notes.sort((a, b) => a.ts - b.ts);
    const rows: Note[][] = [];
    for (const n of notes) {
      const last = rows[rows.length - 1];
      const head = last?.[0];
      if (head && kindOf.get(head) === kindOf.get(n) && Math.floor(head.ts / 60_000) === Math.floor(n.ts / 60_000)) last.push(n);
      else rows.push([n]);
    }
    return rows;
  }
}
