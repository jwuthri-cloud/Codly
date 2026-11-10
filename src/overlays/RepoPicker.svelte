<!--
  "Open repository" in the browser (`codly serve`). A browser page cannot show a folder dialog, so this lists
  the git repositories under the home folder of the machine Codly runs on, newest first. Type to filter, or
  type a full path ("/…") to open any other folder. The desktop app keeps its folder dialog.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { RepoEntry } from '../lib/gen/RepoEntry';
  import { api } from '../lib/api';
  import { openWorkspace } from '../lib/state/actions';
  import { ui } from '../lib/state/ui.svelte';
  import { filterRepos, typedPath } from '../lib/util/repos';
  import { tildify } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';

  let repos = $state<RepoEntry[]>([]);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  let query = $state('');
  let active = $state(0);
  let input: HTMLInputElement | undefined = $state();

  const shown = $derived(filterRepos(repos, query));
  const path = $derived(typedPath(query));
  /** What Enter and the arrow keys move through: the typed path first, then the repositories. */
  const rows = $derived([...(path ? [path] : []), ...shown.map((r) => r.path)]);

  onMount(() => {
    input?.focus();
    api
      .listRepos()
      .then((list) => (repos = list))
      .catch((e) => (problem = String(e)))
      .finally(() => (loading = false));
  });

  const close = () => (ui.repoPickerOpen = false);

  function open(p: string) {
    close();
    void openWorkspace(p);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = Math.min(active + 1, rows.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = Math.max(active - 1, 0);
    } else if (e.key === 'Enter' && rows[active]) {
      e.preventDefault();
      open(rows[active]);
    }
  }
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label="Open a repository" tabindex="-1" {onkeydown}>
    <header>
      <h2>Open a repository</h2>
      <button type="button" class="icon-btn" aria-label="Close" onclick={close}><Icon name="x" /></button>
    </header>
    <div class="body">
      <input
        bind:this={input}
        bind:value={query}
        oninput={() => (active = 0)}
        placeholder="Search, or type a full path (/…)"
        aria-label="Search the repositories, or type a full path"
        spellcheck="false"
        autocomplete="off"
      />
      <div class="list">
        {#if path}
          <button type="button" class="row" class:on={active === 0} onclick={() => open(path)} onpointermove={() => (active = 0)}>
            <Icon name="folder" /><span class="name">Open {path}</span>
          </button>
        {/if}
        {#each shown as r, i (r.path)}
          {@const n = i + (path ? 1 : 0)}
          <button type="button" class="row" class:on={active === n} onclick={() => open(r.path)} onpointermove={() => (active = n)}>
            <Icon name="repo" /><span class="name">{r.name}</span><span class="where">{tildify(r.path)}</span>
          </button>
        {/each}
      </div>
      {#if loading}
        <p class="note">Looking for repositories…</p>
      {:else if problem}
        <p class="note">Couldn’t list the repositories ({problem}). Type a full path instead.</p>
      {:else if !repos.length && !path}
        <p class="note">No git repositories in the home folder. Type a full path, or clone one first.</p>
      {:else if !rows.length}
        <p class="note">No repository matches. Type a full path (/…) to open another folder.</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); display: grid; place-items: center; animation: fade-in var(--d-3) var(--ease-out); }
  .sheet { width: min(560px, calc(100vw - 32px)); border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; justify-content: space-between; height: 64px; padding: 0 14px 0 26px; box-shadow: inset 0 -1px 0 var(--line); }
  h2 { font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); }
  .body { display: flex; flex-direction: column; gap: 10px; padding: 18px 20px 20px; }
  .list { display: flex; flex-direction: column; gap: 2px; max-height: min(420px, 55vh); overflow-y: auto; }
  .row { display: flex; align-items: center; gap: 10px; min-height: 36px; padding: 6px 10px; border-radius: var(--r-md); text-align: left; font-size: var(--t-md); }
  .row.on { background: var(--surface-2); }
  .name { font-weight: var(--w-med); white-space: nowrap; }
  .where { margin-left: auto; color: var(--ink-3); font-size: var(--t-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .note { color: var(--ink-3); font-size: var(--t-sm); }
</style>
