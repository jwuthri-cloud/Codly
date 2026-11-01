<!--
  Anchored menu (popover). Keyboard: ↑/↓ (or J/K), ↵ runs, Esc closes. An item with `stay` keeps
  the menu open (a picker you may change several times); the menu follows its new items. Positions under the anchor,
  flips above when there's no room. Rendered fixed so it escapes clipping containers.
-->
<script lang="ts" module>
  export type MenuItem =
    | { kind?: 'item'; label: string; icon?: string; kbd?: string; hint?: string; danger?: boolean; disabled?: boolean; checked?: boolean; stay?: boolean; run: () => void }
    | { kind: 'header'; label: string }
    | { kind: 'separator' };
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';

  interface Props {
    anchor: HTMLElement;
    items: MenuItem[];
    onclose: () => void;
    align?: 'start' | 'end';
    width?: number;
  }
  let { anchor, items, onclose, align = 'start', width = 240 }: Props = $props();

  let el: HTMLDivElement | undefined = $state();
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
    if (el && !el.contains(t) && !anchor.contains(t)) onclose();
  }

  // New items can change its height: an upward menu keeps its bottom on the anchor.
  $effect(() => {
    void items;
    tick().then(place);
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
      el?.focus();
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
  role="menu"
  tabindex="-1"
  style:left="{pos.x}px"
  style:top="{pos.y}px"
  style:width="{width}px"
  onkeydown={key}
>
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
        {#if it.hint}<span class="hint truncate">{it.hint}</span>{/if}
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
  .sep { height: 1px; margin: 4px 8px; background: var(--line); }
  .hdr { padding: 8px 10px 3px; }
</style>
