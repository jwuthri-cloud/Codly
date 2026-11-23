// A code search (`api.search`) that failed, in the words a view shows.

/** The core's own words when the open repo has no index yet (`search` in orchestrator/workspace.rs). */
const NOT_READY = /code index is not ready/i;

/**
 * Why a code search gave nothing: the index is not ready (only when the core says so), else that
 * the search failed, with the core's reason. Both can work a moment later: the view offers Try again.
 */
export function searchFailure(e: unknown): string {
  const why = e instanceof Error ? e.message : String(e);
  return NOT_READY.test(why) ? 'The code index is not ready yet.' : `Code search failed: ${why}`;
}
