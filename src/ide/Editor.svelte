<!-- A deliberately light editor: a monospace textarea with a line-number gutter. No syntax
     highlighting, no dependency. ⌘S saves (handled by the overlay); Tab inserts two spaces. -->
<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import { ide, type Tab } from './ide.svelte';

  interface Props {
    tab: Extract<Tab, { kind: 'file' }>;
  }
  let { tab }: Props = $props();

  let ta: HTMLTextAreaElement | undefined = $state();
  let gutter: HTMLPreElement | undefined = $state();
  let caret = $state({ line: 1, col: 1 });

  const count = $derived(tab.content.split('\n').length);
  const numbers = $derived(Array.from({ length: count }, (_, i) => i + 1).join('\n'));
  const dirty = $derived(ide.isDirty(tab));

  function track() {
    if (!ta) return;
    const before = ta.value.slice(0, ta.selectionStart);
    const nl = before.lastIndexOf('\n');
    caret = { line: before.split('\n').length, col: before.length - nl };
  }
  const sync = () => ta && gutter && (gutter.scrollTop = ta.scrollTop);

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Tab' && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      // execCommand keeps the browser's undo stack; fires the input event for us.
      document.execCommand('insertText', false, '  ');
    }
  }

  // Jump to a requested line (search results) once the file has loaded.
  $effect(() => {
    const line = tab.line;
    if (line == null || tab.loading || !ta) return;
    const lines = tab.content.split('\n');
    const at = lines.slice(0, Math.max(0, line - 1)).reduce((n, l) => n + l.length + 1, 0);
    tick().then(() => {
      if (!ta) return;
      ta.focus();
      ta.setSelectionRange(at, at + (lines[line - 1]?.length ?? 0));
      ta.scrollTop = Math.max(0, (line - 4) * 19);
      sync();
      track();
      ide.clearLine(tab.id);
    });
  });
</script>

<div class="ed">
  {#if tab.loading}
    <p class="note">Opening {tab.path}…</p>
  {:else if tab.error}
    <div class="note bad"><Icon name="alert" size={15} /><span>{tab.error}</span></div>
  {:else}
    <div class="frame">
      <pre class="gutter num" bind:this={gutter} aria-hidden="true">{numbers}</pre>
      <textarea
        bind:this={ta}
        class="text selectable"
        value={tab.content}
        wrap="off"
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
        aria-label="Contents of {tab.path}"
        oninput={(e) => (ide.edit(tab.id, e.currentTarget.value), track())}
        onscroll={sync}
        onkeydown={keydown}
        onkeyup={track}
        onclick={track}
      ></textarea>
    </div>
    <footer>
      <span class="num">Ln {caret.line}, Col {caret.col}</span>
      <span>{count} {count === 1 ? 'line' : 'lines'}</span>
      <span class="grow"></span>
      {#if tab.saving}<span>Saving…</span>
      {:else if dirty}<span class="unsaved">Unsaved changes</span><button class="btn sm" onclick={() => ide.save(tab.id)}>Save <span class="k">⌘S</span></button>
      {:else}<span>Saved</span>{/if}
    </footer>
  {/if}
</div>

<style>
  .ed { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .frame { flex: 1; min-height: 0; display: flex; background: var(--code-bg); }
  .gutter, .text { margin: 0; font-family: var(--font-mono); font-size: var(--mono); line-height: var(--lh-mono); padding: 10px 0; }
  .gutter { width: 52px; flex: none; overflow: hidden; text-align: right; padding-right: 12px; color: var(--ink-4); user-select: none; box-shadow: inset -1px 0 0 var(--line); }
  .text { flex: 1; min-width: 0; box-shadow: none; border: 0; outline: 0; resize: none; background: transparent; padding-left: 14px; padding-right: 14px; tab-size: 2; white-space: pre; overflow: auto; }
  footer { display: flex; align-items: center; gap: 14px; height: 32px; padding: 0 14px; font-size: var(--t-sm); color: var(--ink-3); box-shadow: inset 0 1px 0 var(--line); }
  .grow { flex: 1; }
  .unsaved { color: var(--amber-ink); }
  .note { display: flex; align-items: center; gap: 8px; padding: 28px; color: var(--ink-3); font-size: var(--t-md); }
  .note.bad { color: var(--bad); }
</style>
