// Keys for the rows of a keyed `{#each}`. Svelte stops updating a list when two of its rows have
// the same key (`each_key_duplicate`), so a key that comes from outside Codly (a CLI's call id)
// must not be trusted to be unique.

/**
 * One key per row, in the rows' order, and no two the same. A row keeps the key `keyOf` gives
 * it; a later row with a key that is taken adds its place among them ("call-1", "call-1#2"), so
 * the rows before it keep theirs when it arrives. For rows with a state of their own (an open
 * step, an animation, the focus); rows that are only text can use their index.
 */
export function uniqueKeys<T>(rows: readonly T[], keyOf: (row: T) => string): string[] {
  const used = new Set<string>();
  return rows.map((row) => {
    const own = keyOf(row);
    let key = own;
    for (let n = 2; used.has(key); n++) key = `${own}#${n}`;
    used.add(key);
    return key;
  });
}
