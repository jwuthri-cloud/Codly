<!--
  Build/Buy: coins (the wallet: every agent's XP in every repo, plus grants) buy furniture. Pick a piece
  in the catalog and put it down anywhere it fits (buy.ts); click one in the office to move it,
  turn it or sell it back for the full price. Cosmetic: furniture never changes how agents work,
  only where they walk (and what they do on a break). Pieces not earned yet show greyed out,
  with what they need (buy.ts `locked`).
-->
<script lang="ts">
  import type { Lot } from '../lib/gen/Lot';
  import type { Placed } from '../lib/gen/Placed';
  import type { Rank } from '../lib/gen/Rank';
  import type { Shop } from '../lib/gen/Shop';
  import { BY_ID, CATEGORIES, FOR_SALE, kitchens, locked, type Furniture } from './buy';
  import type { OfficeEngine } from './engine';
  import { officeLayout } from './layout';
  import { thumbnail } from './gfx/decor';
  import { sound } from './sound';
  import { coins } from '../lib/util/format';

  interface Props {
    engine: OfficeEngine;
    shop: Shop;
    /** The office's lot (null: the Studio), for its kitchen. */
    lot?: Lot | null;
    /** The ranks on the crew: some pieces need someone at a rank. */
    crew: Rank[];
    insets: { top: number; right: number; bottom: number; left: number };
    onplace: (piece: Placed) => void;
    onsell: (uid: string) => void;
    /** Save the office (lot and crew, no code) as a file. */
    onexport?: () => void;
    /** Open with a new one of this piece in hand (build mode's eyedropper); `n` makes each ask new. */
    copy?: { item: string; n: number } | null;
    /** Go to the Build tab (none: no Build here). */
    onswitch?: () => void;
    open?: boolean;
  }
  let { engine, shop, lot = null, crew, insets, onplace, onsell, onexport, copy = null, onswitch, open = $bindable(false) }: Props = $props();

  let cat = $state(CATEGORIES[0]);
  /** The piece following the cursor: its catalog entry, the placed one when moving, why not here. */
  let carrying = $state<{ f: Furniture; uid: string | null; why: string | null } | null>(null);
  let note = $state<string | null>(null);
  const left = $derived(shop.wallet.balance);
  const items = $derived(FOR_SALE.filter((f) => f.cat === cat));
  const hasKitchen = $derived(kitchens(officeLayout(lot ?? undefined), shop.placed).size > 0);
  const why = (f: Furniture) => locked(f, crew, hasKitchen);

  $effect(() => {
    engine.setBuildMode(open ? pick : null);
    return () => engine.setBuildMode(null);
  });

  $effect(() => {
    const f = copy && BY_ID.get(copy.item);
    if (!f || f.hidden) return;
    open = true;
    // Once Build/Buy mode is on (the effect above), pick it up.
    setTimeout(() => carry(f));
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

{#if open}
  <div class="panel" style:bottom="{insets.bottom + 46}px" style:left="{insets.left + 10}px" style:right="{insets.right + 10}px" role="dialog" aria-label="Buy furniture">
    {#if carrying}
      {@const c = carrying}
      <div class="carry">
        <span class="what"><b>{c.f.label}</b> {c.uid ? '· moving' : `· ${coins(c.f.price)}`}</span>
        <span class="hint" class:bad={c.why}>{c.why ?? 'Click the floor to put it down'}</span>
        <span class="keys">
          <button onclick={() => engine.rotateCarry()} title="Turn it (R)">⟳ Turn <kbd>R</kbd></button>
          {#if c.uid}
            {@const uid = c.uid}
            <button class="sell" onclick={() => sell(uid)} title="Sell it back (Delete)">Sell for {coins(c.f.price)}</button>
          {/if}
          <button onclick={() => engine.endCarry()} title={c.uid ? 'Put it back (Esc)' : 'Cancel (Esc)'}>{c.uid ? 'Put back' : 'Cancel'} <kbd>Esc</kbd></button>
        </span>
      </div>
    {:else}
      <header>
        <div>
          {#if onswitch}
            <span class="seg" role="tablist"><button role="tab" aria-selected="false" onclick={onswitch}>🔨 Build</button><button role="tab" aria-selected="true" class="on">🛋 Buy</button></span>
          {:else}<div class="kicker">Buy</div>{/if}
          <b>{coins(left)} to spend</b> <small>{coins(shop.wallet.earned)} earned by your agents{shop.wallet.granted ? ` · ${coins(shop.wallet.granted)} given` : ''} · click a piece in the office to move or sell it</small>
        </div>
        <span class="side">
          {#if onexport}<button class="export" onclick={onexport} title="Save this office (its lot and its crew's names, looks and ranks; no code) as a file">⤓ Export office</button>{/if}
          <button class="x" aria-label="Close" onclick={close}>✕</button>
        </span>
      </header>
      {#if note}<p class="note bad">{note}</p>{/if}
      <div class="tabs" role="tablist">
        {#each CATEGORIES as c (c)}
          <button role="tab" aria-selected={c === cat} class:on={c === cat} onclick={() => (cat = c)}>{c}</button>
        {/each}
      </div>
      <ul>
        {#each items as f (f.id)}
          {@const lock = why(f)}
          <li>
            <button class:locked={lock} disabled={!!lock || left < f.price} onclick={() => carry(f)} title={lock ?? (left < f.price ? `${coins(f.price - left)} to go` : `Put a ${f.label.toLowerCase()} in the office`)}>
              <img use:thumb={f.id} alt="" width="64" height="64" />
              <span class="n">{f.label}</span>
              {#if lock}<span class="why">🔒 {lock}</span>{:else}<span class="p">{coins(f.price)}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .panel { font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif; color: #eef1f7; }
  .seg { display: inline-flex; gap: 2px; padding: 2px; margin-bottom: 4px; border-radius: 999px; background: rgba(255, 255, 255, 0.05); }
  .seg button { padding: 4px 10px; border: 0; border-radius: 999px; background: none; color: inherit; font: inherit; cursor: pointer; }
  .seg button.on { background: rgba(126, 224, 161, 0.22); color: #c8f5d6; }
  .panel { position: absolute; z-index: 401; max-width: 980px; margin: 0 auto; padding: 12px; border-radius: 16px; background: rgba(22, 26, 36, 0.96); box-shadow: 0 24px 60px -24px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); }
  header { display: flex; justify-content: space-between; align-items: start; margin-bottom: 8px; gap: 8px; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; margin-bottom: 3px; }
  small { font-size: 11px; opacity: 0.6; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 15px; cursor: pointer; }
  .side { display: flex; align-items: center; gap: 6px; flex: none; }
  .export { padding: 4px 10px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.08); color: inherit; font: inherit; font-size: 11.5px; cursor: pointer; }
  .export:hover { background: rgba(255, 255, 255, 0.14); }
  .tabs { display: flex; gap: 4px; overflow-x: auto; margin-bottom: 8px; scrollbar-width: none; }
  .tabs button { flex: none; padding: 4px 11px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.06); color: inherit; font: inherit; cursor: pointer; }
  .tabs button.on { background: #7ee0a1; color: #0f2a1a; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0 0 4px; display: grid; grid-auto-flow: column; grid-auto-columns: 92px; grid-template-rows: auto; gap: 6px; overflow-x: auto; }
  li button { width: 100%; display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 6px 4px; border: 0; border-radius: 12px; background: rgba(255, 255, 255, 0.05); color: inherit; font: inherit; cursor: pointer; }
  li button:hover:not(:disabled) { background: rgba(126, 224, 161, 0.14); }
  li button:disabled { opacity: 0.4; cursor: default; }
  li button.locked { opacity: 0.5; filter: grayscale(1); }
  .why { font-size: 10px; line-height: 1.2; text-align: center; color: #f0c27e; }
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
