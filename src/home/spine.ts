// The conversation spine: everything important lands in the thread as an item, in order.
// A request thread ("lead" / "lead:<id>") = your messages, its Lead's replies and (folded) work,
// approvals from the agents on its plan where they happened, questions they asked its Lead,
// task-done and plan-done cards. A 1:1 thread = the same for one agent.
// Each request is its own thread: another request's plan, approvals and cards never show here.
//
// The list is built again only when something of this thread changed (`buildSpine`), and a row
// that did not change keeps its object (`keepRows`), so the screen updates that row alone. A
// message or a thought being written does not build the list again on each new word: its row
// reads the words as they are now (`streamText`, `thinkingText`).
import type { Approval } from '../lib/gen/Approval';
import type { ChatMessage } from '../lib/gen/ChatMessage';
import type { MessagePhase } from '../lib/gen/MessagePhase';
import type { JsonValue } from '../lib/gen/serde_json/JsonValue';
import { store, type Outgoing, type Seg, type SubAgent, type Thought } from '../lib/state/store.svelte';
import { groupSame } from '../lib/state/approvalGroups';
import { isSlotNote } from '../lib/state/queue';
import { stoppedByYou } from '../lib/state/stops';
import { isRequestThread } from '../lib/state/ui.svelte';
import { canRetry, messageOf, troublesOf, type TroubleKind } from './outcomes';

export type Item =
  | { kind: 'user'; key: string; ts: number; msg: ChatMessage; outgoing: Outgoing | null }
  | { kind: 'system'; key: string; ts: number; text: string; card: { type: 'plan' | 'plan-done'; planId: string } | null }
  | {
      kind: 'say';
      key: string;
      ts: number;
      agentId: string;
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
    };

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

export function attachmentOf(att: JsonValue | null | undefined): Record<string, unknown> | null {
  return att && typeof att === 'object' && !Array.isArray(att) ? (att as Record<string, unknown>) : null;
}
export const planIdOf = (att: JsonValue | null | undefined): string | null => {
  const a = attachmentOf(att);
  return a && typeof a.plan_id === 'string' ? a.plan_id : null;
};

// Same millisecond: a message's ts is when it finished, a work row's when its first call
// started, so the message comes first.
const RANK: Record<Item['kind'], number> = { user: 0, system: 1, say: 2, ask: 2, work: 3, trouble: 4, approval: 5, 'task-done': 6 };

export function buildSpine(thread: string): Item[] {
  const owner = store.threadAgent(thread);
  const isLead = isRequestThread(thread);
  const items: Item[] = [];

  // ── persisted chat ──
  const chats = store.chats.filter((c) => c.thread === thread);
  // A message keeps one key from its first streamed word to its final chat row (no remount).
  const itemOf = new Map<string, string>();
  if (owner) for (const m of store.live[owner.id]?.messages ?? []) itemOf.set(normOf(m), m.itemId);
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
      const itemId = m.author === owner?.id ? itemOf.get(normOf(m)) : undefined;
      items.push({
        kind: 'say', key: itemId ? SAY + itemId : `c:${m.id}`, ts: m.ts, agentId: m.author, text: m.text, phase: 'final', streaming: false, header: true, card,
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
    const chatTexts = new Set(chats.filter((c) => c.author === owner.id).map(normOf));
    type W = { ts: number; seg?: Seg; thought?: Thought };
    const work: W[] = [];
    if (live) {
      for (const m of live.messages) {
        if (chatTexts.has(normOf(m))) continue;
        items.push({ kind: 'say', key: SAY + m.itemId, ts: m.ts, agentId: owner.id, text: m.text, phase: m.phase, streaming: false, header: true, card: null, replyTo: null });
      }
      for (const s of live.segments) work.push({ ts: s.start, seg: s });
      for (const r of live.reasoning) work.push({ ts: r.ts, thought: r });
    }
    let thinking: string | null = null;
    let thinkingTs = 0;
    for (const s of store.streams.values()) {
      if (s.agentId !== owner.id) continue;
      if (s.kind === 'text') items.push({ kind: 'say', key: SAY + s.itemId, ts: s.ts, agentId: owner.id, text: s.text, phase: 'unknown', streaming: true, header: true, card: null, replyTo: null });
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
    for (const t of troublesOf(store.events, owner.id, (start) => stoppedByYou(owner.id, start)))
      items.push({
        kind: 'trouble', key: `e:${t.eventId}`, ts: t.ts, agentId: owner.id, eventId: t.eventId, what: t.what, reason: t.reason,
        retry: canRetry(t.what) ? messageOf(t.turn, yours) : null,
      });
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
    items.push({ kind: 'task-done', key: `d:${t.id}`, ts: marks[marks.length - 1].ts, taskId: t.id });
  }

  items.sort((a, b) => a.ts - b.ts || RANK[a.kind] - RANK[b.kind]);
  // Who speaks is shown once for messages in a row (a work row between two does not count).
  let before: Item | null = null;
  for (const it of items) {
    if (it.kind === 'work') continue;
    if (it.kind === 'say') it.header = !(before?.kind === 'say' && before.agentId === it.agentId && it.ts - before.ts < 180_000);
    before = it;
  }
  return items;
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
  if (!it.streaming) return it.text;
  void store.liveRevBy.get(it.agentId);
  return store.streams.get(it.key.slice(SAY.length))?.text ?? it.text;
}

/** The thought the agent of a work row is writing, as it is now (null: the row shows none). */
export function thinkingText(it: Item & { kind: 'work' }): string | null {
  if (it.thinking == null) return null;
  void store.liveRevBy.get(it.agentId);
  let text = it.thinking;
  for (const s of store.streams.values()) if (s.agentId === it.agentId && s.kind === 'reasoning') text = s.text;
  return text;
}
