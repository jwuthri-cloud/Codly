// Work of the agent sheet that walks the event log (History's rows and facts, the Traces tab,
// the MCP section's line, the compaction line of Controls). It is done only while the part that
// shows it is open, and again only when its key changes: the key holds the agent's own version
// (`store.revBy`, within the snapshot it counts from), so an event of another agent costs it
// nothing, and a snapshot that brought missed events builds it again.
// One place, so the sheet and the tests agree on when the work runs.
import { store } from '../lib/state/store.svelte';

/** A value worked out from the log, kept until its key changes. */
export type LogWork<T> = {
  /** The value for `key` while `open` (built again only when `key` changed), else null. */
  read(open: boolean, key: string): T | null;
};

/**
 * `build` runs only on a read that is open and has a key other than the last build's. A read
 * while closed builds nothing and keeps the last value for when the part opens again with the
 * same key.
 */
export function logWork<T>(build: () => T): LogWork<T> {
  let built: { key: string; value: T } | null = null;
  return {
    read(open, key) {
      if (!open) return null;
      if (built?.key !== key) built = { key, value: build() };
      return built.value;
    },
  };
}

/**
 * The key of one agent's log work: the agent, its own version and what else the work reads
 * (`more`: a filter, a run, a task). A snapshot starts the per-agent counts at 0 again, so the
 * count alone can repeat across one: the snapshot's number goes with it. Read inside a
 * `$derived`, it runs again on this agent's events and on a snapshot, and on no other agent's.
 */
export const agentLogKey = (agentId: string, ...more: (string | number | null | undefined)[]): string =>
  [agentId, store.snapshots, store.revBy.get(agentId) ?? 0, ...more.map((m) => m ?? '')].join('\u0000');
