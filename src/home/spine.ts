// The conversation spine: everything important lands in the thread as an item, in order.
// A request thread ("lead" / "lead:<id>") = your messages, its Lead's replies and (folded) work,
// approvals from the agents on its plan where they happened, questions they asked its Lead,
// task-done and plan-done cards, and what its agent's CLI told about the run (a hook's word, a
// goal, an MCP server that did not start; one row for all of one turn). A 1:1 thread = the same
// for one agent.
// Each request is its own thread: another request's plan, approvals and cards never show here.
//
// The list is built again only when something of this thread changed (`buildSpine`), and a row
// that did not change keeps its object (`keepRows`), so the screen updates that row alone. A
// message or a thought being written does not build the list again on each new word: its row
// reads the words as they are now (`streamText`, `thinkingText`).
//
// A row's key is made from the id of what it shows (a chat row, a message of the agent, a
// call, an event), never from its words: an agent that says the same thing twice has two rows.
import type { Approval } from '../lib/gen/Approval';
import type { ChatMessage } from '../lib/gen/ChatMessage';
import type { MessagePhase } from '../lib/gen/MessagePhase';
import type { JsonValue } from '../lib/gen/serde_json/JsonValue';
import { store, type AgentMsg, type Outgoing, type Seg, type SubAgent, type Thought } from '../lib/state/store.svelte';
import { groupSame } from '../lib/state/approvalGroups';
import { isSlotNote } from '../lib/state/queue';
import { killedAt, killLine, stoppedByYou, stoppedTurns } from '../lib/state/stops';
import { isRequestThread } from '../lib/state/ui.svelte';
import { noticeRows, type Note } from './notices';
import { canRetry, endedByKill, messageOf, troublesOf, type TroubleKind } from './outcomes';

export type Item =
  | { kind: 'user'; key: string; ts: number; msg: ChatMessage; outgoing: Outgoing | null }
  | { kind: 'system'; key: string; ts: number; text: string; card: { type: 'plan' | 'plan-done'; planId: string } | null }
  | {
      kind: 'say';
      key: string;
      ts: number;
      agentId: string;
      /**
       * The message of the agent this row shows: its key, and the words it reads while they are
       * written. Null for a chat row with no held message behind it: the message is not held
       * (any more), the row's author is not the thread's agent (another Lead served the request
       * then, the thread has no agent now, or a Lead wrote to a worker: the core posts that in
       * the worker's thread with the Lead as author), the row's id belongs to a later message by
       * now (a CLI used the id again: a newer row shows that message, or it was said after this
       * row), or the core posted the row for a turn result that no message carried.
       */
      itemId: string | null;
      text: string;
      phase: MessagePhase;
      /** It is being written: `text` is what it said when the row was made, `streamText` what it says now. */
      streaming: boolean;
      /** It shows who speaks: consecutive messages from the same agent share one header. */
      header: boolean;
      /** Which card rides under this message. */
      card: { type: 'plan' | 'plan-done'; planId: string } | null;
      replyTo: string | null;
    }
  // `thinking`: the thought being written, as it was when the row was made (`thinkingText`: as it is now).
  // `helpers`: the subagents started by calls of this row (their own calls are in `segs` too).
  | { kind: 'work'; key: string; ts: number; agentId: string; segs: Seg[]; thoughts: Thought[]; thinking: string | null; helpers: SubAgent[] }
  | { kind: 'approval'; key: string; ts: number; approval: Approval; /** Other agents' requests that wait for the same answer. */ twins: Approval[] }
  | { kind: 'ask'; key: string; ts: number; agentId: string; msg: ChatMessage }
  | { kind: 'task-done'; key: string; ts: number; taskId: string }
  | {
      kind: 'trouble';
      key: string;
      ts: number;
      agentId: string;
      /** The event behind it (Details opens the agent's history there). */
      eventId: number;
      what: TroubleKind;
      reason: string | null;
      /** Your message to send again (null: sending again cannot help, or no message of yours started that turn). */
      retry: string | null;
    }
  // What the agent's CLI told about the run, in its own words: all of one turn, or the same
  // words told again outside a turn (`noticeRows`). `notes`: oldest first; `ts`: the newest one's.
  | { kind: 'notice'; key: string; ts: number; agentId: string; notes: Note[] };

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();
/** `norm` of a message's text, worked out once (a message object is never changed). */
const normed = new WeakMap<{ text: string }, string>();
const normOf = (m: { text: string }): string => {
  let n = normed.get(m);
  if (n === undefined) normed.set(m, (n = norm(m.text)));
  return n;
};
/** A streamed message keeps one key, made from its item id, from its first word to its end. */
const SAY = 'say:';

