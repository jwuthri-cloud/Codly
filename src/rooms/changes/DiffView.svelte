<!-- Unified diff for one file: hunk headers, old/new line numbers, +/− gutters, and the hue of
     the agent that wrote it along the edge. Dense on purpose: this is where scanning matters. -->
<script lang="ts">
  import type { DiffFile } from '../../lib/util/diff';
  import { ui } from '../../lib/state/ui.svelte';
  import Icon from '../../components/Icon.svelte';

  interface Props {
    file: DiffFile;
    hue: string;
    who?: string;
    collapsed?: boolean;
    ontoggle?: () => void;
  }
  let { file, hue, who, collapsed = false, ontoggle }: Props = $props();
  const STATUS: Record<string, string> = { added: 'New', deleted: 'Deleted', modified: '', renamed: 'Renamed', binary: 'Binary' };
  const dir = $derived(file.path.includes('/') ? file.path.slice(0, file.path.lastIndexOf('/') + 1) : '');
  const base = $derived(file.path.slice(dir.length));
</script>

<section class="file" id="diff-{file.path}" role="group" aria-label={file.path} style:--hue={hue} onpointerenter={() => ui.setHover({ path: file.path })} onpointerleave={() => ui.setHover(null)}>
  <header>
    <button class="fh" onclick={ontoggle} aria-expanded={!collapsed}>
      <span class="chev" class:open={!collapsed}><Icon name="chevron-right" size={12} stroke={2} /></span>
      <span class="path truncate"><span class="dir">{dir}</span>{base}</span>
    </button>
    {#if STATUS[file.status]}<span class="status s-{file.status}">{STATUS[file.status]}</span>{/if}
    {#if who}<span class="who">{who}</span>{/if}
    <span class="stat num"><span class="add">+{file.added}</span><span class="del">−{file.removed}</span></span>
  </header>
  {#if !collapsed}
    {#if file.status === 'binary'}
      <p class="binary">Binary file, not shown.</p>
    {:else}
      <div class="code selectable" role="table" aria-label="Diff of {file.path}">
        {#each file.hunks as h, hi (hi)}
          <div class="hunk" role="row"><span>{h.header}</span></div>
          {#each h.lines as l, li (li)}
            <div class="ln k-{l.kind}" role="row">
              <span class="no">{l.oldNo ?? ''}</span>
              <span class="no">{l.newNo ?? ''}</span>
              <span class="sign">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>
              <span class="txt">{l.text || ' '}</span>
            </div>
          {/each}
        {/each}
      </div>
    {/if}
  {/if}
</section>

<style>
  .file { flex-shrink: 0; border-radius: var(--r-md); background: var(--surface-1); overflow: clip; box-shadow: inset 3px 0 0 var(--hue); }
  header { position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 10px; height: 38px; padding: 0 14px 0 8px; background: var(--glass-solid); box-shadow: inset 3px 0 0 var(--hue), inset 0 -1px 0 var(--line); }
  .fh { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; height: 30px; padding: 0 6px; border-radius: var(--r-sm); text-align: left; }
  .chev { color: var(--ink-3); transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .path { font-family: var(--font-mono); font-size: 12px; color: var(--ink-1); }
  .dir { color: var(--ink-3); }
  .status { font-size: var(--t-xs); color: var(--ink-3); }
  .s-added { color: var(--ok); }
  .s-deleted { color: var(--bad); }
  .who { font-size: var(--t-xs); color: var(--hue); }
  .stat { display: inline-flex; gap: 6px; font-size: var(--t-sm); font-family: var(--font-mono); }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .code { font-family: var(--font-mono); font-size: var(--mono); line-height: var(--lh-mono); overflow-x: auto; padding-bottom: 4px; font-variant-ligatures: none; font-feature-settings: 'calt' 0, 'liga' 0; }
  .hunk { padding: 4px 14px; background: var(--surface-1); color: var(--ink-3); white-space: pre; font-size: 11.5px; }
  .ln { display: grid; grid-template-columns: 46px 46px 20px 1fr; min-width: max-content; }
  .no { padding: 0 8px; text-align: right; color: var(--ink-4); user-select: none; -webkit-user-select: none; font-variant-numeric: tabular-nums; }
  .sign { text-align: center; color: var(--ink-3); user-select: none; -webkit-user-select: none; }
  .txt { white-space: pre; padding-right: 20px; color: var(--ink-1); }
  .k-ctx .txt { color: var(--ink-2); }
  .k-add { background: var(--diff-add); }
  .k-add .no { background: var(--diff-add-strong); color: var(--ok); }
  .k-add .sign { color: var(--ok); }
  .k-del { background: var(--diff-del); }
  .k-del .no { background: var(--diff-del-strong); color: var(--bad); }
  .k-del .sign { color: var(--bad); }
  .k-meta .txt { color: var(--ink-3); font-style: italic; }
  .binary { padding: 14px; color: var(--ink-3); font-size: var(--t-sm); }
</style>
