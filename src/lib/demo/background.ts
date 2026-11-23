// The browser demo's side of what an agent leaves at work in the background. Claude Code says
// its whole list of background tasks at every change, with what left the list and how it ended
// (`background_tasks` events): the demo says the same. The story starts a command or a helper
// and ends it; "Stop" on the agent sheet ends it sooner. A helper has a row of its own too
// (`sub_agent`), and after its end Claude Code starts a turn of the agent by itself. A helper
// whose `Agent` call waits for it is in no list: "Stop" reaches it by that call (`stopWaited`).
import type { BackgroundTask } from '../gen/BackgroundTask';
import { Cancelled, uid, type Actor, type Sim } from './sim';

/** How Claude Code ends the call a helper was in when the helper is stopped: the start of its text (recording `r5-stop-subagent-1`). */
const REJECTED = "The user doesn't want to proceed with this tool use. The tool use was rejected.";
/** What the `Agent` call that waited for a helper returns when the helper is stopped (recording `r6-shutdown-helper-foreground-stop-1`). */
const INTERRUPTED = '[Request interrupted by user for tool use]';

/** One piece of work in an agent's list. A helper has the id of its `Agent` call (its row on the sheet) and the call of its own that runs now. */
type Work = { task: BackgroundTask; helper: { call: string; open: string | null } | null };

export class DemoBackground {
  /** What each agent's run lists now, by agent. */
  private listed = new Map<string, { run: string | null; work: Work[] }>();
  /** By agent: its helpers whose `Agent` call waits for them, by that call: what each was asked, and the call of its own it is in. They are in no list. */
  private waited = new Map<string, Map<string, { description: string; open: string | null }>>();
  constructor(private sim: Sim) {}

  /** A helper starts, and the agent's `Agent` call waits for it (`call`): it ends with the agent's turn. */
  waits(agentId: string, call: string, description: string) {
    const of = this.waited.get(agentId) ?? new Map<string, { description: string; open: string | null }>();
    this.waited.set(agentId, of.set(call, { description, open: null }));
  }

  /** The call of its own that a waited-for helper is in now. */
  waitedIn(agentId: string, call: string, own: string | null) {
    const helper = this.waited.get(agentId)?.get(call);
    if (helper) helper.open = own;
  }

  /** What the helper of this `Agent` call was asked, while its call waits for it. Null: no such helper works. */
  waiting(agentId: string, call: string): string | null {
    return this.waited.get(agentId)?.get(call)?.description ?? null;
  }

  /** The helper reported: its call waits no more. */
  reported(agentId: string, call: string) {
    this.waited.get(agentId)?.delete(call);
  }

  /**
   * The agent's turn was stopped. Like Claude Code's adapter, after the turn's end: a helper
   * whose `Agent` call still waited for it is over with the turn, and its row says "stopped".
   */
  turnStopped(act: Actor) {
    for (const [call, { description }] of this.waited.get(act.id) ?? []) act.emit({ type: 'sub_agent', call_id: call, description, status: 'stopped' });
    this.waited.delete(act.id);
  }

  /**
   * "Stop" on a helper whose `Agent` call waits for it (recorded: part 4 section 2). Its row
   * says "stopped", the call of its own it was in ends rejected, and the `Agent` call returns
   * an error: the agent's turn goes on, and says what became of the helper. False: no such
   * helper works.
   */
  stopWaited(agentId: string, call: string): boolean {
    const helper = this.waited.get(agentId)?.get(call);
    if (!helper) return false;
    this.waited.get(agentId)!.delete(call);
    const act = this.sim.actor(agentId);
    act.emit({ type: 'sub_agent_progress', call_id: call, name: null, step: null, tool_call_id: null, tool_uses: null, tokens: null, report: helper.description });
    act.emit({ type: 'sub_agent', call_id: call, description: helper.description, status: 'stopped' });
    if (helper.open) act.emit({ type: 'tool_completed', call_id: helper.open, ok: false, output: REJECTED, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null });
    act.emit({ type: 'tool_completed', call_id: call, ok: false, output: INTERRUPTED, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null });
    return true;
  }

