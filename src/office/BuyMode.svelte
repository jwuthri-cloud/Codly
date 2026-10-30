<!--
  Build/Buy: the XP the agents earned in this repo (retired ones too) buys furniture. Pick a piece
  in the catalog and put it down anywhere it fits (buy.ts); click one in the office to move it,
  turn it or sell it back for the full price. Cosmetic: furniture never changes how agents work,
  only where they walk.
-->
<script lang="ts">
  import type { Placed } from '../lib/gen/Placed';
  import type { Shop } from '../lib/gen/Shop';
  import { BY_ID, CATEGORIES, FOR_SALE, type Furniture } from './buy';
  import type { OfficeEngine } from './engine';
  import { thumbnail } from './gfx/decor';
  import { sound } from './sound';

  interface Props {
    engine: OfficeEngine;
    shop: Shop;
    insets: { top: number; right: number; bottom: number; left: number };
    onplace: (piece: Placed) => void;
    onsell: (uid: string) => void;
  }
  let { engine, shop, insets, onplace, onsell }: Props = $props();

  let open = $state(false);
  let cat = $state(CATEGORIES[0]);
  /** The piece following the cursor: its catalog entry, the placed one when moving, why not here. */
  let carrying = $state<{ f: Furniture; uid: string | null; why: string | null } | null>(null);
  let note = $state<string | null>(null);
  const left = $derived(shop.earned - shop.spent);
  const items = $derived(FOR_SALE.filter((f) => f.cat === cat));

  // Build/Buy has its own music, like the Sims.
  $effect(() => {
    engine.setBuildMode(open ? pick : null);
    sound.music(open ? 'music-buy' : null);
    return () => {
      engine.setBuildMode(null);
      sound.music(null);
    };
  });

  let seq = 0;
  function carry(f: Furniture, uid: string | null = null) {
    note = null;
    sound.play('ui-click', { gain: 0.6 });
    const n = ++seq; // ending the last carry (below) must not clear this one
    engine.carry(
      f.id,
      {
        onplace: (p) => {
          if (!uid) sound.play('ui-buy', { gain: 0.8 });
          onplace(p);
        },
        onsell: sell,
        onstate: (why) => n === seq && carrying && (carrying.why = why),
        onend: () => n === seq && (carrying = null),
      },
      uid,
    );
    carrying = { f, uid, why: null };
  }

  // R, Esc and Delete work wherever the focus is (after a click in the catalog, say); the office
  // handles them itself when it has the focus.
  function onkey(e: KeyboardEvent) {
    const c = carrying;
    if (!c || e.defaultPrevented || e.metaKey || e.ctrlKey || (e.target as HTMLElement)?.closest?.('input, textarea, [contenteditable]')) return;
    if (e.key === 'r' || e.key === 'R') engine.rotateCarry();
    else if (e.key === 'Escape') engine.endCarry();
    else if ((e.key === 'Delete' || e.key === 'Backspace') && c.uid) sell(c.uid);
    else return;
    e.preventDefault();
  }

  function pick(uid: string) {
    const p = shop.placed.find((x) => x.uid === uid);
    const f = p && BY_ID.get(p.item);
    if (!f) return;
    if (!engine.movable(uid)) {
      note = `Clear the top of the ${f.label.toLowerCase()} first.`;
      sound.play('ui-error', { gain: 0.6 });
    }
    else carry(f, uid);
  }

  function sell(uid: string) {
    engine.endCarry();
    sound.play('ui-sell', { gain: 0.8 });
    onsell(uid);
  }

  function thumb(img: HTMLImageElement, id: string) {
    let live = true;
    void thumbnail(id).then((src) => live && (img.src = src));
    return { destroy: () => (live = false) };
  }

  const close = () => {
    engine.endCarry();
    sound.play('ui-close', { gain: 0.5 });
    open = false;
  };
</script>

<svelte:window onkeydown={onkey} />

<button class="buy" class:on={open} style:bottom="{insets.bottom + 46}px" style:right="{insets.right + 10}px" onclick={() => (open ? close() : (sound.play('ui-open', { gain: 0.5 }), (open = true)))} aria-expanded={open} aria-label="Build/Buy: {left} XP to spend on furniture">
  🛋 Buy · <b>{left}</b> XP
</button>

