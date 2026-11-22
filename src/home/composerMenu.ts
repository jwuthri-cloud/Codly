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
};

/**
 * The menu the caret is in, if any. `@` and `#` start a word anywhere; a file's path may hold
 * `/`, `.` and `-`. `/` opens the actions only when the message is nothing but that one word
 * (the whole word is the query, wherever the caret is in it): a path or a sentence that starts
 * with a slash stays plain text.
 */
export function triggerAt(text: string, caret: number): Trigger | null {
  const slash = /^\/([\w-]*)\s*$/.exec(text);
  if (slash) return caret > 0 ? { trigger: '/', start: 0, query: slash[1], atStart: true } : null;
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

/** One of Codly's actions in the `/` menu. `name` is what you type after the slash. */
export type SlashCommand = { name: string; label: string; hint: string; icon: string; run: () => void };

/**
 * The actions that match what is typed after `/`: those whose name starts with it first, in
 * the list's order, then those that hold it in their name or label. Nothing typed: all of them.
 */
export function filterSlash<T extends { name: string; label: string }>(commands: T[], query: string): T[] {
  const q = query.toLowerCase();
  if (!q) return commands;
  const starts = commands.filter((c) => c.name.startsWith(q));
  const holds = commands.filter((c) => !c.name.startsWith(q) && (c.name.includes(q) || c.label.toLowerCase().includes(q)));
  return [...starts, ...holds];
}
