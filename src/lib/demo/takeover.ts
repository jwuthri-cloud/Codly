// The browser demo's side of "Open in my terminal". It follows the rules of the real core
// (crates/codly-core/src/orchestrator/takeover.rs): only an agent that rests, on no task that
// still runs, with a session and a CLI that has a resume command; while it is in your terminal
// a message to it is refused; "Give it back" ends that. The demo has no process to stop.
import type { Agent } from '../gen/Agent';
import type { Provider } from '../gen/Provider';
import type { Task } from '../gen/Task';
import type { TakeoverState } from '../gen/TakeoverState';
import { PROVIDER_LABEL } from '../util/labels';

const RESTING = ['idle', 'done', 'failed', 'offline'];
const IN_FLIGHT = ['running', 'verifying', 'reviewing', 'changes_requested', 'merging'];
/** The CLIs with a command that opens one session by its id (`resume_command` in the core). */
const RESUME: Partial<Record<Provider, string>> = { claude: 'claude --resume', codex: 'codex resume' };

/** A path as one word of a shell line. */
const quote = (path: string) => (/^[A-Za-z0-9/._~+-]+$/.test(path) ? path : `'${path.replace(/'/g, `'\\''`)}'`);

/** The resume command for the agent's session, or why there is none. */
export function resumeCommand(agent: Agent): { command: string } | { why: string } {
  if (!agent.session_id) return { why: `${agent.name} has no session yet: send it a message first` };
  const resume = RESUME[agent.provider];
  if (!resume) return { why: `Codly knows no ${PROVIDER_LABEL[agent.provider]} command that opens one session by its id` };
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(agent.session_id)) return { why: `the session id of ${PROVIDER_LABEL[agent.provider]} is not a plain word, so Codly does not put it in a command` };
  return { command: `${resume} ${agent.session_id}` };
}

/** Why the agent cannot go to your terminal now; null when it can. */
export function blocked(agent: Agent, tasks: Task[]): string | null {
  if (agent.archived) return `${agent.name} no longer works here`;
  if (agent.role === 'lead') return 'the Lead works through Codly’s own tools, and a terminal does not have them';
  const line = resumeCommand(agent);
  if ('why' in line) return line.why;
  const task = tasks.find((t) => IN_FLIGHT.includes(t.status) && (t.id === agent.task_id || t.assignee === agent.id));
  if (task) return `${agent.name} is on ${task.key}, which is still running: stop ${agent.name} first, or wait for the task`;
  if (!RESTING.includes(agent.status)) return `${agent.name} is in the middle of a turn: wait for it, or stop it first`;
  return null;
}

export class DemoTakeovers {
  /** Agent → since when it is in your terminal. */
  private since = new Map<string, number>();

  has(agentId: string): boolean {
    return this.since.has(agentId);
  }

  state(agent: Agent | undefined, tasks: Task[], repo: string): TakeoverState {
    if (!agent) throw new Error('unknown agent');
    const since = this.since.get(agent.id);
    if (since === undefined) return { takeover: null, blocked: blocked(agent, tasks) };
    const line = resumeCommand(agent);
    if ('why' in line) throw new Error(line.why);
    const cwd = agent.worktree ?? repo;
    return { takeover: { agent_id: agent.id, cwd, command: line.command, copy_command: `cd ${quote(cwd)} && ${line.command}`, since }, blocked: null };
  }

  take(agent: Agent | undefined, tasks: Task[], repo: string, now: number): TakeoverState {
    if (!agent) throw new Error('unknown agent');
    if (!this.since.has(agent.id)) {
      const why = blocked(agent, tasks);
      if (why) throw new Error(why);
      this.since.set(agent.id, now);
    }
    return this.state(agent, tasks, repo);
  }

  giveBack(agent: Agent | undefined, tasks: Task[], repo: string): TakeoverState {
    if (!agent) throw new Error('unknown agent');
    this.since.delete(agent.id);
    return this.state(agent, tasks, repo);
  }
}
