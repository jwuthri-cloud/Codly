// ↑ in an empty composer brings back what you sent in this thread, newest first, like a shell.
// ↑ again goes further back; ↓ comes forward, and past the newest message the line is empty.

/** `at`: how far back you are (0 = your newest message). */
export type Recalled = { at: number; text: string };

/**
 * One step through `sent` (your messages, oldest first) from `at` (null: not recalling).
 * Returns where you land (`at` null: back on the empty line), or null when there is nothing
 * that way and the key should do what it usually does.
 */
export function recall(sent: string[], at: number | null, dir: 'older' | 'newer'): { at: number | null; text: string } | null {
  const next = dir === 'older' ? (at == null ? 0 : at + 1) : at == null ? null : at - 1;
  if (next == null) return null;
  if (next < 0) return { at: null, text: '' };
  const text = sent[sent.length - 1 - next];
  return text === undefined ? null : { at: next, text };
}
