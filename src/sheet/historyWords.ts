// Words for rows of an agent's History that are built from ids or lists: a fact of the knowledge
// graph, a task that changed, a helper, a change of what runs in the background, a compaction,
// a tool call, a decision as a sentence. Which rows of a task say something new, and which rows
// are one act said once (a send-back, an answer to a request, a tool call from its start to its
// end). And what a row needs to know about the agent's other events: which calls showed a plan,
// which conversation rows only repeat a reply, which calls were a helper's, how each call
// ended, what ran in the background before, which reconnects were restarts.
// Pure, so the sheet and the tests agree; the sheet passes the names.
import { CODLY_TOOL } from '../lib/narrate';
import type { Approval } from '../lib/gen/Approval';
import type { ApprovalRule } from '../lib/gen/ApprovalRule';
import type { EventKind } from '../lib/gen/EventKind';
import type { EventRow } from '../lib/gen/EventRow';
import type { Provider } from '../lib/gen/Provider';
import type { Task } from '../lib/gen/Task';
import type { Triple } from '../lib/gen/Triple';
import type { JsonValue } from '../lib/gen/serde_json/JsonValue';
import { dur, plural, tokens } from '../lib/util/format';
import { pastVerb, phrase, subagentMark, TASK_STATUS_LABEL, targetLabel } from '../lib/util/labels';
import { askedLine } from '../lib/util/outcome';
import { isPlanCall, planStep, showsPlan } from '../lib/util/planRequest';
import { oneLine } from '../lib/util/sanitize';
import { CODLY_SERVER, mcpToolOf } from '../lib/util/toolWords';

/** How the sheet looks up what an id is called: an agent's name, a task's key ("t1"). */
export type Names = { agent: (id: string) => string | undefined; task: (id: string) => string | undefined };

/**
 * A graph entity ("agent:<id>", "task:<id>", "file:<path>", "concept:<name>") as a person reads
 * it. An agent or a task that is not known any more is "an agent" / "a task", never its raw id.
 */
export function entityWord(entity: string, names: Names): string {
  const at = entity.indexOf(':');
  if (at < 0) return entity;
  const type = entity.slice(0, at);
  const value = entity.slice(at + 1);
  if (type === 'agent') return names.agent(value) ?? 'an agent';
  if (type === 'task') return names.task(value) ?? 'a task';
  return value || entity;
}

/** "Ada created src/theme/ThemeProvider.tsx" */
export function tripleLine(t: Pick<Triple, 'subject' | 'predicate' | 'object'>, names: Names): string {
  return `${entityWord(t.subject, names)} ${t.predicate.replace(/_/g, ' ')} ${entityWord(t.object, names)}`;
}

/** What the History rows of an agent need to know about its other events. */
export type HistoryFacts = {
  /** The calls that showed you a plan (the plan tool): their rows say so, and a plan you sent back is no failed call. */
  planCalls: Set<string>;
  /** The agent's own messages. A reply is also the row of the conversation it went to (`chat`): History lists it once. */
  said: Set<string>;
  /** Its helpers, by the id of the call that started each: the CLI's last word for where each stands. */
  helpers: Map<string, string>;
  /** A call a helper made of its own → the id of the call that started that helper. */
  helperOf: Map<string, string>;
  /** By the id of a `background_tasks` row: the ids of what the list before it in the same run named (null: none is at hand). */
  backgroundBefore: Map<number, ReadonlySet<string> | null>;
  /** How each call ended, by its id: its row says it, in the past, from its start. */
  ends: Map<string, ToolCompleted>;
  /** What each call was, by its id: the row of an end whose start is not shown says it. */
  starts: Map<string, ToolStarted>;
  /** The ids of the records of a reconnect that asked a connected server to start again: a restart (`restartLine`). */
  restarts: Set<number>;
};

