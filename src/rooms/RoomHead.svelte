<!-- A room's header: eyebrow, serif title, one line of facts, lenses and actions on the right. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props {
    eyebrow: string;
    title: string;
    sub?: Snippet;
    right?: Snippet;
  }
  let { eyebrow, title, sub, right }: Props = $props();
</script>

<header class="rh" data-tauri-drag-region>
  <div class="l">
    <p class="eyebrow">{eyebrow}</p>
    <h1 class="serif truncate">{title}</h1>
    {#if sub}<div class="sub">{@render sub()}</div>{/if}
  </div>
  {#if right}<div class="r">{@render right()}</div>{/if}
</header>

<style>
  .rh { display: flex; align-items: flex-end; gap: 16px; padding: 24px 28px 18px; flex-shrink: 0; }
  .l { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  h1 { font-size: var(--d-lg); line-height: 1.05; color: var(--ink-1); }
  .sub { font-size: var(--t-md); color: var(--ink-3); display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .r { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
</style>