/**
 * Which message of the agent each of its chat rows shows (chat id -> item id). The core posts
 * a reply to the thread right after the agent said it and writes the message's id on the row
 * (`ChatMessage.item_id`): the row shows the held message with that id, and their words are
 * never compared. A CLI may use an id again in a later session, so the message must have been
 * said before the row, and the newest row with the id takes it.
 *
 * A row without an id was stored before rows had one (or shows no message: a turn result). It
 * is matched as it always was: with the newest message with its words that was said before it
 * and that no other chat row shows.
 *
 * A message is in one chat row at most: a reply the agent repeats is a row of its own, also
 * before its chat row has come. `chats` and `said`: oldest first, as the store keeps them.
 */
function chatItems(chats: ChatMessage[], agentId: string, said: AgentMsg[]): Map<string, string> {
  const itemOf = new Map<string, string>();
  /** By id: the held messages no chat row shows yet. */
  const free = new Map(said.map((m) => [m.itemId, m]));
  /** The agent's rows without an id, newest first. */
  const unnamed: ChatMessage[] = [];
  for (let i = chats.length - 1; i >= 0; i--) {
    const c = chats[i];
    if (c.author !== agentId) continue;
    if (!c.item_id) {
      unnamed.push(c);
      continue;
    }
    const m = free.get(c.item_id);
    // Said after this row: the message of a later session that uses the id again.
    if (!m || m.ts > c.ts) continue;
    itemOf.set(c.id, m.itemId);
    free.delete(m.itemId);
  }
  if (!unnamed.length) return itemOf;
  /** By their words, oldest first: the messages that are still free. */
  const byWords = new Map<string, AgentMsg[]>();
  for (const m of said) {
    if (free.get(m.itemId) !== m) continue;
    const same = byWords.get(normOf(m));
    if (same) same.push(m);
    else byWords.set(normOf(m), [m]);
  }
  for (const c of unnamed) {
    const same = byWords.get(normOf(c));
    if (!same) continue;
    // Said after this chat row: a later message, which no older chat row can show either.
    let at = same.length - 1;
    while (at >= 0 && same[at].ts > c.ts) at--;
    if (at >= 0) itemOf.set(c.id, same[at].itemId);
    same.length = Math.max(at, 0);
  }
  return itemOf;
}

/** Two rows never share a key. Their ids make it so; should a CLI give one id to two things
 *  (a call id used again in a later turn), the later row adds its place among them. */
function uniqueKeys(items: Item[]) {
  const used = new Set<string>();
  for (const it of items) {
    let key = it.key;
    for (let n = 2; used.has(key); n++) key = `${it.key}#${n}`;
    used.add((it.key = key));
  }
}

export function attachmentOf(att: JsonValue | null | undefined): Record<string, unknown> | null {
  return att && typeof att === 'object' && !Array.isArray(att) ? (att as Record<string, unknown>) : null;
}
export const planIdOf = (att: JsonValue | null | undefined): string | null => {
  const a = attachmentOf(att);
  return a && typeof a.plan_id === 'string' ? a.plan_id : null;
};

// Same millisecond: a message's ts is when it finished, a work row's when its first call
// started, so the message comes first.
const RANK: Record<Item['kind'], number> = { user: 0, system: 1, say: 2, ask: 2, work: 3, trouble: 4, notice: 4, approval: 5, 'task-done': 6 };

