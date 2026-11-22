<!--
  Find in the conversation (⌘F): type, ↵ goes to the next match, ⇧↵ to the previous one, Esc
  closes. It searches the text the open conversation shows, so it reads the page from outside:
  the conversation knows nothing about it, and what streams in is searched as it arrives.
  Matches are painted with the CSS highlight API, which changes nothing in the page (where the
  webview has none, the paragraph of the current match gets an outline). History that is not
  loaded is not searched: the bar says so and offers to load it. Work folded into one line is
  searched by that line.
-->
<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import { findMatches, stepMatch, type Piece } from './find';

  interface Props {
    /** The conversation on screen: a thread, or a closed request. */
    thread: string;
    /** The column that shows it; its scroll box is looked up inside. */
    scope: HTMLElement | undefined;
  }
  let { thread, scope }: Props = $props();

  /** The conversation's scroll box (Conversation.svelte), or a view that marks its own. */
  const BOX = '[data-find-root], .conv > .scroll';
  /** Controls and decorations: not what was said. */
  const SKIP = '.acts, .copy, .earlier, [aria-hidden="true"], time, kbd, input, textarea, select';
  /** Inline elements: the text on both sides of one is the same run of text. */
  const INLINE = new Set(['A', 'ABBR', 'B', 'BUTTON', 'CODE', 'DEL', 'EM', 'I', 'INS', 'MARK', 'S', 'SMALL', 'SPAN', 'STRONG', 'SUB', 'SUP', 'U']);
  /** More matches than this are not painted or counted one by one. */
  const MOST = 2000;
  const ALL = 'codly-find';
  const HERE = 'codly-find-here';
  const HIT = 'data-find-hit';
  /** Safari 17.2 and later; null on an older webview. */
  const registry = typeof CSS !== 'undefined' && 'highlights' in CSS ? CSS.highlights : null;

  let query = $state('');
  let input: HTMLInputElement | undefined = $state();
  /** The matches, in the page's order (ranges follow the page as it changes). */
  let ranges: Range[] = [];
  let total = $state(0);
  let more = $state(false);
  let current = $state(0);
  /** The paragraph outlined for the current match, on a webview without the highlight API. */
  let outlined: Element | null = null;

  const box = (): HTMLElement | null => scope?.querySelector<HTMLElement>(BOX) ?? null;

  /** The text under `root` as the page has it: one piece per text node, grouped by paragraph. */
  function collect(root: HTMLElement): { nodes: Text[]; pieces: Piece[] } {
    const nodes: Text[] = [];
    const pieces: Piece[] = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let lastBlock: Element | null = null;
    let block = -1;
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const text = n as Text;
      const parent = text.parentElement;
      if (!text.data || !parent || parent.closest(SKIP)) continue;
      let el: Element | null = parent;
      while (el && el !== root && INLINE.has(el.tagName)) el = el.parentElement;
      if (el !== lastBlock) {
        lastBlock = el;
        block++;
      }
      nodes.push(text);
      pieces.push({ text: text.data, block });
    }
    return { nodes, pieces };
  }

  /** The first match that is not above what you see (the last one if all are above). */
  function firstInView(root: HTMLElement): number {
    const top = root.getBoundingClientRect().top;
    let lo = 0;
    let hi = ranges.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (ranges[mid].getBoundingClientRect().bottom < top) lo = mid + 1;
      else hi = mid;
    }
    return Math.min(lo, Math.max(0, ranges.length - 1));
  }

  function paint() {
    const here = ranges[current];
    if (registry) {
      if (!here) {
        registry.delete(ALL);
        registry.delete(HERE);
        return;
      }
      registry.set(ALL, new Highlight(...ranges));
      const top = new Highlight(here);
      top.priority = 1;
      registry.set(HERE, top);
      return;
    }
    const el = here?.startContainer.parentElement ?? null;
    if (el === outlined) return;
    outlined?.removeAttribute(HIT);
    el?.setAttribute(HIT, '');
    outlined = el;
  }

  /** Scroll the current match into view, unless it already is. */
  function reveal(root: HTMLElement) {
    const here = ranges[current];
    if (!here) return;
    const at = here.getBoundingClientRect();
    if (!at.width && !at.height) return; // text the page does not lay out (a folded block): nowhere to scroll to
    const view = root.getBoundingClientRect();
    if (at.top < view.top + 32 || at.bottom > view.bottom - 32) root.scrollTop += at.top - view.top - view.height / 3;
  }

  /** Search what is on screen now. `jump`: a new search, so go to a match; otherwise the page
   *  changed under the same search and the match you are on stays the current one. */
  function search(jump: boolean) {
    const root = box();
    const was = ranges[current];
    if (!root || !query) ranges = [];
    else {
      const { nodes, pieces } = collect(root);
      const found = findMatches(pieces, query, MOST + 1);
      more = found.length > MOST;
      ranges = found.slice(0, MOST).map((m) => {
        const r = document.createRange();
        r.setStart(nodes[m.start], m.startOffset);
        r.setEnd(nodes[m.end], m.endOffset);
        return r;
      });
    }
    total = ranges.length;
    if (!total) more = false;
    if (jump && root) current = firstInView(root);
    else {
      const same = was ? ranges.findIndex((r) => r.startContainer === was.startContainer && r.startOffset === was.startOffset) : -1;
      current = same >= 0 ? same : Math.min(current, Math.max(0, total - 1));
    }
    paint();
    if (jump && root) reveal(root);
  }

  function step(by: 1 | -1) {
    const root = box();
    if (!total || !root) return;
    current = stepMatch(current, total, by);
    paint();
    reveal(root);
  }

  function close() {
    ui.findOpen = false;
    ui.composerFocus++;
  }

  // A new text to find, or another conversation on screen: search, and go to a match.
  $effect(() => {
    void query;
    void thread;
    let live = true;
    tick().then(() => live && search(true));
    return () => (live = false);
  });

  // The page changes (a message streams in, earlier history loads, a row opens): search again,
  // at most every 150 ms, and stay on the match you are on.
  $effect(() => {
    void thread;
    let observer: MutationObserver | undefined;
    let waiting: ReturnType<typeof setTimeout> | undefined;
    let live = true;
    tick().then(() => {
      const root = box();
      if (!live || !root) return;
      observer = new MutationObserver(() => {
        if (!query || waiting) return;
        waiting = setTimeout(() => {
          waiting = undefined;
          search(false);
        }, 150);
      });
      observer.observe(root, { childList: true, subtree: true, characterData: true });
    });
    return () => {
      live = false;
      observer?.disconnect();
      clearTimeout(waiting);
    };
  });

  // ⌘F again: back in the field, with its text selected.
  $effect(() => {
    void ui.findFocus;
    tick().then(() => {
      input?.focus();
      input?.select();
    });
  });

  onDestroy(() => {
    ranges = [];
    paint();
    ui.findOpen = false;
  });

  // ── history that is not loaded is not searched ──
  const earlier = $derived(store.earlier[thread]);
  let loadFailed = $state<string | null>(null);
  async function loadEarlier() {
    loadFailed = null;
    const root = box();
    const height = root?.scrollHeight ?? 0;
    try {
      await store.loadEarlier(thread);
    } catch (e) {
      loadFailed = e instanceof Error ? e.message : String(e);
      return;
    }
    await tick();
    // What you were reading stays where it is while older rows come in above it.
    if (root) root.scrollTop += root.scrollHeight - height;
    search(false);
  }

  function key(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      step(e.shiftKey ? -1 : 1);
    } else if (e.key === 'Escape') {
      // Here, not in the global key map: Esc in a field only leaves the field there.
      e.preventDefault();
      close();
    }
  }
