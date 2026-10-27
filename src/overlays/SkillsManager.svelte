<!--
  Skills manager. Installed: every skill the agent CLIs can load (your project, your user folders,
  plugins, Codex, and Codly's own), with the engine and scope on each row, search, open folder and
  uninstall (confirmed first; the core refuses anything outside a skills folder). Browse: a small
  bundled catalog of public skills with a category filter and an install button (git clone into
  ~/.claude/skills).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { CatalogSkill } from '../lib/gen/CatalogSkill';
  import type { Skill } from '../lib/gen/Skill';
  import { api } from '../lib/api';
  import { skillsUi } from '../lib/state/skills.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import Icon from '../components/Icon.svelte';

  const project = $derived(store.workspace?.repo_path ?? null);
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const ENGINE = { claude_code: 'Claude Code', codex: 'Codex' } as const;
  const SCOPE = { project: 'Project', user: 'User', bundled: 'Bundled' } as const;

  let skills = $state<Skill[] | null>(null);
  let catalog = $state<CatalogSkill[] | null>(null);
  let problem = $state<string | null>(null);
  let query = $state('');
  let category = $state('All');
  /** Path of the row asking "remove it?", and ids being installed / removed. */
  let confirming = $state<string | null>(null);
  let installing = $state<string[]>([]);
  let removing = $state<string | null>(null);
  let el: HTMLDivElement | undefined = $state();

  const close = () => (skillsUi.open = false);
  const reloadInstalled = () => api.listSkills(project).then((l) => (skills = l));
  const reloadCatalog = () => api.skillCatalog().then((c) => (catalog = c));
  async function load() {
    problem = null;
    try {
      await Promise.all([reloadInstalled(), reloadCatalog()]);
    } catch (e) {
      problem = msg(e);
    }
  }
  onMount(() => {
    el?.focus();
    load();
  });

  const match = (q: string, ...fields: (string | null)[]) => {
    const n = q.trim().toLowerCase();
    return !n || fields.some((f) => f?.toLowerCase().includes(n));
  };
  const shown = $derived((skills ?? []).filter((s) => match(query, s.name, s.description, s.path, s.origin)));
  const categories = $derived(['All', ...new Set((catalog ?? []).map((c) => c.category))]);
  const browse = $derived((catalog ?? []).filter((c) => (category === 'All' || c.category === category) && match(query, c.id, c.description, c.category, c.author)));
  const pretty = (p: string) => p.replace(/^\/Users\/[^/]+/, '~');

  async function install(c: CatalogSkill) {
    installing = [...installing, c.id];
    try {
      await api.installSkill(c.id);
      toast({ kind: 'success', title: `Installed ${c.id}`, body: 'Agents pick it up on their next session.' });
      await Promise.all([reloadInstalled(), reloadCatalog()]);
    } catch (e) {
      toast({ kind: 'error', title: `Couldn’t install ${c.id}`, body: msg(e) });
    } finally {
      installing = installing.filter((x) => x !== c.id);
    }
  }
  async function uninstall(s: Skill) {
    removing = s.path;
    try {
      await api.uninstallSkill(s.path, project);
      toast({ kind: 'success', title: `Removed ${s.name}` });
      confirming = null;
      await Promise.all([reloadInstalled(), reloadCatalog()]);
    } catch (e) {
      toast({ kind: 'error', title: `Couldn’t remove ${s.name}`, body: msg(e) });
    } finally {
      removing = null;
    }
  }
  const openFolder = (s: Skill) => api.openSkillFolder(s.path, project).catch((e) => toast({ kind: 'error', title: 'Couldn’t open the folder', body: msg(e) }));

  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    if (confirming) confirming = null;
    else close();
  }
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label="Skills" tabindex="-1" bind:this={el} onkeydown={key}>
    <header>
      <h2>Skills</h2>
      <div class="lenses" role="tablist" aria-label="Skills">
        <button role="tab" aria-selected={skillsUi.tab === 'installed'} onclick={() => (skillsUi.tab = 'installed')}>Installed {#if skills}<span class="n">{skills.length}</span>{/if}</button>
        <button role="tab" aria-selected={skillsUi.tab === 'browse'} onclick={() => (skillsUi.tab = 'browse')}>Browse {#if catalog}<span class="n">{catalog.length}</span>{/if}</button>
      </div>
      <button class="icon-btn" aria-label="Close" onclick={close}><Icon name="x" /></button>
    </header>

    <div class="bar">
      <input class="input search" type="search" placeholder={skillsUi.tab === 'installed' ? 'Search installed skills' : 'Search the catalog'} bind:value={query} spellcheck="false" aria-label="Search skills" />
      {#if skillsUi.tab === 'browse'}
        <div class="cats" role="radiogroup" aria-label="Category">
          {#each categories as c (c)}
            <button class="chip cat" role="radio" aria-checked={category === c} class:on={category === c} onclick={() => (category = c)}>{c}</button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="list">
      {#if problem}
        <p class="note">Couldn’t read the skills ({problem}). <button class="link" onclick={load}>Try again</button></p>
      {:else if skillsUi.tab === 'installed'}
        {#if !skills}
          <p class="note muted">Looking for skills…</p>
        {:else if !shown.length}
          <p class="note muted">{query ? 'No skill matches that.' : 'No skills installed yet. Browse the catalog to add one.'}</p>
        {/if}
        {#each shown as s (s.path)}
          <div class="row" class:asking={confirming === s.path}>
            <div class="main">
              <div class="line">
                <span class="name">{s.name}</span>
                <span class="chip sm engine">{ENGINE[s.engine]}</span>
                <span class="chip sm scope-{s.scope}">{SCOPE[s.scope]}</span>
                {#if s.origin}<span class="muted origin">{s.origin}</span>{/if}
              </div>
              {#if s.description}<p class="desc">{s.description}</p>{/if}
              <p class="path mono truncate" title={s.path}>{pretty(s.path)}</p>
            </div>
            <div class="actions">
              {#if confirming === s.path}
                <span class="ask">Delete this folder?</span>
                <button class="btn sm ghost" onclick={() => (confirming = null)}>Keep</button>
                <button class="btn sm danger" disabled={removing === s.path} onclick={() => uninstall(s)}>{removing === s.path ? 'Removing…' : 'Uninstall'}</button>
              {:else}
                {#if s.scope !== 'bundled'}
                  <button class="btn sm ghost" onclick={() => openFolder(s)}><Icon name="folder" size={13} />Open folder</button>
                {/if}
                {#if s.removable}
                  <button class="btn sm ghost" onclick={() => (confirming = s.path)}><Icon name="trash" size={13} />Uninstall</button>
                {/if}
              {/if}
            </div>
          </div>
        {/each}
      {:else}
        {#if !catalog}
          <p class="note muted">Loading the catalog…</p>
        {:else if !browse.length}
          <p class="note muted">No skill matches that.</p>
        {/if}
        {#each browse as c (c.id)}
          <div class="row">
            <div class="main">
              <div class="line">
                <span class="name">{c.id}</span>
                <span class="chip sm">{c.category}</span>
                <span class="muted origin">{c.author}</span>
              </div>
              <p class="desc">{c.description}</p>
              <p class="path mono truncate" title={c.repo}>{c.repo.replace('https://', '').replace(/\.git$/, '')}{c.subdir ? ` · ${c.subdir}` : ''}</p>
            </div>
            <div class="actions">
              {#if c.installed}
                <span class="chip sm done"><Icon name="check" size={12} />Installed</span>
              {:else}
                <button class="btn sm primary" disabled={installing.includes(c.id)} onclick={() => install(c)}>{installing.includes(c.id) ? 'Cloning…' : 'Install'}</button>
              {/if}
            </div>
          </div>
        {/each}
        <p class="foot muted">Installs with <span class="mono">git clone --depth 1</span> into <span class="mono">~/.claude/skills</span>. Review a skill before you rely on it: it is instructions your agents will follow.</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); display: grid; place-items: center; animation: fade-in var(--d-3) var(--ease-out); }
  .sheet { display: flex; flex-direction: column; width: min(820px, calc(100vw - 32px)); height: min(680px, calc(100vh - 48px)); border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; overflow: hidden; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-shrink: 0; height: 64px; padding: 0 14px 0 26px; box-shadow: inset 0 -1px 0 var(--line); }
  h2 { font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); margin-right: auto; }
  .bar { display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; padding: 14px 26px 10px; }
  .search { width: 100%; }
  .cats { display: flex; flex-wrap: wrap; gap: 6px; }
  .cat { cursor: pointer; }
  .cat:hover { background: var(--surface-3); }
  .cat.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 14px 18px; }
  .row { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; padding: 12px; border-radius: var(--r-md); }
  .row:hover, .row.asking { background: var(--surface-1); }
  .main { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 4px; }
  .line { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .name { font-weight: var(--w-semi); font-size: var(--t-md); }
  .origin { font-size: var(--t-sm); }
  .engine { background: var(--surface-3); color: var(--ink-1); }
  .scope-project { color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-3); background: transparent; }
  .scope-bundled { color: var(--ink-3); }
  .desc { color: var(--ink-2); font-size: var(--t-md); line-height: var(--lh-md); }
  .path { color: var(--ink-3); font-size: var(--t-xs); }
  .actions { display: flex; align-items: center; gap: 4px; flex-shrink: 0; min-height: 26px; }
  .ask { font-size: var(--t-sm); color: var(--ink-2); margin-right: 4px; }
  .done { color: var(--ink-2); }
  .note { padding: 40px 12px; text-align: center; font-size: var(--t-md); }
  .foot { padding: 14px 12px 0; font-size: var(--t-sm); line-height: var(--lh-sm); }
  .link { color: var(--ink-1); font-weight: var(--w-med); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
  @media (max-width: 640px) { .row { flex-direction: column; } }
</style>
