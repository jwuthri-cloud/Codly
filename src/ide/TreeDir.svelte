<!-- One folder of the explorer, loaded when first opened; folders nest by rendering themselves. -->
<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { ide } from './ide.svelte';
  import TreeDir from './TreeDir.svelte';

  interface Props {
    path: string;
    depth?: number;
  }
  let { path, depth = 0 }: Props = $props();
  const entries = $derived(ide.dirs[path]);
  const KIND_LABEL: Record<string, string> = { modified: 'M', added: 'A', deleted: 'D', renamed: 'R', untracked: 'U' };
</script>

{#if !entries}
  <p class="hint" style:padding-left="{12 + depth * 14}px">Loading…</p>
{:else if !entries.length}
  <p class="hint" style:padding-left="{12 + depth * 14}px">Empty folder</p>
{:else}
  {#each entries as e (e.path)}
    {@const kind = e.is_dir ? undefined : ide.changeOf.get(e.path)}
    <button
      class="row"
      class:active={!e.is_dir && ide.active === `file:${e.path}`}
      style:padding-left="{8 + depth * 14}px"
      aria-expanded={e.is_dir ? !!ide.expanded[e.path] : undefined}
      onclick={() => (e.is_dir ? ide.toggleDir(e.path) : ide.openFile(e.path))}
    >
      <span class="chev" class:open={ide.expanded[e.path]} class:none={!e.is_dir}><Icon name="chevron-right" size={11} stroke={2} /></span>
      <Icon name={e.is_dir ? 'folder' : 'file'} size={13} />
      <span class="name truncate">{e.name}</span>
      {#if kind}<span class="mark k-{kind}" data-tip={kind}>{KIND_LABEL[kind]}</span>{/if}
    </button>
    {#if e.is_dir && ide.expanded[e.path]}<TreeDir path={e.path} depth={depth + 1} />{/if}
  {/each}
{/if}

<style>
  .row { display: flex; align-items: center; gap: 6px; width: 100%; height: 26px; padding-right: 10px; border-radius: var(--r-sm); font-size: var(--t-md); color: var(--ink-2); text-align: left; }
  .row:hover { background: var(--surface-2); color: var(--ink-1); }
  .row.active { background: var(--surface-3); color: var(--ink-1); }
  .chev { display: inline-flex; width: 11px; color: var(--ink-3); transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .chev.none { visibility: hidden; }
  .name { flex: 1; }
  .mark { font-size: var(--t-xs); font-weight: var(--w-semi); font-family: var(--font-mono); }
  .k-modified, .k-renamed { color: var(--amber-ink); }
  .k-added, .k-untracked { color: var(--ok); }
  .k-deleted { color: var(--bad); }
  .hint { font-size: var(--t-sm); color: var(--ink-3); height: 24px; line-height: 24px; }
</style>
