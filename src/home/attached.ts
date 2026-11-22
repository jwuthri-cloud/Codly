// The message the composer sends for a text with attached files.

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
