// A message being written is rendered in parts: the finished blocks once, the rest again on each
// new word. Whatever the parts are, together they must be the HTML of the whole text.
import { Marked } from 'marked';
import { describe, expect, it } from 'vitest';
import { markdownBlocks } from '../markdown';
import { SLOW_EVERY, SLOW_TAIL, finishedLength, streamRenderer } from '../mdstream';
import { clean } from '../sanitize';

// The tests run without a browser, so without the sanitizer: this is the parser `renderMarkdown`
// uses, with the same options. The sanitizer works on each part's HTML, and the HTML of whole
// blocks is balanced (raw HTML, the one way to leave a tag open, keeps a text whole).
const md = new Marked({ gfm: true, breaks: false, async: false });
const render = (src: string) => md.parse(clean(src)) as string;

const DOCS: Record<string, string> = {
  paragraphs: 'First paragraph with **bold** text.\n\nSecond paragraph\nwith two lines.\n\nThird one.\n',
  headings: '# Title\n\nIntro text.\n\n## Part\nBody right after.\n\nMore.\n',
  'setext headings': 'Title\n=====\n\nText\n\nSub\n---\n\nEnd\n',
  'code fences with empty lines': 'Before\n\n```ts\nconst a = 1;\n\nconst b = 2;\n```\n\nAfter the code.\n\n~~~\ntilde\n\n~~~\n\nEnd.\n',
  'a fence with no empty line around it': 'Text\n```\ncode\n\nmore\n```\nafter\n\nlast\n',
  'a fence inside a list': '- item\n\n  ```\n  code\n\n  more\n  ```\n\n- next\n\nparagraph\n\n```\ntop\n```\n\nend\n',
  'a list that turns loose': 'Intro:\n\n- one\n- two\n\n- three (loose now)\n\nAfter list.\n\nAnother.\n',
  'an ordered list with gaps': '1. a\n\n2. b\n\n3. c\n\ntext\n\nmore text\n',
  'two ordered lists': '1. a\n\ntext between\n\n2. b\n\nend\n\nreally end\n',
  'a list item with two paragraphs': '- a\n\n  continued paragraph of a\n\n- b\n\nout\n\nout2\n',
  'lists with other bullets': '- a\n\n* b\n\n+ c\n\ndone\n\nyes\n',
  'a line that becomes a list item': 'a\n\n-\n\nb\n\n1\n\n2. x\n\nc\n\nd\n',
  quotes: '> quoted\n> more\n\n> second quote\n\ntext\n\n> q\nlazy\n\nend\n',
  'a list in a quote': '> - a\n>\n> - b\n\n> c\n\nd\n\ne\n',
  tables: 'Intro\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n| 3 | 4 |\n\nend\n',
  rules: 'a\n\n---\n\nb\n\n***\nc\n\nd\n',
  'indented code': 'para\n\n    code1\n\n    code2\n\ntext\n\n    code3\ntext2\n\nend\n',
  'an HTML block': 'para\n\n<div>\n\ninside\n\n</div>\n\nafter\n\nlast\n',
  'a tag left open in a line': 'para with <b>bold\n\nnext para\n\nthird\n',
  'an HTML comment over empty lines': 'a\n\n<!-- open\n\nstill comment\n\n-->\n\nb\n\nc\n',
  'a link defined after its use': 'See [the docs][d] here.\n\nMore text.\n\n[d]: https://example.com\n\nEnd [d].\n',
  'a short link defined late': 'A [link] first.\n\nmiddle\n\nmiddle2\n\n[link]: https://example.com "t"\n\nend\n',
  tabs: 'a\n\n\tcode with tab\n\nb\n\n- x\n\t- y\n\nc\n\nd\n',
  'Windows line ends': 'a\r\n\r\nb\r\n\r\nc\r\n',
  'terminal colours': 'a\n\n\u001b[31mred\u001b[0m\n\nb\n\nc\n',
  'empty lines first': '\n\nstarts blank\n\nnext\n\nlast\n',
  'lines of spaces': 'a\n   \nb\n \t \nc\n\nd\n',
  'emphasis left open': 'a *open\n\nstill* b\n\nc _x\n\ny_ d\n\ne\n',
};

/** Feed `doc` to a new renderer `step` characters at a time; the parts after each step. */
function* stream(doc: string, step: number) {
  const next = streamRenderer(render, markdownBlocks);
  for (let end = step; ; end += step) {
    const src = doc.slice(0, end);
    yield { src, parts: next(src) };
    if (end >= doc.length) return;
  }
}

