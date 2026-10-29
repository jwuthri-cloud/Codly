<!--
  Build/Buy: the XP the agents earned in this repo (retired ones too) buys furniture for the
  office. Each piece has its spot (layout.ts DECOR); selling gives the full price back.
  Cosmetic: furniture never changes how agents work.
-->
<script lang="ts">
  import type { Shop } from '../lib/gen/Shop';

  interface Props {
    shop: Shop;
    insets: { top: number; right: number; bottom: number; left: number };
    ontrade: (item: string, buy: boolean) => void;
  }
  let { shop, insets, ontrade }: Props = $props();

  let open = $state(false);
  const left = $derived(shop.earned - shop.spent);
  const ICON: Record<string, string> = { poster: '🖼️', ficus: '🪴', 'floor-lamp': '💡', cooler: '🚰', armchair: '🪑', bookshelf: '📚', bench: '🛋️', sofa: '🛋️' };
</script>

<button class="buy" class:on={open} style:bottom="{insets.bottom + 46}px" style:right="{insets.right + 10}px" onclick={() => (open = !open)} aria-expanded={open} aria-label="Build/Buy: {left} XP to spend on furniture">
  🛋 Buy · <b>{left}</b> XP
</button>

{#if open}
  <div class="panel" style:bottom="{insets.bottom + 84}px" style:right="{insets.right + 10}px" role="dialog" aria-label="Buy furniture">
    <header>
      <div>
        <div class="kicker">Build/Buy</div>
        <b>{left} XP to spend</b> <small>of {shop.earned} earned by the team</small>
      </div>
      <button class="x" aria-label="Close" onclick={() => (open = false)}>✕</button>
    </header>
    <ul>
      {#each shop.items as it (it.id)}
        {@const owned = shop.owned.includes(it.id)}
        <li class:owned>
          <span class="ic" aria-hidden="true">{ICON[it.id] ?? '📦'}</span>
          <span class="n">{it.label}<small>{it.price} XP</small></span>
          {#if owned}
            <button class="sell" onclick={() => ontrade(it.id, false)} title="Sell it back for {it.price} XP">Sell</button>
          {:else}
            <button disabled={left < it.price} onclick={() => ontrade(it.id, true)} title={left < it.price ? `${it.price - left} XP to go` : `Put it in the office`}>Buy</button>
          {/if}
        </li>
      {/each}
    </ul>
    <p class="note">XP comes from real work: commits, passing checks, approved reviews and merges. Furniture is just for looks.</p>
  </div>
{/if}

<style>
  .buy, .panel { font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif; color: #eef1f7; }
  .buy { position: absolute; z-index: 400; height: 30px; padding: 0 12px; border: 0; border-radius: 999px; background: rgba(18, 22, 32, 0.82); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08); cursor: pointer; }
  .buy.on { box-shadow: inset 0 0 0 1.5px #7ee0a1; }
  .buy b { color: #8ff0b0; }
  .panel { position: absolute; z-index: 401; width: min(300px, calc(100% - 20px)); padding: 12px; border-radius: 16px; background: rgba(22, 26, 36, 0.96); box-shadow: 0 24px 60px -24px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); }
  header { display: flex; justify-content: space-between; align-items: start; margin-bottom: 8px; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; margin-bottom: 3px; }
  small { display: block; font-size: 11px; opacity: 0.6; }
  header small { display: inline; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 15px; cursor: pointer; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  li { display: grid; grid-template-columns: 24px 1fr auto; gap: 8px; align-items: center; padding: 5px 4px; border-radius: 10px; }
  li.owned { background: rgba(126, 224, 161, 0.08); }
  .ic { font-size: 17px; text-align: center; }
  li button { padding: 4px 12px; border: 0; border-radius: 999px; background: #7ee0a1; color: #0f2a1a; font: 600 12px 'Inter Variable', Inter, system-ui, sans-serif; cursor: pointer; }
  li button:disabled { background: rgba(255, 255, 255, 0.08); color: rgba(238, 241, 247, 0.4); cursor: default; }
  li button.sell { background: rgba(255, 255, 255, 0.1); color: #eef1f7; }
  .note { margin: 8px 0 0; font-size: 11px; opacity: 0.55; }
</style>
