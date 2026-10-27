// The browser demo's stand-in for the Find and Messages lenses' commands (`exact_search`,
// `message_graph`): a small TypeScript port of crates/codly-core/src/memory/panel.rs, so the
// mock answers from the demo's own tasks, memories and chats. Never loaded inside the Tauri app.
// Keep the act keywords in step with the Rust file.
import type { Agent } from '../gen/Agent';
import type { ChatMessage } from '../gen/ChatMessage';
import type { ExactHit } from '../gen/ExactHit';
import type { GraphEdge } from '../gen/GraphEdge';
import type { GraphNode } from '../gen/GraphNode';
import type { Memory } from '../gen/Memory';
import type { MessageGraph } from '../gen/MessageGraph';
import type { MsgAct } from '../gen/MsgAct';
import type { Task } from '../gen/Task';

const CONTEXT = 48;

/** Snippet around the match, with "…" where trimmed; offsets are in UTF-16 units (the demo is ASCII). */
function snippet(text: string, at: number, len: number): Pick<ExactHit, 'snippet' | 'at' | 'len'> {
  const start = Math.max(0, at - CONTEXT);
  const end = Math.min(text.length, at + len + CONTEXT);
  const body = text.slice(start, end).replace(/\n/g, ' ');
  return { snippet: `${start > 0 ? '…' : ''}${body}${end < text.length ? '…' : ''}`, at: at - start + (start > 0 ? 1 : 0), len };
}

/** Case-insensitive literal search over tasks and memories. Title matches first, then newest. */
export function exactSearch(query: string, tasks: Task[], memories: Memory[], limit = 60): ExactHit[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const hits: { top: boolean; hit: ExactHit }[] = [];
  for (const t of tasks) {
    const fields: [string, string][] = [['title', t.title], ['key', t.key], ['goal', t.goal], ['acceptance', t.acceptance.join('\n')], ['summary', t.summary ?? ''], ['status', t.status_reason ?? '']];
    for (const [field, text] of fields) {
      const at = text.toLowerCase().indexOf(needle);
      if (at < 0) continue;
      hits.push({ top: field === 'title' || field === 'key', hit: { kind: 'task', id: t.id, title: `${t.key} · ${t.title}`, field, ...snippet(text, at, needle.length), who: t.assignee, ts: t.updated_at } });
      break;
    }
  }
  for (const m of memories) {
    if (m.superseded_by) continue;
    const at = m.text.toLowerCase().indexOf(needle);
    if (at < 0) continue;
    hits.push({ top: false, hit: { kind: m.kind === 'note' ? 'board' : 'memory', id: m.id, title: m.text.split('\n')[0].slice(0, 60), field: 'text', ...snippet(m.text, at, needle.length), who: m.author, ts: m.created_at } });
  }
  hits.sort((a, b) => Number(b.top) - Number(a.top) || b.hit.ts - a.hit.ts || (a.hit.id < b.hit.id ? -1 : 1));
  return hits.slice(0, limit).map((h) => h.hit);
}

const words = (text: string) => ` ${text.toLowerCase().replace(/’/g, "'").split(/[^\p{L}\p{N}']+/u).filter(Boolean).join(' ')} `;
const hasAny = (padded: string, phrases: string[]) => phrases.some((p) => padded.includes(` ${p} `));
const startsWith = (padded: string, ws: string[]) => ws.some((w) => padded.startsWith(` ${w} `));

/** Refuse, propose, query, agree/done, request; otherwise the default. */
export function classifyAct(text: string, fallback: MsgAct): MsgAct {
  const p = words(text);
  if (hasAny(p, ["can't", 'cannot', "won't", 'refuse', 'refused', 'decline', 'declined', 'unable', 'not possible', 'rejected', 'no way'])) return 'refuse';
  if (hasAny(p, ['i propose', 'proposal', 'i suggest', 'suggest', "let's", 'how about', 'we could', 'what if', "i'd recommend"])) return 'propose';
  if (text.trimEnd().endsWith('?') || startsWith(p, ['who', 'what', 'where', 'when', 'why', 'how', 'which', 'can', 'could', 'should', 'is', 'are', 'does'])) return 'query';
  if (hasAny(p, ['done', 'completed', 'finished', 'merged', 'agreed', 'agree', 'approved', 'lgtm', 'sounds good', 'will do', 'ok', 'okay', 'yes'])) return 'agree_done';
  if (startsWith(p, ['please', 'pls', 'make', 'add', 'fix', 'implement', 'update', 'run', 'write', 'create', 'remove', 'refactor', 'check', 'review', 'build', 'need', 'must'])) return 'request';
  return fallback;
}

