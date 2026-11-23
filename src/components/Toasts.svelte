<!--
  The toast stack: top right, under the top bar (and under the Resume bar while that one shows),
  so it never sits on the composer's Send or a sheet's footer. While the agent or task sheet is open the stack moves left of it, off the
  sheet's own controls. Dialogs are drawn above it. The newest toast is at the top.
  Nothing on the page moves for a toast: the stack lies over the page. It is put where it covers
  nothing you can press: if a control of the page is under it, it goes down to just under that
  control, a few times at most, not far and never out of the window (lib/util/toastPlace.ts).
  When the whole stack finds no free place, it folds: one toast shows (the newest, or the newest
  error) with "+2 more" under it, and that smaller stack looks for a place the same way. With no
  free place for it either it stays where it last looked. A press on "+2 more" shows the whole
  stack for as long as the pointer or the focus is on it; it may then cover a control, by your
  own act. A toast that is put away does not count down: its time goes on when it shows again,
  so none goes without ever being seen.
  A toast's text shows six lines; "Show all" opens a longer one in place. The place counts the
  short height: a text you opened moves nothing. A stack that is higher than the window ends at
  the window's bottom and scrolls.
  It looks for its place when the toasts change (also the height of one), when the window's size
  changes, and when the page's layout changes under it: the sheet, the Ask me panel, the IDE,
  the find bar or the Resume bar opens or closes, the rail gets wider, the office takes the whole
  window or gives it back, another room, lens, thread, screen or level of the first-run quest
  shows, the closed requests are read again, a menu or a dialog comes or goes. Also when what is
  in the page's main column gets higher or lower (text that streams in, a card that arrives), at
  most four times a second.
  Not when the page scrolls: what scrolls under a toast can be scrolled out again, and a toast
  takes presses only on itself.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { ide } from '../ide/ide.svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toasts } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { contentWatch, controlBoxes, frontToast, grown, ms, paced, reachDown, restLook, stackPlace, type Shape } from '../lib/util/toastPlace';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';
  import { RESUME_BAR_ROOM } from './ResumeWork.svelte';

  const ICON: Record<string, string> = { success: 'check-circle', error: 'x-circle', info: 'info', progress: 'refresh', attention: 'flag' };

  /** How often the stack goes down to get off a control, at most. */
  const MOVES = 3;
  /** How far down it goes at most: this part of the window's height. */
  const REACH = 0.4;
  /** The gap kept between the stack and a control, in px. */
  const CLEAR = 4;
  /** The room kept between the stack and the window's bottom, in px. */
  const EDGE = 12;
  /** The gap between two toasts, in px. */
  const GAP = 8;
  /** The height of the line "+2 more" under a folded stack, in px. */
  const MORE = 22;
  /** What arrives in the page makes the stack look again at most once in this many ms. */
  const PACE = 250;

  /** The newest toast first: it is the one next to the top bar. */
  const newestFirst = $derived([...toasts.list].reverse());
  /** The one toast that shows when the stack is folded. */
  const front = $derived(frontToast(newestFirst));

  let stack: HTMLDivElement | undefined = $state();
  /** The stack's height in px (0 with no toast): a toast whose text changes can change it. */
  let height = $state(0);
  /** The window changed size this many times. */
  let resizes = $state(0);
  /** What is in the page's main column changed its height this many times (counted once in PACE ms). */
  let arrivals = $state(0);
  /** How far under its first place (right under the top bar) the stack is, in px. */
  let down = $state(0);
  /** A change of place is a move you see. Not for a first toast: it shows at its place. */
  let slides = $state(false);
  /** The whole stack found no free place: one toast shows, with "+2 more" under it. */
  let folded = $state(false);
  /** You pressed "+2 more": the whole stack shows while the pointer or the focus is on it. */
  let unfolded = $state(false);
  /** The toasts whose text is longer than it shows: each ends with "Show all". */
  let long = $state<Record<number, boolean>>({});
  /** The toasts whose whole text you opened, each with the height it had before, in px. */
  let opened = $state<Record<number, number>>({});
  /** The pointer is on the stack. */
  let over = false;

  /** All toasts but `front` are put away. */
  const hides = $derived(folded && !unfolded);
  /** The toasts that are put away now. */
  const away = $derived(hides ? newestFirst.filter((t) => t.id !== front?.id).map((t) => t.id) : []);
  // None of them counts down: it would go without ever being seen.
  $effect(() => toasts.putAway(away));

  /** The stack ends at the window's bottom and scrolls, when it is higher than the room under `at`. */
  function fit(el: HTMLDivElement, at: number, stackHeight: number) {
    const room = window.innerHeight - EDGE - at;
    if (stackHeight <= room) return;
    el.style.maxHeight = `${room}px`;
    el.style.overflowY = 'auto';
  }

  /** Puts the stack at its place: how far down it is, and whether it is folded. */
  function place(el: HTMLDivElement) {
    // What an earlier call set is taken off: the stack has its whole height.
    el.style.maxHeight = '';
    el.style.overflowY = '';
    const top = el.offsetTop;
    // Not a toast that is fading out.
    const here = new Set(toasts.list.map((t) => t.id));
    const items = [...el.querySelectorAll<HTMLElement>('.toast')].filter((t) => here.has(Number(t.dataset.id)));
    const first = items.find((t) => Number(t.dataset.id) === front?.id);
    const stacked = (heights: number[]) => heights.reduce((sum, h) => sum + h, 0) + GAP * (heights.length - 1);
    const real = stacked(items.map((t) => t.offsetHeight));
    if (unfolded) {
      // You unfolded it: it stays where it is.
      fit(el, top + down, real);
      return;
    }
    /** What the place counts for a toast: its short height, also when you opened its text. */
    const counted = (t: HTMLElement) => opened[Number(t.dataset.id)] ?? t.offsetHeight;
    const shape = (h: number): Shape => ({ height: h, reach: reachDown(window.innerHeight, REACH, top, h, EDGE) });
    const whole = shape(stacked(items.map(counted)));
    const one = first && items.length > 1 ? shape(counted(first) + GAP + MORE) : null;
    const now = el.getBoundingClientRect();
    const sides = { left: now.left, right: now.right };
    // Everything the stack can reach, with the gap around it.
    const area = grown({ ...sides, top, bottom: top + whole.height + whole.reach }, CLEAR);
    const controls = controlBoxes(el, area).map((c) => grown(c, CLEAR));
    const at = stackPlace(sides, whole, one, controls, top, MOVES);
    down = at.top - top;
    folded = at.folded;
    // Higher than the window (three long errors in a small window): it scrolls, the newest toast in view.
    fit(el, at.top, at.folded && first ? first.offsetHeight + GAP + MORE : real);
  }

  /** The page's own moves (the sheet sliding in, the stage giving way to it, a room coming in)
   *  take `--d-4`: after this many ms, two frames more, every box is where it stays. */
  const settled = (el: Element): number => ms(getComputedStyle(el).getPropertyValue('--d-4')) + 34;

  /** Once more when the page has come to rest: only then are the boxes final. */
  const rest = restLook(
    () => (stack ? settled(stack) : 0),
    () => {
      slides = true;
      if (stack && toasts.list.length) place(stack);
    },
  );
  const arrived = paced(() => arrivals++, PACE);
  const content = contentWatch(arrived);

  /** After two frames the stack was painted at its first place: from then on a change of place is a move. */
  let painted = 0;
  function showMoves() {
    if (slides || painted) return;
    painted = requestAnimationFrame(() => {
      painted = requestAnimationFrame(() => {
        painted = 0;
        slides = toasts.list.length > 0;
      });
    });
  }

  /** Looks for the stack's place now, and once more when the page is at rest. `moved`: the page's layout changed. */
  function look(moved: boolean) {
    const el = stack;
    if (!el || !toasts.list.length) {
      rest.stop();
      arrived.stop();
      content.stop();
      cancelAnimationFrame(painted);
      painted = 0;
      slides = false;
      folded = false;
      unfolded = false;
      return;
    }
    if (toasts.list.length < 2) unfolded = false;
    place(el);
    content.look();
    rest.after(moved);
    showMoves();
  }

  // The page's layout changes under the stack: its boxes move for a moment.
  $effect(() => {
    void [
      resizes,
      ui.sheetOpen, blockers.panel, ide.visible, ui.findOpen, resumeWork.offer, resumeWork.later,
      ui.railOpen, ui.mode, ui.room, ui.planLens, ui.changesLens, ui.knowledgeLens, ui.thread, ui.closedRev,
      store.workspace?.id, ui.onboarded, ui.onboardingOpen, ui.layoutRev,
    ];
    untrack(() => look(true));
  });
  // The stack changes, or what is in the page: no box of the page moves.
  $effect(() => {
    void [toasts.list, height, unfolded, arrivals];
    untrack(() => look(false));
  });
  $effect(() => () => {
    rest.stop();
    arrived.stop();
    content.stop();
    cancelAnimationFrame(painted);
    // With no stack on screen no toast is put away.
    toasts.putAway([]);
  });

  // Which texts are longer than they show. Read again when a text or the stack's width changes.
  $effect(() => {
    void [toasts.list, resizes];
    const el = stack;
    if (!el) return;
    const next: Record<number, boolean> = {};
    for (const body of el.querySelectorAll<HTMLElement>('.body.clamp')) {
      const id = Number(body.closest<HTMLElement>('.toast')?.dataset.id);
      if (body.scrollHeight > body.clientHeight) next[id] = true;
    }
    untrack(() => {
      if (Object.keys(next).join() !== Object.keys(long).join()) long = next;
      // A toast that went takes its opened text with it.
      const gone = Object.keys(opened).filter((id) => !toasts.list.some((t) => t.id === Number(id)));
      if (gone.length) opened = Object.fromEntries(Object.entries(opened).filter(([id]) => !gone.includes(id)));
    });
  });

  /** "Show all": the whole text, in place. The place goes on counting the height the toast has now. */
  function showAll(id: number, button: HTMLElement) {
    const toast = button.closest<HTMLElement>('.toast');
    if (toast) opened = { ...opened, [id]: toast.offsetHeight };
  }

  /** The pointer left the stack: it folds again, unless the keyboard's focus is in it (a press
   *  with the pointer also leaves the focus on a button: that one does not hold the stack open). */
  function left() {
    over = false;
    if (!stack?.querySelector(':focus-visible')) unfolded = false;
  }
  /** The focus left the stack: it folds again, unless the pointer is on it. */
  function blurred(e: FocusEvent) {
    if (!over && !(e.relatedTarget instanceof Node && stack?.contains(e.relatedTarget))) unfolded = false;
  }
  // The stack itself takes no press and has no role: it only hears what happens to its toasts.
  $effect(() => {
    const el = stack;
    if (!el) return;
    const entered = () => (over = true);
    el.addEventListener('pointerenter', entered);
    el.addEventListener('pointerleave', left);
    el.addEventListener('focusout', blurred);
    return () => {
      el.removeEventListener('pointerenter', entered);
      el.removeEventListener('pointerleave', left);
      el.removeEventListener('focusout', blurred);
    };
  });