  private work(agentId: string): Work[] {
    const now = this.listed.get(agentId);
    // (the list of a run went with that run's process: a stopped agent has no run, and its next one starts with nothing.
    // An agent that left the office has none either.)
    return now && now.run === this.sim.agents.get(agentId)?.run_id ? now.work : [];
  }

  /** What the agent's run has at work in the background now. */
  of(agentId: string): BackgroundTask[] {
    return this.work(agentId).map((w) => w.task);
  }

  /**
   * The agent leaves a command running in the background, or a helper at work there
   * (`helperCall`: the helper's `Agent` call): its CLI lists it. Returns the CLI's id of it.
   */
  start(act: Actor, description: string, helperCall: string | null = null): string {
    const task: BackgroundTask = { id: uid('bg-'), kind: helperCall ? 'helper' : 'command', description };
    const work = [...this.work(act.id), { task, helper: helperCall ? { call: helperCall, open: null } : null }];
    this.listed.set(act.id, { run: act.a.run_id, work });
    act.emit({ type: 'background_tasks', tasks: work.map((w) => w.task), ended: [] });
    return task.id;
  }

  /** The call of its own that a helper is in now (null: in none). A stop of the helper ends that call. */
  helperIn(agentId: string, taskId: string, own: string | null) {
    const helper = this.work(agentId).find((w) => w.task.id === taskId)?.helper;
    if (helper) helper.open = own;
  }

  /**
   * It is over, in the CLI's word: "completed" by itself (`report`: what a helper found),
   * "stopped" by the agent or by you. False: it was over already.
   * A helper's end is said as Claude Code's adapter says it: the helper's row first (what it
   * reported, then how it ended), then the list. The call a stopped helper was in ends as
   * rejected, and the agent is told in a turn that Claude Code starts by itself (`told`).
   */
  end(agentId: string, taskId: string, status: 'completed' | 'stopped', report: string | null = null): boolean {
    const act = this.sim.actor(agentId);
    const all = this.work(agentId);
    const over = all.find((w) => w.task.id === taskId);
    if (!over) return false;
    const { task, helper } = over;
    const rest = all.filter((w) => w !== over);
    this.listed.set(agentId, { run: act.a.run_id, work: rest });
    if (helper) {
      // (for a stopped helper Claude Code gives what the helper was asked to do as its report)
      act.emit({ type: 'sub_agent_progress', call_id: helper.call, name: null, step: null, tool_call_id: null, tool_uses: null, tokens: null, report: report ?? task.description });
      act.emit({ type: 'sub_agent', call_id: helper.call, description: task.description, status });
    }
    act.emit({ type: 'background_tasks', tasks: rest.map((w) => w.task), ended: [{ ...task, status }] });
    if (!helper) return true;
    if (helper.open) act.emit({ type: 'tool_completed', call_id: helper.open, ok: false, output: REJECTED, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null });
    void this.told(act, status === 'completed' ? `The helper is done. ${report ?? ''}`.trim() : 'The helper was stopped before it reported, so I do not have its answer.');
    return true;
  }

  /**
   * After the end of a background helper Claude Code starts a turn of the agent by itself, with
   * no message of yours, in which the agent says what became of the helper (recordings
   * `r5-stop-subagent-1` and `-2`). Not while a turn runs: Claude Code starts none then, and
   * the notice goes into the turn that runs (recording `r4-background-and-queue-fold`).
   */
  private async told(act: Actor, text: string) {
    const a = act.a;
    if (act.inTurn || a.status === 'offline') return;
    const before = a.status;
    try {
      await act.turn(null, () => act.say(text, { thread: a.role === 'lead' ? (a.thread ?? 'lead') : `agent:${a.id}` }));
    } catch (e) {
      if (!(e instanceof Cancelled)) throw e;
      // You stopped this turn: the agent takes its next one as ever.
      this.sim.cancelled.delete(act.id);
    }
    const now = this.sim.agents.get(act.id);
    if (now && now.status !== 'offline') act.status(before);
  }
}

const demos = new WeakMap<Sim, DemoBackground>();

/** The background work of one demo: the story and the agent sheet's "Stop" act on the same lists. */
export function demoBackground(sim: Sim): DemoBackground {
  let b = demos.get(sim);
  if (!b) demos.set(sim, (b = new DemoBackground(sim)));
  return b;
}
