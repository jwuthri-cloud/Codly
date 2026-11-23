// The little markdown that one-line texts carry (a task's goal, a memory, a message's first
// words): `code` spans and **bold**. InlineText.svelte renders the parts as text nodes, so
// nothing in the text is ever parsed as HTML. Everything else stays as it was written.

export type InlinePart = { kind: 'text' | 'code' | 'bold'; t: string };

// These texts are often commands (a task's check), where `**` is an operator: stars are a bold
// pair only with no space just inside them and no letter, digit or star just outside them, so
// `x ** 2 + y ** 2`, `x**2 + y**2` and `f(**a, **b)` stay as written. The character before the
// pair is checked in the loop below: WKWebView had no lookbehind before macOS 13.3.
const MARKED = /`[^`\n]+`|\*\*[^\s*](?:[^*\n]*[^\s*])?\*\*(?![\w*])/g;
const WORD_OR_STAR = /[\w*]/;

/** `text` split into plain text, `code` spans and **bold** spans, in order, without their marks. */
export function inlineParts(text: string): InlinePart[] {
  const parts: InlinePart[] = [];
  const marked = new RegExp(MARKED);
  let end = 0; // where the last marked span ended
  for (let m = marked.exec(text); m; m = marked.exec(text)) {
    const bold = m[0].startsWith('**');
    if (bold && WORD_OR_STAR.test(text.charAt(m.index - 1))) {
      // Not a pair: look again right after these two stars (a code span can start inside).
      marked.lastIndex = m.index + 2;
      continue;
    }
    if (m.index > end) parts.push({ kind: 'text', t: text.slice(end, m.index) });
    parts.push(bold ? { kind: 'bold', t: m[0].slice(2, -2) } : { kind: 'code', t: m[0].slice(1, -1) });
    end = m.index + m[0].length;
  }
  if (end < text.length) parts.push({ kind: 'text', t: text.slice(end) });
  return parts;
}