/** `provider`: the agent's CLI (none when Codly does not know the agent): only a Claude agent's call of the plan tool shows a plan. */
export function historyFacts(events: readonly EventRow[], agentId: string, provider: Provider | null = null): HistoryFacts {
  const facts: HistoryFacts = { planCalls: new Set(), said: new Set(), helpers: new Map(), helperOf: new Map(), backgroundBefore: new Map(), ends: new Map(), starts: new Map(), restarts: new Set() };
  /** The ids of the last list of background work of each run. */
  const listed = new Map<string | null, ReadonlySet<string>>();
  /** Where each server of each run stood last ("<run> <server>" → its status). */
  const servers = new Map<string, string>();
  for (const e of events) {
    if (e.agent_id !== agentId) continue;
    const k = e.kind;
    if (k.type === 'tool_started') {
      facts.starts.set(k.call_id, k);
      if (isPlanCall(k.name, provider)) facts.planCalls.add(k.call_id);
    } else if (k.type === 'tool_completed') facts.ends.set(k.call_id, k);
    else if (k.type === 'mcp_server') servers.set(`${e.run_id ?? ''} ${k.name}`, k.status);
    else if (k.type === 'decision') {
      const asked = reconnectAsked(k.summary);
      if (asked && servers.get(`${e.run_id ?? ''} ${asked.server}`) === 'connected') facts.restarts.add(e.id);
    } else if (k.type === 'message') facts.said.add(k.text);
    else if (k.type === 'sub_agent') facts.helpers.set(k.call_id, k.status);
    else if (k.type === 'sub_agent_progress' && k.tool_call_id != null) facts.helperOf.set(k.tool_call_id, k.call_id);
    else if (k.type === 'background_tasks') {
      facts.backgroundBefore.set(e.id, listed.get(e.run_id) ?? null);
      listed.set(e.run_id, new Set(k.tasks.map((t) => t.id)));
    }
  }
  return facts;
}

/** The CLI's words for a helper that did not finish because it was stopped (`helperEndWord`, helperRows.ts). */
const STOPPED = ['stopped', 'killed'];

/**
 * A call that ended without a result because its helper was stopped: "helper" for the call
 * that started the helper (the helper's own row says that it stopped, so the call's end is no
 * row), "own" for a call the helper made (it stopped with its helper: it did not fail). Null
 * for every other call.
 */
export function stoppedWithHelper(callId: string, facts: Pick<HistoryFacts, 'helpers' | 'helperOf'>): 'helper' | 'own' | null {
  if (STOPPED.includes(facts.helpers.get(callId) ?? '')) return 'helper';
  const helper = facts.helperOf.get(callId);
  return helper != null && STOPPED.includes(facts.helpers.get(helper) ?? '') ? 'own' : null;
}

/** Whether a `chat` row only repeats a message of the agent whose history this is. */
export function repeatsReply(e: EventRow, agentId: string, facts: HistoryFacts): boolean {
  return e.kind.type === 'chat' && e.kind.message.author === agentId && facts.said.has(e.kind.message.text);
}

/** "Task t1 is now In review" */
export function taskChangedLine(task: Pick<Task, 'key' | 'status'>): string {
  return `Task ${task.key} is now ${TASK_STATUS_LABEL[task.status]}`;
}

/**
 * History rows (newest first) without the `task_changed` rows that say nothing new. The core
 * writes one for every update of a task (a new assignee, a branch, a summary), and the row
 * says only the status: a row shows when its status is not the one of that task's row before
 * it. The oldest row of a task in the list always shows.
 */
export function withoutRepeatedTaskRows<T extends { kind: EventKind }>(rows: T[]): T[] {
  const last = new Map<string, string>();
  const keep: T[] = [];
  for (let i = rows.length - 1; i >= 0; i--) {
    const k = rows[i].kind;
    if (k.type === 'task_changed') {
      const same = last.get(k.task.id) === k.task.status;
      last.set(k.task.id, k.task.status);
      if (same) continue;
    }
    keep.push(rows[i]);
  }
  return keep.reverse();
}

