<!--
  Work was under way when Codly last closed, and nothing of it starts before you say so
  (Settings → Long runs → "Resume work when Codly opens"). With Ask this is the question:
  Resume, or Not now. After Not now, and with Never, it is one line with the Resume work
  button. It stays until you press it: the waiting tasks name this button.
  The bar is part of the page under the top bar: it lies on no sheet and not on the Ask me
  panel. While one of them is open the bar ends left of it: its text gets shorter (the whole
  text is in the tooltip), its buttons stay.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { asksNow, resumeQuestion } from '../lib/util/resumeWork';
  import Icon from './Icon.svelte';

  const offer = $derived(resumeWork.offer);
  const asking = $derived(asksNow(store.settings.resume_on_launch, resumeWork.later));
  /** The bar ends left of an open sheet or Ask me panel: only then can its text be cut. */
  const beside = $derived(ui.sheetOpen || blockers.panel);

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
  <div class="bar" class:beside-sheet={ui.sheetOpen} class:beside-ask={blockers.panel} role="region" aria-label="Resume work">
    <span class="ic"><Icon name="pause" size={15} /></span>
    <div class="txt" role="status" data-tip={beside ? `${title} ${sub}` : undefined}>
      <p class="title truncate">{title}</p>
      <p class="sub truncate">{sub}</p>
    </div>
    {#if asking}
      <button class="btn sm" disabled={resumeWork.busy} onclick={() => resumeWork.notNow()}>Not now</button>
      <button class="btn sm primary" disabled={resumeWork.busy} onclick={() => resumeWork.resume()}>Resume</button>
    {:else}
      <button class="btn sm primary" disabled={resumeWork.busy} onclick={() => resumeWork.resume()}>Resume work</button>
    {/if}
  </div>
{/if}

<style>
  /* Centred in the room it has: the window, less 16px at the left and `--end` at the right. Under the sheet in the order of layers. */
  .bar { --end: 16px; position: fixed; top: calc(var(--top-h) + 4px); left: 16px; right: var(--end); margin: 0 auto; z-index: calc(var(--z-sheet) - 1); display: flex; align-items: center; gap: 10px; width: fit-content; max-width: min(620px, calc(100vw - 16px - var(--end))); padding: 10px 12px 10px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); transition: right var(--d-4) var(--spring), max-width var(--d-4) var(--spring); }
  /* Its room ends where the sheet begins: it gives way as the stage does (shell/Shell.svelte). */
  .bar.beside-sheet { --end: calc(var(--sheet-w) + var(--gutter) * 2); }
  /* The Ask me panel is 440px wide, or the window less its two gutters (home/AskMe.svelte). It
     is there at once, so the bar gives way at once. */
  .bar.beside-ask { --end: calc(min(440px, 100vw - 2 * var(--gutter)) + var(--gutter) * 2); transition: none; }
  .ic { flex-shrink: 0; color: var(--amber); }
  .txt { flex: 1; min-width: 0; }
  .title { font-weight: var(--w-med); color: var(--ink-1); }
  .sub { margin-top: 2px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .btn { flex-shrink: 0; }
</style>
