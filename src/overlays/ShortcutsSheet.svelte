<script lang="ts">
  import { SHORTCUTS } from '../lib/state/keys';
  import { ui } from '../lib/state/ui.svelte';
  import Dialog from '../components/Dialog.svelte';
  import Kbd from '../components/Kbd.svelte';

  const close = () => (ui.shortcutsOpen = false);
  /** `?` opens this sheet, so it closes it too. */
  function key(e: KeyboardEvent) {
    if (e.key !== '?') return;
    e.preventDefault();
    e.stopPropagation();
    close();
  }
</script>

<Dialog label="Keyboard shortcuts" width={800} style="max-height: calc(100vh - 32px)" onclose={close} onkeydown={key}>
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
</Dialog>

<style>
  /* In a small window the sheet stops at the window's edge and its rows scroll. */
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 28px; padding: 22px 26px 26px; min-height: 0; overflow-y: auto; }
  h3 { margin-bottom: 10px; }
  ul { display: flex; flex-direction: column; gap: 2px; }
  /* a label of two lines makes its row taller; the keys keep their size */
  li { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 30px; padding: 3px 0; font-size: var(--t-md); line-height: 18px; color: var(--ink-2); }
  li > span { min-width: 0; }
  li > :global(.kbds) { flex-shrink: 0; }
  @media (max-width: 720px) { .grid { grid-template-columns: 1fr; } }
</style>
