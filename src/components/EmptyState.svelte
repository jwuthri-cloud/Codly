<!-- One line of character, in the serif, plus (optionally) one sentence and one action. -->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    body?: string;
    children?: Snippet;
    compact?: boolean;
  }
  let { title, body, children, compact = false }: Props = $props();
</script>

<div class="empty" class:compact>
  <p class="title serif">{title}</p>
  {#if body}<p class="body">{body}</p>{/if}
  {#if children}<div class="actions">{@render children()}</div>{/if}
</div>

<style>
  .empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; text-align: center; padding: 48px 24px; height: 100%; min-height: 180px; animation: fade-in var(--d-4) var(--ease-out); }
  .empty.compact { padding: 24px 16px; min-height: 0; height: auto; gap: 6px; }
  .title { font-size: var(--d-md); line-height: 1.15; color: var(--ink-1); max-width: 22ch; }
  .compact .title { font-size: var(--d-sm); }
  .body { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-3); max-width: 46ch; }
  .actions { margin-top: 10px; display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
</style>
