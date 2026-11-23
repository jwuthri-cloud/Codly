// A Copy button on every code block of rendered markdown. The buttons are added to parsed
// elements, never to the HTML text: a model can put "<pre" inside an attribute value, and only
// a parser tells that from a real code block. Markdown.svelte handles the click.

/** Put each `<pre>` element under `root` in a box with a Copy button. */
export function addCopyButtons(root: DocumentFragment): void {
  const doc = root.ownerDocument;
  for (const pre of root.querySelectorAll('pre')) {
    const box = doc.createElement('div');
    box.className = 'code';
    const button = doc.createElement('button');
    button.className = 'copy';
    button.type = 'button';
    button.setAttribute('aria-label', 'Copy code');
    button.textContent = 'Copy';
    pre.replaceWith(box);
    box.append(button, pre);
  }
}

/** Sanitized markdown HTML with a Copy button on each code block. The browser parses and writes the HTML. */
export function withCopyButtons(html: string): string {
  if (!html.includes('<pre')) return html;
  const template = document.createElement('template');
  template.innerHTML = html;
  addCopyButtons(template.content);
  return template.innerHTML;
}

/** What Copy puts on the clipboard: the code without the line break markdown adds at its end. */
export const codeToCopy = (code: string): string => code.replace(/\n$/, '');
