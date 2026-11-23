<!--
  The toast stack: top right, under the top bar, so it never sits on the composer's Send or a
  sheet's footer. While the agent or task sheet is open the stack moves left of it, off the
  sheet's own controls. Dialogs are drawn above it. The newest toast is at the top.
  Nothing on the page moves for a toast: the stack lies over the page. In a wide window it
  stands in the free room beside the page's panel (Home's conversation column, the first-run
  quest's level: the panel that carries `data-toast-beside`), as wide as that room. Everywhere it
  is put where it covers nothing you can press: if a control of the page is under it, it goes
  down to just under that control, a few times at most, not far and never out of the window
  (lib/util/toastPlace.ts); with no free place it stays where it last looked. A stack that is
  higher than the window ends at the window's bottom and scrolls.
  It looks for its place when the toasts change (also the height of one), when the window's size
  changes, and when the page's layout changes under it: the sheet, the Ask me panel, the IDE,
  the find bar or the Resume bar opens or closes, the rail gets wider, the office takes the whole
  window or gives it back, another room, lens, thread or screen shows.
  Not when the page scrolls: what scrolls under a toast can be scrolled out again, and a toast
  takes presses only on itself.
  A toast's button is under its text, so the text has the toast's whole width. A long text
  scrolls inside its toast: no toast takes half the window.
