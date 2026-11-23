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
  import BudgetMark from './BudgetMark.svelte';
  import CrewStrip from './CrewStrip.svelte';
  import AskMe from '../home/AskMe.svelte';

  interface Props {
    narrow?: boolean;
    /** The bar's width in px: the crew strip gets what the two sides leave (see `.top`). */
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
  /** The widths of the two sides, in px: what they leave of the bar is the crew strip's. */
  let whereW = $state(0);
  let actionsW = $state(0);
  /** The gap between the bar's three parts, in px (`column-gap` of `.top`). */
  const GAP = 12;
  /** The bar's own box, measured (in the app it has a padding that `frameW` does not know). */
  let bar = $state<DOMRectReadOnly>();
  /** The most the repo pill may take, in px: what the actions leave of the bar. A longer name is cut. */
  const whereMax = $derived((bar?.width ?? frameW) - actionsW - 2 * GAP);
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

<header class="top" class:tauri bind:contentRect={bar} data-tauri-drag-region>
  <div class="where" bind:clientWidth={whereW} style:max-width="{whereMax}px" data-tauri-drag-region>
    <button class="ws" bind:this={wsBtn} onclick={toggleMenu} aria-haspopup="menu" aria-expanded={wsMenu} data-tip={ws ? `${ws.name} · ${tildify(ws.repo_path)}` : undefined}>
      <span class="name serif">{ws?.name ?? 'Codly'}</span>
      {#if !narrow}<span class="branch"><Icon name="branch" size={12} />{branch}</span>{/if}
    </button>
  </div>
  <div class="crew"><CrewStrip room={frameW - whereW - actionsW - 2 * GAP} /></div>
  <div class="actions" bind:clientWidth={actionsW}>
    {#if siteDemo}
      <a class="demo" href="/" data-tip="A fake repo and a scripted team: nothing runs on your machine. Back to the Codly site.">
        <span class="dot" aria-hidden="true"></span>{narrow ? 'Demo' : 'Live demo'}
      </a>
    {/if}
    <BudgetMark />
    <AskMe />
    <button class="search" onclick={() => ui.openPalette()} aria-label="Search and commands" data-tip="Agents, tasks, files, commands — or ask the Lead">
      <Icon name="search" size={14} />{#if !narrow}<span>Search</span>{/if}<kbd>⌘K</kbd>
    </button>
  </div>
</header>
{#if wsMenu && wsBtn}<Menu anchor={wsBtn} {items} onclose={() => (wsMenu = false)} width={300} />{/if}

<style>
  /* Three parts: where you are, the crew, the actions. With room to spare the two sides are
     equally wide, so the crew is centred on the stage (and so on the conversation column). In
     a narrow bar each side is as wide as what it holds and the crew strip gets all the rest,
     where it scrolls sideways. A name longer than what the actions leave of the bar (the
     max-width of `.where`) ends in an ellipsis inside its pill: the actions never leave the bar. */
  .top { display: grid; grid-template-columns: minmax(max-content, 1fr) minmax(0, max-content) minmax(max-content, 1fr); align-items: center; column-gap: 12px; height: var(--top-h); }
  .where { display: flex; align-items: center; justify-self: start; min-width: 0; }
  /* The office behind the bar keeps its own hour (a bright scene under the night theme, a dark
     one under the day theme), so the name and the branch stand on a backing of the app's theme:
     the inks hold AA on --glass-strong over any scene (src/styles/__tests__/contrast.test.ts). */
  .ws { display: inline-flex; align-items: baseline; gap: 10px; height: 34px; padding: 0 14px; border-radius: var(--r-full); min-width: 0; background: var(--glass-strong); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); }
  .ws:hover { box-shadow: var(--glass-edge), inset 0 0 0 1px var(--line-3); }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; font-size: 22px; line-height: 34px; font-style: italic; color: var(--ink-1); white-space: nowrap; }
  .branch { display: inline-flex; flex-shrink: 0; align-items: center; gap: 4px; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-3); white-space: nowrap; }
  .crew { min-width: 0; height: 100%; display: flex; align-items: center; }
  .crew > :global(.crew-strip) { min-width: 0; }
  .actions { display: flex; justify-self: end; align-items: center; gap: 8px; }
  .demo { display: inline-flex; align-items: center; gap: 7px; height: 34px; padding: 0 13px; border-radius: var(--r-full); color: var(--ink-2); font-size: var(--t-sm); font-weight: var(--w-med); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); }
  .demo:hover { color: var(--ink-1); }
  .demo .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .search { display: inline-flex; align-items: center; gap: 8px; height: 34px; padding: 0 8px 0 13px; border-radius: var(--r-full); color: var(--ink-3); font-size: var(--t-md); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); transition: color var(--d-2) var(--ease-out); }
  .search:hover { color: var(--ink-1); }
  .search span { margin-right: 6px; }
</style>
