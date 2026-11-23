<!--
  The start-up error screen: why Codly could not start, "Try again", and something to attach to
  a bug report. When the database is the reason (`start_problem` in src-tauri/src/boot.rs) it
  also offers to start from the last backup the app offers (its message says why when it offers
  none), to show the data folder, and to start with no data. The app moves the database aside
  for these; it is never deleted.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { revealInFinder } from '../lib/state/actions';
  import { startRecovery as recovery } from '../lib/state/startRecovery.svelte';
  import { store } from '../lib/state/store.svelte';
  import { fullDate } from '../lib/util/format';
  import { bytes } from '../lib/util/storage';
  import DiagnosticsReport from './DiagnosticsReport.svelte';
  import EmptyState from './EmptyState.svelte';

  onMount(() => void recovery.load());

  const problem = $derived(recovery.problem);
  /** "Start with no data" asks once more before it runs. */
  let confirmEmpty = $state(false);
</script>

<EmptyState title="Codly couldn’t start." body={problem?.message ?? store.error ?? undefined}>
  {#if problem}
    {@const p = problem}
    {@const last = p.backups[0]}
    <div class="notes">
      {#if last}
        <p>The last backup is from {fullDate(last.created_at)} ({bytes(last.bytes)}). What you did in Codly after that is not in it.</p>
      {/if}
      {#if p.start_empty}
        <p>The database Codly cannot use is kept: it is moved aside in the data folder, not deleted. A start with no data moves its backups aside with it.</p>
      {/if}
      {#if recovery.failed}<p class="bad" role="alert">That did not work: {recovery.failed}</p>{/if}
    </div>
    {#if last}
      <button class="btn primary" disabled={recovery.busy} onclick={() => recovery.recover(last.path)}>Start from the last backup</button>
    {/if}
    <button class="btn" class:primary={!last} disabled={recovery.busy} onclick={() => store.retry()}>Try again</button>
    <button class="btn ghost" onclick={() => revealInFinder(p.database)}>Open the data folder</button>
    {#if p.start_empty}
      {#if confirmEmpty}
        <button class="btn danger" disabled={recovery.busy} onclick={() => recovery.recover(null)}>Yes, start with no data</button>
        <button class="btn ghost" onclick={() => (confirmEmpty = false)}>Cancel</button>
      {:else}
        <button class="btn ghost" disabled={recovery.busy} onclick={() => (confirmEmpty = true)}>Start with no data…</button>
      {/if}
    {/if}
  {:else}
    <button class="btn primary" onclick={() => store.retry()}>Try again</button>
  {/if}
  <!-- Something to attach to a bug report: it is built without the core that just failed. -->
  <DiagnosticsReport />
</EmptyState>

<style>
  /* A row of its own above the buttons. */
  .notes { flex: 0 0 100%; display: flex; flex-direction: column; align-items: center; gap: 6px; margin-bottom: 4px; }
  .notes p { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); max-width: 60ch; }
  .notes .bad { color: var(--bad); }
</style>
