// The browser demo's answer to `agent_checkpoints`, `restore_checkpoint` and `last_turn_diff`: a
// few points for an agent with a worktree, a restore that follows the rules of the real core
// (orchestrator/checkpoints.rs): refused while the agent works, files only, one file or all of
// them, and the state it replaces becomes the newest point, so it can be undone. The last turn is
// the newest point taken before a turn, with a small diff while files differ from it.
import type { Agent } from '../gen/Agent';
import type { AgentCheckpoints } from '../gen/AgentCheckpoints';
import type { Checkpoint } from '../gen/Checkpoint';
import type { CheckpointRestore } from '../gen/CheckpointRestore';
import type { LastTurn } from '../gen/LastTurn';

const MIN = 60_000;
const RESTING = ['idle', 'done', 'failed', 'offline'];
/** What the demo's last turn did to the one file its newest point lists. */
const TURN_DIFF = `diff --git a/src/theme/ThemeProvider.tsx b/src/theme/ThemeProvider.tsx
index 3f1c2aa..9d0e4b7 100644
--- a/src/theme/ThemeProvider.tsx
+++ b/src/theme/ThemeProvider.tsx
@@ -12,6 +12,7 @@ export function ThemeProvider({ children }: Props) {
   const [theme, setTheme] = useState<Theme>(initial);
   useEffect(() => {
     document.documentElement.dataset.theme = theme;
+    localStorage.setItem('theme', theme);
   }, [theme]);
   return <ThemeContext.Provider value={{ theme, setTheme }}>{children}</ThemeContext.Provider>;
 }
`;

export class DemoCheckpoints {
  private byAgent = new Map<string, Checkpoint[]>();
  /** `changed`: has the agent changed a file yet? Codly takes a point only when a turn changed something: an agent that changed nothing has none. */
  constructor(private changed: (agentId: string) => boolean = () => true) {}

  private points(agent: Agent, now: number): Checkpoint[] {
    if (!this.changed(agent.id)) return [];
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
    if (!agent?.worktree) return { off: `${agent?.name ?? 'This agent'} has no worktree now, so there are no checkpoints.`, checkpoints: [], task: null, left_out: [] };
    return { off: null, checkpoints: this.points(agent, now), task: null, left_out: [] };
  }

  /** `path`: put back that one file of the point; null: all of them. */
  restore(agent: Agent | undefined, n: number, now: number, path: string | null = null): CheckpointRestore {
    if (!agent?.worktree) throw new Error(this.list(agent, now).off ?? 'No checkpoints.');
    if (!RESTING.includes(agent.status)) throw new Error(`${agent.name} is still working; restore files when its turn ends`);
    const list = this.points(agent, now);
    const target = list.find((c) => c.n === n);
    if (!target) throw new Error('that point is gone: Codly keeps the last 50 of an agent');
    if (!target.changed) throw new Error('the files are already as they were at that point');
    if (path != null && !target.files.includes(path)) throw new Error(`${path} is already as it was at that point`);
    const files = path == null ? target.files : [path];
    const undo = Math.max(...list.map((c) => c.n)) + 1;
    // The files that went back are now as at `target`; what they were is the new point on top.
    const left = target.files.filter((f) => !files.includes(f));
    const rest = list.map((c) => (c.n === n ? { ...c, changed: left.length, files: left } : c));
    this.byAgent.set(agent.id, [{ n: undo, kind: 'restore', ts: now, label: target.label, changed: files.length, files }, ...rest]);
    return { files, undo };
  }

  /** The agent's last turn: the newest point taken before a turn, and what differs from it now. */
  lastTurn(agent: Agent | undefined, now: number): LastTurn | null {
    if (!agent?.worktree) return null;
    const turn = this.points(agent, now).find((c) => c.kind === 'turn');
    return turn ? { label: turn.label, ts: turn.ts, diff: turn.changed ? TURN_DIFF : '', task: null } : null;
  }
}
