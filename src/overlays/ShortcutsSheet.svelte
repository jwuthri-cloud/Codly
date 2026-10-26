<script lang="ts">
  import { onMount } from 'svelte';
  import { SHORTCUTS } from '../lib/state/keys';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import Kbd from '../components/Kbd.svelte';

  let el: HTMLDivElement | undefined = $state();
  onMount(() => el?.focus());
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape' || e.key === '?') {
      e.preventDefault();
      e.stopPropagation();
      ui.shortcutsOpen = false;
    }
  }
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (ui.shortcutsOpen = false)}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1" bind:this={el} onkeydown={key}>
    <header>
      <h2>Keyboard shortcuts</h2>
      <button class="icon-btn" aria-label="Close" onclick={() => (ui.shortcutsOpen = false)}><Icon name="x" /></button>
    </header>
    <div class="grid">
      {#each SHORTCUTS as g (g.group)}
        <section>
          <h3 class="eyebrow">{g.group}</h3>
          <ul>
            {#each g.items as [k, label] (k + label)}
              <li><span>{label}</span><Kbd keys={k} /></li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); display: grid; place-items: center; animation: fade-in var(--d-3) var(--ease-out); }
  .sheet { width: min(800px, calc(100vw - 32px)); border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; justify-content: space-between; height: 64px; padding: 0 14px 0 26px; box-shadow: inset 0 -1px 0 var(--line); }
  h2 { font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 28px; padding: 22px 26px 26px; }
  h3 { margin-bottom: 10px; }
  ul { display: flex; flex-direction: column; gap: 2px; }
  li { display: flex; align-items: center; justify-content: space-between; gap: 12px; height: 30px; font-size: var(--t-md); color: var(--ink-2); }
  @media (max-width: 720px) { .grid { grid-template-columns: 1fr; } }
</style>
