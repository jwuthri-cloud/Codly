// Words for Settings → Storage: sizes, what state a worktree is in, and what a delete would
// lose. The core decides what may go (crates/codly-core/src/housekeeping/storage.rs); this
// only says it.
import type { DbBackup } from '../gen/DbBackup';
import type { SetAside } from '../gen/SetAside';
import type { StorageCleanup } from '../gen/StorageCleanup';
import type { StoredBranch } from '../gen/StoredBranch';
import type { StoredWorktree } from '../gen/StoredWorktree';
import { plural } from './format';

/** "0 KB", "640 KB", "12 MB", "1.4 GB" (1000-based, as Finder shows sizes). */
export function bytes(n: number): string {
  if (n < 1e6) return `${Math.round(n / 1e3)} KB`;
  if (n < 1e9) return `${Math.round(n / 1e6)} MB`;
  return `${(n / 1e9).toFixed(1)} GB`;
}

/** What a copy of Codly's database is: made once a day, or before an update changed the data. */
export const backupText = (b: Pick<DbBackup, 'kind'>): string => (b.kind === 'daily' ? 'Daily copy' : 'Copy from before an update');

/** What a start from a backup, or with no data, moved aside: the older backups, or the database Codly could not use. */
export const setAsideText = (s: Pick<SetAside, 'kind'>): string => (s.kind === 'backups' ? 'Backups of the data you had before' : 'Database that Codly could not use');

/** What deleting the backups that a start with no data moved aside loses, for the confirm. */
export const setAsideLossText = 'You lose every backup of the data you had before you started with no data. They cannot be brought back.';

/** What deleting one backup loses, for the confirm. `others`: how many more backups stay. */
export function backupLossText(others: number): string {
  if (others > 0) return `Codly can no longer start from this copy. ${plural(others, 'other backup')} ${others === 1 ? 'stays' : 'stay'}.`;
  return 'This is the only backup. Until Codly makes the next one, there is no copy to start from if its data is damaged.';
}

/** One line for the list: "Merged, clean", "3 files not committed, 2 commits not merged". */
export function stateText(w: Pick<StoredWorktree, 'state' | 'dirty_count' | 'unmerged_commits'>): string {
  if (w.state === 'merged') return 'Merged, clean';
  if (w.state === 'unknown') return 'Not readable';
  const parts: string[] = [];
  if (w.dirty_count > 0) parts.push(`${plural(w.dirty_count, 'file')} not committed`);
  if (w.unmerged_commits > 0) parts.push(`${plural(w.unmerged_commits, 'commit')} not merged`);
  return parts.join(', ');
}

/** "a.rs, b.rs, c.rs and 2 more". */
function names(files: string[], total: number): string {
  const shown = files.slice(0, 3);
  const more = total - shown.length;
  return more > 0 ? `${shown.join(', ')} and ${more} more` : shown.join(', ');
}

/**
 * What deleting a dirty or unmerged worktree loses, for the confirm. `withBranch`: the user
 * ticked "delete the branch too". Commits on a branch that stays are not lost.
 */
export function lossText(w: Pick<StoredWorktree, 'branch' | 'dirty_files' | 'dirty_count' | 'unmerged_commits'>, withBranch: boolean): string {
  const lost: string[] = [];
  if (w.dirty_count > 0) lost.push(`${plural(w.dirty_count, 'file')} that ${w.dirty_count === 1 ? 'is' : 'are'} not committed (${names(w.dirty_files, w.dirty_count)})`);
  if (w.unmerged_commits > 0 && (withBranch || !w.branch)) {
    lost.push(`${plural(w.unmerged_commits, 'commit')} that ${w.branch ? 'no other branch has' : w.unmerged_commits === 1 ? 'is on no branch' : 'are on no branch'}`);
  }
  const stay = `${plural(w.unmerged_commits, 'commit')} ${w.unmerged_commits === 1 ? 'stays' : 'stay'} on ${w.branch}`;
  if (!lost.length) return `Nothing is lost: its ${stay}.`;
  const kept = w.unmerged_commits > 0 && w.branch && !withBranch ? ` Its ${stay}.` : '';
  return `You lose ${lost.join(' and ')}.${kept}`;
}

/** What deleting a branch loses. */
export function branchLossText(b: Pick<StoredBranch, 'unmerged_commits'>): string {
  return `You lose ${plural(b.unmerged_commits, 'commit')} that no other branch has.`;
}

/** The toast after "Clean up now": what went, or that there was nothing. */
export function cleanupText(c: StorageCleanup): string {
  const parts = [
    c.worktrees ? plural(c.worktrees, 'worktree') : '',
    c.branches ? plural(c.branches, 'branch', 'branches') : '',
    c.logs ? plural(c.logs, 'run log') : '',
  ].filter(Boolean);
  if (!parts.length) return 'Nothing to clean up.';
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}` : parts[0];
  return `Removed ${list} (${bytes(c.bytes)}).`;
}
