<!--
  Top of every room: where you are (repo · branch), your crew, and search (⌘K).
  It is also the window's drag region.
-->
<script lang="ts">
  import { api, isSiteDemo, isTauriApp } from '../lib/api';
  import type { RecentWorkspace } from '../lib/gen/RecentWorkspace';
  import { openWorkspace } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { shortSha } from '../lib/util/format';
  import { tildify } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import CrewStrip from './CrewStrip.svelte';
  import AskMe from '../home/AskMe.svelte';

  interface Props {
    narrow?: boolean;
    /** The bar's width in px: the crew strip gets what the two sides leave (see `.crew`). */
    frameW: number;
  }
  let { narrow = false, frameW }: Props = $props();

  const tauri = isTauriApp();
  const siteDemo = isSiteDemo();
  let wsBtn: HTMLButtonElement | undefined = $state();
  let wsMenu = $state(false);
  let recent = $state<RecentWorkspace[]>([]);
  async function toggleMenu() {
    wsMenu = !wsMenu;
    if (wsMenu) recent = await api.recentWorkspaces().catch(() => []);
  }
  const ws = $derived(store.workspace);
  const branch = $derived(ws?.current_branch ?? ws?.default_branch ?? 'detached');
  const items = $derived<MenuItem[]>([
    { kind: 'header', label: 'Recent' },
    ...recent.slice(0, 6).map((r) => ({ label: r.name, hint: tildify(r.repo_path), icon: 'repo', checked: r.repo_path === ws?.repo_path, run: () => openWorkspace(r.repo_path) })),
    { kind: 'separator' },
    { label: 'Open repository…', icon: 'folder', kbd: '⌘O', run: () => openWorkspace() },
    { kind: 'separator' },
    { label: `${branch} · ${shortSha(ws?.head)}${ws?.dirty ? ' · uncommitted changes' : ''}`, icon: 'branch', disabled: true, run: () => {} },
    { label: 'Codly never checks out or pushes', icon: 'lock', disabled: true, run: () => {} },
  ]);
</script>

<header class="top" class:tauri data-tauri-drag-region>
  <div class="where" data-tauri-drag-region>
    <button class="ws" bind:this={wsBtn} onclick={toggleMenu} aria-haspopup="menu" aria-expanded={wsMenu} data-tip={tildify(ws?.repo_path)}>
      <span class="name serif">{ws?.name ?? 'Codly'}</span>
      {#if !narrow}<span class="branch"><Icon name="branch" size={12} />{branch}</span>{/if}
    </button>
  </div>
  <div class="crew"><CrewStrip room={frameW - 400} /></div>
  <div class="actions">
    {#if siteDemo}
      <a class="demo" href="/" data-tip="A fake repo and a scripted team: nothing runs on your machine. Back to the Codly site.">
        <span class="dot" aria-hidden="true"></span>{narrow ? 'Demo' : 'Live demo'}
      </a>
    {/if}
    <AskMe />
    <button class="search" onclick={() => ui.openPalette()} aria-label="Search and commands" data-tip="Agents, tasks, files, commands — or ask the Lead">
      <Icon name="search" size={14} />{#if !narrow}<span>Search</span>{/if}<kbd>⌘K</kbd>
    </button>
  </div>
</header>
{#if wsMenu && wsBtn}<Menu anchor={wsBtn} {items} onclose={() => (wsMenu = false)} width={300} />{/if}

<style>
  .top { position: relative; display: flex; align-items: center; justify-content: space-between; gap: 12px; height: var(--top-h); }
  .where { display: flex; align-items: center; min-width: 0; }
  .ws { display: inline-flex; align-items: baseline; gap: 10px; height: 34px; padding: 0 12px; margin-left: -12px; border-radius: var(--r-full); min-width: 0; }
  .ws:hover { background: var(--surface-2); }
  .name { font-size: 22px; line-height: 34px; font-style: italic; color: var(--ink-1); white-space: nowrap; }
  .branch { display: inline-flex; align-items: center; gap: 4px; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-3); white-space: nowrap; }
  /* centred on the stage (and so on the conversation column), whatever sits left and right;
     the 2 × 200px kept for the sides is the 400 taken off the strip's `room` above */
  .crew { position: absolute; left: 50%; top: 0; height: 100%; transform: translateX(-50%); width: max-content; max-width: calc(100% - 2 * 200px); display: flex; align-items: center; }
  .crew > :global(.strip) { min-width: 0; }
  .actions { display: flex; justify-content: flex-end; align-items: center; gap: 8px; }
  .demo { display: inline-flex; align-items: center; gap: 7px; height: 34px; padding: 0 13px; border-radius: var(--r-full); color: var(--ink-2); font-size: var(--t-sm); font-weight: var(--w-med); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); }
  .demo:hover { color: var(--ink-1); }
  .demo .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .search { display: inline-flex; align-items: center; gap: 8px; height: 34px; padding: 0 8px 0 13px; border-radius: var(--r-full); color: var(--ink-3); font-size: var(--t-md); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); transition: color var(--d-2) var(--ease-out); }
  .search:hover { color: var(--ink-1); }
  .search span { margin-right: 6px; }
</style>
