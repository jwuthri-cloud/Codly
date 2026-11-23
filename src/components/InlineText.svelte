<!-- Plain text with `code` spans and **bold** (no other markdown). Safe: rendered as text nodes. -->
<script lang="ts">
  import { inlineParts } from './inlineParts';

  interface Props {
    text: string;
  }
  let { text }: Props = $props();
  const parts = $derived(inlineParts(text));
</script>

{#each parts as p, i (i)}{#if p.kind === 'code'}<code class="ic">{p.t}</code>{:else if p.kind === 'bold'}<b class="ib">{p.t}</b>{:else}{p.t}{/if}{/each}

<style>
  .ic { font-family: var(--font-mono); font-size: 0.88em; padding: 1px 5px; border-radius: 5px; background: var(--surface-2); color: var(--ink-1); }
  .ib { font-weight: var(--w-semi); }
</style>
