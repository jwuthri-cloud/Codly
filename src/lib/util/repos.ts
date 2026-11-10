// The browser's "Open repository" picker: filter the repositories the core found, or take a typed path.
import type { RepoEntry } from '../gen/RepoEntry';

/** The repositories whose name or path holds every word of `query` (any case), in list order. */
export function filterRepos(repos: RepoEntry[], query: string): RepoEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return repos;
  return repos.filter((r) => {
    const text = `${r.name} ${r.path}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

/** A full path typed in the search box, to open as is; null when the text is a search. */
export function typedPath(query: string): string | null {
  const q = query.trim();
  return q.startsWith('/') ? q : null;
}