export function buildSpine(thread: string): Item[] {
  const owner = store.threadAgent(thread);
  const isLead = isRequestThread(thread);
  const items: Item[] = [];
  /** When a notice row or a task-done card began (its first note, its first mark): see `lastOfPlan`. */
  const began = new Map<Item, number>();

  // ── persisted chat ──
  const chats = store.chats.filter((c) => c.thread === thread);
  // A message keeps one key from its first streamed word to its final chat row (no remount).
  const itemOf = owner ? chatItems(chats, owner.id, store.live[owner.id]?.messages ?? []) : new Map<string, string>();
  const seenPlans = new Set<string>();
  // The first message that mentions a plan carries its card; a later one, once the plan is
  // done, carries the plan-done card. Codly itself posts "Plan ready" / "Plan done" lines.
  const cardFor = (att: JsonValue | null | undefined): { type: 'plan' | 'plan-done'; planId: string } | null => {
    const pid = planIdOf(att);
    if (!pid) return null;
    const plan = store.planMap.get(pid);
    let card: { type: 'plan' | 'plan-done'; planId: string } | null = null;
    if (!seenPlans.has(pid)) card = { type: 'plan', planId: pid };
    else if (plan && plan.status === 'done') card = { type: 'plan-done', planId: pid };
    seenPlans.add(pid);
    return card;
  };
  for (const m of chats) {
    if (m.author === 'user') items.push({ kind: 'user', key: store.chatKeys.get(m.id) ?? `c:${m.id}`, ts: m.ts, msg: m, outgoing: null });
    else if (m.author === 'codly') {
      // "Queued: all Leads are busy" shows on your message itself (see queue.ts), not as a line.
      if (!isSlotNote(m.author, m.text)) items.push({ kind: 'system', key: `c:${m.id}`, ts: m.ts, text: m.text, card: cardFor(m.attachment) });
    }
    else if (isLead && owner && m.author !== owner.id && store.agentMap.get(m.author)?.role !== 'lead')
      items.push({ kind: 'ask', key: `c:${m.id}`, ts: m.ts, agentId: m.author, msg: m });
    else {
      const card = cardFor(m.attachment);
      const a = attachmentOf(m.attachment);
      const itemId = itemOf.get(m.id) ?? null;
      items.push({
        kind: 'say', key: itemId ? SAY + itemId : `c:${m.id}`, ts: m.ts, agentId: m.author, itemId, text: m.text, phase: 'final', streaming: false, header: true, card,
        replyTo: a && typeof a.reply_to === 'string' ? a.reply_to : null,
      });
    }
  }

  // ── what you sent that the core has not echoed yet ──
  for (const o of store.outbox) {
    if (o.thread !== thread) continue;
    items.push({ kind: 'user', key: `o:${o.id}`, ts: o.ts, msg: { id: o.id, thread, author: 'user', text: o.text, attachment: null, ts: o.ts }, outgoing: o });
  }

  if (owner) {
    const live = store.live[owner.id];
    const inChat = new Set(itemOf.values());
    type W = { ts: number; seg?: Seg; thought?: Thought };
    const work: W[] = [];
    if (live) {
      for (const m of live.messages) {
        if (inChat.has(m.itemId)) continue;
        items.push({ kind: 'say', key: SAY + m.itemId, ts: m.ts, agentId: owner.id, itemId: m.itemId, text: m.text, phase: m.phase, streaming: false, header: true, card: null, replyTo: null });
      }
      for (const s of live.segments) work.push({ ts: s.start, seg: s });
      for (const r of live.reasoning) work.push({ ts: r.ts, thought: r });
    }
    let thinking: string | null = null;
    let thinkingTs = 0;
    for (const s of store.streams.values()) {
      if (s.agentId !== owner.id) continue;
      if (s.kind === 'text') items.push({ kind: 'say', key: SAY + s.itemId, ts: s.ts, agentId: owner.id, itemId: s.itemId, text: s.text, phase: 'unknown', streaming: true, header: true, card: null, replyTo: null });
      else {
        thinking = s.text;
        thinkingTs = s.ts;
      }
    }
    // Fold tool calls and thoughts between two messages into one quiet "work" row (a new turn
    // starts a new row too: work from two turns never folds together).
    const anchors = [...items.filter((i) => i.kind !== 'work').map((i) => i.ts), ...(live?.turns.map((t) => t.start) ?? [])].sort((a, b) => a - b);
    work.sort((a, b) => a.ts - b.ts);
    let group: Item & { kind: 'work' } | null = null;
    let groupEnd = Infinity;
    // Both lists are in time order: the next anchor is found by walking on, never from the start.
    let next = 0;
    /** Which row shows a call. */
    const rowOf = new Map<string, Item & { kind: 'work' }>();
    for (const w of work) {
      // A helper's own call stays with the call that started the helper, also when the agent
      // has spoken since (a helper in the background works on after the turn).
      const home = w.seg?.parent != null ? rowOf.get(w.seg.parent) : undefined;
      if (w.seg && home) {
        home.segs.push(w.seg);
        rowOf.set(w.seg.callId, home);
        continue;
      }
      if (!group || w.ts >= groupEnd) {
        while (next < anchors.length && anchors[next] <= w.ts) next++;
        groupEnd = anchors[next] ?? Infinity;
        group = { kind: 'work', key: `w:${w.seg?.callId ?? w.thought?.itemId}`, ts: w.ts, agentId: owner.id, segs: [], thoughts: [], thinking: null, helpers: [] };
        items.push(group);
      }
      if (w.seg) {
        group.segs.push(w.seg);
        rowOf.set(w.seg.callId, group);
      }
      if (w.thought) group.thoughts.push(w.thought);
    }
    for (const h of live?.subagents ?? []) rowOf.get(h.callId)?.helpers.push(h);
    if (thinking != null) {
      // The newest work row is the last one made (each starts after the one before).
      const lastAnchor = anchors[anchors.length - 1] ?? 0;
      if (group && group.ts > lastAnchor) group.thinking = thinking;
      else items.push({ kind: 'work', key: `t:live:${owner.id}`, ts: thinkingTs, agentId: owner.id, segs: [], thoughts: [], thinking, helpers: [] });
    }

    // ── what went wrong, where it happened (added last: a problem never splits a work row) ──
    const yours = chats.filter((c) => c.author === 'user');
    // A turn you killed says so in its own line ("You killed …"): its end is not a trouble.
    const kills = killedAt(owner.id);
    for (const t of troublesOf(store.events, owner.id, (start) => stoppedByYou(owner.id, start)))
      if (!endedByKill(t, kills)) items.push({
        kind: 'trouble', key: `e:${t.eventId}`, ts: t.ts, agentId: owner.id, eventId: t.eventId, what: t.what, reason: t.reason,
        retry: canRetry(t.what) ? messageOf(t.turn, yours) : null,
      });

    // ── a Kill of yours: what you did, when ──
    for (const at of kills) items.push({ kind: 'system', key: `k:${owner.id}:${at}`, ts: at, text: killLine(owner.name), card: null });

    // ── a turn you stopped while it waited for your answer: the card's closed row says the
    // CLI stopped it, which is how the CLI tells it; this line says that you did ──
    const stopped = stoppedTurns(owner.id);
    for (const turn of live?.turns ?? []) {
      const end = turn.end;
      if (end == null || !stopped.includes(turn.start)) continue;
      const waited = store.approvals.some((ap) => ap.agent_id === owner.id && ap.status === 'cancelled' && ap.created_at >= turn.start && ap.created_at <= end);
      if (waited) items.push({ kind: 'system', key: `s:${owner.id}:${turn.start}`, ts: end, text: `You stopped ${owner.name} while it waited for your answer`, card: null });
    }

    // ── what its CLI told about the run: one row for a turn, where the newest was told (so
    // after the work it was told in) ──
    for (const notes of noticeRows(store.events, owner.id)) {
      const row: Item = { kind: 'notice', key: `n:${notes[0].eventId}`, ts: notes[notes.length - 1].ts, agentId: owner.id, notes };
      items.push(row);
      began.set(row, notes[0].ts);
    }
  }

  // ── approvals, where they happened (a worker's land in its plan's request) ──
  // The same waiting request from several agents is one card, where the first one asked.
  const here = store.approvals.filter((ap) => (isLead ? store.approvalThread(ap) === thread : ap.agent_id === owner?.id));
  for (const { approval: ap, twins } of groupSame(here)) items.push({ kind: 'approval', key: `a:${ap.id}`, ts: ap.created_at, approval: ap, twins });

  // ── finished tasks (observed in this session) ──
  for (const t of store.tasks) {
    if (t.status !== 'done') continue;
    if (isLead ? store.taskThread(t.id) !== thread : t.assignee !== owner?.id) continue;
    const marks = store.taskMarks[t.id];
    if (!marks?.length) continue;
    const row: Item = { kind: 'task-done', key: `d:${t.id}`, ts: marks[marks.length - 1].ts, taskId: t.id };
    items.push(row);
    began.set(row, marks[0].ts);
  }

  items.sort((a, b) => a.ts - b.ts || RANK[a.kind] - RANK[b.kind]);
  lastOfPlan(items, began);
  uniqueKeys(items);
  // Who speaks is shown once for messages in a row (a work row between two does not count).
  let before: Item | null = null;
  for (const it of items) {
    if (it.kind === 'work') continue;
    if (it.kind === 'say') it.header = !(before?.kind === 'say' && before.agentId === it.agentId && it.ts - before.ts < 180_000);
    before = it;
  }
  return items;
}

