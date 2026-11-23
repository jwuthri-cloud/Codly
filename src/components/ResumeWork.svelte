<!--
  Work was under way when Codly last closed, and nothing of it starts before you say so
  (Settings → Long runs → "Resume work when Codly opens"). With Ask this is the question:
  Resume, or Not now. After Not now, and with Never, it is one line with the Resume work
  button. It stays until you press it: the waiting tasks name this button.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { asksNow, resumeQuestion } from '../lib/util/resumeWork';
  import Icon from './Icon.svelte';

  const offer = $derived(resumeWork.offer);
  const asking = $derived(asksNow(store.settings.resume_on_launch, resumeWork.later));

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
  <div class="bar" role="region" aria-label="Resume work">
    <span class="ic"><Icon name="pause" size={15} /></span>
    <div class="txt" role="status">
      {#if asking}
        <p class="title">{resumeQuestion(offer)}</p>
        <p class="sub">Nothing starts until you answer.</p>
      {:else}
        <p class="title">Work is on hold since Codly opened.</p>
        <p class="sub">Nothing starts until you resume it.</p>
      {/if}
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
  .bar { position: fixed; top: calc(var(--top-h) + 4px); left: 50%; transform: translateX(-50%); z-index: var(--z-pop); display: flex; align-items: center; gap: 10px; width: max-content; max-width: min(620px, calc(100vw - 32px)); padding: 10px 12px 10px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); }
  .ic { color: var(--amber); }
  .txt { flex: 1; min-width: 0; }
  .title { font-weight: var(--w-med); color: var(--ink-1); }
  .sub { margin-top: 2px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .btn { flex-shrink: 0; }
</style>