{#if open}
  <div class="panel" style:bottom="{insets.bottom + 84}px" style:left="{insets.left + 10}px" style:right="{insets.right + 10}px" role="dialog" aria-label="Buy furniture">
    {#if carrying}
      {@const c = carrying}
      <div class="carry">
        <span class="what"><b>{c.f.label}</b> {c.uid ? '· moving' : `· ${c.f.price} XP`}</span>
        <span class="hint" class:bad={c.why}>{c.why ?? 'Click the floor to put it down'}</span>
        <span class="keys">
          <button onclick={() => engine.rotateCarry()} title="Turn it (R)">⟳ Turn <kbd>R</kbd></button>
          {#if c.uid}
            {@const uid = c.uid}
            <button class="sell" onclick={() => sell(uid)} title="Sell it back (Delete)">Sell for {c.f.price} XP</button>
          {/if}
          <button onclick={() => engine.endCarry()} title={c.uid ? 'Put it back (Esc)' : 'Cancel (Esc)'}>{c.uid ? 'Put back' : 'Cancel'} <kbd>Esc</kbd></button>
        </span>
      </div>
    {:else}
      <header>
        <div>
          <div class="kicker">Build/Buy</div>
          <b>{left} XP to spend</b> <small>of {shop.earned} earned by the team · click a piece in the office to move or sell it</small>
        </div>
        <button class="x" aria-label="Close" onclick={close}>✕</button>
      </header>
      {#if note}<p class="note bad">{note}</p>{/if}
      <div class="tabs" role="tablist">
        {#each CATEGORIES as c (c)}
          <button role="tab" aria-selected={c === cat} class:on={c === cat} onclick={() => (cat = c)}>{c}</button>
        {/each}
      </div>
      <ul>
        {#each items as f (f.id)}
          <li>
            <button disabled={left < f.price} onclick={() => carry(f)} title={left < f.price ? `${f.price - left} XP to go` : `Put a ${f.label.toLowerCase()} in the office`}>
              <img use:thumb={f.id} alt="" width="64" height="64" />
              <span class="n">{f.label}</span>
              <span class="p">{f.price} XP</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .buy, .panel { font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif; color: #eef1f7; }
  .buy { position: absolute; z-index: 400; height: 30px; padding: 0 12px; border: 0; border-radius: 999px; background: rgba(18, 22, 32, 0.82); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08); cursor: pointer; }
  .buy.on { box-shadow: inset 0 0 0 1.5px #7ee0a1; }
  .buy b { color: #8ff0b0; }
  .panel { position: absolute; z-index: 401; max-width: 980px; margin: 0 auto; padding: 12px; border-radius: 16px; background: rgba(22, 26, 36, 0.96); box-shadow: 0 24px 60px -24px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); }
  header { display: flex; justify-content: space-between; align-items: start; margin-bottom: 8px; gap: 8px; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; margin-bottom: 3px; }
  small { font-size: 11px; opacity: 0.6; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 15px; cursor: pointer; }
  .tabs { display: flex; gap: 4px; overflow-x: auto; margin-bottom: 8px; scrollbar-width: none; }
  .tabs button { flex: none; padding: 4px 11px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.06); color: inherit; font: inherit; cursor: pointer; }
  .tabs button.on { background: #7ee0a1; color: #0f2a1a; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0 0 4px; display: grid; grid-auto-flow: column; grid-auto-columns: 92px; grid-template-rows: auto; gap: 6px; overflow-x: auto; }
  li button { width: 100%; display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 6px 4px; border: 0; border-radius: 12px; background: rgba(255, 255, 255, 0.05); color: inherit; font: inherit; cursor: pointer; }
  li button:hover:not(:disabled) { background: rgba(126, 224, 161, 0.14); }
  li button:disabled { opacity: 0.4; cursor: default; }
  img { width: 64px; height: 64px; }
  .n { font-size: 11.5px; line-height: 1.2; text-align: center; min-height: 2.4em; }
  .p { font-size: 11px; color: #8ff0b0; }
  .carry { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; }
  .what { font-size: 13px; }
  .hint { opacity: 0.75; flex: 1; }
  .bad { color: #ff9b8f; opacity: 1; }
  .keys { display: flex; gap: 6px; }
  .keys button { padding: 5px 11px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.1); color: inherit; font: inherit; cursor: pointer; }
  .keys button.sell { background: #f0c27e; color: #2a1e0f; font-weight: 600; }
  kbd { font: 600 10px ui-monospace, monospace; opacity: 0.6; margin-left: 3px; }
  .note { margin: 0 0 6px; font-size: 12px; }
</style>
