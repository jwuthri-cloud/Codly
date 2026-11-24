<!--
  Anchored menu (popover). Keyboard: ↑/↓ (or J/K), ↵ runs, Esc closes. Closed by Esc or by a
  press outside, it gives the focus back to the button that opened it (a press on something that
  takes the focus itself keeps it there). An item with `stay` keeps
  the menu open (a picker you may change several times); the menu follows its new items. Positions under the anchor,
  flips above when there's no room. Rendered fixed so it escapes clipping containers. With `wrap`, a
  row whose hint does not fit beside its label puts the hint on a second line under the label,
  whole; other menus keep one line per row and cut the hint.
-->
<script lang="ts" module>
  export type MenuItem =
    | { kind?: 'item'; label: string; icon?: string; kbd?: string; hint?: string; danger?: boolean; disabled?: boolean; checked?: boolean; stay?: boolean; run: () => void }
    | { kind: 'header'; label: string }
    | { kind: 'separator' };
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { fade } from 'svelte/transition';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';

  interface Props {
    anchor: HTMLElement;
    items: MenuItem[];
    onclose: () => void;
    align?: 'start' | 'end';
    width?: number;
    /** A search field on top (the caller filters its items); null for none. */
    search?: string | null;
    searchPlaceholder?: string;
    /** A hint that does not fit goes to a second line, whole (the mode menus: their help must be read). */
    wrap?: boolean;
  }
  let { anchor, items, onclose, align = 'start', width = 240, search = $bindable(null), searchPlaceholder = 'Search', wrap = false }: Props = $props();

  let el: HTMLDivElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let pos = $state({ x: 0, y: 0, up: false });
  let active = $state(-1);
  const actionable = $derived(items.map((it, i) => ((!it.kind || it.kind === 'item') && !(it as { disabled?: boolean }).disabled ? i : -1)).filter((i) => i >= 0));

  function place() {
    const r = anchor.getBoundingClientRect();
    const h = el?.offsetHeight ?? 200;
    const up = r.bottom + 6 + h > window.innerHeight - 8 && r.top - 6 - h > 8;
    let x = align === 'end' ? r.right - width : r.left;
    x = Math.max(8, Math.min(x, window.innerWidth - width - 8));
    pos = { x, y: up ? r.top - 6 - h : r.bottom + 6, up };
  }

  function run(i: number) {
    const it = items[i];
    if (!it || (it.kind && it.kind !== 'item') || (it as { disabled?: boolean }).disabled) return;
    if (!(it as { stay?: boolean }).stay) onclose();
    (it as { run: () => void }).run();
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      anchor.focus();
    } else if (e.target === input && (e.key === 'j' || e.key === 'k')) {
      // typing in the search
    } else if (e.key === 'ArrowDown' || e.key === 'j') {
      e.preventDefault();
      const idx = actionable.indexOf(active);
      active = actionable[(idx + 1) % actionable.length] ?? -1;
    } else if (e.key === 'ArrowUp' || e.key === 'k') {
      e.preventDefault();
      const idx = actionable.indexOf(active);
      active = actionable[(idx - 1 + actionable.length) % actionable.length] ?? -1;
    } else if (e.key === 'Enter' && active >= 0) {
      e.preventDefault();
      run(active);
    }
  }

  function outside(e: PointerEvent) {
    const t = e.target as Node;
    if (!el || el.contains(t) || anchor.contains(t)) return;
    onclose();
    // After the press has moved the focus: it went nowhere (the menu is gone, the place pressed
    // takes no focus), so it goes back to the button, as after Esc.
    setTimeout(() => {
      const at = document.activeElement;
      if (anchor.isConnected && (!at || at === document.body || !at.isConnected || el?.contains(at))) anchor.focus();
    });
  }

  // New items can change its height: an upward menu keeps its bottom on the anchor.
  $effect(() => {
    void items;
    tick().then(place);
  });

  // An open menu hides the controls under it: the toast stack looks again when it comes and when it goes.
  onMount(() => {
    ui.layoutRev++;
    return () => void ui.layoutRev++;
  });

  onMount(() => {
    // Rendered under <body>: a transformed or filtered ancestor would otherwise become the
    // containing block of `position: fixed` and clip the menu at its own edge.
    // Svelte only removes nodes where it put them, so the menu removes itself on teardown.
    const node = el;
    if (node) document.body.appendChild(node);
    place();
    tick().then(() => {
      place();
      (input ?? el)?.focus();
    });
    window.addEventListener('pointerdown', outside, true);
    window.addEventListener('resize', onclose);
    return () => {
      window.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('resize', onclose);
      node?.remove();
    };
  });
