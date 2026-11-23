// Never blocking: you can always send. A message to an agent whose turn is running waits:
// Claude Code and Codex read it during that turn (after the step they are in), the other CLIs
// run it as the agent's next turn. This works out, for one thread, which of your messages are
// still waiting (from the agent's turns), in order, and says in words where they go.
//
// A queued message is delivered by the first later `turn_started` whose prompt contains it
// (Codly wraps your words, it never rewrites them). A turn with no prompt is one the CLI started
// by itself (a background task ended and woke it): it delivers nothing, and what waits goes on
// waiting. Claude and Codex can take a message into the running turn instead of
// starting a new one: the turn's `prompt_steered` delivers it the same way. And once the agent
// has been idle a few seconds, nothing it was sent is still waiting.
// When every Lead slot is busy, Codly says so in the thread ("Queued: …", author codly): what
// you sent since the request's last turn waits for a free Lead (nothing to interrupt yet).
import type { Provider } from '../gen/Provider';
import { store } from './store.svelte';
import { ui } from './ui.svelte';

/** agentId null: waiting for a free Lead slot (no turn to interrupt). */
export type Waiting = { position: number; total: number; agentId: string | null; reason: string | null };
export type Sent = { key: string; ts: number; text: string };
/** `steers`: the prompts the turn took in while it ran (`prompt_steered`). */
export type Turn = { start: number; end: number | null; input: string | null; steers?: { ts: number; input: string }[] };

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();
export const STEER_GRACE = 5000;
export const isSlotNote = (author: string, text: string) => author === 'codly' && /^Queued:/i.test(text);

/**
 * Pure core: which of `sent` still wait, in order, given the agent's turns, the time, and the
 * latest "all Leads busy" note (if any). `slot` true in the result = held for a free Lead.
 */
export function waitingOf(turns: Turn[], sent: Sent[], now: number, slotNote: { ts: number } | null = null): { waiting: Sent[]; slot: boolean } {
  const busyAt = (ts: number) => turns.some((t) => t.start <= ts && (t.end == null || t.end > ts));
  // `input`: the prompt of a turn that starts, or one the running turn took in.
  type Ev = { ts: number; msg?: Sent; input?: string | null };
  const evs: Ev[] = [
    ...sent.map((m) => ({ ts: m.ts, msg: m })),
    ...turns.flatMap((t) => [{ ts: t.start, input: t.input }, ...(t.steers ?? []).map((s) => ({ ts: s.ts, input: s.input }))]),
  ];
  // A message and the turn it starts can share a millisecond: the message comes first.
  evs.sort((a, b) => a.ts - b.ts || (a.msg ? -1 : 1));
  let waiting: Sent[] = [];
  for (const e of evs) {
    if (e.msg) {
      if (waiting.length || busyAt(e.msg.ts)) waiting.push(e.msg);
      continue;
    }
    if (!waiting.length || !e.input) continue;
    const input = norm(e.input);
    const i = waiting.findIndex((m) => input.includes(norm(m.text)));
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

/** Claude Code and Codex take a message into the turn that runs; the other CLIs run it as the next turn. */
const readsMidTurn = (provider: Provider | undefined) => provider === 'claude' || provider === 'codex';

/** Where the first waiting message goes: the words after "Queued — ". */
export const queuedHint = (name: string, provider: Provider | undefined) =>
  readsMidTurn(provider) ? `${name} reads it after its current step` : `runs when ${name} finishes this turn`;

/** What "Send now" costs: the tip of the button on the first waiting message. */
export const sendNowTip = (name: string, provider: Provider | undefined) =>
  readsMidTurn(provider) ? `Stop what ${name} is doing so it reads this now` : `Interrupt ${name}’s turn so this runs next`;

/** The composer's placeholder while the agent is in a turn. */
export const busyHint = (name: string, provider: Provider | undefined) =>
  readsMidTurn(provider) ? `${name} is mid-turn — it reads your message after its current step` : `${name} is mid-turn — your message queues and runs next`;

/** The composer's placeholder while the agent's turn waits for your answer to a request shown above it. */
export const waitingHint = (name: string, provider: Provider | undefined) =>
  readsMidTurn(provider) ? `${name} waits for your answer above — it reads your message after that` : `${name} waits for your answer above — your message runs after this turn`;

/** When a steer note reaches the agent. */
export const steerHint = (provider: Provider | undefined) =>
  readsMidTurn(provider) ? 'Arrives as context during its current turn, or as its next turn' : 'Arrives as context on its next turn';

/** Whether a thread's agent is in a turn right now (your next message would queue). */
export function busy(thread: string): boolean {
  const agent = store.threadAgent(thread);
  if (!agent) return false;
  void store.rev;
  const turns = store.live[agent.id]?.turns ?? [];
  return !!turns.length && turns[turns.length - 1].end == null && agent.status !== 'offline';
}