/**
 * A finished plan's card is the last row of its plan: a notice row or a task-done card that
 * began before the plan's done message goes above the card, also when its last note or mark is
 * newer (it rides on that). Rows that began later stay below. `items`: sorted; changed in place.
 */
function lastOfPlan(items: Item[], began: Map<Item, number>) {
  for (let i = 0; i < items.length; i++) {
    const card = items[i];
    if ((card.kind !== 'system' && card.kind !== 'say') || card.card?.type !== 'plan-done') continue;
    const earlier = items.slice(i + 1).filter((it) => (began.get(it) ?? Infinity) < card.ts);
    if (!earlier.length) continue;
    const moved = new Set(earlier);
    const after = items.slice(i + 1).filter((it) => !moved.has(it));
    items.splice(i, items.length - i, ...earlier, card, ...after);
    i += earlier.length;
  }
}

/** The same value, the same list of the same objects, or an object with the same fields. */
function sameField(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a)) return Array.isArray(b) && a.length === b.length && a.every((v, i) => v === b[i]);
  const x = a as Record<string, unknown>;
  const y = b as Record<string, unknown>;
  const keys = Object.keys(x);
  return keys.length === Object.keys(y).length && keys.every((k) => x[k] === y[k]);
}

/** Both rows are being written and `field` holds their words so far. The row reads the words
 *  itself (`streamText`, `thinkingText`), so more words make no new row. */
