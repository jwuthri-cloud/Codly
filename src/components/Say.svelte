<!-- Renders a narration sentence: plain words, `code` targets, a ticking timer. -->
<script lang="ts">
  import type { Part } from '../lib/narrate';
  interface Props {
    parts: Part[];
  }
  let { parts }: Props = $props();
</script>

{#each parts as p, i (i)}{#if p.code}<code class="say-code">{p.t}</code>{:else if p.timer}<span class="say-timer num">{p.t}</span>{:else}{p.t}{/if}{/each}

<style>
  .say-code { font-family: var(--font-mono); font-size: 0.9em; color: var(--ink-1); }
  .say-timer { color: var(--ink-3); }
</style>
