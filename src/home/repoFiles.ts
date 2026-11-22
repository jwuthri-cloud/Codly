// The repo's files for the composer's `@` menu: the paths the code index knows (a file it has
// not indexed is not listed). Asked for when the menu opens and kept for a minute: the list is
// big, and it changes as agents work.
import { api } from '../lib/api';

const FRESH_MS = 60_000;
let held: { office: string; at: number; paths: Promise<string[]> } | null = null;

/** The indexed files of office `office`, as paths from the repo's root. */
export function repoFiles(office: string, now = Date.now()): Promise<string[]> {
  if (held && held.office === office && now - held.at <= FRESH_MS) return held.paths;
  const paths = api.repoMap().then((files) => files.map((f) => f.path));
  const mine = (held = { office, at: now, paths });
  // An empty list (the index is still being built) or a failed read is not kept: the next `@` asks again.
  paths
    .then((p) => p.length > 0, () => false)
    .then((keep) => {
      if (!keep && held === mine) held = null;
    });
  return paths;
}
