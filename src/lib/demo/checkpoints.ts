// The browser demo's answer to `agent_checkpoints` and `restore_checkpoint`: a few points for an
// agent with a worktree, and a restore that follows the rules of the real core
// (orchestrator/checkpoints.rs): refused while the agent works, files only, and the state it
// replaces becomes the newest point, so it can be undone.
import type { Agent } from '../gen/Agent';
import type { AgentCheckpoints } from '../gen/AgentCheckpoints';
import type { Checkpoint } from '../gen/Checkpoint';
import type { CheckpointRestore } from '../gen/CheckpointRestore';

const MIN = 60_000;
const RESTING = ['idle', 'done', 'failed', 'offline'];

export class DemoCheckpoints {
  private byAgent = new Map<string, Checkpoint[]>();

  private points(agent: Agent, now: number): Checkpoint[] {
    let list = this.byAgent.get(agent.id);
    if (!list) {
      list = [
        { n: 3, kind: 'turn', ts: now - 4 * MIN, label: 'Wire the theme toggle into the header', changed: 1, files: ['src/theme/ThemeProvider.tsx'] },
        { n: 2, kind: 'turn', ts: now - 19 * MIN, label: 'Add a useTheme hook that follows the system setting', changed: 2, files: ['src/theme/ThemeProvider.tsx', 'src/theme/useTheme.ts'] },
        { n: 1, kind: 'turn', ts: now - 47 * MIN, label: 'Task t1: Dark mode', changed: 3, files: ['src/theme/ThemeProvider.tsx', 'src/theme/theme.test.tsx', 'src/theme/useTheme.ts'] },
      ];
      this.byAgent.set(agent.id, list);
    }
    return list;
  }

  list(agent: Agent | undefined, now: number): AgentCheckpoints {
    if (!agent?.worktree) return { off: `${agent?.name ?? 'This agent'} has no worktree now, so there are no checkpoints.`, checkpoints: [] };
    return { off: null, checkpoints: this.points(agent, now) };
  }

  restore(agent: Agent | undefined, n: number, now: number): CheckpointRestore {
    if (!agent?.worktree) throw new Error(this.list(agent, now).off ?? 'No checkpoints.');
    if (!RESTING.includes(agent.status)) throw new Error(`${agent.name} is still working; restore files when its turn ends`);
    const list = this.points(agent, now);
    const target = list.find((c) => c.n === n);
    if (!target) throw new Error('that point is gone: Codly keeps the last 50 of an agent');
    if (!target.changed) throw new Error('the files are already as they were at that point');
    const files = target.files;
    const undo = Math.max(...list.map((c) => c.n)) + 1;
    // The files are now as at `target`; what they were is the new point on top.
    const rest = list.map((c) => (c.n === n ? { ...c, changed: 0, files: [] } : c));
    this.byAgent.set(agent.id, [{ n: undo, kind: 'restore', ts: now, label: target.label, changed: files.length, files }, ...rest]);
    return { files, undo };
  }
}
