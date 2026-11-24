// The browser demo's answer to `agent_checkpoints`, `restore_checkpoint` and `last_turn_diff`.
// The points are the agent's own turns that changed files (sim.ts `editedTurns`), each kept
// before its turn, in the worktree it works in now: the turns of the task it is on, or of its
// own work. A restore follows the rules of the real core (orchestrator/checkpoints.rs): refused
// while the agent works, files only, one file or all of them, and the state it replaces becomes
// the newest point, so it can be undone. Like the core it leaves a row in the agent's
// conversation and one in History. Each point counts the files that differ between it and the
// files now, so a restore or a later turn changes the counts of the other points too. The last
// turn is the newest point taken before a turn, with a small diff of the files that differ from it.
import type { Agent } from '../gen/Agent';
import type { AgentCheckpoints } from '../gen/AgentCheckpoints';
import type { Checkpoint } from '../gen/Checkpoint';
import type { CheckpointRestore } from '../gen/CheckpointRestore';
import type { LastTurn } from '../gen/LastTurn';
import { editedTurns, type Sim } from './sim';

const RESTING = ['idle', 'done', 'failed', 'offline'];
/** The core keeps the last 50 points of an agent. */
const KEPT = 50;
/** A file as it was before the demo's first turn. */
const ORIGINAL = 'original';

/** Each file's state: which turn wrote it last (`turn <n>`), or `ORIGINAL`. A file not named is `ORIGINAL`. */
type Files = Map<string, string>;
/** A restore you made: the files it put back, and the state each one went back to. */
type Restore = { n: number; ts: number; label: string; task: string | null; put: Files };

const state = (files: Files, f: string) => files.get(f) ?? ORIGINAL;

/** The files that differ between `a` and `b`, by name. */
function differ(a: Files, b: Files): string[] {
  return [...new Set([...a.keys(), ...b.keys()])].filter((f) => state(a, f) !== state(b, f)).sort();
}

export class DemoCheckpoints {
  /** The number of each agent's turn (by its row id), given once in order: a later point has a higher one. */
  private numbers = new Map<string, Map<number, number>>();
  private last = new Map<string, number>();
  private restores = new Map<string, Restore[]>();

  constructor(private sim: Pick<Sim, 'log' | 'tasks' | 'chat' | 'decision'>) {}

  private take(agentId: string): number {
    const n = (this.last.get(agentId) ?? 0) + 1;
    this.last.set(agentId, n);
    return n;
  }

  /** The key of the task whose worktree this is, when it is not the agent's own: a 1:1 agent on a task works there. */
  private taskKey(agent: Agent): string | null {
    return agent.role === 'solo' && agent.task_id ? (this.sim.tasks.get(agent.task_id)?.key ?? null) : null;
  }

  /** The points of the worktree the agent works in now, newest first, the files at each, and the files now. */
  private points(agent: Agent): { list: Checkpoint[]; at: Map<number, Files>; now: Files } {
    const numbers = this.numbers.get(agent.id) ?? new Map<number, number>();
    this.numbers.set(agent.id, numbers);
    const turns = editedTurns(this.sim, agent.id);
    for (const t of turns) if (!numbers.has(t.id)) numbers.set(t.id, this.take(agent.id));
    type Step = { n: number; ts: number; label: string; kind: 'turn'; files: string[] } | ({ kind: 'restore' } & Restore);
    const steps: Step[] = [
      ...turns.filter((t) => t.task === agent.task_id).map((t) => ({ n: numbers.get(t.id)!, ts: t.ts, label: t.label, kind: 'turn' as const, files: t.files })),
      ...(this.restores.get(agent.id) ?? []).filter((r) => r.task === agent.task_id).map((r) => ({ kind: 'restore' as const, ...r })),
    ].sort((a, b) => a.n - b.n);
    const now: Files = new Map();
    const at = new Map<number, Files>();
    for (const s of steps) {
      at.set(s.n, new Map(now));
      if (s.kind === 'turn') for (const f of s.files) now.set(f, `turn ${s.n}`);
      else for (const [f, was] of s.put) now.set(f, was);
    }
    const list = steps
      .map((s): Checkpoint => {
        const files = differ(at.get(s.n)!, now);
        return { n: s.n, kind: s.kind, ts: s.ts, label: s.label, changed: files.length, files };
      })
      .reverse()
      .slice(0, KEPT);
    return { list, at, now };
  }

  // `_now` (list, lastTurn): the mock passes its clock to each call; the points carry the times of their turns.
  list(agent: Agent | undefined, _now: number): AgentCheckpoints {
    if (!agent?.worktree) return { off: `${agent?.name ?? 'This agent'} has no worktree now, so there are no checkpoints.`, checkpoints: [], task: null, left_out: [] };
    return { off: null, checkpoints: this.points(agent).list, task: this.taskKey(agent), left_out: [] };
  }

  /** `path`: put back that one file of the point; null: all of them. */
  restore(agent: Agent | undefined, n: number, now: number, path: string | null = null): CheckpointRestore {
    if (!agent?.worktree) throw new Error(this.list(agent, now).off ?? 'No checkpoints.');
    if (!RESTING.includes(agent.status)) throw new Error(`${agent.name} is still working; restore files when its turn ends`);
    const { list, at } = this.points(agent);
    const target = list.find((c) => c.n === n);
    if (!target) throw new Error('that point is gone: Codly keeps the last 50 of an agent');
    if (!target.changed) throw new Error('the files are already as they were at that point');
    if (path != null && !target.files.includes(path)) throw new Error(`${path} is already as it was at that point`);
    const files = path == null ? target.files : [path];
    const was = at.get(n)!;
    const undo = this.take(agent.id);
    // What the files are now is the new point on top: restoring it undoes this.
    const restores = this.restores.get(agent.id) ?? [];
    restores.push({ n: undo, ts: now, label: target.label, task: agent.task_id, put: new Map(files.map((f) => [f, state(was, f)])) });
    this.restores.set(agent.id, restores);
    // As the core says it, in its record and in the agent's conversation.
    const point = target.kind === 'turn' ? `before “${target.label}”` : 'before an earlier restore';
    const what = `${path ?? `${files.length} ${files.length === 1 ? 'file' : 'files'}`} of ${agent.name} back to ${point}`;
    this.sim.decision('checkpoint', 'human', `you put ${what}`, { agent_id: agent.id, checkpoint: n, undo, files });
    const rest = path != null ? 'The other files stay as they are, and' : 'Files only:';
    this.sim.chat(`agent:${agent.id}`, 'codly', `Put ${what}. ${rest} ${agent.name} still remembers the later turns. To undo, restore the newest point.`);
    return { files, undo };
  }

  /** The agent's last turn: the newest point taken before a turn, and what differs from it now. */
  lastTurn(agent: Agent | undefined, _now: number): LastTurn | null {
    if (!agent?.worktree) return null;
    const turn = this.points(agent).list.find((c) => c.kind === 'turn');
    if (!turn) return null;
    // The demo has no file contents: each file that differs shows one line the turn added.
    const diff = turn.files.map((f) => `diff --git a/${f} b/${f}\n--- a/${f}\n+++ b/${f}\n@@ -1,0 +1,1 @@\n+// ${turn.label}\n`).join('');
    return { label: turn.label, ts: turn.ts, diff, task: this.taskKey(agent) };
  }
}
