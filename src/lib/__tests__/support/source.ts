// For the tests that pin a fact written in another file's source (a Rust constant, a CSS
// rule): they read that text by its structure, not by its layout, so a reformat (a line break,
// more spaces, the other quote) keeps them passing and only a change of the fact fails them.

const OPEN: Record<string, string> = { '(': ')', '[': ']', '{': '}' };

/** Where the text from `at` ends: past a string (one of `quotes`) or a comment, else `at` itself. */
function skip(source: string, at: number, quotes: string): number {
  const c = source[at];
  if (quotes.includes(c)) {
    let i = at + 1;
    while (i < source.length && source[i] !== c) i += source[i] === '\\' ? 2 : 1;
    return i + 1;
  }
  if (source.startsWith('//', at)) {
    const end = source.indexOf('\n', at);
    return end < 0 ? source.length : end;
  }
  if (source.startsWith('/*', at)) {
    const end = source.indexOf('*/', at + 2);
    return end < 0 ? source.length : end + 2;
  }
  return at;
}

/**
 * The text inside the first bracket (`(`, `[` or `{`) at or after `from`, up to the bracket
 * that matches it. Brackets inside strings (`quotes`) and comments do not count.
 */
export function bracketed(source: string, from: number, quotes = '"'): string {
  let start = from;
  while (start < source.length && !(source[start] in OPEN)) start++;
  if (start >= source.length) throw new Error(`no bracket after offset ${from}`);
  const stack: string[] = [];
  for (let i = start; i < source.length; ) {
    const next = skip(source, i, quotes);
    if (next !== i) {
      i = next;
      continue;
    }
    const c = source[i];
    if (c in OPEN) stack.push(OPEN[c]);
    else if (c === stack[stack.length - 1]) {
      stack.pop();
      if (!stack.length) return source.slice(start + 1, i);
    }
    i++;
  }
  throw new Error(`the bracket at offset ${start} is never closed`);
}

/** The value of the Rust constant `name`: the text after its `=`, up to the `;` that ends it. */
export function rustConst(source: string, name: string): string {
  const head = new RegExp(`\\bconst\\s+${name}\\s*:[^=]*=`).exec(source);
  if (!head) throw new Error(`no const ${name}`);
  const from = head.index + head[0].length;
  for (let i = from; i < source.length; ) {
    const next = skip(source, i, '"');
    if (next !== i) {
      i = next;
      continue;
    }
    if (source[i] in OPEN) {
      i += bracketed(source, i).length + 2;
      continue;
    }
    if (source[i] === ';') return source.slice(from, i).trim();
    i++;
  }
  throw new Error(`const ${name} has no end`);
}

/** The text of the Rust `&str` constant `name`, its escapes read. */
export function rustStr(source: string, name: string): string {
  const value = rustConst(source, name);
  const literal = /^"((?:[^"\\]|\\.)*)"$/.exec(value);
  if (!literal) throw new Error(`const ${name} is not one string literal: ${value}`);
  return literal[1].replace(/\\(.)/g, (_, c: string) => ({ n: '\n', t: '\t' })[c] ?? c);
}

/**
 * The body of the CSS rule whose selector list is `selector`, matched without regard to
 * spacing, line breaks or the quote style of attribute values.
 */
export function cssBlock(source: string, selector: string): string {
  const pattern = selector
    .trim()
    .split(/\s*,\s*/)
    .map((part) =>
      part
        .split(/\s+/)
        .map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/["']/g, `["']`))
        .join('\\s+'),
    )
    .join('\\s*,\\s*');
  const head = new RegExp(`(?:^|[};/])\\s*(${pattern})\\s*\\{`).exec(source);
  if (!head) throw new Error(`no rule for ${selector}`);
  return bracketed(source, head.index + head[0].length - 1, `"'`);
}
