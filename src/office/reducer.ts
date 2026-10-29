// officeReducer: (model, EventRow) → model' + effects. Pure and deterministic: event
// timestamps are the only clock, so replaying the same log always produces the same office.
// Effects are one-shot visuals (errands, paper planes, confetti, a card sliding on the board)
// that the engine plays live and drops during replay; the model is the truth every label,
// pose and prop is derived from.

import type { Agent } from '../lib/gen/Agent';
import type { AgentStatus } from '../lib/gen/AgentStatus';
import type { Approval } from '../lib/gen/Approval';
import type { EventRow } from '../lib/gen/EventRow';
import type { Task } from '../lib/gen/Task';
import { columnOf, emptyModel, type AgentState, type Fx, type OfficeModel, type TaskCard, type Tone, type ToolCall } from './model';
import { basename, clean, commandHead, hash, host } from './text';
import { setChosenTraits } from './traits';

/** Excursion thresholds (design-world §9.4 rule 3): walk to a station only if still live after this. */
export const EXCURSION_MS: Partial<Record<string, number>> = {
  test: 400,
  execute: 1500,
  search: 1200,
  fetch: 400,
  plan: 400,
  review: 400,
  commit: 400,
  merge: 400,
  memory: 400,
  note: 400,
  message: 400,
};

export { emptyModel };

export interface ReduceContext {
  /** Replay: fold state only; effects other than tickers are not emitted. */
  replay: boolean;
}

function placeholder(id: string, ts: number): AgentState {
  return {
    id,
    name: 'Agent',
    provider: 'claude',
    role: 'worker',
    badge: null,
    color: hash(id) % 8,
    seed: id,
    thread: null,
    status: 'idle',
    statusAt: 0,
    taskId: null,
    present: true,
    tools: new Map(),
    focus: null,
    thinkingSince: null,
    replyingSince: null,
    compactingSince: null,
    retrying: null,
    approval: null,
    blocked: null,
    lastError: null,
    rateLimited: null,
    subagents: new Map(),
    todo: [],
    todoAt: 0,
    outcome: null,
    readBurst: { count: 0, at: 0 },
    doneAt: null,
    lastEventAt: ts,
    lastToolAt: {},
    filesTouched: 0,
    lastTool: null,
    lastToolEnd: 0,
    cheer: null,
  };
}

export function ensureAgent(m: OfficeModel, id: string, ts: number): AgentState {
  let a = m.agents.get(id);
  if (!a) {
    a = placeholder(id, ts);
    m.agents.set(id, a);
  }
  return a;
}

function copyAgent(a: AgentState, ag: Agent) {
  a.name = clean(ag.name, 32) || 'Agent';
  a.provider = ag.provider;
  a.role = ag.role;
  a.badge = clean(ag.title ?? '', 24) || (ag.consultant ? 'Consultant' : null);
  a.color = ag.color;
  a.seed = ag.look_seed || ag.id;
  a.thread = ag.thread ?? (ag.role === 'lead' ? 'lead' : null);
  a.taskId = ag.task_id;
  setChosenTraits(ag.id, ag.career?.traits ?? []);
}

/** Merge the app's roster (the `agents` prop) into the model. */
export function syncAgents(m: OfficeModel, agents: Agent[], now: number) {
  const seen = new Set<string>();
  for (const ag of agents) {
    seen.add(ag.id);
    const known = m.agents.has(ag.id);
    const a = ensureAgent(m, ag.id, now);
    copyAgent(a, ag);
    // The roster may lag the event stream: never let an older record undo a newer status.
    const stale = ag.updated_at > 0 && ag.updated_at < a.statusAt;
    if ((ag.status !== a.status || !known) && !stale) applyStatus(a, ag.status, ag.updated_at > 0 ? ag.updated_at : now);
  }
  for (const [id, a] of m.agents) {
    if (!seen.has(id) && agents.length) {
      a.present = false;
      m.agents.delete(id);
    }
  }
}

