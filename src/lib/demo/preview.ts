// The browser demo's answer to `preview`. There is no process behind it: Run "starts" a dev
// server whose output appears over a few seconds, on the worktree's own port, and Stop ends it.
import type { Preview } from '../gen/Preview';
import type { PreviewAction } from '../gen/PreviewAction';
import type { PreviewRun } from '../gen/PreviewRun';
import type { PreviewSeen } from '../gen/PreviewSeen';
import type { PreviewSource } from '../gen/PreviewSource';
import type { DemoWorld } from './pullrequest';

/** As the core gives them out: blocks of 10 ports from 20000. */
const PORT_FIRST = 20_000;
const PORT_BLOCK = 10;

type Run = { id: string; command: string; started_at: number; stopped_at: number | null };

/** What the demo dev server prints, and after how many ms. */
function script(port: number): [number, string][] {
  return [
    [150, '> acme-web@2.4.0 dev'],
    [150, `> vite --port ${port}`],
    [900, ''],
    [900, '  VITE v7.1.4  ready in 412 ms'],
    [950, ''],
    [950, `  ➜  Local:   http://localhost:${port}/`],
    [950, '  ➜  press h + enter to show help'],
    [4200, '[vite] hmr update /src/theme/ThemeProvider.tsx'],
  ];
}

export class PreviewDemo {
  /** The command you ran last in the demo repo. */
  private command: string | null = null;
  /** By worktree: the run there, live or ended. */
  private runs = new Map<string, Run>();
  private ports = new Map<string, number>();
  private seq = 0;

  private worktree(source: PreviewSource, world: DemoWorld): string {
    if (source.kind === 'plan') {
      const plan = world.plans.get(source.id);
      if (!plan) throw new Error(`unknown plan ${source.id}`);
      if (!plan.integration_worktree) throw new Error('the plan never ran, so there is nothing to try');
      return plan.integration_worktree;
    }
    const agent = world.agents.get(source.id);
    if (!agent) throw new Error(`unknown agent ${source.id}`);
    if (!agent.worktree) throw new Error(`${agent.name} has no worktree yet, so there is nothing to try`);
    return agent.worktree;
  }

  private view(run: Run, port: number, seen: PreviewSeen | null, now: number): PreviewRun {
    const elapsed = (run.stopped_at ?? now) - run.started_at;
    const lines = script(port).filter(([at]) => at <= elapsed).map(([, line]) => line);
    const have = seen?.run === run.id ? seen.lines : 0;
    const local = `http://localhost:${port}/`;
    return {
      id: run.id,
      command: run.command,
      started_at: run.started_at,
      ended: run.stopped_at === null ? null : 'stopped',
      url: lines.some((l) => l.includes(local)) ? local : null,
      lines: lines.slice(have),
      printed: lines.length,
    };
  }

  /** `preview`: the worktree's preview, after `action` (start or stop) when one is given. */
  handle(a: Record<string, unknown>, world: DemoWorld): Preview {
    const worktree = this.worktree(a.source as PreviewSource, world);
    const action = (a.action as PreviewAction | null) ?? null;
    const live = this.runs.get(worktree);
    if (action?.do === 'start') {
      const command = action.command.trim();
      if (!command) throw new Error('write the command to run first');
      if (live && live.stopped_at === null) throw new Error('a command already runs in this worktree; stop it first');
      if (!this.ports.has(worktree)) this.ports.set(worktree, PORT_FIRST + PORT_BLOCK * this.ports.size);
      this.command = command;
      this.runs.set(worktree, { id: `preview-${++this.seq}`, command, started_at: world.now(), stopped_at: null });
    } else if (action?.do === 'stop' && live && live.stopped_at === null) {
      live.stopped_at = world.now();
    }
    const run = this.runs.get(worktree);
    const port = this.ports.get(worktree) ?? null;
    return {
      worktree,
      command: this.command,
      port_base: port,
      run: run && port !== null ? this.view(run, port, (a.seen as PreviewSeen | null) ?? null, world.now()) : null,
    };
  }
}
