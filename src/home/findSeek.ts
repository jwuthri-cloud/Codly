// The find bar opened on a text from somewhere else (a message found by the palette): it goes to
// that text as soon as it is on screen, the conversation may still be loading. Away from the
// component so the rule can be tested.

/** What the find bar seeks: the text it was opened on, and the keys the picked message's row may have (`messageRows`; empty: none). */
export type Seek = { query: string; keys: string[] };

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

/**
 * The keys the row of a message may have in the conversation, the likelier first (spine.ts keys
 * rows so): `c:<id>`, or the key it had while on its way (`keys`); an agent's reply that shows
 * the message it said (`item`) is `say:<item>`.
 */
export const messageRows = (id: string, item: string | null, keys: ReadonlyMap<string, string>): string[] => [keys.get(id) ?? `c:${id}`, ...(item ? [`say:${item}`] : [])];

/** Of the page's row keys (`rows`), the picked message's row: the first of `keys` that is there.
 *  Null: not on the page. Should a CLI give one id to two things, spine adds `#<n>` to the later
 *  row: that is a newer stream of a later session, not the message, so only the key itself counts. */
export const pickRow = (keys: readonly string[], rows: readonly string[]): string | null => keys.find((k) => rows.includes(k)) ?? null;
