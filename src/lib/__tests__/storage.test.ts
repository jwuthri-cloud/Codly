import { describe, expect, it } from 'vitest';
import { StorageDemo } from '../demo/storage';
import type { StorageCleanup } from '../gen/StorageCleanup';
import type { StorageReport } from '../gen/StorageReport';
import { backupLossText, backupText, branchLossText, bytes, cleanupText, lossText, setAsideText, stateText } from '../util/storage';

describe('storage words', () => {
  it('sizes read as in Finder', () => {
    expect(bytes(0)).toBe('0 KB');
    expect(bytes(640_000)).toBe('640 KB');
    expect(bytes(12_400_000)).toBe('12 MB');
    expect(bytes(1_440_000_000)).toBe('1.4 GB');
  });

  it('a state says what is not safe yet', () => {
    expect(stateText({ state: 'merged', dirty_count: 0, unmerged_commits: 0 })).toBe('Merged, clean');
    expect(stateText({ state: 'unmerged', dirty_count: 0, unmerged_commits: 1 })).toBe('1 commit not merged');
    expect(stateText({ state: 'dirty', dirty_count: 3, unmerged_commits: 2 })).toBe('3 files not committed, 2 commits not merged');
    expect(stateText({ state: 'unknown', dirty_count: 0, unmerged_commits: 0 })).toBe('Not readable');
  });

  it('the confirm names what is lost, and what stays on the branch', () => {
    const w = { branch: 'codly/p/t1', dirty_files: ['a.rs', 'b.rs', 'c.rs', 'd.rs'], dirty_count: 5, unmerged_commits: 2 };
    expect(lossText(w, false)).toBe('You lose 5 files that are not committed (a.rs, b.rs, c.rs and 2 more). Its 2 commits stay on codly/p/t1.');
    expect(lossText(w, true)).toBe('You lose 5 files that are not committed (a.rs, b.rs, c.rs and 2 more) and 2 commits that no other branch has.');
    // Only commits, and the branch stays: nothing goes.
    expect(lossText({ branch: 'codly/p/t1', dirty_files: [], dirty_count: 0, unmerged_commits: 1 }, false)).toBe('Nothing is lost: its 1 commit stays on codly/p/t1.');
    // On no branch, the commits go with the folder whatever is ticked.
    expect(lossText({ branch: null, dirty_files: ['x'], dirty_count: 1, unmerged_commits: 1 }, false)).toBe('You lose 1 file that is not committed (x) and 1 commit that is on no branch.');
    expect(branchLossText({ unmerged_commits: 3 })).toBe('You lose 3 commits that no other branch has.');
  });

  it('a backup says when Codly made it', () => {
    expect(backupText({ kind: 'daily' })).toBe('Daily copy');
    expect(backupText({ kind: 'update' })).toBe('Copy from before an update');
  });

  it('what was moved aside is named, and the confirm of a backup says what is left', () => {
    expect(setAsideText({ kind: 'backups' })).toBe('Backups of the data you had before');
    expect(setAsideText({ kind: 'database' })).toBe('Database that Codly could not use');
    expect(backupLossText(3)).toBe('Codly can no longer start from this copy. 3 other backups stay.');
    expect(backupLossText(1)).toBe('Codly can no longer start from this copy. 1 other backup stays.');
    expect(backupLossText(0)).toBe('This is the only backup. Until Codly makes the next one, there is no copy to start from if its data is damaged.');
  });

  it('a cleanup says what went', () => {
    const none: StorageCleanup = { worktrees: 0, branches: 0, logs: 0, bytes: 0, problems: [] };
    expect(cleanupText(none)).toBe('Nothing to clean up.');
    expect(cleanupText({ ...none, logs: 1, bytes: 2_000_000 })).toBe('Removed 1 run log (2 MB).');
    expect(cleanupText({ ...none, worktrees: 2, branches: 3, logs: 14, bytes: 1_200_000_000 })).toBe('Removed 2 worktrees, 3 branches and 14 run logs (1.2 GB).');
  });
});

