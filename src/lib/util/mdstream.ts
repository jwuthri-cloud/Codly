// A message that is still being written, rendered without parsing all of it again on every new
// word. Its text only grows at the end, so the blocks before the one being written are finished:
// each is rendered once and kept (`done`), and only the rest is parsed again (`tail`). The parts,
// one after the other, are always the HTML the whole text renders to.
//
// A block is finished when an empty line and another block follow it. Three things keep a text
// whole instead, because a later line can still change what an earlier one means:
// - a list (the next line may be one more item, which also changes how its items are spaced);
// - raw HTML (a tag can stay open past its block) and a link definition (`[name]: url` names a
//   link anywhere in the text): from then on nothing is finished before the end;
// - characters that are removed before parsing, or a line end the parser rewrites: the positions
//   in the parsed text are then not the positions in the text.
import { clean } from './sanitize';

/** One top-level block of a markdown text, as the parser read it. */
export type Block = { type: string; raw: string };
/** `entangled`: the text has raw HTML or a link definition. */
export type Blocks = { blocks: Block[]; entangled: boolean };
/** `done`: the HTML of each finished part, in order; a part never changes once it is here. */
export type Parts = { done: string[]; tail: string };

/** An empty line ends at `end`. */
function blankBefore(text: string, end: number): boolean {
  if (text[end - 1] !== '\n') return false;
  let i = end - 2;
  while (i >= 0 && (text[i] === ' ' || text[i] === '\t')) i--;
  return i >= 0 && text[i] === '\n';
}

/** How much of the start of `text` is finished blocks (0: none yet). */
export function finishedLength(text: string, blocks: Block[]): number {
  // The parser may rewrite what it reads (tabs, line ends): then its blocks are no map of the text.
  if (blocks.map((b) => b.raw).join('') !== text) return 0;
  let last = blocks.length - 1;
  while (last >= 0 && blocks[last].type === 'space') last--;
  const ends: number[] = [];
  let end = 0;
  for (const b of blocks) ends.push((end += b.raw.length));
  for (let k = last - 1; k >= 0; k--) {
    const before = blocks[k].type === 'space' ? blocks[k - 1] : blocks[k];
    if (before && before.type !== 'list' && blankBefore(text, ends[k])) return ends[k];
  }
  return 0;
}

/**
 * The renderer of one message. `render` makes HTML from markdown, `split` reads markdown into
 * its blocks. Call the result with the text so far each time it grows; `mark` is put at the very
 * end of the text (the caret). A text that is not the last one plus more starts again.
 */
export function streamRenderer(render: (src: string) => string, split: (src: string) => Blocks): (src: string, mark?: string) => Parts {
  let seen = '';
  /** The finished start of `seen`. */
  let head = '';
  let done: string[] = [];
  /** Raw HTML or a link definition was read: nothing is finished before the end. */
  let whole = false;
  return (src, mark = '') => {
    if (!src.startsWith(seen)) {
      head = '';
      done = [];
      whole = false;
    }
    seen = src;
    const rest = src.slice(head.length);
    if (!whole && clean(rest) === rest && !rest.includes('\r')) {
      const read = split(rest);
      if (read.entangled) {
        whole = true;
        head = '';
        done = [];
      } else {
        const n = finishedLength(rest, read.blocks);
        if (n) {
          head = src.slice(0, head.length + n);
          done = [...done, render(rest.slice(0, n))];
        }
      }
    }
    return { done, tail: render(src.slice(head.length) + mark) };
  };
}