</script>

<svelte:window onresize={() => resizes++} />

<div
  class="stack"
  class:beside-sheet={ui.sheetOpen}
  class:slides
  class:unfolded
  bind:this={stack}
  bind:clientHeight={height}
  style:gap="{GAP}px"
  style:--bar-room="{resumeWork.offer ? RESUME_BAR_ROOM : 0}px"
  style:transform="translateY({down}px)"
  aria-live="polite"
  aria-relevant="additions text"
>
  {#each newestFirst as t (t.id)}
    <div
      class="toast k-{t.kind}"
      class:away={away.includes(t.id)}
      data-id={t.id}
      role={t.kind === 'error' ? 'alert' : 'status'}
      onpointerenter={() => toasts.pause(t.id)}
      onpointerleave={() => toasts.resume(t.id)}
      out:fade={{ duration: ui.reducedMotion ? 0 : 120 }}
    >
      <span class="ic" class:spin={t.kind === 'progress'}><Icon name={ICON[t.kind]} size={16} /></span>
      <div class="txt">
        <p class="title">{t.title}</p>
        {#if t.body}
          <p class="body selectable" class:clamp={opened[t.id] === undefined}>{t.body}</p>
          {#if long[t.id] && opened[t.id] === undefined}<button class="all" onclick={(e) => showAll(t.id, e.currentTarget)}>Show all</button>{/if}
        {/if}
      </div>
      {#if t.action}
        <button class="btn sm" onclick={() => { t.action!.run(); toasts.dismiss(t.id); }}>
          {t.action.label}
          {#if t.action.kbd}<Kbd keys={t.action.kbd} subtle />{/if}
        </button>
      {/if}
      <button class="icon-btn sm close" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
  {#if folded && toasts.list.length > 1}
    <button class="more" style:height="{MORE}px" aria-expanded={unfolded} onclick={() => (unfolded = !unfolded)}>
      {unfolded ? 'Show less' : `+${toasts.list.length - 1} more`}
    </button>
  {/if}
</div>

<style>
  /* While the Resume bar shows, the stack starts under the room that bar has (ResumeWork.svelte), as the stage does. */
  .stack { position: fixed; right: var(--gutter); top: calc(var(--top-h) + 4px + var(--bar-room)); z-index: var(--z-toast); display: flex; flex-direction: column; width: min(360px, calc(100vw - 2 * var(--gutter))); pointer-events: none; transition: right var(--d-4) var(--spring); }
  /* A first toast shows at its place; after that the stack moves to a new place (reduced motion: at once, see base.css). */
  .stack.slides { transition: right var(--d-4) var(--spring), transform 160ms var(--ease-out); }
  .stack.beside-sheet { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  /* The whole stack is one thing under the pointer: the gap between two toasts does not fold it again. */
  .stack.unfolded { pointer-events: auto; }
  /* No shrink: a stack higher than the window scrolls, its toasts keep their height. */
  .toast { flex-shrink: 0; pointer-events: auto; display: flex; align-items: flex-start; gap: 10px; padding: 12px 10px 12px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); animation: pop-in var(--d-3) var(--spring); }
  /* Put away in a folded stack: not seen, not pressed, not read out, and its time stands still. It keeps its width, so its height can be measured. */
  .toast.away { position: absolute; top: 0; left: 0; right: 0; visibility: hidden; pointer-events: none; }
  .ic { margin-top: 2px; color: var(--ink-2); }
  .k-success .ic { color: var(--ok); }
  .k-error .ic { color: var(--bad); }
  .k-attention .ic { color: var(--amber); }
  .k-progress .ic { color: var(--live); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  .txt { flex: 1; min-width: 0; }
  /* A long word with no place to break (a branch, a file name) breaks anywhere: it stays in its toast. */
  .title { font-weight: var(--w-med); color: var(--ink-1); overflow-wrap: anywhere; }
  .body { margin-top: 2px; color: var(--ink-2); font-size: var(--t-sm); line-height: var(--lh-sm); overflow-wrap: anywhere; }
  /* Six lines; a longer text ends with "Show all" (no scroll inside a toast). */
  .body.clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 6; line-clamp: 6; overflow: hidden; }
  .all { margin-top: 2px; font-size: var(--t-sm); line-height: var(--lh-sm); font-weight: var(--w-med); color: var(--ink-1); text-decoration: underline; text-underline-offset: 2px; }
  .btn { margin-top: -2px; }
  .close { margin: -3px -2px 0 -2px; color: var(--ink-3); }
  .more { flex-shrink: 0; align-self: center; pointer-events: auto; padding: 0 12px; border-radius: var(--r-full); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .more:hover { color: var(--ink-1); }
</style>
