// A code search can answer from an index that is behind the files on disk; the result then
// says so (`SearchResult.warnings`). Every view that shows the matches of `api.search` must
// show these lines too: without them a stale match looks like a current one. Pins the source
// of every .svelte file. The exact list of the views that search is the point of the first
// test: a new view that calls the search fails it until it is added there, and so is checked.
import { describe, expect, it } from 'vitest';

const views = import.meta.glob<string>(['/src/**/*.svelte'], { query: '?raw', import: 'default', eager: true });
const searching = Object.entries(views).filter(([, source]) => /\bapi\s*\.\s*search\s*\(/.test(source));

describe('every view that searches the code', () => {
  it('is found: the palette and the two Knowledge lenses', () => {
    expect(searching.map(([file]) => file.split('/').pop()).sort()).toEqual(['FindLens.svelte', 'Palette.svelte', 'SearchLens.svelte']);
  });

  it('shows the warnings of the result', () => {
    const silent = searching.filter(([, source]) => !/\{#each [^}]*\.warnings\b[^}]*\}/.test(source)).map(([file]) => file);
    expect(silent).toEqual([]);
  });
});