type SubAgentRow = Extract<EventKind, { type: 'sub_agent' }>;
type ToolStarted = Extract<EventKind, { type: 'tool_started' }>;
type ToolCompleted = Extract<EventKind, { type: 'tool_completed' }>;
type DecisionRow = Extract<EventKind, { type: 'decision' }>;
type BackgroundTasks = Extract<EventKind, { type: 'background_tasks' }>;
type Compacting = Extract<EventKind, { type: 'compacting' }>;

/** "Helper completed: Run the tests": a helper (subagent) with its CLI's word for where it stands. One word for it on every screen: "helper". */
export function helperRowLine(k: Pick<SubAgentRow, 'description' | 'status'>): string {
  return `Helper ${oneLine(k.status, 24)}: ${oneLine(k.description, 90)}`;
}

/** "Background command": what a piece of background work that is no helper is called in a row. */
const backgroundWord = (kind: string) => (kind === 'command' ? 'Background command' : 'Background task');

/**
 * One change of what an agent runs in the background, as its CLI lists it: what ended, with the
 * CLI's word for it, then what still runs. "Background command stopped: npm run dev · still
 * runs: npm test"; with nothing ended, "Runs in the background: npm run dev".
 *
 * The end of a helper is not said here: its own row says it ("Helper stopped: Run the tests",
 * the adapter writes one for every helper that leaves the list), and History says a thing once.
 * Null: the list has nothing left to say (all that ended were helpers, and nothing started
 * with it: `before`, the ids of the list before it in the same run, says what is new), so it
 * is no row.
 */
export function backgroundLine(k: Pick<BackgroundTasks, 'tasks' | 'ended'>, before: ReadonlySet<string> | null = null): string | null {
  const runs = oneLine(k.tasks.map((t) => t.description).join(', '), 120);
  if (!k.ended.length) return k.tasks.length ? `Runs in the background: ${runs}` : 'Nothing runs in the background';
  const others = k.ended.filter((e) => e.kind !== 'helper');
  if (!others.length) {
    // (what started in the same list is new: `before`, the list before it, did not name it)
    const fresh = before ? k.tasks.filter((t) => !before.has(t.id)) : [];
    return fresh.length ? `Runs in the background: ${oneLine(fresh.map((t) => t.description).join(', '), 120)}` : null;
  }
  const ended = others.map((e) => `${backgroundWord(e.kind)} ${e.status}: ${oneLine(e.description, 90)}`).join('; ');
  return k.tasks.length ? `${ended} · still runs: ${runs}` : ended;
}

/**
 * A row of a compaction, by the rule of the row under "Compact now" (`compactionLine`,
 * state/control.svelte.ts): only an event with token numbers says that a compaction is done.
 * Claude Code sends one without numbers when a compaction begins, so that row says "started";
 * its boundary line, which says how big the context was before, comes when the compaction is
 * done. From another CLI an event without numbers can be the start or the end (Codex sends
 * both, and nothing tells them apart): its row says neither.
 */
export function compactionRowLine(k: Pick<Compacting, 'pre_tokens' | 'post_tokens'>, provider: Provider | null): string {
  if (k.post_tokens != null) return `Compacted context${k.pre_tokens == null ? '' : ` ${tokens(k.pre_tokens)} → ${tokens(k.post_tokens)}`}`;
  if (provider === 'claude') return k.pre_tokens == null ? 'Compaction started' : `Compacted context, ${tokens(k.pre_tokens)} tokens before`;
  return 'Context compaction';
}

/**
 * The row of a tool call that starts: what it does, then the CLI's name of the tool ("Running
 * npm test · Bash"). A tool of an MCP server is named in words, never by its raw name
 * (`mcp__<server>__<tool>`): one of Codly's own by what it does ("Submitting the plan"),
 * another by its server and its name ("Using linear: create_issue").
 */