-->
<script lang="ts">
  import { fade } from 'svelte/transition';
  import { ide } from '../ide/ide.svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toasts } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { BESIDE, controlBoxes, grown, ms, panelShows, reachDown, toastTop, widthBeside, type Box } from '../lib/util/toastPlace';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';

  const ICON: Record<string, string> = { success: 'check-circle', error: 'x-circle', info: 'info', progress: 'refresh', attention: 'flag' };

  /** How often the stack goes down to get off a control, at most. */
  const MOVES = 3;
  /** How far down it goes at most: this part of the window's height. */
  const REACH = 0.4;
  /** The gap kept between the stack and a control, in px. */
  const CLEAR = 4;
  /** Beside the page's panel: the gap to the panel and the least the stack is wide, in px. The
   *  most is its own width (the style below). */
  const GAP = 12;
  const LEAST = 220;
  /** The room kept between the stack and the window's bottom, in px. */
  const EDGE = 12;

  /** The newest toast first: it is the one next to the top bar. */
  const newestFirst = $derived([...toasts.list].reverse());
  /** Home's conversation is on screen: its column is a panel the stack stands beside. */
  const conversationShown = $derived(!!store.workspace && ui.canFind);

  let stack: HTMLDivElement | undefined = $state();
  /** The stack's height in px (0 with no toast): a toast whose text changes can change it. */
  let height = $state(0);
  /** The window changed size this many times. */
  let resizes = $state(0);
  /** How far under its first place (right under the top bar) the stack is, in px. */
  let down = $state(0);
  /** A change of place is a move you see. Not for a first toast: it shows at its place. */
  let slides = $state(false);

  /** The width for the free room beside the page's panel; null: no such room (no panel on
   *  screen, a narrow window, an open sheet), the stack keeps its own width. Call it while the
   *  stack has its own width. */
  function roomBeside(el: HTMLDivElement): number | null {
    const marked = [...document.querySelectorAll(`[${BESIDE}]`)];
    if (import.meta.env.DEV && conversationShown && !marked.length) {
      console.warn(`Toasts: Home's conversation is on screen and no panel carries ${BESIDE}. The toast stack keeps its own width and can lie on the conversation.`);
    }
    // The one a press reaches: not the office's column under the quest or under the IDE.
    const panel = marked.find((p) => panelShows(p, el));
    if (!panel) return null;
    const own = el.getBoundingClientRect();
    return widthBeside(panel.getBoundingClientRect().right, own.right, GAP, LEAST, own.width);
  }

  /** Puts the stack at its place: first its width (its height follows from it), then how far down. */
  function place(el: HTMLDivElement) {
    // What an earlier call set is taken off: the stack has its own width and its whole height.
    el.style.width = '';
    el.style.maxHeight = '';
    el.style.overflowY = '';
    const width = roomBeside(el);
    if (width !== null) el.style.width = `${width}px`;
    const top = el.offsetTop;
    // Higher than the window (three long errors in a small window): it ends at the window's
    // bottom and scrolls, the newest toast in view.
    const room = window.innerHeight - EDGE - top;
    if (el.offsetHeight > room) {
      el.style.maxHeight = `${room}px`;
      el.style.overflowY = 'auto';
    }
    const now = el.getBoundingClientRect();
    const reach = reachDown(window.innerHeight, REACH, top, now.height, EDGE);
    const first: Box = { left: now.left, right: now.right, top, bottom: top + now.height };
    // Everything the stack can reach, with the gap around it.
    const area = grown({ ...first, bottom: first.bottom + reach }, CLEAR);
    const controls = controlBoxes(el, area).map((c) => grown(c, CLEAR));
    down = toastTop(first, controls, top, MOVES, reach) - top;
  }

  /** A text that scrolls inside its toast can be reached with the keyboard, to scroll it, and
   *  says what it is. Asked again when the toast's title or text changes and when the text's box
   *  changes size (the stack got another width). */
  function keyboardScroll(text: HTMLElement, shown: [title: string, body?: string]) {
    let title = shown[0];
    const set = () => {
      if (text.scrollHeight > text.clientHeight) {
        text.tabIndex = 0;
        text.setAttribute('role', 'group');
        text.setAttribute('aria-label', `${title}: details`);
      } else {
        for (const name of ['tabindex', 'role', 'aria-label']) text.removeAttribute(name);
      }
    };
    set();
    const sized = new ResizeObserver(set);
    sized.observe(text);
    return {
      update(next: [title: string, body?: string]) {
        title = next[0];
        set();
      },
      destroy: () => sized.disconnect(),
    };
  }

  /** The page's own moves (the sheet sliding in, the stage giving way to it, a room coming in)
   *  take `--d-4`: after this many ms, two frames more, every box is where it stays. */
  const settled = (el: Element): number => ms(getComputedStyle(el).getPropertyValue('--d-4')) + 34;

  $effect(() => {
    // What changes the page's layout under the stack.
    void [
      toasts.list, height, resizes,
      ui.sheetOpen, blockers.panel, ide.visible, ui.findOpen, resumeWork.offer, resumeWork.later,
      ui.railOpen, ui.mode, ui.room, ui.planLens, ui.changesLens, ui.knowledgeLens, ui.thread,
      store.workspace?.id, ui.onboarded, ui.onboardingOpen,
    ];
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
  {#each newestFirst as t (t.id)}
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
        {#if t.body}<p class="body selectable" use:keyboardScroll={[t.title, t.body]}>{t.body}</p>{/if}
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
  /* The one place that says how wide a toast is at most: beside the page's panel it is as wide as the room there. */
  .stack { position: fixed; right: var(--gutter); top: calc(var(--top-h) + 4px); z-index: var(--z-toast); display: flex; flex-direction: column; gap: 8px; width: min(360px, calc(100vw - 2 * var(--gutter))); pointer-events: none; transition: right var(--d-4) var(--spring); }
  /* A first toast shows at its place; after that the stack moves to a new place (reduced motion: at once, see base.css). */
  .stack.slides { transition: right var(--d-4) var(--spring), transform 160ms var(--ease-out); }
  .stack.beside-sheet { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  /* No shrink: a stack higher than the window scrolls, its toasts keep their height. */
  .toast { flex-shrink: 0; pointer-events: auto; display: flex; align-items: flex-start; gap: 10px; padding: 12px 10px 12px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); animation: pop-in var(--d-3) var(--spring); }
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
