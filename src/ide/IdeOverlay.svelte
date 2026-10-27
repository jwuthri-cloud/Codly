<!--
  The workspace IDE: a full-window overlay over one agent's worktree. Left rail = Explorer,
  Search, Source; main = tabs (Home, files, diffs). Esc closes it (tabs and unsaved edits are
  kept until you open another agent's IDE). Sits just under the toasts so errors stay visible.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import Kbd from '../components/Kbd.svelte';
  import { isTauriApp } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { basename } from '../lib/util/sanitize';
  import DiffTab from './DiffTab.svelte';
  import Editor from './Editor.svelte';
  import HomeTab from './HomeTab.svelte';
  import { ide, type Rail } from './ide.svelte';
  import SearchPane from './SearchPane.svelte';
  import SourcePane from './SourcePane.svelte';
  import TreeDir from './TreeDir.svelte';

  const RAILS: { id: Rail; label: string; icon: string }[] = [
    { id: 'explorer', label: 'Explorer', icon: 'folder' },
    { id: 'search', label: 'Search', icon: 'search' },
    { id: 'source', label: 'Source', icon: 'branch' },
  ];
  const name = $derived((ide.agentId ? store.agentMap.get(ide.agentId)?.name : null) ?? ide.info?.agent_name ?? 'Agent');
  let root: HTMLDivElement | undefined = $state();

  // Capture phase: the overlay owns Esc and ⌘S while it is open, before the app's own keys see them.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!ide.visible) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopImmediatePropagation();
        ide.close();
      } else if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 's') {
        e.preventDefault();
        e.stopImmediatePropagation();
        void ide.save(ide.active);
      }
    };
    window.addEventListener('keydown', onKey, true);
    root?.focus();
    return () => window.removeEventListener('keydown', onKey, true);
  });

  const tabTitle = (t: (typeof ide.tabs)[number]) => (t.kind === 'home' ? 'Home' : t.kind === 'diff' ? `${basename(t.path)} (diff)` : basename(t.path));
  function middleClose(e: MouseEvent, id: string) {
    if (e.button === 1) {
      e.preventDefault();
      ide.closeTab(id);
    }
  }
</script>