describe('a message being written', () => {
  for (const [name, doc] of Object.entries(DOCS)) {
    it(`renders as the whole text would at every step: ${name}`, () => {
      for (const step of [1, 3, 7]) for (const { src, parts } of stream(doc, step)) expect(parts.done.join('') + parts.tail, JSON.stringify(src)).toBe(render(src));
    });
  }

  it('parses a finished block once: its part never changes, and only the rest is rendered again', () => {
    const doc = DOCS.paragraphs + '\n```\ncode\n```\n\n' + DOCS.tables;
    const rendered: string[] = [];
    const next = streamRenderer((src) => (rendered.push(src), render(src)), markdownBlocks);
    let done = next('').done;
    for (let end = 1; end <= doc.length; end++) {
      const parts = next(doc.slice(0, end));
      // Parts are only added, and a list with no new part is the same list (nothing to update).
      expect(parts.done.slice(0, done.length)).toEqual(done);
      if (parts.done.length === done.length) expect(parts.done).toBe(done);
      done = parts.done;
    }
    expect(done.length).toBeGreaterThanOrEqual(6);
    // The whole text was never given to the parser again once its first block was finished.
    const first = doc.indexOf('\n\n') + 2;
    expect(rendered.filter((src) => src.length > first && doc.startsWith(src))).toEqual([]);
    expect(Math.max(...rendered.slice(-20).map((src) => src.length))).toBeLessThan(doc.length / 2);
  });

  it('keeps a list whole until a block after it is finished too', () => {
    const next = streamRenderer(render, markdownBlocks);
    // "2" may still become "2. b", an item of the same list.
    expect(next('1. a\n\n2').done).toEqual([]);
    expect(next('1. a\n\n2. b\n\ntext').done).toEqual([]);
    expect(next('1. a\n\n2. b\n\ntext\n\nmore').done).toHaveLength(1);
  });

  it('keeps a text with raw HTML or a link definition whole, from the moment it has one', () => {
    const next = streamRenderer(render, markdownBlocks);
    expect(next('one\n\ntwo\n\nthree').done).toHaveLength(1);
    expect(next('one\n\ntwo\n\nthree <b>bold\n\nfour\n\nfive').done).toEqual([]);
    expect(next('one\n\ntwo\n\nthree <b>bold\n\nfour\n\nfive\n\nsix\n\nseven').done).toEqual([]);
    const linked = streamRenderer(render, markdownBlocks);
    expect(linked('see [d]\n\ntwo\n\nthree').done).toHaveLength(1);
    expect(linked('see [d]\n\ntwo\n\nthree\n\n[d]: https://example.com\n\nend').done).toEqual([]);
  });

  it('starts again for a text that is not the last one plus more', () => {
    const next = streamRenderer(render, markdownBlocks);
    expect(next('one\n\ntwo\n\nthree').done).toHaveLength(1);
    const other = next('uno\n\ndos');
    expect(other.done.join('') + other.tail).toBe(render('uno\n\ndos'));
    // Raw HTML kept the old text whole; the new one has none.
    const again = streamRenderer(render, markdownBlocks);
    again('a <i>b\n\nc\n\nd');
    expect(again('one\n\ntwo\n\nthree').done).toHaveLength(1);
  });

  it('renders a long tail at most every 100 ms, says when to come back, and ends as the whole text renders', () => {
    // A list is never finished before the end: all of it is the tail.
    const doc = Array.from({ length: 400 }, (_, i) => `- item ${i} of a long list`).join('\n');
    expect(doc.length).toBeGreaterThan(SLOW_TAIL);
    let now = 0;
    const rendered: number[] = [];
    const next = streamRenderer((src) => (rendered.push(src.length), render(src)), markdownBlocks, () => now);
    let parts = next(doc.slice(0, 100));
    // Short: each new word renders.
    expect(next(doc.slice(0, 120)).wait).toBe(0);
    now += SLOW_EVERY;
    parts = next(doc.slice(0, SLOW_TAIL + 10));
    expect(parts.wait).toBe(0);
    const at = rendered.length;
    now += 30;
    parts = next(doc.slice(0, SLOW_TAIL + 40));
    // Rendered 30 ms ago: the tail is the one before, and it comes back in 70 ms.
    expect(rendered.length).toBe(at);
    expect(parts.wait).toBe(SLOW_EVERY - 30);
    expect(parts.tail).toBe(render(doc.slice(0, SLOW_TAIL + 10)));
    now += 70;
    parts = next(doc.slice(0, SLOW_TAIL + 40));
    expect(parts.wait).toBe(0);
    expect(parts.done.join('') + parts.tail).toBe(render(doc.slice(0, SLOW_TAIL + 40)));
    // The last state, once the clock allows it: the HTML of the whole text.
    now += 5;
    expect(next(doc).wait).toBeGreaterThan(0);
    now += SLOW_EVERY;
    parts = next(doc);
    expect(parts.done.join('') + parts.tail).toBe(render(doc));
  });

  it('a long tail that waits never holds a block that is finished now', () => {
    let now = 0;
    const big = 'x'.repeat(SLOW_TAIL + 100);
    const next = streamRenderer(render, markdownBlocks, () => now);
    next(`${big}\n\n`);
    now += 10;
    const parts = next(`${big}\n\nnext`);
    expect(parts.wait).toBe(0);
    expect(parts.done.join('') + parts.tail).toBe(render(`${big}\n\nnext`));
  });

  it('puts the caret mark at the very end, in the part being written', () => {
    const next = streamRenderer(render, markdownBlocks);
    const parts = next('one\n\ntwo\n\nthr', '');
    expect(parts.done.join('')).not.toContain('');
    expect(parts.tail).toBe(render('thr'));
  });
});

describe('finishedLength', () => {
  const blocks = (src: string) => markdownBlocks(src).blocks;

  it('ends at the last empty line that another block follows', () => {
    const text = 'one\n\ntwo\n\nthree';
    expect(text.slice(0, finishedLength(text, blocks(text)))).toBe('one\n\ntwo\n\n');
    expect(finishedLength('one\n\n', blocks('one\n\n'))).toBe(0); // nothing follows yet
    expect(finishedLength('one\ntwo', blocks('one\ntwo'))).toBe(0);
  });

  it('is nothing when the blocks do not join back to the text', () => {
    expect(finishedLength('one\n\ntwo\n\nthree', [{ type: 'paragraph', raw: 'one' }, { type: 'space', raw: '\n\n' }, { type: 'paragraph', raw: 'two!' }])).toBe(0);
  });
});
