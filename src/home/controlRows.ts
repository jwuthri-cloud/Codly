// What you did to the agent of a thread, as quiet rows in the thread: a compaction that is done,
// each change the core recorded for the agent ("agent_control": a mode switch, a compaction you
// asked for, an MCP server turned on or off, a steer note, a stop of background work), a change
// of its model or thinking effort, and the last context reading of this window. Rows of the same
// minute join into one, as notices do. Pure.
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
  const notes: Note[] = [];
  let model: string | null | undefined;
  /** The model the agent's newest run said at its start, while no turn has started since. */
  let started: string | null = null;
  for (const ev of events) {
    if (ev.agent_id !== agent.id) continue;
    const k = ev.kind;
    switch (k.type) {
      case 'session_started':
        started = k.model;
        break;
      case 'turn_started':
        started = null;
        break;
      case 'compacting': {
        const text = compacted(agent.name, k);
        if (text) notes.push(note(ev, ev.id, ev.ts, text));
        break;
      }
      case 'decision':
        if (k.decision_type === 'agent_control') notes.push(note(ev, ev.id, ev.ts, historySentence(k.summary, k.method, keeps)));
        break;
      case 'agent_changed': {
        const now = k.agent.model;
        const fromStart = started != null && splitModel(now).name === started;
        if (model !== undefined && now !== model && !fromStart) notes.push(note(ev, ev.id, ev.ts, `${agent.name} now works with ${modelWords(now)}`));
        model = now;
        break;
      }
    }
  }
  if (context) notes.push(note(context, 0, context.ts, `Context: ${context.line}`));
  notes.sort((a, b) => a.ts - b.ts);

  // One row for each minute that has some.
  const rows: Note[][] = [];
  for (const n of notes) {
    const last = rows[rows.length - 1];
    if (last && Math.floor(last[0].ts / 60_000) === Math.floor(n.ts / 60_000)) last.push(n);
    else rows.push([n]);
  }
  return rows;
}
