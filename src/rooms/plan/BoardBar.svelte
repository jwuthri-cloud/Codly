<!--
  Above the Plan room's list: the "Archived" filter and what the board wants a second look at
  (tasks untouched for weeks, questions that waited too long). The core decides both; this
  sweeps when it opens, then every ten minutes while it is open.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { housekeeping as hk } from '../../lib/state/housekeeping.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import Icon from '../../components/Icon.svelte';

  const wsId = $derived(store.workspace?.id ?? null);
  // A new workspace, a new board.
  $effect(() => {
    if (wsId) void hk.sweep();
  });
  onMount(() => {
    const t = setInterval(() => void hk.sweep(), 10 * 60_000);
    return () => clearInterval(t);
  });

  const archivedCount = $derived(hk.archivedIds.size);
  const flags = $derived(hk.flags);
  const label = (f: (typeof flags)[number]) => (f.reason === 'question_waiting' ? `Waiting ${f.days} days for your answer` : `Untouched for ${f.days} days — archives soon`);
  function open(f: (typeof flags)[number]) {
    if (f.kind === 'task') ui.selectTask(f.id);
  }
</script>

<div class="bar">
  {#if flags.length && !hk.showArchived}
    <ul class="flags" aria-label="Needs a look">
      {#each flags as f (f.id)}
        <li>
          <button class="flag" class:q={f.kind === 'question'} onclick={() => open(f)} disabled={f.kind !== 'task'} data-tip={label(f)}>
            <Icon name={f.kind === 'question' ? 'question' : 'keep'} size={13} />
            <span class="t truncate">{f.title}</span>
            <span class="d">{label(f)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="filters" role="radiogroup" aria-label="Board filter">
    <button role="radio" aria-checked={!hk.showArchived} onclick={() => hk.setArchived(false)}>Board</button>
    <button role="radio" aria-checked={hk.showArchived} onclick={() => hk.setArchived(true)} data-tip="Done tasks leave the board after a couple of days; their text stays searchable here">
      Archived{#if archivedCount}<span class="n num">{archivedCount}</span>{/if}
    </button>
  </div>
</div>

<style>
  .bar { display: flex; flex-direction: column; gap: 10px; max-width: 860px; margin: 0 auto 14px; width: 100%; }
  .flags { display: flex; flex-direction: column; gap: 4px; }
  .flag { display: flex; align-items: center; gap: 8px; width: 100%; padding: 7px 14px; border-radius: var(--r-md); text-align: left; font-size: var(--t-sm); color: var(--ink-2); background: color-mix(in srgb, var(--amber) 8%, var(--surface-1)); }
  .flag:hover:not(:disabled) { background: color-mix(in srgb, var(--amber) 14%, var(--surface-1)); }
  .flag :global(svg) { color: var(--amber-ink); flex-shrink: 0; }
  .flag.q { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 40%, transparent); }
  .t { color: var(--ink-1); font-weight: var(--w-med); min-width: 0; }
  .d { margin-left: auto; color: var(--amber-ink); white-space: nowrap; }
  .filters { display: inline-flex; align-self: flex-start; gap: 2px; padding: 2px; border-radius: var(--r-full); background: var(--surface-1); }
  .filters button { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-3); }
  .filters button:hover { color: var(--ink-1); }
  .filters button[aria-checked='true'] { background: var(--surface-3); color: var(--ink-1); }
  .n { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