const written = (a: Item, b: Item, field: string): boolean =>
  (field === 'text' && a.kind === 'say' && b.kind === 'say' && a.streaming && b.streaming) ||
  (field === 'thinking' && a.kind === 'work' && b.kind === 'work' && a.thinking != null && b.thinking != null);

/** Two rows that show the same thing. */
function sameRow(a: Item, b: Item): boolean {
  const x = a as Record<string, unknown>;
  const y = b as Record<string, unknown>;
  for (const k in x) if (!sameField(x[k], y[k]) && !written(a, b, k)) return false;
  return true;
}

/**
 * `next` with the rows of `prev` put back where they show the same thing, so a keyed list
 * updates only the rows that changed. When no row changed, the result is `prev` itself.
 */
export function keepRows(prev: Item[], next: Item[]): Item[] {
  const was = new Map(prev.map((it) => [it.key, it]));
  let same = prev.length === next.length;
  const rows = next.map((it, i) => {
    const old = was.get(it.key);
    const row = old && sameRow(old, it) ? old : it;
    if (row !== prev[i]) same = false;
    return row;
  });
  return same ? prev : rows;
}

/**
 * Which messages and thoughts the agent is writing right now (not their words). Read it in a
 * `$derived`: it changes when one starts or ends, which is when its thread is built again.
 */
export function openStreams(agentId: string): string {
  void store.revBy.get(agentId);
  void store.liveRevBy.get(agentId);
  let open = '';
  for (const s of store.streams.values()) if (s.agentId === agentId) open += `${s.kind} ${s.itemId} ${s.ts}\n`;
  return open;
}

/** The words of a message, as they are now. A row reads this, so a new word changes that row only. */
export function streamText(it: Item & { kind: 'say' }): string {
  if (!it.streaming || it.itemId == null) return it.text;
  void store.liveRevBy.get(it.agentId);
  return store.streams.get(it.itemId)?.text ?? it.text;
}

/** The thought the agent of a work row is writing, as it is now (null: the row shows none). */
export function thinkingText(it: Item & { kind: 'work' }): string | null {
  if (it.thinking == null) return null;
  void store.liveRevBy.get(it.agentId);
  let text = it.thinking;
  for (const s of store.streams.values()) if (s.agentId === it.agentId && s.kind === 'reasoning') text = s.text;
  return text;
}
