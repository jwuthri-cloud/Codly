<!--
  Settings → Storage: what Codly keeps on your disk and in your repositories, and what may go.
  Run logs have a time limit. Worktrees and codly/… branches of finished, merged work are
  cleaned up by the core: a day after the plan ended, or at once with "Clean up now". The list
  is loaded when you ask for it, because it measures every worktree. A delete that loses work
  asks first and names what is lost; the core refuses it without that confirm. The list also
  shows the backups of Codly's own database (the core makes and removes them).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { StoragePolicy } from '../lib/gen/StoragePolicy';
  import type { StorageReport } from '../lib/gen/StorageReport';
  import type { StoredBranch } from '../lib/gen/StoredBranch';
  import type { StoredWorktree } from '../lib/gen/StoredWorktree';
  import { Loader } from '../lib/state/loader.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ago, plural } from '../lib/util/format';
  import { backupText, branchLossText, bytes, cleanupText, lossText, stateText } from '../lib/util/storage';
  import Icon from '../components/Icon.svelte';
  import LoadError from '../components/LoadError.svelte';

  const policy = new Loader<StoragePolicy>(() => api.storagePolicy());
  const report = new Loader<StorageReport>(() => api.storageReport());
  const p = $derived(policy.value);
  const r = $derived(report.value);
  onMount(() => void policy.load());

  /** How long run logs can be kept; 0 = for ever. A saved value that is not here is shown too. */
  const DAYS = [7, 14, 30, 90, 365, 0];
  const dayChoices = $derived(p && !DAYS.includes(p.log_days) ? [p.log_days, ...DAYS] : DAYS);
  const daysLabel = (d: number) => (d === 0 ? 'For ever' : d === 365 ? '1 year' : `${d} days`);

  let busy = $state(false);
  /** The row whose delete waits for your confirm: `<office id>:<worktree path or branch name>`. */
  let confirming = $state<string | null>(null);
  let withBranch = $state(false);

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function save(next: StoragePolicy) {
    // The value on screen changes only once the core has saved it.
    try {
      policy.set(await api.storagePolicy(next));
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t save the storage rule', body: message(e) });
    }
  }

  /** Run one change, say when it failed, and read the list again either way. */
  async function change(what: string, f: () => Promise<string>) {
    busy = true;
    try {
      toast({ kind: 'success', title: await f() });
    } catch (e) {
      toast({ kind: 'error', title: `Couldn’t ${what}`, body: message(e) });
    } finally {
      busy = false;
      confirming = null;
      withBranch = false;
      if (r) void report.load();
    }
  }

  async function cleanNow() {
    busy = true;
    try {
      const c = await api.storageClean();
      toast({ kind: c.problems.length ? 'error' : 'success', title: cleanupText(c), body: c.problems.join('\n') || undefined });
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t clean up', body: message(e) });
    } finally {
      busy = false;
      if (r) void report.load();
    }
  }

  function ask(key: string) {
    confirming = key;
    withBranch = false;
  }

  /** Delete a worktree. One with nothing to lose goes at once; any other asks first. */
  function removeWorktree(officeId: string, w: StoredWorktree, confirmed: boolean) {
    if (w.state !== 'merged' && !confirmed) return ask(`${officeId}:${w.path}`);
    const branchToo = w.state === 'merged' || withBranch;
    void change('delete the worktree', async () => {
      await api.storageDeleteWorktree(officeId, w.path, w.head, confirmed, branchToo);
      return branchToo && w.branch ? `Deleted the worktree and ${w.branch}` : 'Deleted the worktree';
    });
  }

  function removeBranch(officeId: string, b: StoredBranch, confirmed: boolean) {
    if (b.unmerged_commits > 0 && !confirmed) return ask(`${officeId}:${b.name}`);
    void change('delete the branch', async () => {
      await api.storageDeleteBranch(officeId, b.name, b.head, confirmed);
      return `Deleted ${b.name}`;
    });
  }

  /** The last two parts of a worktree's path: "<plan>/<task>". */
  const shortPath = (path: string) => path.split('/').slice(-2).join('/');
</script>

