<!--
  The board's "Archived" filter: tasks that left the board, with their text, searchable.
  The text comes from the archive file the core keeps, so it is still here after the card is gone.
-->
<script lang="ts">
  import { housekeeping as hk } from '../../lib/state/housekeeping.svelte';
  import { ago } from '../../lib/util/format';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';

  let timer: ReturnType<typeof setTimeout> | undefined;
  function typing(e: Event) {
    hk.query = (e.currentTarget as HTMLInputElement).value;
    clearTimeout(timer);
    timer = setTimeout(() => void hk.search(), 160);
  }
  const WHY = { done: 'Finished', stale: 'Untouched for weeks' } as const;
</script>

<div class="arch">
  <label class="find">
    <Icon name="search" size={14} />
    <input type="search" placeholder="Search archived tasks" value={hk.query} oninput={typing} aria-label="Search archived tasks" spellcheck="false" autocomplete="off" />
  </label>
  {#if !hk.results.length}
    <EmptyState compact title={hk.query.trim() ? 'No archived task matches.' : 'Nothing archived yet.'} body={hk.query.trim() ? 'Search looks through titles, goals, acceptance criteria and results.' : 'Finished tasks move here after a couple of days; untouched ones after three weeks.'} />
  {:else}
    <ol>
      {#each hk.results as it (it.id)}
        <li>
          <header>
            <span class="key num">{it.key}</span>
            <h3 class="truncate">{it.title}</h3>
            <span class="why">{WHY[it.reason]} · archived {ago(it.archived_at)}</span>
          </header>
          {#if it.text}<p>{it.text}</p>{/if}
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .arch { display: flex; flex-direction: column; gap: 12px; }
  .find { display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 12px; border-radius: var(--r-md); background: var(--surface-1); color: var(--ink-3); box-shadow: inset 0 0 0 1px var(--line); }
  .find:focus-within { box-shadow: inset 0 0 0 1px var(--ink-3); }
  input { flex: 1; min-width: 0; background: transparent; color: var(--ink-1); font: inherit; font-size: var(--t-sm); outline: none; }
  ol { display: flex; flex-direction: column; gap: 4px; }
  li { display: flex; flex-direction: column; gap: 4px; padding: 10px 14px; border-radius: var(--r-md); background: var(--surface-1); }
  header { display: flex; align-items: baseline; gap: 8px; }
  .key { font-size: var(--t-sm); color: var(--ink-3); }
  h3 { font-size: var(--t-md); font-weight: var(--w-med); color: var(--ink-1); min-width: 0; }
  .why { margin-left: auto; font-size: var(--t-sm); color: var(--ink-4); white-space: nowrap; }
  p { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); white-space: pre-line; }
</style>
