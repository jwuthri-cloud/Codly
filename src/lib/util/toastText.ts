// A toast's title and text as they are drawn (components/Toasts.svelte). Much of it comes from
// elsewhere (the core's errors, a CLI's words), with Markdown's backticks around code. Pure.

/** A piece of a toast's text: words, or code (drawn as code, without its backticks). */
export type Part = { text: string; code: boolean };

/**
 * `text` in pieces: what stands between two backticks is code; a backtick with no other one
 * after it stays as it is. No letter is changed: a text can start with a name (a branch, an
 * office, a file) whose case counts, and what is drawn is what you copy.
 */
export function toastParts(text: string): Part[] {
  const pieces = text.split('`');
  // An odd number of backticks: the last one opens nothing.
  if (pieces.length % 2 === 0) pieces.splice(-2, 2, `${pieces[pieces.length - 2]}\`${pieces[pieces.length - 1]}`);
  return pieces.map((piece, i) => ({ text: piece, code: i % 2 === 1 })).filter((p) => p.text);
}
