// Never blocking: you can always send. A message to an agent whose turn is running waits in
// Codly's queue and runs as that agent's next turn. This works out, for one thread, which of
// your messages are still waiting (from the agent's turns), in order.
//
// A queued message is delivered by the first later `turn_started` whose prompt contains it
// (Codly wraps your words, it never rewrites them); a turn with no recorded prompt takes the
// oldest waiting message. Codex can steer a message into the running turn instead of starting a
// new one: once the agent has been idle a few seconds, nothing it was sent is still waiting.
// When every Lead slot is busy, Codly says so in the thread ("Queued: …", author codly): what
// you sent since the request's last turn waits for a free Lead (nothing to interrupt yet).
import { store } from './store.svelte';
import { ui } from './ui.svelte';

/** agentId null: waiting for a free Lead slot (no turn to interrupt). */
export type Waiting = { position: number; total: number; agentId: string | null; reason: string | null };
export type Sent = { key: string; ts: number; text: string };
export type Turn = { start: number; end: number | null; input: string | null };

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();
export const STEER_GRACE = 5000;
export const isSlotNote = (author: string, text: string) => author === 'codly' && /^Queued:/i.test(text);

/**
 * Pure core: which of `sent` still wait, in order, given the agent's turns, the time, and the
 * latest "all Leads busy" note (if any). `slot` true in the result = held for a free Lead.
 */
export function waitingOf(turns: Turn[], sent: Sent[], now: number, slotNote: { ts: number } | null = null): { waiting: Sent[]; slot: boolean } {
  const busyAt = (ts: number) => turns.some((t) => t.start <= ts && (t.end == null || t.end > ts));
  type Ev = { ts: number; msg?: Sent; turn?: Turn };
  const evs: Ev[] = [...sent.map((m) => ({ ts: m.ts, msg: m })), ...turns.map((t) => ({ ts: t.start, turn: t }))];
  // A message and the turn it starts can share a millisecond: the message comes first.
  evs.sort((a, b) => a.ts - b.ts || (a.msg ? -1 : 1));
  let waiting: Sent[] = [];
  for (const e of evs) {
    if (e.msg) {
      if (waiting.length || busyAt(e.msg.ts)) waiting.push(e.msg);
      continue;
    }
    if (!waiting.length) continue;
    const input = e.turn!.input ? norm(e.turn!.input) : '';
    const i = input ? waiting.findIndex((m) => input.includes(norm(m.text))) : 0;
    if (i >= 0) waiting = waiting.filter((_, j) => j !== i);
  }
  const last = turns[turns.length - 1];
  if (waiting.length && last && last.end != null && now - last.end > STEER_GRACE) waiting = [];
  const lastStart = last?.start ?? -Infinity;
  if (slotNote && slotNote.ts > lastStart) {
    for (const m of sent) if (m.ts > lastStart && !waiting.includes(m)) waiting.push(m);
    return { waiting: waiting.sort((a, b) => a.ts - b.ts), slot: true };
  }
  return { waiting, slot: false };
}

/** key → its place in the queue, for the messages that still wait in this thread. */
export function queued(thread: string, sent: Sent[]): Map<string, Waiting> {
  const out = new Map<string, Waiting>();
  const agent = store.threadAgent(thread);
  void store.rev;
  const turns = agent && agent.status !== 'offline' ? (store.live[agent.id]?.turns ?? []) : [];
  let note: { ts: number; text: string } | null = null;
  for (const c of store.chats) if (c.thread === thread && isSlotNote(c.author, c.text) && (!note || c.ts > note.ts)) note = { ts: c.ts, text: c.text };
  const { waiting, slot } = waitingOf(turns, sent, ui.now, note);
  if (!slot && !agent) return out;
  const reason = slot && note ? note.text.replace(/^Queued:\s*/i, '') : null;
  waiting.forEach((m, i) => out.set(m.key, { position: i + 1, total: waiting.length, agentId: slot ? null : agent!.id, reason }));
  return out;
}

/** Whether a thread's agent is in a turn right now (your next message would queue). */
export function busy(thread: string): boolean {
  const agent = store.threadAgent(thread);
  if (!agent) return false;
  void store.rev;
  const turns = store.live[agent.id]?.turns ?? [];
  return !!turns.length && turns[turns.length - 1].end == null && agent.status !== 'offline';
}