</script>

<div class="find" role="search">
  <div class="row">
    <Icon name="search" size={14} />
    <input bind:this={input} bind:value={query} onkeydown={key} placeholder="Find in this conversation" aria-label="Find in this conversation" spellcheck="false" autocomplete="off" />
    <span class="count num" aria-live="polite">{#if query}{total ? `${current + 1} of ${total}${more ? '+' : ''}` : 'No matches'}{/if}</span>
    <button class="icon-btn sm" disabled={!total} onclick={() => step(-1)} aria-label="Previous match" data-tip="Previous match" data-tip-kbd="⇧↵"><Icon name="chevron-up" size={14} /></button>
    <button class="icon-btn sm" disabled={!total} onclick={() => step(1)} aria-label="Next match" data-tip="Next match" data-tip-kbd="↵"><Icon name="chevron-down" size={14} /></button>
    <button class="icon-btn sm" onclick={close} aria-label="Close find" data-tip="Close" data-tip-kbd="Esc"><Icon name="x" size={14} /></button>
  </div>
  {#if earlier?.more}
    <p class="older">
      <span>Older messages are not loaded, so they are not searched.</span>
      <button class="btn ghost sm" disabled={earlier.loading} onclick={loadEarlier}>{earlier.loading ? 'Loading…' : 'Load earlier'}</button>
    </p>
  {/if}
  {#if loadFailed}<p class="older bad" role="alert">Couldn’t load earlier history — {loadFailed}</p>{/if}
</div>

<style>
  .find { flex-shrink: 0; padding: 8px 12px 8px 16px; box-shadow: inset 0 -1px 0 var(--line); animation: fade-in var(--d-2) var(--ease-out); }
  .row { display: flex; align-items: center; gap: 6px; color: var(--ink-3); }
  input { flex: 1; min-width: 0; height: 28px; padding: 0 4px; border: 0; outline: none; box-shadow: none; background: transparent; font-size: var(--t-md); color: var(--ink-1); user-select: text; -webkit-user-select: text; }
  .count { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; padding: 0 4px; }
  .older { display: flex; align-items: center; gap: 10px; padding: 4px 0 0 20px; font-size: var(--t-sm); color: var(--ink-3); }
  .older.bad { color: var(--bad); }
</style>