<section>
  <h3 class="eyebrow">Storage</h3>
  <p class="desc">What Codly keeps on your disk and in your repositories. Work that exists only there is never deleted unless you confirm it.</p>

  {#if p}
    <div class="row">
      <div class="r-text">
        <span class="r-title">Keep agent run logs for</span>
        <span class="desc">The raw output of each agent run. It can hold file contents and command output. Older logs are deleted; the conversation stays.</span>
      </div>
      <select class="input sel" value={p.log_days} aria-label="Keep agent run logs for" onchange={(e) => save({ ...p, log_days: +e.currentTarget.value })}>
        {#each dayChoices as d (d)}<option value={d}>{daysLabel(d)}</option>{/each}
      </select>
    </div>
    <label class="row toggle">
      <div class="r-text">
        <span class="r-title">Clean up after finished plans</span>
        <span class="desc">A day after a plan is merged, cancelled or failed, Codly removes its worktrees and <code>codly/…</code> branches that are clean and merged. Uncommitted or unmerged work always stays.</span>
      </div>
      <input type="checkbox" class="switch" checked={p.clean_merged} onchange={(e) => save({ ...p, clean_merged: e.currentTarget.checked })} />
    </label>
  {:else if policy.error}
    <LoadError what="the storage rules" error={policy.error} loading={policy.loading} onretry={() => policy.load()} />
  {:else}
    <p class="desc">Loading…</p>
  {/if}

  <div class="row">
    <div class="r-text">
      <span class="r-title">What Codly keeps</span>
      <span class="desc">
        {#if r}
          Run logs: {plural(r.logs.items, 'run')}, {bytes(r.logs.bytes)}. Copies of files agents made: {plural(r.artifacts.items, 'file')}, {bytes(r.artifacts.bytes)}.
        {:else}
          Worktrees, branches and run logs, with their size. Measuring them can take a moment.
        {/if}
      </span>
    </div>
    <button class="btn sm" disabled={report.loading} onclick={() => report.load()}>
      {#if r}<Icon name="refresh" size={13} />{/if}{report.loading ? 'Measuring…' : r ? 'Refresh' : 'Show'}
    </button>
    <button class="btn sm" disabled={busy} title="Removes merged, clean worktrees and merged branches of finished work, and run logs past their limit, now" onclick={cleanNow}>Clean up now</button>
  </div>
  {#if report.error}
    <LoadError what="the storage list" error={report.error} loading={report.loading} onretry={() => report.load()} />
  {/if}

  {#if r}
    <div class="office">
      <div class="o-head"><span class="o-name">Backups of Codly’s data</span></div>
      <p class="desc">Codly copies its database once a day and before an update changes it, and keeps the last few of each. If the database is ever damaged, Codly offers to start from the newest copy.</p>
      {#if r.backups.length}
        <ul class="items" aria-label="Backups of Codly’s data">
          {#each r.backups as b (b.path)}
            <li>
              <div class="i-top">
                <span class="i-name" title={b.path}>{backupText(b)}</span>
                <span class="grow"></span>
                <span class="muted small num">{bytes(b.bytes)}</span>
              </div>
              <span class="muted small">made {ago(b.created_at)}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="desc">No backup yet. The first one is made a moment after Codly starts.</p>
      {/if}
    </div>
    {#each r.offices as o (o.office_id)}
      <div class="office">
        <div class="o-head"><span class="o-name">{o.name}</span><span class="muted small path">{o.repo_path}</span></div>
        {#if o.problem}
          <p class="desc warn">Couldn’t read this repository: {o.problem}</p>
        {:else if !o.worktrees.length && !o.branches.length}
          <p class="desc">Nothing of Codly’s is left here.</p>
        {/if}

        {#if o.worktrees.length}
          <ul class="items" aria-label={`Worktrees of ${o.name}`}>
            {#each o.worktrees as w (w.path)}
              {@const key = `${o.office_id}:${w.path}`}
              <li>
                <div class="i-top">
                  <span class="i-name" title={w.path}>{w.owner ?? shortPath(w.path)}</span>
                  <span class="badge" class:ok={w.state === 'merged'} class:bad={w.state === 'dirty'} class:amber={w.state === 'unmerged'}>{stateText(w)}</span>
                  <span class="grow"></span>
                  <span class="muted small num">{bytes(w.size_bytes)}</span>
                  {#if confirming !== key}
                    <button class="btn sm ghost rm" disabled={busy || !!w.locked} title={w.locked ? `It stays for now: ${w.locked}` : undefined} onclick={() => removeWorktree(o.office_id, w, false)}>
                      <Icon name="trash" size={13} />{w.state === 'merged' ? 'Delete' : 'Delete…'}
                    </button>
                  {/if}
                </div>
                <span class="muted small">
                  {#if w.branch}<code class="mono">{w.branch}</code>{:else}on no branch{/if} · last used {ago(w.last_used_at)}{#if w.locked} · stays for now: {w.locked}{/if}
                </span>
                {#if confirming === key}
                  <div class="confirm" role="group" aria-label="Confirm the delete">
                    <span class="small">{lossText(w, withBranch)}</span>
                    {#if w.branch}
                      <label class="small tick"><input type="checkbox" bind:checked={withBranch} /> Delete the branch <code class="mono">{w.branch}</code> too</label>
                    {/if}
                    <div class="c-actions">
                      <button class="btn sm danger" disabled={busy} onclick={() => removeWorktree(o.office_id, w, true)}>Delete the worktree</button>
                      <button class="btn sm ghost" onclick={() => (confirming = null)}>Keep</button>
                    </div>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        {#if o.branches.length}
          <span class="muted small">Branches without a worktree</span>
          <ul class="items" aria-label={`Codly branches of ${o.name} without a worktree`}>
            {#each o.branches as b (b.name)}
              {@const key = `${o.office_id}:${b.name}`}
              <li>
                <div class="i-top">
                  <code class="mono i-name">{b.name}</code>
                  <span class="badge" class:ok={b.unmerged_commits === 0} class:amber={b.unmerged_commits > 0}>{b.unmerged_commits === 0 ? 'Merged' : `${plural(b.unmerged_commits, 'commit')} not merged`}</span>
                  <span class="grow"></span>
                  {#if confirming === key}
                    <span class="small warn">{branchLossText(b)}</span>
                    <button class="btn sm danger" disabled={busy} onclick={() => removeBranch(o.office_id, b, true)}>Delete</button>
                    <button class="btn sm ghost" onclick={() => (confirming = null)}>Keep</button>
                  {:else}
                    <button class="btn sm ghost rm" disabled={busy || !!b.locked} title={b.locked ? `It stays for now: ${b.locked}` : undefined} onclick={() => removeBranch(o.office_id, b, false)}>
                      <Icon name="trash" size={13} />{b.unmerged_commits === 0 ? 'Delete' : 'Delete…'}
                    </button>
                  {/if}
                </div>
                <span class="muted small">{b.owner ?? 'No record of what it was for'} · last commit {ago(b.last_commit_at)}{#if b.locked} · stays for now: {b.locked}{/if}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
    <p class="desc">Sizes count every file in full; files that macOS shares between copies take less space than shown.</p>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .muted { color: var(--ink-3); }
  .small { font-size: var(--t-sm); }
  .warn { color: var(--amber-ink); }
  .grow { flex: 1; }
  .mono { font-family: var(--font-mono); }
  .row { display: flex; align-items: center; gap: 8px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; margin-right: 8px; }
  .r-title { font-weight: var(--w-med); }
  .sel { width: 120px; }
  .office { display: flex; flex-direction: column; gap: 8px; }
  .o-head { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  .o-name { font-weight: var(--w-semi); }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .items { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .items li { display: flex; flex-direction: column; gap: 4px; padding: 10px 14px; }
  .items li + li { box-shadow: inset 0 1px 0 var(--line); }
  .i-top { display: flex; align-items: center; gap: 8px; min-height: 26px; }
  .i-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--w-med); }
  .badge { font-size: var(--t-xs); padding: 1px 7px; border-radius: var(--r-full); background: var(--surface-3); color: var(--ink-2); white-space: nowrap; }
  .badge.ok { background: var(--ok-soft); color: var(--ok); }
  .badge.bad { background: var(--bad-soft); color: var(--bad); }
  .badge.amber { color: var(--amber-ink); }
  .rm { color: var(--ink-3); flex-shrink: 0; }
  .rm:hover:not(:disabled) { color: var(--bad); background: var(--bad-soft); }
  .confirm { display: flex; flex-direction: column; gap: 8px; margin-top: 4px; padding: 10px 12px; border-radius: var(--r-sm); background: var(--surface-2); }
  .tick { display: flex; align-items: center; gap: 6px; }
  .c-actions { display: flex; gap: 8px; }
</style>
