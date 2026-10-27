<!-- Sanitized markdown (marked → DOMPurify). Links open in the system browser. -->
<script lang="ts">
  import { openExternal, renderMarkdown } from '../lib/util/markdown';

  interface Props {
    text: string;
    streaming?: boolean;
    /** A task brief: takes the surrounding font size; headings become small labels. */
    brief?: boolean;
  }
  let { text, streaming = false, brief = false }: Props = $props();
  // The caret rides inline at the end of the streamed text (a private-use marker survives
  // markdown + sanitizing and is swapped for a span afterwards).
  const CARET = '\uE000';
  const html = $derived(
    streaming ? renderMarkdown(text + CARET).replace(CARET, '<span class="caret" aria-hidden="true"></span>') : renderMarkdown(text),
  );

  function click(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest('a');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href');
    if (href) openExternal(href);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="md selectable" class:streaming class:brief onclick={click}>{@html html}</div>

<style>
  .md { font-size: var(--t-lg); line-height: var(--lh-lg); color: var(--ink-1); overflow-wrap: anywhere; }
  .md :global(p) { margin: 0 0 10px; }
  .md :global(p:last-child), .md :global(ul:last-child), .md :global(ol:last-child) { margin-bottom: 0; }
  .md :global(strong) { font-weight: var(--w-semi); }
  .md :global(ul), .md :global(ol) { margin: 2px 0 12px; padding-left: 20px; }
  .md :global(ul) { list-style: disc; }
  .md :global(ol) { list-style: decimal; }
  .md :global(li) { margin: 4px 0; padding-left: 2px; }
  .md :global(li::marker) { color: var(--ink-3); }
  .md :global(code) { font-family: var(--font-mono); font-size: 0.84em; padding: 1px 5px; border-radius: 5px; background: var(--surface-2); color: var(--ink-1); white-space: break-spaces; }
  .md :global(pre) { margin: 8px 0 12px; padding: 12px 14px; border-radius: var(--r-md); background: var(--code-bg); overflow-x: auto; }
  .md :global(pre code) { padding: 0; background: none; font-size: var(--mono); line-height: var(--lh-mono); white-space: pre; }
  .md :global(blockquote) { margin: 8px 0; padding-left: 12px; border-left: 2px solid var(--line-2); color: var(--ink-2); }
  .md :global(h1), .md :global(h2), .md :global(h3) { font-family: var(--font-display); font-weight: 400; margin: 16px 0 6px; line-height: 1.15; }
  .md :global(h1) { font-size: 30px; }
  .md :global(h2) { font-size: 23px; }
  .md :global(h3) { font-size: 19px; }
  .md :global(h1:first-child), .md :global(h2:first-child) { margin-top: 0; }
  .md :global(h1 code), .md :global(h2 code), .md :global(h3 code) { background: none; padding: 0; font-size: 0.72em; letter-spacing: 0; }
  .md :global(hr) { margin: 12px 0; }
  .md :global(table) { border-collapse: collapse; margin: 8px 0 12px; font-size: var(--t-md); }
  .md :global(th), .md :global(td) { padding: 6px 10px; box-shadow: inset 0 -1px 0 var(--line); text-align: left; }
  .md :global(th) { color: var(--ink-2); font-weight: var(--w-med); }
  .md.brief { font-size: inherit; line-height: inherit; color: inherit; }
  .md.brief :global(h1), .md.brief :global(h2), .md.brief :global(h3) { font-family: inherit; font-size: var(--t-xs); font-weight: var(--w-semi); line-height: inherit; letter-spacing: 0.06em; text-transform: uppercase; color: var(--ink-3); margin: 14px 0 4px; }
  .md.brief :global(p), .md.brief :global(li) { overflow-wrap: anywhere; }
  .md :global(.caret) { display: inline-block; width: 8px; height: 15px; margin-left: 2px; vertical-align: -2px; border-radius: 2px; background: var(--aurora); animation: caret 1.1s steps(1) infinite; }
</style>
