<!--
  The window's first Tab stop: it takes the keyboard past the office, the rail and the top bar to
  the conversation's composer (from there ⇧⇥ goes back into the thread). Hidden until it has the
  focus. From another room, or the full office, it goes back to Home first.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';

  async function go() {
    if (ui.mode === 'office' || ui.mode === 'map') ui.mode = 'blend';
    if (ui.room !== 'home') ui.go('home');
    await tick();
    document.querySelector<HTMLTextAreaElement>('[aria-label="Composer"] textarea')?.focus();
  }
</script>

{#if store.workspace}<button class="skip" onclick={go}>Skip to the conversation</button>{/if}

<style>
  .skip { position: fixed; top: 8px; left: 8px; z-index: var(--z-tip); height: 32px; padding: 0 14px; border-radius: var(--r-md); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-md); color: var(--ink-1); transform: translateY(calc(-100% - 16px)); }
  .skip:focus-visible { transform: none; }
</style>