/** Merge the app's tasks (the `tasks` prop) into the model, without effects. */
export function syncTasks(m: OfficeModel, tasks: Task[]) {
  const seen = new Set<string>();
  for (const t of tasks) {
    seen.add(t.id);
    const cur = m.tasks.get(t.id);
    if (cur && t.updated_at > 0 && t.updated_at < cur.updatedAt) continue;
    m.tasks.set(t.id, cardOf(t, cur));
  }
  if (tasks.length) for (const id of [...m.tasks.keys()]) if (!seen.has(id)) m.tasks.delete(id);
}

function cardOf(t: Task, prev?: TaskCard): TaskCard {
  return {
    id: t.id,
    key: clean(t.key, 8) || 't',
    title: clean(t.title, 80) || 'Task',
    status: t.status,
    column: columnOf(t.status),
    assignee: t.assignee,
    planId: t.plan_id,
    attempt: t.attempt,
    updatedAt: Math.max(t.updated_at, prev?.updatedAt ?? 0),
    createdAt: t.created_at || prev?.createdAt || 0,
  };
}

function clearWork(a: AgentState) {
  a.tools.clear();
  a.focus = null;
  a.thinkingSince = null;
  a.replyingSince = null;
  a.compactingSince = null;
  a.retrying = null;
  if (a.approval?.id.startsWith('q:')) a.approval = null;
}

export function applyStatus(a: AgentState, status: AgentStatus, ts: number) {
  const was = a.status;
  a.status = status;
  a.statusAt = ts;
  a.present = status !== 'offline';
  if (status !== 'waiting_approval' && a.approval?.id === 'status') a.approval = null;
  if (status === 'working' || status === 'thinking') {
    a.blocked = null;
    a.lastError = null;
    a.rateLimited = null;
    a.doneAt = null;
  }
  switch (status) {
    case 'offline':
      clearWork(a);
      a.subagents.clear();
      a.approval = null;
      break;
    case 'idle':
      clearWork(a);
      a.rateLimited = null;
      break;
    case 'done':
      clearWork(a);
      a.subagents.clear();
      a.blocked = null;
      if (was !== 'done') a.doneAt = ts;
      break;
    case 'thinking':
      if (a.thinkingSince == null) a.thinkingSince = ts;
      break;
    case 'waiting_approval':
      if (!a.approval) a.approval = { id: 'status', text: '', since: ts };
      break;
    case 'blocked':
    case 'failed':
      if (!a.blocked) a.blocked = { reason: status === 'failed' ? 'failed' : 'blocked', since: ts };
      break;
    case 'rate_limited':
      if (!a.rateLimited) a.rateLimited = { since: ts, resetsAt: null };
      break;
  }
}

export function approvalText(ap: Approval): string {
  const s = ap.subject;
  switch (s.kind) {
    case 'command':
      return commandHead(s.command, 4);
    case 'file_change':
      return s.paths.length ? `edit ${clean(basename(s.paths[0]), 28)}${s.paths.length > 1 ? ` +${s.paths.length - 1}` : ''}` : 'a file change';
    case 'tool':
      return `use ${clean(s.name, 30)}`;
    case 'question':
      return 'a question';
    default:
      return 'permissions';
  }
}

/** Test counts from a verify tail ("12 passed", "1 failed", cargo's summary). Inferred. */
export function parseTestCounts(tail: string): { passed: number | null; failed: number | null } {
  const s = clean(tail, 4000);
  const cargo = /test result: \w+\. (\d+) passed; (\d+) failed/.exec(s);
  if (cargo) return { passed: +cargo[1], failed: +cargo[2] };
  const tests = /Tests:?\s+(?:(\d+) failed[,| ]+)?(\d+) passed/i.exec(s);
  if (tests) return { passed: +tests[2], failed: tests[1] ? +tests[1] : 0 };
  const p = /(\d+)\s+passed/i.exec(s);
  const f = /(\d+)\s+failed/i.exec(s);
  return { passed: p ? +p[1] : null, failed: f ? +f[1] : null };
}

export function testOutcome(passed: number | null, failed: number | null, ok: boolean): { text: string; tone: Tone; ok: boolean } {
  const pass = failed != null ? failed === 0 && ok !== false : ok;
  if (passed != null || failed != null) {
    if (!pass) return { text: `${failed ?? '?'} test${failed === 1 ? '' : 's'} failing`, tone: 'danger', ok: false };
    return { text: `Tests pass ✓ ${passed ?? 0}`, tone: 'success', ok: true };
  }
  return pass ? { text: 'Tests pass ✓', tone: 'success', ok: true } : { text: 'Tests failing', tone: 'danger', ok: false };
}

