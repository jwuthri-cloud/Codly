// The conversation spine: everything important lands in the thread as an item, in order.
// A request thread ("lead" / "lead:<id>") = your messages, its Lead's replies and (folded) work,
// approvals from the agents on its plan where they happened, questions they asked its Lead,
// task-done and plan-done cards. A 1:1 thread = the same for one agent.
// Each request is its own thread: another request's plan, approvals and cards never show here.
import type { Approval } from '../lib/gen/Approval';
import type { ChatMessage } from '../lib/gen/ChatMessage';
import type { MessagePhase } from '../lib/gen/MessagePhase';
import type { JsonValue } from '../lib/gen/serde_json/JsonValue';
import { store, type Outgoing, type Seg, type Thought } from '../lib/state/store.svelte';
import { isSlotNote } from '../lib/state/queue';
import { isRequestThread } from '../lib/state/ui.svelte';

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
      streaming: boolean;
      /** Which card rides under this message. */
      card: { type: 'plan' | 'plan-done'; planId: string } | null;
      replyTo: string | null;
    }
  | { kind: 'work'; key: string; ts: number; agentId: string; segs: Seg[]; thoughts: Thought[]; thinking: string | null }
  | { kind: 'approval'; key: string; ts: number; approval: Approval }
  | { kind: 'ask'; key: string; ts: number; agentId: string; msg: ChatMessage }
  | { kind: 'task-done'; key: string; ts: number; taskId: string };

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();

export function attachmentOf(att: JsonValue | null | undefined): Record<string, unknown> | null {
  return att && typeof att === 'object' && !Array.isArray(att) ? (att as Record<string, unknown>) : null;
}
export const planIdOf = (att: JsonValue | null | undefined): string | null => {
  const a = attachmentOf(att);
  return a && typeof a.plan_id === 'string' ? a.plan_id : null;
};

// Same millisecond: a message's ts is when it finished, a work row's when its first call
// started, so the message comes first.
const RANK: Record<Item['kind'], number> = { user: 0, system: 1, say: 2, ask: 2, work: 3, approval: 4, 'task-done': 5 };

export function buildSpine(thread: string): Item[] {
  const owner = store.threadAgent(thread);
  const isLead = isRequestThread(thread);
  const items: Item[] = [];

  // ── persisted chat ──
  const chats = store.chats.filter((c) => c.thread === thread);
  // A message keeps one key from its first streamed word to its final chat row (no remount).
  const itemOf = new Map<string, string>();
  if (owner) for (const m of store.live[owner.id]?.messages ?? []) itemOf.set(norm(m.text), m.itemId);
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
      const itemId = m.author === owner?.id ? itemOf.get(norm(m.text)) : undefined;
      items.push({
        kind: 'say', key: itemId ? `say:${itemId}` : `c:${m.id}`, ts: m.ts, agentId: m.author, text: m.text, phase: 'final', streaming: false, card,
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
    const chatTexts = new Set(chats.filter((c) => c.author === owner.id).map((c) => norm(c.text)));
    type W = { ts: number; seg?: Seg; thought?: Thought };
    const work: W[] = [];
    if (live) {
      for (const m of live.messages) {
        if (chatTexts.has(norm(m.text))) continue;
        items.push({ kind: 'say', key: `say:${m.itemId}`, ts: m.ts, agentId: owner.id, text: m.text, phase: m.phase, streaming: false, card: null, replyTo: null });
      }
      for (const s of live.segments) work.push({ ts: s.start, seg: s });
      for (const r of live.reasoning) work.push({ ts: r.ts, thought: r });
    }
    let thinking: string | null = null;
    let thinkingTs = 0;
    for (const s of store.streams.values()) {
      if (s.agentId !== owner.id) continue;
      if (s.kind === 'text') items.push({ kind: 'say', key: `say:${s.itemId}`, ts: s.ts, agentId: owner.id, text: s.text, phase: 'unknown', streaming: true, card: null, replyTo: null });
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
    for (const w of work) {
      if (!group || w.ts >= groupEnd) {
        groupEnd = anchors.find((t) => t > w.ts) ?? Infinity;
        group = { kind: 'work', key: `w:${w.seg?.callId ?? w.thought?.itemId}`, ts: w.ts, agentId: owner.id, segs: [], thoughts: [], thinking: null };
        items.push(group);
      }
      if (w.seg) group.segs.push(w.seg);
      if (w.thought) group.thoughts.push(w.thought);
    }
    if (thinking != null) {
      const last = items.filter((i): i is Item & { kind: 'work' } => i.kind === 'work').sort((a, b) => b.ts - a.ts)[0];
      const lastAnchor = anchors[anchors.length - 1] ?? 0;
      if (last && last.ts > lastAnchor) last.thinking = thinking;
      else items.push({ kind: 'work', key: `t:live:${owner.id}`, ts: thinkingTs, agentId: owner.id, segs: [], thoughts: [], thinking });
    }
  }

  // ── approvals, where they happened (a worker's land in its plan's request) ──
  for (const ap of store.approvals) {
    if (isLead ? store.approvalThread(ap) !== thread : ap.agent_id !== owner?.id) continue;
    items.push({ kind: 'approval', key: `a:${ap.id}`, ts: ap.created_at, approval: ap });
  }

  // ── finished tasks (observed in this session) ──
  for (const t of store.tasks) {
    if (t.status !== 'done') continue;
    if (isLead ? store.taskThread(t.id) !== thread : t.assignee !== owner?.id) continue;
    const marks = store.taskMarks[t.id];
    if (!marks?.length) continue;
    items.push({ kind: 'task-done', key: `d:${t.id}`, ts: marks[marks.length - 1].ts, taskId: t.id });
  }

  items.sort((a, b) => a.ts - b.ts || RANK[a.kind] - RANK[b.kind]);
  return items;
}
