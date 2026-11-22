// A Copy button on every code block of rendered markdown. The button is static markup added
// after sanitizing (the model's output never supplies it); Markdown.svelte handles its click.

const BUTTON = '<button class="copy" type="button" aria-label="Copy code">Copy</button>';

/**
 * Wrap each `<pre>` of sanitized markdown HTML in a box with a Copy button. The input is
 * DOMPurify output: text is escaped there, so "<pre" only ever opens a real code block and
 * every one is closed.
 */
export function withCopyButtons(html: string): string {
  return html.replace(/<pre(\s[^>]*)?>/g, (open) => `<div class="code">${BUTTON}${open}`).replace(/<\/pre>/g, '</pre></div>');
}

/** What Copy puts on the clipboard: the code without the line break markdown adds at its end. */
export const codeToCopy = (code: string): string => code.replace(/\n$/, '');