function refocus(a: AgentState) {
  let f: ToolCall | null = null;
  for (const t of a.tools.values()) f = t;
  a.focus = f;
}

function pushHistory(m: OfficeModel, ok: boolean) {
  m.testHistory.push(ok);
  if (m.testHistory.length > 12) m.testHistory.shift();
}

const nameOf = (m: OfficeModel, id: string | null) => (id ? (m.agents.get(id)?.name ?? 'Agent') : 'Codly');

/** The Lead an agent reports to: its task's plan thread, else the first Lead around. */
export function leadOf(m: OfficeModel, agentId: string): AgentState | null {
  const a = m.agents.get(agentId);
  if (!a || a.role === 'lead') return null;
  const task = a.taskId ? m.tasks.get(a.taskId) : null;
  const thread = task?.planId ? m.plans.get(task.planId)?.thread : null;
  let first: AgentState | null = null;
  for (const l of m.agents.values()) {
    if (l.role !== 'lead' || !l.present) continue;
    if (thread && l.thread === thread) return l;
    first ??= l;
  }
  return first;
}

const isAsk = (t: ToolCall) => /ask_lead|SendMessage|send_input/i.test(t.name) || t.kind === 'message';

/**
 * Fold one event. Returns the effects to play (empty in replay, except tickers).
 */
