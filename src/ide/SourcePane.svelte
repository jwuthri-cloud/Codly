<!-- Source rail: files with uncommitted changes in this worktree; a click opens the diff. -->
<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { basename, dirname } from '../lib/util/sanitize';
  import { ide } from './ide.svelte';

  const LABEL: Record<string, string> = { modified: 'Modified', added: 'Added', deleted: 'Deleted', renamed: 'Renamed', untracked: 'New' };
  const LETTER: Record<string, string> = { modified: 'M', added: 'A', deleted: 'D', renamed: 'R', untracked: 'U' };
</script>

<div class="pane">
  <div class="bar">
    <span class="eyebrow">{ide.changes.length} changed</span>
    <button class="icon-btn sm" aria-label="Refresh" data-tip="Refresh" onclick={() => ide.refreshChanges()}><Icon name="refresh" size={13} /></button>
  </div>
  {#if !ide.changes.length}
    <p class="hint">Nothing changed since the last commit in this worktree.</p>
  {:else}
    <div class="list">
      {#each ide.changes as c (c.path)}
        <button class="row" class:active={ide.active === `diff:${c.path}`} onclick={() => ide.openDiff(c.path)} data-tip={LABEL[c.kind]}>
          <span class="name truncate">{basename(c.path)}</span>
          <span class="dir truncate">{dirname(c.path)}</span>
          <span class="mark k-{c.kind}">{LETTER[c.kind]}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .pane { display: flex; flex-direction: column; min-height: 0; flex: 1; padding: 0 8px 8px; }
  .bar { display: flex; align-items: center; justify-content: space-between; padding: 0 4px 6px; }
  .list { overflow: auto; display: flex; flex-direction: column; }
  .row { display: flex; align-items: baseline; gap: 8px; width: 100%; height: 26px; padding: 0 8px; border-radius: var(--r-sm); font-size: var(--t-md); color: var(--ink-2); text-align: left; }
  .row:hover { background: var(--surface-2); color: var(--ink-1); }
  .row.active { background: var(--surface-3); color: var(--ink-1); }
  .name { flex: 0 1 auto; }
  .dir { flex: 1; font-size: var(--t-sm); color: var(--ink-3); }
  .mark { font-family: var(--font-mono); font-size: var(--t-xs); font-weight: var(--w-semi); }
  .k-modified, .k-renamed { color: var(--amber-ink); }
  .k-added, .k-untracked { color: var(--ok); }
  .k-deleted { color: var(--bad); }
  .hint { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); padding: 8px 4px; }
</style>
