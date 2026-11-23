// A code search can answer from an index that is behind the files on disk; the result then
// says so (`SearchResult.warnings`). Every view that shows the matches of `api.search` must
// show these lines too: without them a stale match looks like a current one.
import { describe, expect, it } from 'vitest';

const views = import.meta.glob<string>(['/src/**/*.svelte'], { query: '?raw', import: 'default', eager: true });
const searching = Object.entries(views).filter(([, source]) => source.includes('api.search('));

describe('every view that searches the code', () => {
  it('is found: the palette and the two Knowledge lenses', () => {
    expect(searching.map(([file]) => file.split('/').pop()).sort()).toEqual(['FindLens.svelte', 'Palette.svelte', 'SearchLens.svelte']);
  });

  it('shows the warnings of the result', () => {
    const silent = searching.filter(([, source]) => !/\{#each [^}]*\.warnings\b[^}]*\}/.test(source)).map(([file]) => file);
    expect(silent).toEqual([]);
  });
});
