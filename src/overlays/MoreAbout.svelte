<!--
  The rest of a help text in Settings, one press away: the first paragraph shows, the rest
  opens under "More about this". Each one stays open or closed as you left it, for the session.
-->
<script module lang="ts">
  /** The ids of the help texts you opened (not saved: a new session starts with all closed). */
  const opened = new Set<string>();
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let { id, children }: { id: string; children: Snippet } = $props();

  function remember(e: Event) {
    if ((e.currentTarget as HTMLDetailsElement).open) opened.add(id);
    else opened.delete(id);
  }
</script>

<details class="more" open={opened.has(id)} ontoggle={remember}>
  <summary>More about this</summary>
  <div class="more-body">{@render children()}</div>
</details>

<style>
  .more { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  summary { width: fit-content; margin-top: 2px; color: var(--ink-2); font-weight: var(--w-med); cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary::before { content: '▸'; display: inline-block; width: 1em; transition: transform var(--d-2) var(--ease-out); }
  .more[open] > summary::before { transform: rotate(90deg); }
  summary:hover { color: var(--ink-1); }
  .more-body { display: flex; flex-direction: column; gap: 6px; padding-top: 6px; }
  .more-body :global(code) { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-2); }
</style>
