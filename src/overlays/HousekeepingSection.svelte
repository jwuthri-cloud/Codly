<!--
  Settings → Board: when cards leave the board. Days since a card was last touched; 0 turns a
  rule off. Archived text stays searchable under the board's "Archived" filter.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { ArchivePolicy } from '../lib/gen/ArchivePolicy';
  import { housekeeping } from '../lib/state/housekeeping.svelte';
  import { Loader } from '../lib/state/loader.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import Icon from '../components/Icon.svelte';
  import LoadError from '../components/LoadError.svelte';

  const policy = new Loader<ArchivePolicy>(() => api.boardPolicy());
  const p = $derived(policy.value);
  onMount(() => void policy.load());

  const ROWS: { key: keyof ArchivePolicy; title: string; body: string }[] = [
    { key: 'done_days', title: 'Archive finished tasks after', body: 'Days after a task is done (or cancelled).' },
    { key: 'stale_flag_days', title: 'Flag untouched tasks after', body: 'A task nobody moved for this long is marked for a second look.' },
    { key: 'stale_archive_days', title: 'Archive untouched tasks after', body: 'Never sooner than the flag. Running tasks are never archived.' },
    { key: 'question_flag_days', title: 'Flag unanswered questions after', body: 'Questions are flagged, never archived: someone has to answer.' },
  ];

  async function step(key: keyof ArchivePolicy, d: number) {
    if (!p) return;
    const next = { ...p, [key]: Math.max(0, Math.min(365, p[key] + d)) };
    // The number on screen changes only once the core has saved it.
    try {
      policy.set(await api.boardPolicy(next));
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t save the board rule', body: e instanceof Error ? e.message : String(e) });
      return;
    }
    void housekeeping.sweep();
  }
</script>

<section>
  <h3 class="eyebrow">Board</h3>
  {#if p}
    {#each ROWS as r (r.key)}
      <div class="row">
        <div class="r-text"><span class="r-title">{r.title}</span><span class="desc">{r.body}</span></div>
        <div class="stepper">
          <button class="icon-btn sm" aria-label="Fewer days" onclick={() => step(r.key, -1)} disabled={p[r.key] <= 0}><Icon name="minus" size={14} /></button>
          <span class="num">{p[r.key] === 0 ? 'Off' : `${p[r.key]} d`}</span>
          <button class="icon-btn sm" aria-label="More days" onclick={() => step(r.key, 1)}><Icon name="plus" size={14} /></button>
        </div>
      </div>
    {/each}
  {:else if policy.error}
    <LoadError what="the board rules" error={policy.error} loading={policy.loading} onretry={() => policy.load()} />
  {:else}
    <p class="desc">Loading…</p>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .stepper { display: inline-flex; align-items: center; gap: 4px; padding: 2px; border-radius: var(--r-full); background: var(--surface-1); }
  .stepper .num { min-width: 40px; text-align: center; font-weight: var(--w-semi); }
</style>
