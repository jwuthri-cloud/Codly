<!--
  "Collect diagnostics", then "Copy diagnostics": the text a bug report needs, built by the app
  (src-tauri/src/diagnostics.rs). Two clicks on purpose: collecting asks every agent CLI for its
  version, which takes a few seconds, and the clipboard only takes text during a click.
  `preview` shows the report, so you can read what you share.
-->
<script lang="ts">
  import { api, type Diagnostics } from '../lib/api';
  import { copyText, revealInFinder } from '../lib/state/actions';
  import { toast } from '../lib/state/toasts.svelte';
  import Icon from './Icon.svelte';

  let { preview = false }: { preview?: boolean } = $props();

  let info = $state.raw<Diagnostics | null>(null);
  let busy = $state(false);

  async function collect() {
    busy = true;
    try {
      info = await api.diagnostics();
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t collect diagnostics', body: e instanceof Error ? e.message : String(e) });
    } finally {
      busy = false;
    }
  }
</script>

<div class="diag">
  <div class="buttons">
    {#if info}
      {@const d = info}
      <button class="btn primary" class:sm={preview} onclick={() => copyText(d.report, 'Diagnostics copied')}><Icon name="copy" size={12} />Copy diagnostics</button>
      {#if d.log_file}
        {@const file = d.log_file}
        <button class="btn ghost" class:sm={preview} onclick={() => revealInFinder(file)}><Icon name="folder" size={12} />Show log file</button>
      {/if}
      <button class="btn ghost" class:sm={preview} disabled={busy} onclick={collect}>{busy ? 'Collecting…' : 'Collect again'}</button>
    {:else}
      <button class="btn" class:sm={preview} disabled={busy} onclick={collect}>{busy ? 'Collecting…' : 'Collect diagnostics'}</button>
    {/if}
  </div>
  {#if info && preview}<pre class="report selectable">{info.report}</pre>{/if}
</div>

<style>
  .diag { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .buttons { display: flex; flex-wrap: wrap; gap: 8px; }
  .report { margin: 0; max-height: 240px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); font-family: var(--font-mono); font-size: var(--t-xs); line-height: var(--lh-xs); color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; text-align: left; }
</style>
