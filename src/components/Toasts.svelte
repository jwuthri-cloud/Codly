<!--
  The toast stack: top right, under the top bar (and under the Resume bar while that one shows;
  not while the editor overlay is drawn over that bar),
  so it never sits on the composer's Send or a sheet's footer. While the agent or task sheet is open the stack moves left of it, off the
  sheet's own controls. Dialogs are drawn above it. The newest toast is at the top.
  Nothing on the page moves for a toast: the stack lies over the page. It is put where it covers
  nothing you can press, and on no card that waits for your answer: if one is under it, it goes
  down to 4 px under it, a few times at most, not far and never out of the window
  (lib/util/toastPlace.ts). Less than 8 px over a control at an edge does not count.
  When the whole stack finds no free place, it folds: one toast shows (the newest, or the newest
  error) with "+2 more" under it, and that smaller stack looks for a place the same way. With no
  free place for it either it stays where it last looked off every waiting card, or goes under
  the cards: it lies on a waiting card only when the window leaves no room. A press on "+2 more" shows the whole
  stack for as long as the pointer or the focus is on it; it may then cover a control, by your
  own act. A toast that is put away does not count down: its time goes on when it shows again,
  so none goes without ever being seen.
  A toast's text shows six lines; "Show all" opens a longer one in place. The place counts the
  short height: a text you opened moves nothing. A stack that is higher than the window ends at
  the window's bottom and scrolls.
  The same message shows once, with a count ("×3"). With two or more toasts that you can dismiss,
  "Dismiss all" closes them (a progress toast stays). Esc dismisses the newest toast that waits for
  you (lib/state/esc.ts).
  A new toast, a change of the window or of the page's layout: the stack looks for its place from
  the top bar down. A toast that goes or changes, or what arrives in the page, moves nothing: the
  stack keeps its place while that is free (toastPlace.ts `lookFor`). While the pointer is on the
  stack or the keyboard's focus is in it, that look waits until you leave it, and a toast you
  dismiss leaves an empty place of its height: the toasts under it stay under your pointer
  (`stackRows`). The places close when you leave the stack. "Dismiss all" leaves no place.
  A toast whose action can no longer be done goes (`ToastAction.gone`). Code in a toast's text
  (between backticks) is drawn as code; no letter of it is changed (lib/util/toastText.ts).
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
  import { flushSync, untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { ide } from '../ide/ide.svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toasts } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { AIR, barRoom, contentWatch, controlBoxes, frontToast, grown, lookFor, ms, paced, reachDown, restLook, stackFit, stackPlace, stackRows, type Gap, type Shape, type StackChange } from '../lib/util/toastPlace';
  import { toastParts } from '../lib/util/toastText';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';
  import { RESUME_BAR_ROOM } from './ResumeWork.svelte';
  import { connectionBarRoom } from '../shell/ConnectionBar.svelte';

  const ICON: Record<string, string> = { success: 'check-circle', error: 'x-circle', info: 'info', progress: 'refresh', attention: 'flag' };

  /** How often the stack goes down to get off a control, at most. */
  const MOVES = 3;
  /** How far down it goes at most: this part of the window's height. */
  const REACH = 0.4;
  /** The room kept between the stack and the window's bottom, in px. */
  const EDGE = 12;
  /** The gap between two toasts, in px. */
  const GAP = 8;
  /** The height of the line under the stack ("+2 more", "Dismiss all"), in px. */
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
  /** A look for the stack's place waits until you no longer work on it (`lookFor`). */
  let owed = false;
  /** The look when the page is at rest looks from the top bar down: the page's layout changed since the last one. */
  let restFresh = false;
  /** The toasts at the stack's last change: one that is not among them is new. */
  let known = new Set<number>();
  /** "Dismiss all" shows: two or more toasts can be dismissed. */
  const all = $derived(toasts.dismissible.length >= 2);
  /** "+2 more" shows. */
  const more = $derived(folded && toasts.list.length > 1);

  /** You work on the stack: the pointer is on it, or the keyboard's focus is in it. */
  const working = () => over || !!stack?.querySelector(':focus-visible');

  /** All toasts but `front` are put away. */
  const hides = $derived(folded && !unfolded);
  /** The toasts that are put away now. */
  const away = $derived(hides ? newestFirst.filter((t) => t.id !== front?.id).map((t) => t.id) : []);
  // None of them counts down: it would go without ever being seen.
  $effect(() => toasts.putAway(away));

  /** The toasts you dismissed while you work on the stack: each keeps an empty place of its own
   *  height until you leave the stack (`stackRows`). Not in a folded stack: it shows one toast. */
  let gaps = $state<Gap[]>([]);
  /** What the stack shows: its toasts, and the places of the ones you dismissed. */
  const drawn = $derived(stackRows(newestFirst, gaps, !hides));
  /** "Dismiss all" was pressed: what it dismisses leaves no place. */
  let clearing = false;
  /** The toasts on screen at the last change of the list. */
  let shownIds: number[] = [];
  // A toast that goes while you work on the stack keeps its place. Measured here, before the page
  // changes: its box is still there.
  $effect.pre(() => {
    const ids = toasts.list.map((t) => t.id);
    untrack(() => {
      const left = shownIds.filter((id) => !ids.includes(id));
      shownIds = ids;
      if (clearing) {
        clearing = false;
        gaps = [];
        return;
      }
      if (!left.length || hides || !stack || !working()) return;
      const kept: Gap[] = [];
      for (const id of left) {
        const el = stack.querySelector<HTMLElement>(`.toast[data-id="${id}"]`);
        if (el) kept.push({ id, height: el.offsetHeight });
      }
      if (kept.length) gaps = [...gaps, ...kept];
    });
  });

  /** "Dismiss all": every toast you can dismiss goes at once, and none keeps its place. */
  function dismissAll() {
    clearing = true;
    toasts.dismissAll();
  }

  // A toast whose action can no longer be done goes (`ToastAction.gone` reads what it needs).
  $effect(() => toasts.dropGone());

  /** The stack ends at the window's bottom and scrolls, when it is higher than the room under `at`
   *  (`stackFit`). `rows`, `first`: as `measure` gives them. */
  function fit(el: HTMLDivElement, rows: HTMLElement[], first: HTMLElement | undefined, at: number) {
    const shows = stackFit({
      heights: rows.map((r) => r.offsetHeight),
      folded: hides && !!first,
      front: first ? rows.indexOf(first) : 0,
      foot: more || all ? MORE : 0,
      gap: GAP,
      top: at,
      bottom: window.innerHeight - EDGE,
    });
    if (!shows.scrolls) return;
    el.style.maxHeight = `${shows.height}px`;
    el.style.overflowY = 'auto';
  }

  const stacked = (heights: number[]) => heights.reduce((sum, h) => sum + h, 0) + GAP * (heights.length - 1);

  /** The stack's rows as they are now (its toasts, not one that fades out, and the places of the
   *  ones you dismissed), with the window's fit taken off: the stack has its whole height.
   *  `first`: the toast that a folded stack shows. */
  function measure(el: HTMLDivElement) {
    el.style.maxHeight = '';
    el.style.overflowY = '';
    const here = new Set(toasts.list.map((t) => t.id));
    const rows = [...el.querySelectorAll<HTMLElement>('.toast, .gap')].filter((r) => r.classList.contains('gap') || here.has(Number(r.dataset.id)));
    const first = rows.find((r) => r.classList.contains('toast') && Number(r.dataset.id) === front?.id);
    return { top: el.offsetTop, rows, first };
  }

  /** The window's fit alone, where the stack is: a look that waits moves nothing, but a stack that
   *  became higher than the room under it (you unfolded it, or opened a long text) scrolls at once. */
  function refit(el: HTMLDivElement) {
    const { top, rows, first } = measure(el);
    fit(el, rows, first, top + down);
  }

  /** Puts the stack at its place: how far down it is, and whether it is folded. `keep`: it keeps the place it has while that is free. */
  function place(el: HTMLDivElement, keep: boolean) {
    const { top, rows, first } = measure(el);
    if (unfolded) {
      // You unfolded it: it stays where it is.
      fit(el, rows, first, top + down);
      return;
    }
    /** What the place counts for a row: a toast's short height, also when you opened its text. */
    const counted = (r: HTMLElement) => opened[Number(r.dataset.id)] ?? r.offsetHeight;
    const shape = (h: number): Shape => ({
      height: h,
      reach: reachDown(window.innerHeight, REACH, top, h, EDGE),
      cardReach: reachDown(window.innerHeight, 1, top, h, EDGE),
    });
    /** The line "Dismiss all" under the whole stack. */
    const foot = all ? GAP + MORE : 0;
    const whole = shape(stacked(rows.map(counted)) + foot);
    const many = rows.filter((r) => r.classList.contains('toast')).length > 1;
    const one = first && many ? shape(counted(first) + GAP + MORE) : null;
    const now = el.getBoundingClientRect();
    const sides = { left: now.left, right: now.right };
    // Everything the stack can reach, with its air around it: down to where it may go to get off a waiting card.
    const area = grown({ ...sides, top, bottom: top + whole.height + whole.cardReach }, AIR);
    const at = stackPlace(sides, whole, one, controlBoxes(el, area), top, MOVES, keep ? top + down : null);
    down = at.top - top;
    folded = at.folded;
    // Higher than the window (three long errors in a small window): it scrolls, the newest toast in view.
    fit(el, rows, first, at.top);
  }

  /** The page's own moves (the sheet sliding in, the stage giving way to it, a room coming in)
   *  take `--d-4`: after this many ms, two frames more, every box is where it stays. */
  const settled = (el: Element): number => ms(getComputedStyle(el).getPropertyValue('--d-4')) + 34;

  /** Once more when the page has come to rest: only then are the boxes final. */
  const rest = restLook(
    () => (stack ? settled(stack) : 0),
    () => {
      slides = true;
      if (!stack || !toasts.list.length) return;
      if (restFresh) place(stack, false);
      else if (working()) owed = true;
      else place(stack, true);
      restFresh = false;
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

  /** Looks for the stack's place now, and once more when the page is at rest (see `lookFor`). `busy`: you work on the stack. */
  function look(change: StackChange, busy = working()) {
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
      owed = false;
      restFresh = false;
      known = new Set();
      if (gaps.length) gaps = [];
      return;
    }
    if (toasts.list.length < 2) unfolded = false;
    const how = lookFor(change, busy);
    if (how === 'wait') {
      owed = true;
      refit(el);
      return;
    }
    owed = false;
    if (change === 'layout') restFresh = true;
    place(el, how === 'keep');
    content.look();
    rest.after(change === 'layout');
    showMoves();
  }

  // The page's layout changes under the stack: its boxes move for a moment.
  $effect(() => {
    void [
      resizes,
      ui.sheetOpen, blockers.panel, ide.visible, ui.findOpen, resumeWork.offer, resumeWork.later,
      ui.railOpen, ui.mode, ui.room, ui.planLens, ui.changesLens, ui.knowledgeLens, ui.thread, ui.closedRev,
      store.workspace?.id, ui.onboarded, ui.onboardingOpen, ui.layoutRev, connectionBarRoom(),
    ];
    untrack(() => look('layout'));
  });
  // The stack changes: no box of the page moves.
  $effect(() => {
    void [toasts.list, height, unfolded];
    untrack(() => {
      const ids = toasts.list.map((t) => t.id);
      const added = ids.some((id) => !known.has(id));
      known = new Set(ids);
      look(added ? 'added' : 'changed');
    });
  });
  // What is in the page's main column got higher or lower.
  $effect(() => {
    void arrivals;
    untrack(() => look('page'));
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

  /** You no longer work on the stack: it folds again, the places of the toasts you dismissed
   *  close, and the look that waits comes now, on the stack as it is drawn after that. */
  function letGo() {
    unfolded = false;
    const closes = gaps.length > 0;
    if (closes) {
      gaps = [];
      flushSync();
    }
    if (owed || closes) look('changed', false);
  }
  /** The pointer left the stack: you let go of it, unless the keyboard's focus is in it (a press
   *  with the pointer also leaves the focus on a button: that one does not hold the stack open). */
  function left() {
    over = false;
    if (!stack?.querySelector(':focus-visible')) letGo();
  }
  /** The focus left the stack: you let go of it, unless the pointer is on it. */
  function blurred(e: FocusEvent) {
    if (!over && !(e.relatedTarget instanceof Node && stack?.contains(e.relatedTarget))) letGo();
  }
  /** A toast has gone from the page. The button that had the focus may have gone with it, and no
   *  'focusout' comes for that: once it is out of the page, you let go unless you still work on it. */
  function gone() {
    requestAnimationFrame(() => {
      if (!working()) letGo();
    });
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

<!-- A title or a text: its code drawn as code (`toastParts`). -->
{#snippet text(words: string)}{#each toastParts(words) as part, i (i)}{#if part.code}<code>{part.text}</code>{:else}{part.text}{/if}{/each}{/snippet}

<div
  class="stack"
  class:beside-sheet={ui.sheetOpen}
  class:slides
  class:unfolded
  class:holds={gaps.length > 0}
  bind:this={stack}
  bind:clientHeight={height}
  style:gap="{GAP}px"
  style:--bar-room="{barRoom(!!resumeWork.offer, ide.visible, RESUME_BAR_ROOM) + connectionBarRoom()}px"
  style:transform="translateY({down}px)"
  aria-live="polite"
  aria-relevant="additions text"
>
  {#each drawn as row (row.id)}
    {#if 'toast' in row}
      {@const t = row.toast}
      <div
        class="toast k-{t.kind}"
        class:away={away.includes(t.id)}
        data-id={t.id}
        role={t.kind === 'error' ? 'alert' : 'status'}
        onpointerenter={() => toasts.pause(t.id)}
        onpointerleave={() => toasts.resume(t.id)}
        onoutroend={gone}
        out:fade|global={{ duration: ui.reducedMotion || gaps.some((g) => g.id === t.id) ? 0 : 120 }}
      >
        <span class="ic" class:spin={t.kind === 'progress'}><Icon name={ICON[t.kind]} size={16} /></span>
        <div class="txt">
          <p class="title">{@render text(t.title)}{#if t.count > 1}<span class="count num">×{t.count}</span>{/if}</p>
          {#if t.body}
            <p class="body selectable" class:clamp={opened[t.id] === undefined}>{@render text(t.body)}</p>
            {#if long[t.id] && opened[t.id] === undefined}<button class="all" onclick={(e) => showAll(t.id, e.currentTarget)}>Show all</button>{/if}
          {/if}
          <!-- Under the text: the text keeps the toast's whole width, so a toast with a button is not a tall narrow column. -->
          {#if t.action}
            <button class="btn sm act" onclick={() => { t.action!.run(); toasts.dismiss(t.id); }}>
              {t.action.label}
              {#if t.action.kbd}<Kbd keys={t.action.kbd} subtle />{/if}
            </button>
          {/if}
        </div>
        <button class="icon-btn sm close" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="x" size={14} /></button>
      </div>
    {:else}
      <!-- The place of a toast you dismissed while you work on the stack: empty, and not read out. -->
      <div class="gap" style:height="{row.gap}px" aria-hidden="true"></div>
    {/if}
  {/each}
  {#if more || all}
    <div class="foot" style:height="{MORE}px">
      {#if more}
        <button class="more" aria-expanded={unfolded} onclick={() => (unfolded = !unfolded)}>
          {unfolded ? 'Show less' : `+${toasts.list.length - 1} more`}
        </button>
      {/if}
      {#if all}<button class="more" onclick={dismissAll}>Dismiss all</button>{/if}
    </div>
  {/if}
</div>

<style>
  /* While the Resume bar shows, the stack starts under the room that bar has (ResumeWork.svelte), as the stage does. */
  .stack { position: fixed; right: var(--gutter); top: calc(var(--top-h) + 4px + var(--bar-room)); z-index: var(--z-toast); display: flex; flex-direction: column; width: min(360px, calc(100vw - 2 * var(--gutter))); pointer-events: none; transition: right var(--d-4) var(--spring); }
  /* A first toast shows at its place; after that the stack moves to a new place (reduced motion: at once, see base.css). */
  .stack.slides { transition: right var(--d-4) var(--spring), transform 160ms var(--ease-out); }
  .stack.beside-sheet { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  /* The whole stack is one thing under the pointer: the gap between two toasts does not fold it
     again, and while it keeps the places of dismissed toasts, does not close them. */
  .stack.unfolded, .stack.holds { pointer-events: auto; }
  /* No shrink: a stack higher than the window scrolls, its toasts keep their height. */
  .toast { flex-shrink: 0; pointer-events: auto; display: flex; align-items: flex-start; gap: 10px; padding: 12px 10px 12px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); animation: pop-in var(--d-3) var(--spring); }
  /* Put away in a folded stack: not seen, not pressed, not read out, and its time stands still. It keeps its width, so its height can be measured. */
  .toast.away { position: absolute; top: 0; left: 0; right: 0; visibility: hidden; pointer-events: none; }
  /* The place of a dismissed toast: nothing shows, but it is part of the stack under the pointer. */
  .gap { flex-shrink: 0; pointer-events: auto; }
  .ic { margin-top: 2px; color: var(--ink-2); }
  .k-success .ic { color: var(--ok); }
  .k-error .ic { color: var(--bad); }
  .k-attention .ic { color: var(--amber); }
  .k-progress .ic { color: var(--live); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  .txt { flex: 1; min-width: 0; }
  /* A long word with no place to break (a branch, a file name) breaks anywhere: it stays in its toast. */
  .title { font-weight: var(--w-med); color: var(--ink-1); overflow-wrap: anywhere; }
  .count { margin-left: 6px; color: var(--ink-3); }
  .body { margin-top: 2px; color: var(--ink-2); font-size: var(--t-sm); line-height: var(--lh-sm); overflow-wrap: anywhere; }
  .title code, .body code { font-size: 0.92em; color: var(--ink-1); }
  /* Six lines; a longer text ends with "Show all" (no scroll inside a toast). */
  .body.clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 6; line-clamp: 6; overflow: hidden; }
  .all { margin-top: 2px; font-size: var(--t-sm); line-height: var(--lh-sm); font-weight: var(--w-med); color: var(--ink-1); text-decoration: underline; text-underline-offset: 2px; }
  .act { margin-top: 8px; }
  .close { margin: -3px -2px 0 -2px; color: var(--ink-3); }
  .foot { flex-shrink: 0; align-self: center; display: flex; gap: 6px; }
  .more { pointer-events: auto; padding: 0 12px; border-radius: var(--r-full); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .more:hover { color: var(--ink-1); }
</style>
