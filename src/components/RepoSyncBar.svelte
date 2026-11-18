<!--
  Your checkout against its remote, and the two git steps Codly can do for you there: commit
  your own uncommitted changes (they block a merge that touches the same files), and push your
  branch once merges have landed on it. Codly never pushes on its own.
-->
<script lang="ts">
  import { api } from '../lib/api';
  import type { RepoSync } from '../lib/gen/RepoSync';
  import { commitCheckout, pushCheckout } from '../lib/state/actions';
  import Icon from './Icon.svelte';

  interface Props {
    /** Changes when something may have moved the checkout (a merge): the bar re-reads it. */
    refresh?: unknown;
  }
  let { refresh = null }: Props = $props();

  let sync = $state<RepoSync | null>(null);
  let busy = $state(false);
  let committing = $state(false);
  let message = $state('');

  async function load() {
    try {
      sync = await api.repoSync();
    } catch {
      sync = null; // no repository open, or git failed: the bar just stays hidden
    }
  }
  $effect(() => {
    void refresh;
    void load();
  });

  const remote = $derived(sync?.upstream ?? `origin/${sync?.branch ?? ''}`);
  const toPush = $derived(!!sync?.branch && (sync.ahead > 0 || !sync.upstream));

  async function commit() {
    if (!message.trim()) return;
    busy = true;
    if (await commitCheckout(message)) {
      message = '';
      committing = false;
    }
    await load();
    busy = false;
  }

  async function push() {
    busy = true;
    const s = await pushCheckout();
    if (s) sync = s;
    else await load();
    busy = false;
  }
</script>

{#if sync && (sync.dirty.length || toPush || sync.behind)}
  <div class="sync">
    {#if sync.dirty.length}
      <div class="row">
        <span class="what" data-tip={sync.dirty.join('\n')}><Icon name="file" size={12} />{sync.dirty.length} uncommitted {sync.dirty.length === 1 ? 'file' : 'files'} in your checkout</span>
        {#if !committing}<button class="btn sm" disabled={busy} onclick={() => (committing = true)}>Commit…</button>{/if}
      </div>
      {#if committing}
        <form class="row" onsubmit={(e) => { e.preventDefault(); void commit(); }}>
          <input class="field" bind:value={message} placeholder="Commit message" aria-label="Commit message" disabled={busy} />
          <button class="btn sm primary" type="submit" disabled={busy || !message.trim()}>Commit</button>
          <button class="btn sm ghost" type="button" disabled={busy} onclick={() => (committing = false)}>Cancel</button>
        </form>
      {/if}
    {/if}
    {#if sync.behind}
      <p class="note">{remote} has {sync.behind} new {sync.behind === 1 ? 'commit' : 'commits'}: pull before you push.</p>
    {/if}
    {#if toPush}
      <div class="row">
        <span class="what"><Icon name="branch" size={12} />{sync.upstream ? `${sync.ahead} ${sync.ahead === 1 ? 'commit' : 'commits'} not on ${remote} yet` : `${sync.branch} is not on the remote yet`}</span>
        <button class="btn sm primary" disabled={busy} onclick={push}><Icon name="arrow-right" size={12} />Push to {remote}</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .sync { display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .what { flex: 1; min-width: 0; display: inline-flex; align-items: center; gap: 6px; font-size: var(--t-sm); color: var(--ink-2); }
  .field { flex: 1; min-width: 0; font-size: var(--t-sm); }
  .note { font-size: var(--t-sm); color: var(--amber-ink, var(--ink-2)); }
</style>