export function toolStartLine(k: Pick<ToolStarted, 'name' | 'activity' | 'target' | 'input'>, provider: Provider | null): string {
  if (isPlanCall(k.name, provider)) return planStep(false);
  const by = subagentMark(k.input);
  const t = mcpToolOf(k.name, k.target);
  if (t?.server === CODLY_SERVER) {
    const tool = t.tool.replace(/^codly\./, '');
    return `${CODLY_TOOL[tool] ?? `Using Codly’s ${tool.replace(/^codly_/, '').replace(/_/g, ' ')} tool`}${by}`;
  }
  if (t) return `Using ${t.server}: ${t.tool}${by}`;
  return `${phrase(k.activity, targetLabel(k.target, k.activity))} · ${k.name}${by}`;
}

/**
 * The row of a tool call that ended, in the past, from its start: "Read package.json · 0.5 s",
 * "Ran npm test · failed · exit 1 · 2 s". The CLI's name of the tool is on the row of a call
 * that runs; the row of a call that is over says what it did and how it ended. Without its
 * start (a row the History does not hold) it says only how it ended ("Finished · 0.5 s").
 */
export function toolEndLine(start: Pick<ToolStarted, 'name' | 'activity' | 'target' | 'input'> | null, end: Pick<ToolCompleted, 'ok' | 'exit_code' | 'tests_passed' | 'tests_failed' | 'duration_ms'>, provider: Provider | null): string {
  const said = start ? `${toolDoneLine(start, provider)}${end.ok ? '' : ' · failed'}` : end.ok ? 'Finished' : 'Failed';
  const exit = end.exit_code != null ? ` · exit ${end.exit_code}` : '';
  const tests = end.tests_passed != null ? ` · ${end.tests_passed} passed${end.tests_failed ? `, ${end.tests_failed} failed` : ''}` : '';
  return `${said}${exit}${tests}${end.duration_ms != null ? ` · ${dur(end.duration_ms)}` : ''}`;
}

/** What a call did, in the past: the start's words (`toolStartLine`) with the verb that says it is done, and without the tool's name. */
export function toolDoneLine(k: Pick<ToolStarted, 'name' | 'activity' | 'target' | 'input'>, provider: Provider | null): string {
  if (isPlanCall(k.name, provider)) return planStep(false);
  const by = subagentMark(k.input);
  const t = mcpToolOf(k.name, k.target);
  if (t?.server === CODLY_SERVER) return `Used Codly’s ${t.tool.replace(/^codly\./, '').replace(/^codly_/, '').replace(/_/g, ' ')} tool${by}`;
  if (t) return `Used ${t.server}: ${t.tool}${by}`;
  const label = oneLine(targetLabel(k.target, k.activity), 60);
  const verb = k.activity === 'test' && label ? 'Ran' : pastVerb(k.activity);
  return `${label ? `${verb} ${label}` : verb}${by}`;
}

/** The changes of an agent's state that its turn's and its calls' rows already say: History shows no row of their own for them. */
const QUIET_STATUS = new Set(['idle', 'starting', 'thinking', 'working']);

/** Whether a `status_changed` row says nothing the rows around it do not ("Status → idle"). */
export const quietStatus = (status: string): boolean => QUIET_STATUS.has(status);

/** The core's record of a reconnect: "asked Ada to reconnect its MCP server docs" (orchestrator/live_session.rs). */
const RECONNECT = /^asked (.+?) to reconnect its MCP server (.+)$/;

/** The agent and the server of the core's record of a reconnect (null: the record is another one). */
export function reconnectAsked(summary: string): { agent: string; server: string } | null {
  const m = RECONNECT.exec(summary.trim());
  return m ? { agent: m[1], server: m[2] } : null;
}

/**
 * The row of a reconnect that was a restart ("Restart" sends Claude Code the request of
 * "Reconnect" for a connected server, session.ts `canRestart`): "You restarted docs for Ada’s
 * run". Null when the record is no reconnect.
 */
export function restartLine(summary: string): string | null {
  const asked = reconnectAsked(summary);
  return asked ? `You restarted ${asked.server} for ${asked.agent}’s run` : null;
}

/** A decision's `detail` as an object (empty when it is none). */
const detailOf = (d: JsonValue | undefined): { [key in string]?: JsonValue } => (d && typeof d === 'object' && !Array.isArray(d) ? d : {});

