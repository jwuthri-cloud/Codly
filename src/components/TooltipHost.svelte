<script lang="ts">
  import { tip } from '../lib/state/tooltip.svelte';
  import Kbd from './Kbd.svelte';

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: -999, y: -999, below: false });

  $effect(() => {
    const r = tip.rect;
    if (!tip.visible || !r || !el) return;
    void tip.text;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let x = r.left + r.width / 2 - w / 2;
    x = Math.max(8, Math.min(x, window.innerWidth - w - 8));
    const below = r.top - h - 8 < 4;
    pos = { x, y: below ? r.bottom + 8 : r.top - h - 8, below };
  });
</script>

<div
  bind:this={el}
  class="tip"
  class:show={tip.visible}
  role="tooltip"
  style:left="{pos.x}px"
  style:top="{pos.y}px"
>
  <span>{tip.text}</span>
  {#if tip.kbd}<Kbd keys={tip.kbd} />{/if}
</div>

<style>
  .tip { position: fixed; z-index: var(--z-tip); max-width: 300px; padding: 7px 10px; display: flex; align-items: center; gap: 8px; border-radius: var(--r-md); background: var(--pop); color: var(--ink-1); font-size: var(--t-sm); line-height: var(--lh-sm); box-shadow: var(--pop-shadow); pointer-events: none; opacity: 0; transform: translateY(2px); transition: opacity var(--d-1) var(--ease-exit), transform var(--d-1) var(--ease-exit); }
  .tip.show { opacity: 1; transform: none; transition-duration: var(--d-2); transition-timing-function: var(--ease-out); }
</style>
