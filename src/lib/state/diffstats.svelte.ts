// Diffstats per task and per plan ("+48 −6 in 2 files"), fetched from the core once the work
// is committed and cached until the task or plan changes again. Narration, task-done cards and
// the Changes room all read the same numbers.
import { api } from '../api';
import { diffStats, parseDiff } from '../util/diff';

export type DiffStat = { files: number; added: number; removed: number };

class DiffStats {
  task = $state<Record<string, DiffStat>>({});
  plan = $state<Record<string, DiffStat>>({});
  private seen = new Map<string, string>();

  private load(kind: 'task' | 'plan', id: string, version: string) {
    const key = `${kind}:${id}`;
    if (this.seen.get(key) === version) return;
    this.seen.set(key, version);
    const p = kind === 'task' ? api.taskDiff(id) : api.planDiff(id);
    p.then((text) => {
      const files = parseDiff(text);
      const s = { files: files.length, ...diffStats(files) };
      if (kind === 'task') this.task = { ...this.task, [id]: s };
      else this.plan = { ...this.plan, [id]: s };
    }).catch(() => this.seen.delete(key));
  }

  /** Ask for a task's stats; `version` (e.g. status + commit count) refetches when it changes. */
  ensureTask(id: string, version: string) {
    this.load('task', id, version);
  }
  ensurePlan(id: string, version: string) {
    this.load('plan', id, version);
  }
  reset() {
    this.task = {};
    this.plan = {};
    this.seen.clear();
  }
}

export const diffstats = new DiffStats();

export const statLine = (s: DiffStat | null | undefined) =>
  s ? `+${s.added} −${s.removed} in ${s.files} ${s.files === 1 ? 'file' : 'files'}` : '';
