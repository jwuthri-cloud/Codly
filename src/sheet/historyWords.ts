// Words for rows of an agent's History that are built from ids: a fact of the knowledge graph
// and a task that changed. And what a row needs to know about the agent's other events: which
// calls showed a plan, and which conversation rows only repeat a reply.
// Pure, so the sheet and the tests agree; the sheet passes the names.
import type { EventRow } from '../lib/gen/EventRow';
import type { Task } from '../lib/gen/Task';
import type { Triple } from '../lib/gen/Triple';
import { TASK_STATUS_LABEL } from '../lib/util/labels';
import { PLAN_TOOL } from '../lib/util/planTool';

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
};

export function historyFacts(events: readonly EventRow[], agentId: string): HistoryFacts {
  const facts: HistoryFacts = { planCalls: new Set(), said: new Set() };
  for (const e of events) {
    if (e.agent_id !== agentId) continue;
    if (e.kind.type === 'tool_started' && e.kind.name === PLAN_TOOL) facts.planCalls.add(e.kind.call_id);
    else if (e.kind.type === 'message') facts.said.add(e.kind.text);
  }
  return facts;
}

/** Whether a `chat` row only repeats a message of the agent whose history this is. */
export function repeatsReply(e: EventRow, agentId: string, facts: HistoryFacts): boolean {
  return e.kind.type === 'chat' && e.kind.message.author === agentId && facts.said.has(e.kind.message.text);
}

/** "Task t1 is now In review" */
export function taskChangedLine(task: Pick<Task, 'key' | 'status'>): string {
  return `Task ${task.key} is now ${TASK_STATUS_LABEL[task.status]}`;
}
