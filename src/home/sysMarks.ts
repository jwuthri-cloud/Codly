// The words of a quiet line in the thread (what Codly posts: "Not merged: … `npx playwright
// test` failed", "**t1** is done"), with the marks a message row shows: code and bold, and
// straight quotes as curly ones. Read as parts of text, never as HTML: the line is drawn with
// text nodes only. Pure.

/** A piece of the line: plain words, code, or bold words. */
export type Mark = { t: string; code?: true; bold?: true };

/** "a" in plain words: curly quotes, as the rest of the app writes them. */
const curly = (s: string) => s.replace(/"([^"\n]*)"/g, '“$1”');

/** The parts of a line, in order. A mark left open (a lone ` or **) stays as it is. */
export function sysMarks(text: string): Mark[] {
  const out: Mark[] = [];
  const plain = (s: string) => {
    for (const [i, part] of s.split(/\*\*([^*\n]+)\*\*/).entries()) if (part) out.push(i % 2 ? { t: curly(part), bold: true } : { t: curly(part) });
  };
  for (const [i, part] of text.split(/`([^`\n]+)`/).entries()) {
    if (i % 2) out.push({ t: part, code: true });
    else plain(part);
  }
  return out;
}
