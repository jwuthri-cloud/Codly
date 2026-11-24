// Every button has a name a screen reader can say. A button whose content is only an icon (or a
// portrait, a thumbnail: no text) needs `aria-label` (or `aria-labelledby`, `title`): its hover
// tip (`data-tip`) is no name. Read from every component's source; the list of exceptions is
// empty.
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob<string>(['/src/**/*.svelte', '!/src/**/__tests__/**'], { query: '?raw', import: 'default', eager: true });

/** Where the tag that starts at `at` (its `<`) ends: past its `>`, over `{…}` expressions and quoted values. */
function tagEnd(s: string, at: number): number {
  let depth = 0;
  let quote: string | null = null;
  for (let i = at + 1; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      if (c === '\\') i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'" || (depth > 0 && c === '`')) quote = c;
    else if (c === '{') depth++;
    else if (c === '}') depth--;
    else if (c === '>' && depth === 0) return i + 1;
  }
  throw new Error(`a tag at ${at} is never closed`);
}

/** What of `html` shows as text: the tags go (an image's `alt` stays), as do Svelte's block tags; `{…}` and `{@render …}` stay. */
function textOf(html: string): string {
  let out = '';
  for (let i = 0; i < html.length; ) {
    if (html.startsWith('<!--', i)) {
      const end = html.indexOf('-->', i);
      i = end < 0 ? html.length : end + 3;
    } else if (html[i] === '<' && /[A-Za-z/]/.test(html[i + 1] ?? '')) {
      const end = tagEnd(html, i);
      out += /^<img\b/.test(html.slice(i)) ? (/\balt=(?:"([^"]*)"|\{([^}]*)\})/.exec(html.slice(i, end))?.slice(1).join('') ?? '') : '';
      i = end;
    } else out += html[i++];
  }
  return out.replace(/\{(?:[#:/]|@const\b)[^}]*\}/g, '').trim();
}

/** The buttons of a component with no name: "file:line". */
function unnamed(file: string, source: string): string[] {
  // Script and style are no markup (their lines are kept, for the line numbers).
  const s = source.replace(/<(script|style)\b[\s\S]*?<\/\1>/g, (m) => m.replace(/[^\n]/g, ' '));
  const out: string[] = [];
  for (let at = s.indexOf('<button'); at >= 0; at = s.indexOf('<button', at + 1)) {
    if (!/[\s>]/.test(s[at + 7])) continue;
    const open = tagEnd(s, at);
    const close = /<\/button\s*>/.exec(s.slice(open));
    if (!close) throw new Error(`${file}: a button is never closed`);
    const attrs = s.slice(at, open);
    if (/\s(aria-label|aria-labelledby|title)=/.test(attrs)) continue;
    if (!textOf(s.slice(open, open + close.index))) out.push(`${file}:${s.slice(0, at).split('\n').length}`);
  }
  return out;
}

describe('the name of a button', () => {
  it('reads what a button with an icon only is called, and finds one without a name', () => {
    expect(unnamed('a', '<button class="x" onclick={() => (open = !open)} data-tip="Close"><Icon name="x" /></button>')).toEqual(['a:1']);
    expect(unnamed('a', '<button aria-label="Close" onclick={() => go()}><Icon name="x" /></button>')).toEqual([]);
    expect(unnamed('a', '<button>{#if on}<Icon name="check" />{/if}Save</button>')).toEqual([]);
    expect(unnamed('a', '<button>{@render keyed(label, kbd)}</button>')).toEqual([]);
    expect(unnamed('a', '<button\n  >{#if src}<img alt={name} src={src} />{/if}</button\n>')).toEqual([]);
  });

  it('is there for every button of the app', () => {
    const missing = Object.entries(sources).flatMap(([file, source]) => unnamed(file, source));
    expect(missing).toEqual([]);
  });
});
