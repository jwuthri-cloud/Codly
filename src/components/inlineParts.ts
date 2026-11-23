// The little markdown that one-line texts carry (a task's goal, a memory, a message's first
// words): `code` spans and **bold**. InlineText.svelte renders the parts as text nodes, so
// nothing in the text is ever parsed as HTML. Everything else stays as it was written.

export type InlinePart = { kind: 'text' | 'code' | 'bold'; t: string };

const MARKED = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/g;

/** `text` split into plain text, `code` spans and **bold** spans, in order, without their marks. */
export function inlineParts(text: string): InlinePart[] {
  // split() with a capturing group: the marked spans are the odd entries, the text between them the even ones.
  return text
    .split(MARKED)
    .map((p, i): InlinePart => (i % 2 === 0 ? { kind: 'text', t: p } : p.startsWith('`') ? { kind: 'code', t: p.slice(1, -1) } : { kind: 'bold', t: p.slice(2, -2) }))
    .filter((p) => p.t !== '');
}
