<!--
  Settings → Safety: the watchdog's limits (orchestrator/watchdog.rs). Three selects, saved at
  once; Off turns a check off. The core reads them at its next look, so a change also counts
  for the agents that work now.
-->
<script lang="ts">
  import type { WatchdogSettings } from '../lib/gen/WatchdogSettings';
  import { updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { WATCHDOG_DETAIL, WATCHDOG_LIMITS, limitChoices, limitLabel, withLimit } from '../lib/util/watchdog';
  import MoreAbout from './MoreAbout.svelte';

  const w = $derived(store.settings.watchdog);
  const set = (key: keyof WatchdogSettings, secs: number) => updateSettings({ watchdog: withLimit(w, key, secs) });
</script>

<section>
  <h3 class="eyebrow">Silent agents</h3>
  <p class="desc">
    No agent sits “running” without end: Codly looks at every working agent four times a minute. An agent that waits for your answer
    to an approval or a question is not silent.
  </p>
  <MoreAbout id="watchdog">
    <p class="desc">Each step shows on the agent’s timeline, and as a notification when “Agent stalled or idle” is on (General).</p>
  </MoreAbout>
  {#each WATCHDOG_LIMITS as l (l.key)}
    <div class="row">
      <div class="r-text">
        <span class="r-title">{l.title}</span><span class="desc">{l.body}</span>
        {#if l.more}<MoreAbout id="watchdog-{l.key}"><span class="desc">{l.more}</span></MoreAbout>{/if}
      </div>
      <select class="field" value={String(w[l.key])} aria-label={l.title} onchange={(e) => set(l.key, Number(e.currentTarget.value))}>
        {#each limitChoices(l.choices, w[l.key]) as secs (secs)}<option value={String(secs)}>{limitLabel(secs)}</option>{/each}
      </select>
    </div>
  {/each}
  <MoreAbout id="watchdog-steps"><p class="desc">{WATCHDOG_DETAIL}</p></MoreAbout>
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .field { width: 180px; font-size: var(--t-sm); }
</style>
