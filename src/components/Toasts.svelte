<!--
  The toast stack: top right, under the top bar, so it never sits on the composer's Send or a
  sheet's footer. While the agent or task sheet is open the stack moves left of it, off the
  sheet's own controls. Dialogs are drawn above it.
  Nothing on the page moves for a toast: the stack lies over the page. In a wide window it
  stands in the free room beside the page's panel (the conversation column), as wide as that
  room. Everywhere it is put where it covers nothing you can press: if a control of the page is
  under it, it goes down to just under that control, a few times at most and not far
  (lib/util/toastPlace.ts); with no free place it stays where it last looked. It looks for its
  place when the toasts change (also the height of one), when the window's size changes, when
  the sheet or the Ask me panel opens or closes and when another room, thread or screen shows.
  Not when the page scrolls: what scrolls under a toast can be scrolled out again, and a toast
  takes presses only on itself.
  A toast's button is under its text, so the text has the toast's whole width. A long text
  scrolls inside its toast: no toast takes half the window.
-->
<script lang="ts">
  import { fade } from 'svelte/transition';
  import { blockers } from '../lib/state/blockers.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toasts } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { controlBoxes, grown, toastTop, widthBeside, type Box } from '../lib/util/toastPlace';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';

  const ICON: Record<string, string> = { success: 'check-circle', error: 'x-circle', info: 'info', progress: 'refresh', attention: 'flag' };

  /** How often the stack goes down to get off a control, at most. */
  const MOVES = 3;
  /** How far down it goes at most: this part of the window's height. */
  const REACH = 0.4;
  /** The gap kept between the stack and a control, in px. */
  const CLEAR = 4;
  /** Beside the page's panel: the gap to the panel, and the least and the most the stack is wide, in px. */
  const GAP = 12;
  const LEAST = 220;
  const MOST = 360;

  let stack: HTMLDivElement | undefined = $state();
  /** The stack's height in px (0 with no toast): a toast whose text changes can change it. */
  let height = $state(0);
  /** The window changed size this many times. */
  let resizes = $state(0);
  /** How far under its first place (right under the top bar) the stack is, in px. */
  let down = $state(0);
  /** A change of place is a move you see. Not for a first toast: it shows at its place. */
  let slides = $state(false);

  /** The width for the free room beside the page's panel (the conversation column, a room,
   *  the first-run quest's level); null: no such room, the stack keeps its own width. */
  function roomBeside(el: HTMLDivElement): number | null {
    const panel = document.querySelector('main .glass');
    // No panel on screen (the Welcome page, the map).
    if (!panel?.getClientRects().length) return null;
    return widthBeside(panel.getBoundingClientRect().right, el.getBoundingClientRect().right, GAP, LEAST, MOST);
  }

  /** Puts the stack at its place: first its width (its height follows from it), then how far down. */
  function place(el: HTMLDivElement) {
    const width = roomBeside(el);
    el.style.width = width === null ? '' : `${width}px`;
    const now = el.getBoundingClientRect();
    const top = el.offsetTop;
    const reach = REACH * window.innerHeight;
    const first: Box = { left: now.left, right: now.right, top, bottom: top + now.height };
    // Everything the stack can reach, with the gap around it.
    const area = grown({ ...first, bottom: first.bottom + reach }, CLEAR);
    const controls = controlBoxes(el, area).map((c) => grown(c, CLEAR));
    down = toastTop(first, controls, top, MOVES, reach) - top;
  }

  /** A text that scrolls inside its toast can be reached with the keyboard, to scroll it.
   *  Asked again when the text or the stack's height changes. */
  function keyboardScroll(text: HTMLElement, _shown: unknown) {
    const set = () => {
      if (text.scrollHeight > text.clientHeight) text.tabIndex = 0;
      else text.removeAttribute('tabindex');
    };
    set();
    return { update: set };
  }

  /** The page's own moves (the sheet sliding in, the stage giving way to it, a room coming in)
   *  take `--d-4`: after this many ms, two frames more, every box is where it stays. */
  const settled = (el: Element): number => parseFloat(getComputedStyle(el).getPropertyValue('--d-4')) + 34;

  $effect(() => {
    void [toasts.list, height, resizes, ui.sheetOpen, blockers.panel, ui.room, ui.thread, store.workspace?.id, ui.onboarded, ui.onboardingOpen];
    const el = stack;
    if (!el || !toasts.list.length) {
      slides = false;
      return;
    }
    place(el);
    // Once more when the page has come to rest: only then are the boxes final.
    const again = setTimeout(() => {
      slides = true;
      place(el);
    }, settled(el));
    return () => clearTimeout(again);
  });
</script>

<svelte:window onresize={() => resizes++} />

<div class="stack" class:beside-sheet={ui.sheetOpen} class:slides bind:this={stack} bind:clientHeight={height} style:transform="translateY({down}px)" aria-live="polite" aria-relevant="additions text">
  {#each toasts.list as t (t.id)}
    <div
      class="toast k-{t.kind}"
      role={t.kind === 'error' ? 'alert' : 'status'}
      onpointerenter={() => toasts.pause(t.id)}
      onpointerleave={() => toasts.resume(t.id)}
      out:fade={{ duration: ui.reducedMotion ? 0 : 120 }}
    >
      <span class="ic" class:spin={t.kind === 'progress'}><Icon name={ICON[t.kind]} size={16} /></span>
      <div class="txt">
        <p class="title">{t.title}</p>
        {#if t.body}<p class="body selectable" use:keyboardScroll={[t.body, height]}>{t.body}</p>{/if}
        {#if t.action}
          <button class="btn sm" onclick={() => { t.action!.run(); toasts.dismiss(t.id); }}>
            {t.action.label}
            {#if t.action.kbd}<Kbd keys={t.action.kbd} subtle />{/if}
          </button>
        {/if}
      </div>
      <button class="icon-btn sm close" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  /* column-reverse: the newest toast is the one next to the top bar */
  .stack { position: fixed; right: var(--gutter); top: calc(var(--top-h) + 4px); z-index: var(--z-toast); display: flex; flex-direction: column-reverse; gap: 8px; width: min(360px, calc(100vw - 2 * var(--gutter))); pointer-events: none; transition: right var(--d-4) var(--spring); }
  /* A first toast shows at its place; after that the stack moves to a new place (reduced motion: at once, see base.css). */
  .stack.slides { transition: right var(--d-4) var(--spring), transform 160ms var(--ease-out); }
  .stack.beside-sheet { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  .toast { pointer-events: auto; display: flex; align-items: flex-start; gap: 10px; padding: 12px 10px 12px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); animation: pop-in var(--d-3) var(--spring); }
  .ic { margin-top: 2px; color: var(--ink-2); }
  .k-success .ic { color: var(--ok); }
  .k-error .ic { color: var(--bad); }
  .k-attention .ic { color: var(--amber); }
  .k-progress .ic { color: var(--live); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  .txt { flex: 1; min-width: 0; }
  .title { font-weight: var(--w-med); color: var(--ink-1); }
  /* Five lines at most: a longer text scrolls inside its toast. */
  .body { margin-top: 2px; max-height: calc(5 * var(--lh-sm)); overflow-y: auto; color: var(--ink-2); font-size: var(--t-sm); line-height: var(--lh-sm); overflow-wrap: anywhere; }
  .btn { margin-top: 8px; }
  .close { margin: -3px -2px 0 -2px; color: var(--ink-3); }
</style>
