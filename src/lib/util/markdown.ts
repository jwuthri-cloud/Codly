// Chat markdown: marked (GFM) → DOMPurify. Model output is untrusted: no raw HTML survives
// sanitization except the formatting tags below, links open externally, images are dropped.
import DOMPurify from 'dompurify';
import { Marked } from 'marked';
import { isTauriApp } from '../api';
import { clean } from './sanitize';

const md = new Marked({ gfm: true, breaks: false, async: false });

let hooked = false;
function hook() {
  if (hooked) return;
  hooked = true;
  DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.tagName === 'A') {
      const href = node.getAttribute('href') ?? '';
      if (!/^(https?:|mailto:)/i.test(href)) node.removeAttribute('href');
      node.setAttribute('target', '_blank');
      node.setAttribute('rel', 'noreferrer noopener');
    }
  });
}

const ALLOWED_TAGS = [
  'p', 'br', 'strong', 'em', 'b', 'i', 'del', 's', 'code', 'pre', 'blockquote', 'ul', 'ol', 'li',
  'a', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'input', 'span',
];

export function renderMarkdown(src: string): string {
  hook();
  const html = md.parse(clean(src)) as string;
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR: ['href', 'target', 'rel', 'type', 'checked', 'disabled', 'align', 'start'],
  });
}

/** Open http(s) links from rendered markdown in the system browser (Tauri) or a new tab. */
export async function openExternal(url: string) {
  if (isTauriApp()) {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
      return;
    } catch {
      /* fall through */
    }
  }
  window.open(url, '_blank', 'noopener,noreferrer');
}