/** Your comments, when the decision is one of yours that sent a task back ("you sent t1 back with review comments", orchestrator/workspace.rs). */
function sentBack(k: Pick<DecisionRow, 'decision_type' | 'method' | 'detail'>): string | null {
  if (k.decision_type !== 'review' || k.method !== 'human') return null;
  const c = detailOf(k.detail).comments;
  return typeof c === 'string' && c.trim() ? c.trim() : null;
}

/** How many comments a send-back held: Send back writes "Review comments (3):" first (util/review.ts, `reviewMessage`). */
export function reviewCount(comments: string): number | null {
  const m = /^Review comments \((\d+)\):/.exec(comments.trim());
  return m ? Number(m[1]) : null;
}

/**
 * A summary the core wrote for a log, as a sentence that starts with a capital and a subject.
 * One that starts with "asked " says who asked: "You asked Ada to stop …" when it was you
 * (`method` "human": you pressed the button), else "Codly asked …". Any other starts with a
 * capital ("you turned postgres off …" reads "You turned postgres off …"), unless its first word
 * is a name or a task key (`keeps`: "t1 …" stays as it is).
 */
export function historySentence(summary: string, method: string, keeps: (word: string) => boolean): string {
  const s = summary.trim();
  if (s.startsWith('asked ')) return `${method === 'human' ? 'You' : 'Codly'} ${s}`;
  const first = s.split(/[\s:;,.'’]/, 1)[0];
  if (!first || keeps(first)) return s;
  return s[0].toUpperCase() + s.slice(1);
}

/** The row of a decision: its summary as a sentence, and a send-back says how many comments you sent ("You sent t1 back with 3 review comments; …"). */
export function decisionLine(k: Pick<DecisionRow, 'summary' | 'method' | 'decision_type' | 'detail'>, keeps: (word: string) => boolean): string {
  const comments = sentBack(k);
  const n = comments ? reviewCount(comments) : null;
  const summary = n == null ? k.summary : k.summary.replace('back with review comments', `back with ${plural(n, 'review comment')}`);
  return historySentence(summary, k.method, keeps);
}

/** How close in time the rows of one act of yours are: the core writes them one right after the other. */
export const ACT_MS = 2000;

/**
 * The request a decision repeats the answer of: a record of the answer (`approval` in its
 * detail) or of a rule you saved with it (its `rule` is the saved rule, with the request's
 * `approval_id`). Not the core's records of why Codly asked ("asking you: …") or which rule
 * answered ("auto-allowed …"): their `rule` is that reason, in words, and says what the answer's
 * row does not.
 */
function answerOf(k: Pick<DecisionRow, 'decision_type' | 'detail'>): string | null {
  const d = detailOf(k.detail);
  const id = k.decision_type === 'approval' ? d.approval : k.decision_type === 'approval_policy' && typeof d.rule === 'object' && d.rule !== null ? d.approval_id : null;
  return typeof id === 'string' ? id : null;
}

/** Whether `r` is part of the send-back `d` (with your `comments`): the task's new status, your comments in the worker's conversation, the turn that took them. */
function partOfSendBack(r: EventRow, d: EventRow, comments: string): boolean {
  if (r === d) return false;
  const k = r.kind;
  const before = r.ts <= d.ts && d.ts - r.ts <= ACT_MS;
  if (k.type === 'task_changed') return before && k.task.id === d.task_id;
  if (k.type === 'chat') return before && k.message.author === 'user' && k.message.text.trim() === comments;
  // (the worker's turn can start a moment after the record)
  if (k.type === 'turn_started' || k.type === 'prompt_steered') return Math.abs(r.ts - d.ts) <= ACT_MS && !!k.input?.includes(comments);
  return false;
}

/**
 * The rows of a History (newest first) with each act said once. A send-back is its decision
 * row: the task's new status, your comments in the worker's conversation and the turn that took
 * them are part of it (same task, within `ACT_MS`); rows of other tasks between them stay. A
 * request that is closed is its answer's row: the row that it asked, a record of the answer,
 * and the calls of a plan it showed (`planCalls`) go. A tool call is the row of its start: the
 * row of its end goes, and the start's row says how it ended (`toolEndLine`). `asked`: the
 * requests whose row went, by id, so that the answer's row can say what was asked
 * (`closedLine`). `at`: for each row that went, by its event id, the id of the row that stands
 * for it, so that a jump to it lands there.
 */
export function foldActs(rows: readonly EventRow[], planCalls: ReadonlySet<string>): { rows: EventRow[]; asked: Map<string, Approval>; at: Map<number, number> } {
  /** The row of each answer, by the request's id. */
  const answers = new Map(rows.flatMap((e) => (e.kind.type === 'approval_resolved' ? [[e.kind.approval_id, e] as const] : [])));
  const asked = new Map<string, Approval>();
  /** Each row that goes, with the row that stands for it. */
  const into = new Map<EventRow, EventRow>();
  /** The calls of a plan whose request is closed, with the answer's row. */
  const planOf = new Map<string, EventRow>();
  for (const e of rows) {
    if (e.kind.type !== 'approval_requested') continue;
    const answer = answers.get(e.kind.approval.id);
    if (!answer) continue;
    asked.set(e.kind.approval.id, e.kind.approval);
    into.set(e, answer);
    const call = e.kind.approval.call_id;
    if (call && planCalls.has(call)) planOf.set(call, answer);
  }
  const startOf = new Map(rows.flatMap((e) => (e.kind.type === 'tool_started' ? [[e.kind.call_id, e] as const] : [])));
  for (const e of rows) {
    const k = e.kind;
    if (k.type === 'tool_started' || k.type === 'tool_completed') {
      const plan = planOf.get(k.call_id);
      if (plan) into.set(e, plan);
      else if (k.type === 'tool_completed' && startOf.has(k.call_id)) into.set(e, startOf.get(k.call_id)!);
    }
    if (k.type !== 'decision') continue;
    const answer = answerOf(k);
    const answerRow = answer != null ? answers.get(answer) : undefined;
    if (answerRow) into.set(e, answerRow);
    const comments = sentBack(k);
    if (comments && e.task_id) for (const r of rows) if (partOfSendBack(r, e, comments)) into.set(r, e);
  }
  // (a row can stand for another that went too: the jump follows to the one that stays)
  const at = new Map<number, number>();
  for (const [r, by] of into) {
    let to = by;
    for (let hops = 0; into.has(to) && hops < into.size; hops++) to = into.get(to)!;
    at.set(r.id, to.id);
  }
  return { rows: rows.filter((r) => !into.has(r)), asked, at };
}

/**
 * The row of a closed request: how it ended, then what it asked ("Denied by you · asked to change
 * README.md"). The answer comes first: a long request is cut at the end of the row. A plan's row
 * says only how it ended ("Plan sent back by you"): that names it.
 */
export function closedLine(asked: Pick<Approval, 'subject'> | null, settled: string, provider: Provider | null): string {
  if (!asked || showsPlan(asked.subject, provider)) return settled;
  const what = askedLine(asked, provider);
  return `${settled} · ${what[0].toLowerCase()}${what.slice(1)}`;
}

/** The rule you saved with your answer to a request, from the core's record of it (orchestrator/approval_rules.rs: its detail holds the rule and the request's id). */
export function savedRuleOf(decisions: readonly { type: string; detail: JsonValue }[], approvalId: string): Pick<ApprovalRule, 'kind' | 'command'> | null {
  for (const d of decisions) {
    if (d.type !== 'approval_policy') continue;
    const detail = detailOf(d.detail);
    const rule = detailOf(detail.rule);
    if (detail.approval_id === approvalId && (rule.kind === 'exact' || rule.kind === 'prefix') && typeof rule.command === 'string') return { kind: rule.kind, command: rule.command };
  }
  return null;
}
