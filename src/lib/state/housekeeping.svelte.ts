// The board's housekeeping on the UI side: which tasks are archived (hidden from the lists),
// what is flagged, and the "Archived" filter with its search. The core does the judging and
// keeps the archive file (crates/codly-core/src/housekeeping/archive.rs); this only asks.
import { api } from '../api';
import type { ArchivedItem } from '../gen/ArchivedItem';
import type { BoardFlag } from '../gen/BoardFlag';
import type { Task } from '../gen/Task';

class Housekeeping {
  /** Tasks on the archive file: the lists leave them out. */
  archivedIds = $state.raw<Set<string>>(new Set());
  flags = $state.raw<BoardFlag[]>([]);
  /** The "Archived" filter is on: show the archive instead of the board. */
  showArchived = $state(false);
  query = $state('');
  results = $state.raw<ArchivedItem[]>([]);
  loading = $state(false);
  private seq = 0;

  /** Archive what is due and refresh flags. Quiet on failure (no workspace yet, old core). */
  async sweep() {
    try {
      const r = await api.boardSweep();
      this.archivedIds = new Set(r.archived_ids);
      this.flags = r.flagged;
      if (this.showArchived || r.newly_archived) void this.search();
    } catch {
      /* nothing to sweep */
    }
  }

  /** Look through the archived text (everything when the query is empty). */
  async search(query = this.query) {
    const mine = ++this.seq;
    this.loading = true;
    try {
      const r = await api.boardArchive(query.trim() || null);
      if (mine === this.seq) this.results = r;
    } catch {
      if (mine === this.seq) this.results = [];
    } finally {
      if (mine === this.seq) this.loading = false;
    }
  }

  setArchived(on: boolean) {
    this.showArchived = on;
    if (on) void this.search();
  }

  /** Tasks that stay on the board. */
  live(tasks: Task[]): Task[] {
    return this.archivedIds.size ? tasks.filter((t) => !this.archivedIds.has(t.id)) : tasks;
  }

  flagOf(taskId: string): BoardFlag | undefined {
    return this.flags.find((f) => f.id === taskId);
  }
}

export const housekeeping = new Housekeeping();
