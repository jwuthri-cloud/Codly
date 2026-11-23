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

/** How a code search ended, as a view keeps it: its result, or why there is none. Never both. */
export type SearchEnd<R> = { res: R; failed: null } | { res: null; failed: string };

/**
 * A code search on its way → how it ended. A search that failed is not a search that was never
 * asked: the view shows `failed` (`searchFailure`) with Try again, not its empty state.
 */
export const searchEnd = <R>(search: Promise<R>): Promise<SearchEnd<R>> =>
  search.then(
    (res): SearchEnd<R> => ({ res, failed: null }),
    (e: unknown): SearchEnd<R> => ({ res: null, failed: searchFailure(e) }),
  );
