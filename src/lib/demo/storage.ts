// The browser-demo stand-in for the core's storage commands (Settings → Storage). One office
// with a worktree in each state, kept in memory, with the same rules as
// crates/codly-core/src/housekeeping/storage.rs (the tested original): a cleanup removes only
// what is finished, merged, clean and not in use; a dirty or unmerged worktree goes only with
// `force`, and keeps its branch unless the branch is asked for too.
import type { StorageCleanup } from '../gen/StorageCleanup';
import type { StoragePolicy } from '../gen/StoragePolicy';
import type { StorageReport } from '../gen/StorageReport';
import type { StoredBranch } from '../gen/StoredBranch';
import type { StoredWorktree } from '../gen/StoredWorktree';

const DAY = 86_400_000;
const ROOT = '~/Library/Application Support/dev.codly.desktop/worktrees/demo-1a2b3c';
/** Bytes of one run log in the demo. */
const LOG = 1_400_000;

/** `finished`: its plan is over, so a cleanup may remove it when nothing is lost. */
type Kept<T> = T & { finished: boolean };

export class StorageDemo {
  private policy: StoragePolicy = { log_days: 30, clean_merged: true };
  private worktrees: Kept<StoredWorktree>[];
  private branches: Kept<StoredBranch>[];
  /** When each run log was last written. */
  private logs: number[];

  constructor(now: number) {
    const wt = (name: string, owner: string, days: number, size: number, more: Partial<Kept<StoredWorktree>>): Kept<StoredWorktree> => ({
      path: `${ROOT}/${name}`,
      branch: `codly/${name}`,
      head: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678',
      owner,
      state: 'merged',
      dirty_files: [],
      dirty_count: 0,
      unmerged_commits: 0,
      locked: null,
      last_used_at: now - days * DAY,
      size_bytes: size,
      finished: true,
      ...more,
    });
    this.worktrees = [
      wt('dark-mode-9f3e/t1', 'Task t1: Add the theme switch', 6, 412e6, {}),
      wt('dark-mode-9f3e/t2', 'Task t2: Persist the theme', 6, 398e6, { state: 'dirty', dirty_files: ['src/theme.ts', 'notes.md'], dirty_count: 2 }),
      wt('export-csv-77ab/t1', 'Task t1: Stream the rows', 19, 1.3e9, { state: 'unmerged', unmerged_commits: 3 }),
      wt('search-2c41/integration', 'Plan: Faster search', 0, 405e6, { state: 'unmerged', unmerged_commits: 5, finished: false, locked: 'its plan is not finished' }),
    ];
    this.branches = [
      { name: 'codly/dark-mode-9f3e/integration', head: 'b2c3d4e5f60718293a4b5c6d7e8f9012345678a1', owner: 'Plan: Dark mode', unmerged_commits: 0, locked: null, last_commit_at: now - 6 * DAY, finished: true },
      { name: 'codly/export-csv-77ab/integration', head: 'c3d4e5f60718293a4b5c6d7e8f9012345678a1b2', owner: 'Plan: Export to CSV', unmerged_commits: 2, locked: null, last_commit_at: now - 19 * DAY, finished: true },
    ];
    this.logs = Array.from({ length: 46 }, (_, i) => now - i * 1.5 * DAY);
  }

  handle(cmd: string, a: Record<string, unknown>, now: number): unknown {
    switch (cmd) {
      case 'storage_report':
        return this.report();
      case 'storage_policy':
        if (a.policy) {
          const p = a.policy as StoragePolicy;
          this.policy = { ...p, log_days: Math.max(0, Math.min(3650, p.log_days)) };
        }
        return this.policy;
      case 'storage_clean':
        return this.clean(now);
      case 'storage_delete_worktree':
        return this.deleteWorktree(String(a.path), a.force === true, a.deleteBranch === true);
      case 'storage_delete_branch':
        return this.deleteBranch(String(a.branch), a.force === true);
    }
    throw new Error(`Unknown command ${cmd}`);
  }

  private report(): StorageReport {
    const strip = <T>({ finished: _, ...rest }: Kept<T>) => rest;
    return {
      offices: [{ office_id: 'demo', name: 'demo', repo_path: '~/code/demo', worktrees: this.worktrees.map(strip), branches: this.branches.map(strip), problem: null }],
      logs: { items: this.logs.length, bytes: this.logs.length * LOG },
      artifacts: { items: 37, bytes: 61e6 },
    };
  }

  private clean(now: number): StorageCleanup {
    const done: StorageCleanup = { worktrees: 0, branches: 0, logs: 0, bytes: 0, problems: [] };
    for (const w of this.worktrees.filter((w) => w.finished && !w.locked && w.state === 'merged')) {
      done.worktrees++;
      done.branches += w.branch ? 1 : 0;
      done.bytes += w.size_bytes;
    }
    this.worktrees = this.worktrees.filter((w) => !(w.finished && !w.locked && w.state === 'merged'));
    const gone = (b: Kept<StoredBranch>) => b.finished && !b.locked && b.unmerged_commits === 0;
    done.branches += this.branches.filter(gone).length;
    this.branches = this.branches.filter((b) => !gone(b));
    if (this.policy.log_days > 0) {
      const old = (t: number) => now - t > this.policy.log_days * DAY;
      done.logs = this.logs.filter(old).length;
      done.bytes += done.logs * LOG;
      this.logs = this.logs.filter((t) => !old(t));
    }
    return done;
  }

  private deleteWorktree(path: string, force: boolean, withBranch: boolean): null {
    const w = this.worktrees.find((x) => x.path === path);
    if (!w) throw new Error(`${path} is not one of Codly's worktrees of this repository`);
    if (w.locked) throw new Error(`it cannot be deleted now: ${w.locked}`);
    const safe = w.state === 'merged';
    if (!safe && !force) throw new Error('not deleted: it has work that is only there; confirm that this can go');
    this.worktrees = this.worktrees.filter((x) => x !== w);
    // The branch stays, and is listed, unless nothing was lost or it was asked for too.
    if (w.branch && w.head && !safe && !withBranch) {
      this.branches.push({ name: w.branch, head: w.head, owner: w.owner, unmerged_commits: w.unmerged_commits, locked: null, last_commit_at: w.last_used_at, finished: w.finished });
    }
    return null;
  }

  private deleteBranch(name: string, force: boolean): null {
    const b = this.branches.find((x) => x.name === name);
    if (!b) throw new Error(`${name} does not exist any more, or a worktree has it checked out`);
    if (b.unmerged_commits > 0 && !force) throw new Error(`not deleted: ${name} has commits that no other branch has; confirm that this can go`);
    this.branches = this.branches.filter((x) => x !== b);
    return null;
  }
}