describe('the demo follows the core’s storage rules', () => {
  const NOW = 1_800_000_000_000;
  const office = (d: StorageDemo) => (d.handle('storage_report', {}, NOW) as StorageReport).offices[0];
  const worktree = (d: StorageDemo, name: string) => office(d).worktrees.find((w) => w.path.endsWith(name));

  it('a cleanup takes only what is finished, merged, clean and not in use', () => {
    const d = new StorageDemo(NOW);
    const before = office(d);
    const c = d.handle('storage_clean', {}, NOW) as StorageCleanup;
    expect([c.worktrees, c.branches]).toEqual([1, 2]);
    expect(c.logs).toBeGreaterThan(0);
    const after = office(d);
    expect(after.worktrees.map((w) => w.state)).toEqual(['dirty', 'unmerged', 'unmerged']);
    expect(after.worktrees).toHaveLength(before.worktrees.length - 1);
    expect(after.branches.map((b) => b.unmerged_commits)).toEqual([2]);
    expect((d.handle('storage_clean', {}, NOW) as StorageCleanup).worktrees).toBe(0);
  });

  it('the report lists the backups of the database, newest first, and a cleanup leaves them', () => {
    const d = new StorageDemo(NOW);
    const backups = () => (d.handle('storage_report', {}, NOW) as StorageReport).backups;
    const before = backups();
    expect(before.map((b) => b.kind)).toEqual(['daily', 'daily', 'daily', 'update']);
    expect(before.map((b) => b.created_at)).toEqual([...before.map((b) => b.created_at)].sort((a, b) => b - a));
    d.handle('storage_clean', {}, NOW);
    expect(backups()).toEqual(before);
  });

  it('a backup can be deleted, a database cannot', () => {
    const d = new StorageDemo(NOW);
    const report = () => d.handle('storage_report', {}, NOW) as StorageReport;
    const before = report();
    expect(before.set_aside.map((s) => s.kind)).toEqual(['backups', 'database']);
    const [folder, database] = before.set_aside;
    // Not a backup: the database that was moved aside, the database in use, a path that is not listed.
    for (const path of [database.path, database.path.replace(/codly\.broken-\d+/, 'codly'), '/etc/hosts']) {
      expect(() => d.handle('storage_delete_backup', { path }, NOW)).toThrow(/not a backup/);
    }
    expect(report()).toEqual(before);

    d.handle('storage_delete_backup', { path: before.backups[0].path }, NOW);
    expect(report().backups).toEqual(before.backups.slice(1));
    d.handle('storage_delete_backup', { path: folder.path }, NOW);
    expect(report().set_aside).toEqual([database]);
    expect(() => d.handle('storage_delete_backup', { path: folder.path }, NOW)).toThrow(/not a backup/);
  });

  it('logs stay for ever when the limit is 0', () => {
    const d = new StorageDemo(NOW);
    d.handle('storage_policy', { policy: { log_days: 0, clean_merged: true } }, NOW);
    expect((d.handle('storage_clean', {}, NOW) as StorageCleanup).logs).toBe(0);
  });

  it('dirty work goes only after a confirm, and its branch stays unless asked for', () => {
    const d = new StorageDemo(NOW);
    const dirty = worktree(d, 'dark-mode-9f3e/t2')!;
    const args = { officeId: 'demo', path: dirty.path, head: dirty.head };
    expect(() => d.handle('storage_delete_worktree', { ...args, force: false, deleteBranch: false }, NOW)).toThrow(/confirm/);
    d.handle('storage_delete_worktree', { ...args, force: true, deleteBranch: false }, NOW);
    expect(worktree(d, 'dark-mode-9f3e/t2')).toBeUndefined();
    expect(office(d).branches.map((b) => b.name)).toContain('codly/dark-mode-9f3e/t2');

    const unmerged = worktree(d, 'export-csv-77ab/t1')!;
    d.handle('storage_delete_worktree', { officeId: 'demo', path: unmerged.path, head: unmerged.head, force: true, deleteBranch: true }, NOW);
    expect(office(d).branches.map((b) => b.name)).not.toContain('codly/export-csv-77ab/t1');

    const running = worktree(d, 'search-2c41/integration')!;
    expect(() => d.handle('storage_delete_worktree', { officeId: 'demo', path: running.path, head: running.head, force: true, deleteBranch: true }, NOW)).toThrow(/not finished/);
    expect(() => d.handle('storage_delete_branch', { officeId: 'demo', branch: 'codly/export-csv-77ab/integration', head: 'x', force: false }, NOW)).toThrow(/confirm/);
    d.handle('storage_delete_branch', { officeId: 'demo', branch: 'codly/export-csv-77ab/integration', head: 'x', force: true }, NOW);
    expect(office(d).branches.map((b) => b.name)).not.toContain('codly/export-csv-77ab/integration');
  });
});