export function reduce(m: OfficeModel, ev: EventRow, ctx: ReduceContext): Fx[] {
  const out: Fx[] = [];
  const at = ev.ts;
  const k = ev.kind;
  if (ev.id > 0) m.lastEventId = Math.max(m.lastEventId, ev.id);
  const aid = ev.agent_id;
  const a = aid ? ensureAgent(m, aid, at) : null;
  if (a && k.type !== 'usage_updated') a.lastEventAt = Math.max(a.lastEventAt, at);
  const fx = (f: Fx) => {
    if (!ctx.replay || f.t === 'ticker') out.push(f);
  };
  const tick = (text: string, code: string | null = null, tone: Tone = 'normal', who: string | null = aid) =>
    fx({ t: 'ticker', agentId: who, text: `${nameOf(m, who)} · ${text}`, code, tone, at, eventId: ev.id });
  const recently = (ag: AgentState, kind: keyof AgentState['lastToolAt'], ms: number) => at - (ag.lastToolAt[kind] ?? -Infinity) < ms;

  switch (k.type) {
    case 'session_started':
      if (a) {
        a.provider = k.provider;
        a.present = true;
      }
      break;
    case 'turn_started':
      if (a) {
        a.thinkingSince ??= at;
        a.doneAt = null;
        a.outcome = null;
      }
      break;
    case 'status_changed':
      if (a) applyStatus(a, k.status, at);
      break;
    case 'text_delta':
      if (a) a.replyingSince ??= at;
      break;
    case 'reasoning_delta':
    case 'reasoning':
      if (a) a.thinkingSince ??= at;
      break;
    case 'message':
      if (a) {
        a.replyingSince = null;
        if (k.phase === 'final') a.thinkingSince = null;
      }
      break;
    case 'tool_started': {
      if (!a || !aid) break;
      const t = k.target;
      const call: ToolCall = {
        callId: k.call_id,
        kind: k.activity,
        name: clean(k.name, 60),
        path: t.path ?? t.paths?.[0] ?? null,
        paths: t.paths ?? [],
        command: t.command ?? null,
        query: t.query ?? null,
        url: t.url ?? null,
        tool: t.tool ?? null,
        server: t.server ?? null,
        since: at,
      };
      a.tools.set(k.call_id, call);
      a.focus = call;
      // A running tool is proof of work, whatever the last status said.
      if (a.status === 'idle' || a.status === 'done' || a.status === 'starting') {
        a.status = 'working';
        a.present = true;
        a.doneAt = null;
      }
      a.thinkingSince = null;
      a.replyingSince = null;
      a.outcome = null;
      a.blocked = null;
      a.lastToolAt[call.kind] = at;
      const file = call.path ? clean(basename(call.path), 28) : null;
      switch (call.kind) {
        case 'read':
          a.readBurst = at - a.readBurst.at < 3000 ? { count: a.readBurst.count + 1, at } : { count: 1, at };
          tick('Reading', file);
          break;
        case 'search':
          tick('Searching', call.query ? `“${clean(call.query, 24)}”` : file);
          break;
        case 'fetch':
          tick('Reading', call.url ? host(call.url) : file);
          break;
        case 'edit':
        case 'create':
        case 'delete':
          a.filesTouched += Math.max(1, call.paths.length);
          if (call.kind === 'create') fx({ t: 'sparkle', agentId: aid, at });
          if (call.kind === 'delete') fx({ t: 'trash', agentId: aid, at });
          tick(call.kind === 'create' ? 'Creating' : call.kind === 'delete' ? 'Deleting' : 'Editing', file);
          break;
        case 'execute':
        case 'test':
          tick('Running', call.command ? commandHead(call.command) : call.kind === 'test' ? 'tests' : null);
          break;
        case 'note':
          tick('Posting a note');
          break;
        case 'message': {
          const lead = leadOf(m, aid);
          fx({ t: 'plane', agentId: aid, to: lead?.id ?? null, at });
          tick(/ask_lead/.test(call.name) ? 'Asking the Lead' : 'Sending a message');
          break;
        }
        case 'memory':
          tick('Saving a memory');
          break;
        case 'plan':
          tick('Updating the plan');
          break;
        case 'mcp':
          tick('Asking', clean(call.server ?? call.tool ?? call.name, 24));
          break;
        case 'review':
          tick('Reviewing');
          break;
        case 'delegate':
          tick('Briefing a helper');
          break;
        case 'await_approval':
          a.approval = { id: 'q:' + call.callId, text: call.query ? clean(call.query, 60) : 'a question', since: at };
          tick('Waiting for you:', a.approval.text, 'warn');
          break;
      }
      break;
    }
    case 'tool_completed': {
      if (!a || !aid) break;
      const call = a.tools.get(k.call_id) ?? null;
      a.tools.delete(k.call_id);
      refocus(a);
      if (call) {
        a.lastTool = call;
        a.lastToolEnd = at;
      }
      if (a.approval?.id === 'q:' + k.call_id) a.approval = null;
      const isTest = call?.kind === 'test' || k.tests_passed != null || k.tests_failed != null;
      if (isTest) {
        const o = testOutcome(k.tests_passed, k.tests_failed, k.ok);
        m.tests = { ok: o.ok, passed: k.tests_passed, failed: k.tests_failed, at, agentId: aid, command: call?.command ? commandHead(call.command, 3) : 'tests' };
        pushHistory(m, o.ok);
        a.outcome = { text: o.text, code: null, tone: o.tone, at };
        a.cheer = { ok: o.ok, at, atMonitor: !!call && at - call.since >= (EXCURSION_MS.test ?? 400) };
        fx({ t: 'tests', agentId: aid, ok: o.ok, at });
        tick(o.text, null, o.tone);
      } else if (call?.kind === 'execute') {
        if (!k.ok) {
          a.outcome = { text: `✕ exit ${k.exit_code ?? 1}`, code: call.command ? commandHead(call.command) : null, tone: 'danger', at };
          tick('Command failed', call.command ? commandHead(call.command) : null, 'danger');
        } else if (at - call.since > 3000) a.outcome = { text: '✓ exit 0', code: call.command ? commandHead(call.command) : null, tone: 'success', at };
      }
      if (!a.tools.size && a.status !== 'idle' && a.status !== 'done') a.thinkingSince = at;
      break;
    }
    case 'files_changed':
      break;
    case 'plan_updated':
      if (a) {
        a.todo = k.steps.slice(0, 12).map((s) => ({ text: clean(s.text, 60), done: s.status === 'completed', active: s.status === 'in_progress' }));
        a.todoAt = at;
      }
      break;
    case 'approval_requested': {
      const ap = k.approval;
      const who = ap.agent_id || aid;
      if (!who || m.resolved.has(ap.id)) break;
      const ag = ensureAgent(m, who, at);
      const text = approvalText(ap);
      ag.approval = { id: ap.id, text, since: at };
      m.approvals.set(ap.id, { agentId: who, text, since: at });
      tick('Waiting for you:', text, 'warn', who);
      break;
    }
    case 'approval_resolved': {
      m.resolved.add(k.approval_id);
      const ap = m.approvals.get(k.approval_id);
      m.approvals.delete(k.approval_id);
      const who = ap?.agentId ?? aid;
      const ag = who ? m.agents.get(who) : null;
      if (ag?.approval && (ag.approval.id === k.approval_id || ag.approval.id === 'status')) ag.approval = null;
      if (ag && k.status === 'denied') ag.outcome = { text: 'Denied', code: ap?.text ?? null, tone: 'muted', at };
      break;
    }
    case 'rate_limit':
      if (a) {
        const hit = k.status === 'rejected' || (k.used_fraction ?? 0) >= 1;
        if (hit) {
          a.rateLimited = { since: a.rateLimited?.since ?? at, resetsAt: k.resets_at };
          tick('Out of quota', null, 'muted');
        } else if (k.status === 'allowed' && a.rateLimited && a.status !== 'rate_limited') a.rateLimited = null;
      }
      break;
    case 'retrying':
      if (a) a.retrying = { attempt: k.attempt, since: at };
      break;
    case 'sub_agent':
      if (a) {
        const finished = /complet|fail|stop|kill|cancel|error|done|finish/i.test(k.status);
        if (finished) a.subagents.delete(k.call_id);
        else {
          const prev = a.subagents.get(k.call_id);
          a.subagents.set(k.call_id, { callId: k.call_id, description: clean(k.description, 80) || prev?.description || 'helper', since: prev?.since ?? at });
          if (!prev) tick('Started a helper', clean(k.description, 36) || null);
        }
      }
      break;
    case 'compacting':
      if (a) {
        a.compactingSince = k.post_tokens == null ? at : null;
        if (k.post_tokens != null && k.pre_tokens != null)
          a.outcome = { text: `${Math.round(k.pre_tokens / 1000)}k → ${Math.round(k.post_tokens / 1000)}k tokens`, code: null, tone: 'muted', at };
      }
      break;
    case 'turn_completed':
      if (a) {
        clearWork(a);
        if (k.status === 'failed') a.blocked = { reason: 'turn failed', since: at };
      }
      break;
    case 'error':
      if (a && k.fatal) {
        a.blocked = { reason: clean(k.message, 60) || 'error', since: at };
        a.lastError = a.blocked.reason;
        tick('Stuck:', clean(k.message, 36), 'danger');
      }
      break;
    case 'exited':
      if (a) clearWork(a);
      break;
    case 'agent_changed': {
      const ag = ensureAgent(m, k.agent.id, at);
      copyAgent(ag, k.agent);
      if (k.agent.status !== ag.status) applyStatus(ag, k.agent.status, at);
      break;
    }
    case 'plan_changed':
      m.plans.set(k.plan.id, { thread: k.plan.thread || 'lead', request: clean(k.plan.request, 120), status: k.plan.status, at });
      break;
    case 'task_changed': {
      const t = k.task;
      const prev = m.tasks.get(t.id);
      const card = cardOf(t, prev);
      card.updatedAt = Math.max(card.updatedAt, at);
      m.tasks.set(t.id, card);
      const from = prev?.column ?? null;
      if (from !== card.column) {
        fx({ t: 'card', taskId: t.id, from, to: card.column, at });
        const who = card.assignee;
        if (who && card.status === 'running' && from !== 'doing') fx({ t: 'errand', agentId: who, kind: 'card-take', taskId: t.id, at });
        if (who && card.column === 'review' && from === 'doing') fx({ t: 'errand', agentId: who, kind: 'card-return', taskId: t.id, at });
        if (card.column === 'done' && card.status === 'done') tick(`Task ${card.key} done`, null, 'success', who);
      }
      break;
    }
    case 'chat': {
      const msg = k.message;
      const author = m.agents.get(msg.author);
      if (author && author.role !== 'lead' && /^lead(:|$)/.test(msg.thread) && !recently(author, 'message', 4000)) {
        author.lastToolAt.message = at;
        const lead = leadOf(m, author.id);
        fx({ t: 'plane', agentId: author.id, to: lead?.id ?? null, at });
      }
      break;
    }
    case 'retrieval':
      fx({ t: 'shelf', at });
      break;
    case 'memory_added': {
      const mem = k.memory;
      const author = m.agents.get(mem.author) ?? null;
      if (mem.kind === 'note') {
        m.notes.push({ id: mem.id, text: clean(mem.text, 140), author: mem.author, at });
        if (m.notes.length > 40) m.notes.splice(0, m.notes.length - 40);
        fx({ t: 'sticky', noteId: mem.id, agentId: author?.id ?? null, at });
        if (author && !recently(author, 'note', 6000)) fx({ t: 'errand', agentId: author.id, kind: 'note', taskId: ev.task_id, at });
        tick('Posted a note', clean(mem.text, 40), 'normal', author?.id ?? aid);
      } else {
        m.memories.push({ id: mem.id, author: mem.author, at });
        if (m.memories.length > 200) m.memories.splice(0, m.memories.length - 200);
        fx({ t: 'drawer', agentId: author?.id ?? null, at });
        if (author && !recently(author, 'memory', 6000)) fx({ t: 'errand', agentId: author.id, kind: 'memory', taskId: ev.task_id, at });
        tick('Remembered', clean(mem.text, 44), 'normal', author?.id ?? aid);
      }
      break;
    }
    case 'triple_added':
      m.triples++;
      fx({ t: 'catalog', at });
      break;
    case 'committed': {
      m.commits.push({ sha: clean(k.sha, 12), agentId: aid, taskId: ev.task_id, at });
      if (m.commits.length > 100) m.commits.splice(0, m.commits.length - 100);
      fx({ t: 'mail', agentId: aid, at });
      if (aid && a && !recently(a, 'commit', 6000)) fx({ t: 'errand', agentId: aid, kind: 'commit', taskId: ev.task_id, at });
      const n = k.files.length;
      tick(`Committed ${n} file${n === 1 ? '' : 's'}`, clean(k.sha, 7), 'normal');
      break;
    }
    case 'verified': {
      const counts = parseTestCounts(k.tail);
      const o = testOutcome(counts.passed, counts.failed, k.ok);
      m.tests = { ok: k.ok, passed: counts.passed, failed: counts.failed, at, agentId: aid, command: commandHead(k.command, 3) };
      pushHistory(m, k.ok);
      fx({ t: 'tests', agentId: null, ok: k.ok, at });
      if (a) {
        a.outcome = { text: k.ok ? 'Verified ✓' : 'Verify failed', code: commandHead(k.command), tone: k.ok ? 'success' : 'danger', at };
        a.cheer = { ok: k.ok, at, atMonitor: false };
      }
      tick(k.ok ? 'Verified' : 'Verify failed', o.ok && counts.passed != null ? `${counts.passed} passed` : commandHead(k.command), k.ok ? 'success' : 'danger');
      break;
    }
    case 'reviewed':
      if (a) {
        const ok = /approve|pass|lgtm|accept/i.test(k.verdict);
        a.outcome = { text: ok ? 'Review: approved ✓' : `Review: ${clean(k.verdict, 24).replace(/_/g, ' ')}`, code: null, tone: ok ? 'success' : 'warn', at };
        tick(a.outcome.text, null, a.outcome.tone);
      }
      break;
    case 'merged':
      m.merges.push({ taskId: ev.task_id, agentId: aid, at, branch: clean(k.branch, 60) });
      if (m.merges.length > 100) m.merges.splice(0, m.merges.length - 100);
      fx({ t: 'merge', agentId: aid, taskId: ev.task_id, at });
      if (aid && a) {
        a.outcome = { text: 'Merged ✓', code: null, tone: 'success', at };
        if (!recently(a, 'merge', 6000)) fx({ t: 'errand', agentId: aid, kind: 'merge', taskId: ev.task_id, at });
      }
      tick('Merged into', clean(k.branch.replace(/^codly\//, ''), 36), 'success');
      break;
    case 'artifact_added': {
      const art = k.artifact;
      m.artifacts.set(art.id, art);
      fx({ t: 'artifact', artifactId: art.id, at });
      tick('Made', clean(art.title || basename(art.path), 36), 'normal', art.agent_id ?? aid);
      break;
    }
  }
  return out;
}