{#if ide.visible}
  <div class="ide" class:tauri={isTauriApp()} role="dialog" aria-modal="true" aria-label="{name}’s workspace" tabindex="-1" bind:this={root}>
    <header data-tauri-drag-region>
      <Icon name="code" size={15} />
      <span class="title serif">{name}’s workspace</span>
      {#if ide.info?.branch}<span class="branch"><Icon name="branch" size={12} />{ide.info.branch}</span>{/if}
      <span class="grow" data-tauri-drag-region></span>
      {#if ide.dirtyCount}<span class="unsaved">{ide.dirtyCount} unsaved</span>{/if}
      <Kbd keys="Esc" subtle />
      <button class="icon-btn" aria-label="Close the IDE" data-tip="Close" onclick={() => ide.close()}><Icon name="x" /></button>
    </header>

    <div class="body">
      <nav class="rail" aria-label="IDE views">
        {#each RAILS as r (r.id)}
          <button class="icon-btn" aria-pressed={ide.rail === r.id} aria-label={r.label} data-tip={r.label} onclick={() => (ide.rail = r.id)}>
            <Icon name={r.icon} size={17} />
            {#if r.id === 'source' && ide.changes.length}<span class="badge num">{ide.changes.length}</span>{/if}
          </button>
        {/each}
      </nav>

      <aside class="side" aria-label={RAILS.find((r) => r.id === ide.rail)?.label}>
        <h2 class="eyebrow">{RAILS.find((r) => r.id === ide.rail)?.label}</h2>
        {#if ide.rail === 'explorer'}
          <div class="tree"><TreeDir path="" /></div>
        {:else if ide.rail === 'search'}
          <SearchPane />
        {:else}
          <SourcePane />
        {/if}
      </aside>

      <main>
        <div class="tabs" role="tablist">
          {#each ide.tabs as t (t.id)}
            <div class="tab" class:active={ide.active === t.id} role="presentation" onauxclick={(e) => middleClose(e, t.id)}>
              <button role="tab" aria-selected={ide.active === t.id} class="t-main" onclick={() => (ide.active = t.id)}>
                {#if t.kind === 'home'}<Icon name="home" size={13} />{:else if t.kind === 'diff'}<Icon name="diff" size={13} />{/if}
                <span class="truncate">{tabTitle(t)}</span>
                {#if ide.isDirty(t)}<span class="dot" aria-label="unsaved"></span>{/if}
              </button>
              {#if t.kind !== 'home'}<button class="t-x" aria-label="Close {tabTitle(t)}" onclick={() => ide.closeTab(t.id)}><Icon name="x" size={11} stroke={2} /></button>{/if}
            </div>
          {/each}
        </div>
        <div class="pane" role="tabpanel">
          {#key ide.active}
            {@const t = ide.tab}
            {#if t.kind === 'home'}<HomeTab />
            {:else if t.kind === 'file'}<Editor tab={t} />
            {:else}<DiffTab path={t.path} />{/if}
          {/key}
        </div>
      </main>
    </div>
  </div>
{/if}

<style>
  .ide { position: fixed; inset: 0; z-index: calc(var(--z-toast) - 1); display: flex; flex-direction: column; background: var(--night); outline: none; animation: fade-in var(--d-3) var(--ease-out); }
  header { display: flex; align-items: center; gap: 10px; height: 46px; padding: 0 12px 0 16px; color: var(--ink-2); box-shadow: inset 0 -1px 0 var(--line); }
  .tauri header { padding-left: 84px; } /* clear of the macOS window buttons */
  .title { font-size: var(--d-sm); color: var(--ink-1); line-height: 1; }
  .branch { display: inline-flex; align-items: center; gap: 4px; font-family: var(--font-mono); font-size: var(--t-sm); color: var(--live); }
  .grow { flex: 1; align-self: stretch; }
  .unsaved { font-size: var(--t-sm); color: var(--amber-ink); }
  .body { flex: 1; min-height: 0; display: flex; }
  .rail { width: 48px; flex: none; display: flex; flex-direction: column; align-items: center; gap: 4px; padding-top: 10px; box-shadow: inset -1px 0 0 var(--line); }
  .rail .icon-btn { position: relative; }
  .badge { position: absolute; top: -3px; right: -4px; min-width: 14px; height: 14px; padding: 0 3px; border-radius: var(--r-full); background: var(--amber); color: #2a1a04; font-size: 9px; font-weight: var(--w-semi); display: grid; place-items: center; }
  .side { width: 268px; flex: none; display: flex; flex-direction: column; min-height: 0; background: var(--night-2); box-shadow: inset -1px 0 0 var(--line); }
  .side h2 { padding: 14px 16px 8px; }
  .tree { flex: 1; min-height: 0; overflow: auto; padding: 0 8px 12px; }
  main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .tabs { display: flex; height: 38px; flex: none; overflow-x: auto; box-shadow: inset 0 -1px 0 var(--line); }
  .tab { display: flex; align-items: center; flex: none; max-width: 240px; box-shadow: inset -1px 0 0 var(--line); color: var(--ink-3); }
  .tab:hover { color: var(--ink-1); background: var(--surface-1); }
  .tab.active { background: var(--code-bg); color: var(--ink-1); box-shadow: inset -1px 0 0 var(--line), inset 0 -2px 0 var(--live); }
  .t-main { display: flex; align-items: center; gap: 6px; height: 100%; padding: 0 6px 0 14px; font-size: var(--t-md); min-width: 0; }
  .t-x { display: grid; place-items: center; width: 20px; height: 20px; margin-right: 8px; border-radius: var(--r-sm); color: var(--ink-3); }
  .t-x:hover { background: var(--surface-3); color: var(--ink-1); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--amber); flex: none; }
  .pane { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: auto; }
  @media (max-width: 760px) { .side { width: 210px; } }
</style>