const clip = (t: string, n: number) => {
  const one = t.split(/\s+/).filter(Boolean).join(' ');
  return one.length <= n ? one : `${one.slice(0, n - 1)}…`;
};

/** Agents, plus "You" and the whiteboard when they took part; one edge per (from, to, act). */
export function buildMessageGraph(agents: Agent[], chats: ChatMessage[], notes: Memory[], tasks: Task[]): MessageGraph {
  const known = (id: string) => agents.some((a) => a.id === id);
  const leadOf = (thread: string) => agents.find((a) => a.role === 'lead' && (a.thread ?? 'lead') === thread) ?? agents.find((a) => a.role === 'lead');
  const acc = new Map<string, GraphEdge>();
  let usesUser = false;
  let usesBoard = false;
  const add = (from: string, to: string, act: MsgAct, ts: number, text: string) => {
    if (from === to) return;
    const k = `${from}\u0000${to}\u0000${act}`;
    const e = acc.get(k) ?? { from, to, act, count: 0, last_ts: -Infinity, sample: '' };
    e.count += 1;
    if (ts >= e.last_ts) {
      e.last_ts = ts;
      e.sample = clip(text, 140);
    }
    acc.set(k, e);
  };

  for (const c of chats) {
    if (c.author === 'codly') continue;
    let route: { from: string; to: string; forced?: MsgAct; fallback: MsgAct } | null = null;
    if (c.thread.startsWith('agent:')) {
      const owner = c.thread.slice(6);
      route = c.author === 'user' ? { from: 'user', to: owner, fallback: 'request' } : c.author === owner ? { from: owner, to: 'user', fallback: 'inform' } : { from: c.author, to: owner, fallback: 'request' };
    } else if (c.thread === 'lead' || c.thread.startsWith('lead:')) {
      const lead = leadOf(c.thread)?.id;
      if (lead) {
        const asker = (c.attachment as { reply_to?: unknown } | null)?.reply_to;
        if (c.author === 'user') route = { from: 'user', to: lead, fallback: 'request' };
        else if (c.author === lead) route = { from: lead, to: typeof asker === 'string' ? asker : 'user', fallback: 'inform' };
        else route = { from: c.author, to: lead, forced: 'query', fallback: 'query' };
      }
    }
    if (!route) continue;
    const ok = (id: string) => id === 'user' || known(id);
    if (!ok(route.from) || !ok(route.to)) continue;
    usesUser ||= route.from === 'user' || route.to === 'user';
    add(route.from, route.to, route.forced ?? classifyAct(c.text, route.fallback), c.ts, c.text);
  }

  for (const n of notes) {
    if (n.kind !== 'note' || n.superseded_by || !known(n.author)) continue;
    const dependents = n.scope === 'task' && n.scope_id ? tasks.filter((t) => t.depends_on.includes(n.scope_id!)).map((t) => t.assignee).filter((a): a is string => !!a && a !== n.author && known(a)) : [];
    if (!dependents.length) {
      usesBoard = true;
      add(n.author, 'board', 'inform', n.created_at, n.text);
    }
    for (const d of dependents) add(n.author, d, 'inform', n.created_at, n.text);
  }

  const nodes: GraphNode[] = agents.map((a) => ({ id: a.id, label: a.name, kind: 'agent', color: a.color, title: a.title ?? null }));
  if (usesUser) nodes.push({ id: 'user', label: 'You', kind: 'user', color: null, title: null });
  if (usesBoard) nodes.push({ id: 'board', label: 'Whiteboard', kind: 'board', color: null, title: null });
  const edges = [...acc.values()].sort((a, b) => (a.from + a.to + a.act < b.from + b.to + b.act ? -1 : 1));
  return { nodes, edges };
}
