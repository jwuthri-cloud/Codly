<!--
  The budget mark in the top bar: there only while a used-up budget holds new work (no task
  starts). It says why, and a click opens Settings at the budgets, where you raise the budget or
  reset the counters.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { budget } from '../lib/state/budget.svelte';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';

  onMount(() => budget.start());
  // A snapshot (first connect, another office): read what holds now. The budgets are one for all offices.
  $effect(() => {
    void store.epoch;
    void budget.refresh();
  });
</script>

{#if budget.hold}
  <button class="mark" onclick={() => budget.openSettings()} aria-label="Budget reached: no new task starts. Open the budgets in Settings" data-tip="Budget reached. No new task starts: {budget.hold}. Click to raise the budget or reset the counters.">
    <Icon name="gauge" size={15} />
  </button>
{/if}

<style>
  .mark { display: inline-grid; place-items: center; width: 34px; height: 34px; flex: none; border-radius: var(--r-full); color: var(--amber-ink); background: color-mix(in srgb, var(--amber) 16%, var(--glass)); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--amber) 45%, transparent); }
  .mark:hover { color: var(--ink-1); }
</style>
