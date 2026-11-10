<!--
  The app's dialog: a scrim, a centred sheet and its header (serif title, your actions, ×). Esc, ×
  and a click on the scrim close it; focus stays inside while it is open and goes back to where it
  was (`modal`). `onkeydown` sees a key before the dialog does: stopPropagation() there keeps it.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { fade } from 'svelte/transition';
  import { ui } from '../lib/state/ui.svelte';
  import { modal } from '../lib/util/modal';
  import Icon from './Icon.svelte';

  interface Props {
    /** The title, also the dialog's name for screen readers. */
    label: string;
    onclose: () => void;
    width?: number;
    /** The sheet's own size rules (a fixed height, a cap); without any it hugs its content. */
    style?: string;
    /** The line under the header. */
    divider?: boolean;
    onkeydown?: (e: KeyboardEvent) => void;
    /** Header, between the title and ×. */
    actions?: Snippet;
    children: Snippet;
  }
  let { label, onclose, width = 620, style, divider = true, onkeydown, actions, children }: Props = $props();
</script>

<!-- The exit runs through the Web Animations API, which the CSS reduced-motion override does not
     reach, so it is told here. `global`: App mounts two of these inside a nested {#if}. -->
<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && onclose()} out:fade|global={{ duration: ui.reducedMotion ? 0 : 120 }}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" {style} style:width="min({width}px, calc(100vw - 32px))" use:modal={() => onclose()} onkeydowncapture={onkeydown}>
    <header class:divider>
      <h2>{label}</h2>
      {@render actions?.()}
      <button class="icon-btn" aria-label="Close" onclick={() => onclose()}><Icon name="x" /></button>
    </header>
    {@render children()}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); display: grid; place-items: center; animation: fade-in var(--d-3) var(--ease-out); }
  .sheet { display: flex; flex-direction: column; border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; flex-shrink: 0; min-height: 64px; padding: 0 14px 0 26px; }
  header.divider { box-shadow: inset 0 -1px 0 var(--line); }
  h2 { margin-right: auto; font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); }
</style>
