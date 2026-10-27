<!-- Diff of one changed file against HEAD, drawn by the app's own diff viewer. -->
<script lang="ts">
  import DiffView from '../rooms/changes/DiffView.svelte';
  import Icon from '../components/Icon.svelte';
  import { api } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { parseDiff, type DiffFile } from '../lib/util/diff';
  import { idVar } from '../lib/util/labels';
  import { ide } from './ide.svelte';

  interface Props {
    path: string;
  }
  let { path }: Props = $props();

  let files = $state<DiffFile[] | null>(null);
  let error = $state<string | null>(null);
  let collapsed = $state(false);
  const agent = $derived(ide.agentId ? store.agentMap.get(ide.agentId) : undefined);
  const hue = $derived(agent ? idVar(agent.color) : 'var(--live)');

  async function load() {
    const id = ide.agentId;
    if (!id) return;
    try {
      files = parseDiff(await api.ideDiff(id, path));
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => {
    void path;
    files = null;
    void load();
  });
</script>

<div class="d">
  <div class="bar">
    <button class="btn sm ghost" onclick={() => ide.openFile(path)}><Icon name="file" size={13} />Open file</button>
    <button class="btn sm ghost" onclick={load}><Icon name="refresh" size={13} />Refresh</button>
  </div>
  <div class="body">
    {#if error}
      <p class="note bad">{error}</p>
    {:else if !files}
      <p class="note">Loading the diff…</p>
    {:else if !files.length}
      <p class="note">No changes in {path} since the last commit.</p>
    {:else}
      {#each files as f (f.path)}
        <DiffView file={f} {hue} {collapsed} ontoggle={() => (collapsed = !collapsed)} />
      {/each}
    {/if}
  </div>
</div>

<style>
  .d { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .bar { display: flex; gap: 6px; padding: 8px 12px; box-shadow: inset 0 -1px 0 var(--line); }
  .body { flex: 1; min-height: 0; overflow: auto; padding: 12px; }
  .note { padding: 24px 8px; color: var(--ink-3); font-size: var(--t-md); }
  .note.bad { color: var(--bad); }
</style>
