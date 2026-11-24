// The find bar opened on a text from somewhere else (a message found by the palette): it goes to
// that text as soon as it is on screen, the conversation may still be loading. Away from the
// component so the rule can be tested.

/** What the find bar seeks: the text it was opened on, and the row of the message picked (`key`, as the conversation marks its rows; null: none). */
export type Seek = { query: string; key: string | null };

/**
 * After a search for `query` that found `total` matches (`inRow`: the first of them in the
 * picked row, -1: none there), what the bar does. A seek ends at the first search that finds
 * something: the bar goes to the picked row's match, else to the first match in view (`'view'`).
 * It ends too as soon as the text changes: a match that streams in later for another text must
 * not move the conversation by itself. `go` null: no jump, the match you are on stays.
 */
export function afterSearch(seek: Seek | null, query: string, total: number, inRow: number): { seek: Seek | null; go: number | 'view' | null } {
  if (!seek || seek.query !== query) return { seek: null, go: null };
  if (!total) return { seek, go: null };
  return { seek: null, go: inRow >= 0 ? inRow : 'view' };
}

/** The row of a message in the conversation (spine.ts keys a message's row so). */
export const messageRow = (id: string, keys: ReadonlyMap<string, string>): string => keys.get(id) ?? `c:${id}`;
