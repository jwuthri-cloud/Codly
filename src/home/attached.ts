// The message the composer sends for a text with attached files, and the rule that turns a long
// paste into an attached text file.

/**
 * `text` and, under a heading, the saved `paths` of the attached files: one `- <path>` line
 * per file. The core finds the attached images by these lines and sends them to the model as
 * images too (crates/codly-core/src/adapters/attachments.rs). So the heading says where the
 * files are and gives no order to read them: an agent that already has an image in the message
 * does not open it a second time. It stays true for an image that went as a path only (an
 * agent that takes no images, a file over the size limit).
 */
export function messageWithFiles(text: string, paths: string[]): string {
  const list = paths.map((p) => `- ${p}`).join('\n');
  const heading = 'Attached files, saved at these paths (an image may already be in this message):';
  return [text.trim() || (list ? 'See the attached files.' : ''), list && `${heading}\n${list}`].filter(Boolean).join('\n\n');
}

/** Pasted text longer than this, in characters or in lines, is attached as a text file. */
export const PASTE_MOST_CHARS = 8000;
export const PASTE_MOST_LINES = 150;

/** How many lines `text` has (a line break at its very end starts no line). */
export function lineCount(text: string): number {
  const lines = text.split(/\r\n|\r|\n/);
  return lines.length > 1 && lines[lines.length - 1] === '' ? lines.length - 1 : lines.length;
}

/** Does a paste of `text` go in as a file, not into the field? A long one does: the field stays readable. */
export const pastesAsFile = (text: string): boolean => text.length > PASTE_MOST_CHARS || lineCount(text) > PASTE_MOST_LINES;

let pastes = 0;
/** The name of the next pasted text's file: `pasted-<n>.txt`. */
export const pastedName = (): string => `pasted-${++pastes}.txt`;

/** What the chip of a pasted text says. */
export const pastedChip = (lines: number): string => `Pasted text · ${lines} ${lines === 1 ? 'line' : 'lines'}`;
