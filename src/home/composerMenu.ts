// What the composer's menu shows as you type: `@` an agent or a file of the repo, `#` a task,
// `/` one of Codly's own actions. The parsing and the ranking live here, away from the
// component, so they can be tested.
import { fuzzy } from '../lib/util/fuzzy';

export type Trigger = {
  trigger: '@' | '#' | '/';
  /** Where the trigger character is in the text. */
  start: number;
  /** What is typed after it, up to the caret (for `/`: the whole word). */
  query: string;
  /** Nothing but spaces before it ("@Ada" at the start talks to Ada 1:1). */
  atStart: boolean;
  /** "/ model": spaces after the slash. Only the action of that very name is offered. */
  exact?: true;
};

/**
 * The menu the caret is in, if any. `@` and `#` start a word anywhere; a file's path may hold
 * `/`, `.` and `-`. `/` opens the actions only when the message is nothing but that one word
 * (the whole word is the query, wherever the caret is in it): a path or a sentence that starts
 * with a slash stays plain text. "/ model" (spaces after the slash) is the action's name typed
 * with a space too much: the menu offers that action only (`exact`).
 */
export function triggerAt(text: string, caret: number): Trigger | null {
  const slash = /^\/([\w-]*)\s*$/.exec(text);
  if (slash) return caret > 0 ? { trigger: '/', start: 0, query: slash[1], atStart: true } : null;
  const spaced = /^\/\s+([\w-]+)\s*$/.exec(text);
  if (spaced) return caret > 0 ? { trigger: '/', start: 0, query: spaced[1], atStart: true, exact: true } : null;
  const before = text.slice(0, caret);
  const m = /(^|\s)([@#])([\w./-]*)$/.exec(before);
  if (!m) return null;
  const start = caret - m[3].length - 1;
  return { trigger: m[2] as '@' | '#', start, query: m[3], atStart: m[2] === '@' && before.slice(0, start).trim() === '' };
}

/** A file in a message: its path from the repo's root, in backticks, so the agent reads it by path. */
export const fileMention = (path: string): string => `\`${path}\` `;

const baseOf = (path: string): string => path.slice(path.lastIndexOf('/') + 1);

/**
 * The files that match what is typed after `@`, best first. A match in the file's name beats
 * one in its folders; a query with a `/` is matched against the whole path. Nothing typed: no
 * files (a repo has too many to list).
 */
export function rankFiles(paths: string[], query: string, limit: number): string[] {
  if (!query) return [];
  const whole = query.includes('/');
  const found: { path: string; score: number }[] = [];
  for (const path of paths) {
    const name = whole ? null : fuzzy(query, baseOf(path));
    const m = name ?? fuzzy(query, path);
    if (m) found.push({ path, score: m.score + (name ? 1000 : 0) });
  }
  return found
    .sort((a, b) => b.score - a.score || a.path.length - b.path.length || a.path.localeCompare(b.path))
    .slice(0, limit)
    .map((f) => f.path);
}

/**
 * One of Codly's actions in the `/` menu. `name` is what you type after the slash. An action
 * that cannot run here or right now has `off`, the reason, in place of what it does: it is
 * still listed, so its name is never sent to the agent as a message. `wait`: it is off only
 * until the core answers whether it can run, a moment from now.
 */
export type SlashCommand = { name: string; label: string; icon: string } & ({ hint: string; run: () => void } | { off: string; wait?: true });

/** What the `@` menu says in place of the repo's files, if anything: the list could not be read,
 *  or it is still being read (on its way, or the code index has no file yet). */
export function filesLine(s: { query: string; failed: boolean; reading: boolean; count: number }): string | null {
  if (!s.query) return null;
  if (s.failed) return 'Couldn’t list the repo’s files';
  return s.reading || !s.count ? 'The file list is still being read' : null;
}

/**
 * The actions that match what is typed after `/`: those whose name starts with it first, in
 * the list's order, then those that hold it in their name or label. Nothing typed: all of them.
 * In each case the actions that cannot run now come last. One that waits for the core's answer
 * (`wait`) keeps its place: while you wait, ↵ on its name runs nothing, and never the action
 * that would be next in its place.
 */
export function filterSlash<T extends { name: string; label: string; off?: string; wait?: boolean }>(commands: T[], query: string): T[] {
  const q = query.toLowerCase();
  const starts = commands.filter((c) => c.name.startsWith(q));
  const holds = q ? commands.filter((c) => !c.name.startsWith(q) && (c.name.includes(q) || c.label.toLowerCase().includes(q))) : [];
  const found = [...starts, ...holds];
  const refused = (c: T) => c.off !== undefined && !c.wait;
  return [...found.filter((c) => !refused(c)), ...found.filter(refused)];
}

/**
 * A message whose first word is a slash command that Codly does not have: it is not sent (it
 * would start a turn the agent is paid for), and the composer says so. The first word is all
 * that follows the "/" up to a space; its name is the letters, digits, `-` and `_` it starts
 * with. It counts when it is a name that is not an action ("/review this"), a name with more
 * after it that is not a path ("/help?", "/model:opus", "/undo."), or a name with more slashes
 * after it that starts with an action's name ("/mcp/terminal …"). An action's name alone after
 * spaces ("/ model") is that action typed with a space too much. A path ("/usr/bin/env", alone
 * or in front of more words), "/" alone or in front of words ("/ is the root") and a slash later
 * in the text are plain text. `closest`: the action you most likely meant, null when none is near.
 */
export function unknownSlash(text: string, names: readonly string[]): { word: string; closest: string | null } | null {
  const spaced = /^\s*\/(\s+([\w-]+))\s*$/.exec(text);
  if (spaced && names.includes(spaced[2].toLowerCase())) return { word: spaced[1], closest: spaced[2].toLowerCase() };
  const m = /^\s*\/(\S*)(.*)$/s.exec(text);
  if (!m || !m[1]) return null;
  const [, word, rest] = m;
  const name = /^[\w-]*/.exec(word)![0].toLowerCase();
  // "/?" or "/..." alone is no name, but sent it would still start a turn.
  if (!name) return rest.trim() ? null : { word, closest: null };
  const known = names.includes(name);
  const tail = word.slice(name.length);
  if (!tail) return known ? null : { word, closest: closestName(name, names) };
  if (tail.includes('/') && !known) return null;
  return { word, closest: known ? name : closestName(name, names) };
}

/** What the composer says about it, above the line: ↵ sends nothing until the text changes. */
export function unknownSlashLine(u: { word: string; closest: string | null }): string {
  return `No such command: /${u.word}. ${u.closest ? `Did you mean /${u.closest}?` : 'Type / alone to see the actions.'} ↵ sends nothing until you change it.`;
}

/**
 * The name that is fewest letters away from `word` (a typo: a letter missing, extra, changed or
 * two swapped). A short word allows fewer: 1 typo up to 4 letters, 2 for a longer one, and never
 * more than half its letters ("/x" is near no action).
 */
function closestName(word: string, names: readonly string[]): string | null {
  const allowed = Math.min(word.length <= 4 ? 1 : 2, Math.floor(word.length / 2));
  let best: { name: string; d: number } | null = null;
  for (const name of names) {
    const d = typos(word, name);
    if (d <= allowed && (!best || d < best.d)) best = { name, d };
  }
  return best?.name ?? null;
}

/** How many typos turn `a` into `b`: letters added, removed or changed, and two neighbours swapped. */
function typos(a: string, b: string): number {
  const d = Array.from({ length: a.length + 1 }, (_, i) => Array.from({ length: b.length + 1 }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)));
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
    }
  }
  return d[a.length][b.length];
}

/**
 * Whether the pointer really moved over the menu: only then does the row under it become the
 * one ↵ runs. A list that scrolls under a resting pointer sends it events at the same place,
 * and those must not take the row from the arrow keys. The first event only tells where it is.
 */
export function pointerMoved(last: { x: number; y: number } | null, now: { x: number; y: number }): boolean {
  return !!last && (last.x !== now.x || last.y !== now.y);
}

/**
 * Which pointer events over the menu pick the row under them (`pointerMoved`). The menu forgets
 * where the pointer was each time it opens: a menu that opens under a resting pointer keeps its
 * first row until the pointer really moves.
 */
export class MenuPointer {
  private last: { x: number; y: number } | null = null;

  /** The menu opened: the next event only tells where the pointer is. */
  opened() {
    this.last = null;
  }

  /** Does this event pick the row under the pointer? */
  picks(now: { x: number; y: number }): boolean {
    const moved = pointerMoved(this.last, now);
    this.last = now;
    return moved;
  }
}
