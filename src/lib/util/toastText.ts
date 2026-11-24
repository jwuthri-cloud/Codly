// A toast's title and text as they are drawn (components/Toasts.svelte). Much of it comes from
// elsewhere (the core's errors, a CLI's words), with Markdown's backticks around code and a
// small first letter. Pure.

/** A piece of a toast's text: words, or code (drawn as code, without its backticks). */
export type Part = { text: string; code: boolean };

/**
 * `text` in pieces: what stands between two backticks is code; a backtick with no other one
 * after it stays as it is. A first word that is a plain small word gets a capital letter
 * ("checks failed" → "Checks failed"); one that is not a plain word (a path, a branch with a
 * "/", code) is left as it is written.
 */
export function toastParts(text: string): Part[] {
  const pieces = text.split('`');
  // An odd number of backticks: the last one opens nothing.
  if (pieces.length % 2 === 0) pieces.splice(-2, 2, `${pieces[pieces.length - 2]}\`${pieces[pieces.length - 1]}`);
  const parts = pieces.map((piece, i) => ({ text: piece, code: i % 2 === 1 })).filter((p) => p.text);
  const first = parts[0];
  if (first && !first.code) parts[0] = { text: first.text.replace(/^[a-z](?=[a-z]*(?:[\s,:;!?'’]|$))/, (c) => c.toUpperCase()), code: false };
  return parts;
}
