// The browser demo's side of what an agent leaves at work in the background. Claude Code says
// its whole list of background tasks at every change, with what left the list and how it ended
// (`background_tasks` events): the demo says the same. The story starts a command and ends it;
// "Stop" on the agent sheet ends it sooner.
import type { BackgroundTask } from '../gen/BackgroundTask';
import { uid, type Actor, type Sim } from './sim';

export class DemoBackground {
  /** What each agent's run lists now, by agent. */
  private listed = new Map<string, { run: string | null; tasks: BackgroundTask[] }>();
  constructor(private sim: Sim) {}

  /** The commands the agent's run has in the background now. */
  of(agentId: string): BackgroundTask[] {
    const now = this.listed.get(agentId);
    // (the list of an earlier run went with that run's process)
    return now && now.run === this.sim.actor(agentId).a.run_id ? now.tasks : [];
  }

  /** The agent leaves a command running in the background: its CLI lists it. Returns the CLI's id of it. */
  start(act: Actor, description: string): string {
    const task: BackgroundTask = { id: uid('bg-'), kind: 'command', description };
    const tasks = [...this.of(act.id), task];
    this.listed.set(act.id, { run: act.a.run_id, tasks });
    act.emit({ type: 'background_tasks', tasks, ended: [] });
    return task.id;
  }

  /** The command is over, in the CLI's word: "completed" by itself, "stopped" by the agent or by you. False: it was over already. */
  end(agentId: string, taskId: string, status: 'completed' | 'stopped'): boolean {
    const act = this.sim.actor(agentId);
    const task = this.of(agentId).find((t) => t.id === taskId);
    if (!task) return false;
    const tasks = this.of(agentId).filter((t) => t !== task);
    this.listed.set(agentId, { run: act.a.run_id, tasks });
    act.emit({ type: 'background_tasks', tasks, ended: [{ ...task, status }] });
    return true;
  }
}

const demos = new WeakMap<Sim, DemoBackground>();

/** The background work of one demo: the story and the agent sheet's "Stop" act on the same lists. */
export function demoBackground(sim: Sim): DemoBackground {
  let b = demos.get(sim);
  if (!b) demos.set(sim, (b = new DemoBackground(sim)));
  return b;
}
