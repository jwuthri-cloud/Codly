<script lang="ts">
  import { tip } from '../lib/state/tooltip.svelte';
  import { tipPlace } from '../lib/util/tipPlace';
  import Kbd from './Kbd.svelte';

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: -999, y: -999, below: false });

  $effect(() => {
    const r = tip.rect;
    if (!tip.visible || !r || !el) return;
    void tip.text;
    pos = tipPlace(r, el.offsetWidth, el.offsetHeight, tip.place, { width: window.innerWidth, height: window.innerHeight });
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
