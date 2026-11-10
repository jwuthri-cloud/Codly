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

<Dialog label="Keyboard shortcuts" width={800} onclose={close} onkeydown={key}>
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
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 28px; padding: 22px 26px 26px; }
  h3 { margin-bottom: 10px; }
  ul { display: flex; flex-direction: column; gap: 2px; }
  li { display: flex; align-items: center; justify-content: space-between; gap: 12px; height: 30px; font-size: var(--t-md); color: var(--ink-2); }
  @media (max-width: 720px) { .grid { grid-template-columns: 1fr; } }
</style>
