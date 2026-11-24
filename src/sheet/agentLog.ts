// Work of the agent sheet that walks the event log (History's rows and facts, the Traces tab,
// the MCP section's line, the compaction line of Controls). It is done only while the part that
// shows it is open, and again only when its key changes: the key holds the agent's own version
// (`store.revBy`), so an event of another agent costs it nothing.
// Pure, so the sheet and the tests agree on when the work runs.

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

/** The key of one agent's log work: the agent, its own version and what else the work reads (`more`: a filter, a run, a task). */
export const agentLogKey = (agentId: string, rev: number | undefined, ...more: (string | number | null | undefined)[]): string =>
  [agentId, rev ?? 0, ...more.map((m) => m ?? '')].join('\u0000');
