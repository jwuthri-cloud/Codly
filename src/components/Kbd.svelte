<!-- Shortcut caps. `keys="⌘K"` → one cap; `keys="G C"` → a sequence of two caps. -->
<script lang="ts">
  interface Props {
    keys: string;
    subtle?: boolean;
  }
  let { keys, subtle = false }: Props = $props();
  const parts = $derived(keys.split(' ').filter(Boolean));
</script>

<span class="kbds" class:subtle aria-label="Shortcut {keys}">
  {#each parts as p, i (i)}
    <kbd>{p}</kbd>
  {/each}
</span>

<style>
  .kbds { display: inline-flex; gap: 3px; align-items: center; }
  .subtle kbd { background: transparent; box-shadow: none; padding: 0 2px; min-width: 0; }
</style>
