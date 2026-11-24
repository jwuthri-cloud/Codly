<!--
  Work was under way when Codly last closed, and nothing of it starts before you say so
  (Settings → Long runs → "Resume work when Codly opens"). With Ask this is the question:
  Resume work, or Not now. After Not now, and with Never, it is one line with the Resume work
  button. It stays until you press it: the waiting tasks name this button by its label, so
  the label is the same in both states (orchestrator/resume.rs, HELD).
  The bar is not a toast: it is there from the launch until you answer. It has room of its own
  under the top bar: while it shows, the stage starts below it (shell/Shell.svelte), so it
  covers nothing of the page, and it is as wide as the stage at most. It lies on no sheet and
  not on the Ask me panel: while one of them is open the bar ends left of it. In a room too
  narrow for both lines (`showsBothLines`: a small window, a sheet beside it) the question takes
  both lines and the line under it goes (the whole text is in the tooltip); its buttons stay. It
  is drawn above the first-run quest, so the question can be answered there too.
-->
<script lang="ts" module>
  /** The gap over the bar, its height and the gap under it, in px. */
  const OVER = 4;
  const HEIGHT = 60;
  const UNDER = 6;
  /** The room the bar takes under the top bar, in px: the stage starts below it (shell/Shell.svelte). */
  export const RESUME_BAR_ROOM = OVER + HEIGHT + UNDER;
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { asksNow, resumeQuestion, showsBothLines } from '../lib/util/resumeWork';
  import Icon from './Icon.svelte';

  const offer = $derived(resumeWork.offer);
  const asking = $derived(asksNow(store.settings.resume_on_launch, resumeWork.later));
  /** The width of the room the bar is centred in, in px: the window less what is beside the bar. */
  let room = $state(0);
  const both = $derived(showsBothLines(room));
  /** In an office the bar starts where the stage starts, right of the rail (as `.frame` in shell/Shell.svelte). */
  const start = $derived(store.ready && store.workspace ? `calc(${ui.railW}px + var(--gutter) * 2)` : null);

  // What the bar says was true when the core was asked. While it is on screen, a task or a plan
  // that changes may have changed it (the tasks that waited were cancelled): the core is asked
  // again (`changed` waits a second, so many changes are one read), and once when the bar shows.
  const shown = $derived(offer != null);
  $effect(() => {
    if (!shown) return;
    void store.tasks;
    void store.plans;
    untrack(() => resumeWork.changed());
  });
</script>

{#if offer}
  {@const title = asking ? resumeQuestion(offer) : 'Work is on hold since Codly opened.'}
  {@const sub = asking ? 'Nothing starts until you answer.' : 'Nothing starts until you resume it.'}
  <div class="room" class:beside-sheet={ui.sheetOpen} class:beside-ask={blockers.panel} style:--start={start} style:top="calc(var(--top-h) + {OVER}px)" style:height="{HEIGHT}px" bind:clientWidth={room}>
    <div class="bar" role="region" aria-label="Resume work">
      <span class="ic"><Icon name="pause" size={15} /></span>
      <div class="txt" role="status" data-tip={both ? undefined : `${title} ${sub}`}>
        <p class="title" class:truncate={both} class:two={!both}>{title}</p>
        {#if both}<p class="sub truncate">{sub}</p>{/if}
      </div>
      {#if asking}
        <button class="btn sm" disabled={resumeWork.busy} onclick={() => resumeWork.notNow()}>Not now</button>
        <button class="btn sm primary" disabled={resumeWork.busy} onclick={() => resumeWork.resume()}>Resume work</button>
      {:else}
        <button class="btn sm primary" disabled={resumeWork.busy} onclick={() => resumeWork.resume()}>Resume work</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* The room the bar is centred in: the window, less `--start` at the left and `--end` at the
     right. It takes no press itself: only the bar does. The shell is a layer of its own, so this
     bar, at the page's root, is drawn above all of it, the sheet and the Ask me panel too: only
     its place keeps it off them. Its z-index puts it above the first-run quest and under a menu,
     a toast and a dialog. */
  .room { --start: 16px; --end: 16px; position: fixed; left: var(--start); right: var(--end); z-index: calc(var(--z-pop) - 1); display: flex; pointer-events: none; transition: left var(--d-4) var(--spring), right var(--d-4) var(--spring); }
  /* Its room ends where the sheet begins: it gives way as the stage does (shell/Shell.svelte). */
  .room.beside-sheet { --end: calc(var(--sheet-w) + var(--gutter) * 2); }
  /* The Ask me panel is `--ask-w` wide, or the window less its two gutters (home/AskMe.svelte).
     It is there at once, so the bar gives way at once. */
  .room.beside-ask { --end: calc(min(var(--ask-w), 100vw - 2 * var(--gutter)) + var(--gutter) * 2); transition: none; }
  .bar { pointer-events: auto; margin: 0 auto; display: flex; align-items: center; gap: 10px; width: fit-content; max-width: min(620px, 100%); padding: 0 12px 0 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); }
  .ic { flex-shrink: 0; color: var(--amber); }
  .txt { flex: 1; min-width: 0; }
  .title { font-weight: var(--w-med); color: var(--ink-1); }
  .title.two { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; line-height: var(--lh-sm); }
  .sub { margin-top: 2px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .btn { flex-shrink: 0; }
</style>
