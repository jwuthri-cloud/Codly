<!-- Plain text with `code` spans (no other markdown). Safe: rendered as text nodes. -->
<script lang="ts">
  interface Props {
    text: string;
  }
  let { text }: Props = $props();
  const parts = $derived(
    text
      .split(/(`[^`\n]+`)/g)
      .filter(Boolean)
      .map((p) => ({ code: p.length > 2 && p.startsWith('`') && p.endsWith('`'), t: p.replace(/^`|`$/g, '') })),
  );
</script>

{#each parts as p, i (i)}{#if p.code}<code class="ic">{p.t}</code>{:else}{p.t}{/if}{/each}

<style>
  .ic { font-family: var(--font-mono); font-size: 0.88em; padding: 1px 5px; border-radius: 5px; background: var(--surface-2); color: var(--ink-1); }
</style>