</script>

<div
  bind:this={el}
  class="menu"
  class:up={pos.up}
  class:wrap
  role="menu"
  tabindex="-1"
  style:left="{pos.x}px"
  style:top="{pos.y}px"
  style:width="{width}px"
  onkeydown={key}
  out:fade={{ duration: ui.reducedMotion ? 0 : 90 }}
>
  {#if search !== null}
    <input class="search" bind:this={input} bind:value={search} placeholder={searchPlaceholder} aria-label={searchPlaceholder} spellcheck="false" autocomplete="off" oninput={() => (active = -1)} />
  {/if}
  {#each items as it, i (i)}
    {#if it.kind === 'separator'}
      <div class="sep" role="separator"></div>
    {:else if it.kind === 'header'}
      <div class="hdr eyebrow">{it.label}</div>
    {:else}
      <button
        class="item"
        class:active={active === i}
        class:danger={it.danger}
        role="menuitem"
        disabled={it.disabled}
        onpointerenter={() => (active = i)}
        onclick={() => run(i)}
      >
        <span class="ic">
          {#if it.checked}<Icon name="check" size={14} />{:else if it.icon}<Icon name={it.icon} size={14} />{/if}
        </span>
        <span class="lbl truncate">{it.label}</span>
        {#if it.hint}<span class="hint" class:truncate={!wrap}>{it.hint}</span>{/if}
        {#if it.kbd}<Kbd keys={it.kbd} />{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu { position: fixed; z-index: var(--z-pop); padding: 5px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); outline: none; animation: pop-in var(--d-3) var(--ease-out); max-height: min(420px, 70vh); overflow: auto; }
  .menu.up { animation-name: rise-in; }
  .item { display: flex; align-items: center; gap: 9px; width: 100%; height: 32px; padding: 0 10px; border-radius: var(--r-md); font-size: var(--t-md); color: var(--ink-1); text-align: left; }
  .item.active { background: var(--surface-3); }
  .item.danger { color: var(--bad); }
  .ic { width: 14px; display: grid; place-items: center; color: var(--ink-2); }
  .item.danger .ic { color: var(--bad); }
  .lbl { flex: 1; }
  .hint { color: var(--ink-3); font-size: var(--t-sm); max-width: 45%; }
  /* The hint stays beside the label while both fit; else it wraps to a line of its own, under the label (past the icon's 14 px and the 9 px gap). */
  .wrap .item { flex-wrap: wrap; row-gap: 1px; height: auto; min-height: 32px; padding-top: 6px; padding-bottom: 6px; }
  .wrap .lbl { flex: 1 0 auto; }
  .wrap .hint { max-width: none; min-width: 0; margin-left: 23px; line-height: var(--lh-sm); }
  .sep { height: 1px; margin: 4px 8px; background: var(--line); }
  .hdr { padding: 8px 10px 3px; }
  .search { position: sticky; top: -5px; z-index: 1; width: 100%; height: 32px; margin: -5px 0 4px; padding: 0 10px; border-radius: var(--r-md) var(--r-md) 0 0; background: var(--pop); box-shadow: inset 0 -1px 0 var(--line); font-size: var(--t-md); color: var(--ink-1); outline: none; }
</style>
