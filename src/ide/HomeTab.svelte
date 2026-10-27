<!-- The IDE's Home tab: whose workspace this is, what changed, and the three ways in. -->
<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { api } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { basename, dirname } from '../lib/util/sanitize';
  import { ide } from './ide.svelte';

  const name = $derived((ide.agentId ? store.agentMap.get(ide.agentId)?.name : null) ?? ide.info?.agent_name ?? 'Agent');
  const project = $derived(ide.info?.project || store.workspace?.name || 'this repository');
  const changed = $derived(ide.info?.changed ?? ide.changes.length);

  type Mode = 'new' | 'open' | null;
  let mode = $state<Mode>(null);
  let value = $state('');
  let matches = $state<string[]>([]);
  let input: HTMLInputElement | undefined = $state();
  let seq = 0;

  function start(m: Exclude<Mode, null>) {
    mode = m;
    value = '';
    matches = [];
    queueMicrotask(() => input?.focus());
  }

  async function submit() {
    const v = value.trim();
    if (!v) return;
    if (mode === 'new') {
      if (await ide.create(v)) mode = null;
    } else if (matches[0]) {
      await ide.openFile(matches[0]);
      mode = null;
    }
  }

  // "Open file": filename matches as you type.
  $effect(() => {
    const q = value.trim();
    const id = ide.agentId;
    if (mode !== 'open' || !id || q.length < 2) {
      matches = [];
      return;
    }
    const mine = ++seq;
    const t = setTimeout(async () => {
      try {
        const r = await api.ideSearch(id, q);
        if (mine === seq) matches = r.files.slice(0, 8);
      } catch {
        if (mine === seq) matches = [];
      }
    }, 150);
    return () => clearTimeout(t);
  });

  function searchInCode() {
    ide.rail = 'search';
    ide.searchFocus++;
  }
</script>

<div class="home">
  <p class="eyebrow">Workspace</p>
  <h1 class="serif">{name}’s workspace</h1>
  <p class="sub">
    {project}{#if ide.info?.branch} on <span class="branch"><Icon name="branch" size={12} />{ide.info.branch}</span>{/if}
    · {changed} changed
  </p>
  {#if ide.info}<p class="root mono truncate" data-tip="Everything here is confined to this folder">{ide.info.root}</p>{/if}

  <div class="actions">
    <button class="btn" onclick={() => start('new')}><Icon name="plus" size={14} />New file</button>
    <button class="btn" onclick={() => start('open')}><Icon name="file" size={14} />Open file</button>
    <button class="btn" onclick={searchInCode}><Icon name="search" size={14} />Search in code</button>
  </div>

  {#if mode}
    <form class="ask" onsubmit={(e) => (e.preventDefault(), submit())}>
      <input
        class="input"
        bind:this={input}
        bind:value
        spellcheck="false"
        placeholder={mode === 'new' ? 'Path of the new file, e.g. src/notes.md' : 'Start typing a file name'}
        aria-label={mode === 'new' ? 'New file path' : 'Open file by name'}
        onkeydown={(e) => e.key === 'Escape' && (e.stopPropagation(), (mode = null))}
      />
      <button class="btn primary" type="submit" disabled={!value.trim() || (mode === 'open' && !matches.length)}>{mode === 'new' ? 'Create' : 'Open'}</button>
    </form>
    {#if mode === 'open' && matches.length}
      <div class="matches">
        {#each matches as m (m)}
          <button class="match truncate" onclick={() => (ide.openFile(m), (mode = null))}>{m}</button>
        {/each}
      </div>
    {/if}
  {/if}

  {#if ide.changes.length}
    <h2 class="eyebrow">Changed files</h2>
    <div class="changes">
      {#each ide.changes.slice(0, 8) as c (c.path)}
        <button class="change" onclick={() => ide.openDiff(c.path)}>
          <span class="n truncate">{basename(c.path)}</span><span class="d truncate">{dirname(c.path)}</span><span class="k k-{c.kind}">{c.kind === 'untracked' ? 'new' : c.kind}</span>
        </button>
      {/each}
      {#if ide.changes.length > 8}<button class="more" onclick={() => (ide.rail = 'source')}>and {ide.changes.length - 8} more in Source</button>{/if}
    </div>
  {/if}
</div>

<style>
  .home { padding: 56px 48px; max-width: 640px; display: flex; flex-direction: column; gap: 6px; overflow: auto; animation: fade-in var(--d-4) var(--ease-out); }
  h1 { font-size: var(--d-lg); line-height: 1.1; font-weight: 400; margin-top: 4px; }
  .sub { font-size: var(--t-lg); color: var(--ink-2); }
  .branch { display: inline-flex; align-items: center; gap: 4px; vertical-align: baseline; color: var(--live); font-family: var(--font-mono); font-size: var(--t-sm); }
  .root { font-size: var(--t-xs); color: var(--ink-4); }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 22px 0 6px; }
  .ask { display: flex; gap: 8px; margin-top: 8px; }
  .ask .input { flex: 1; }
  .matches { display: flex; flex-direction: column; margin-top: 2px; }
  .match { height: 26px; padding: 0 10px; border-radius: var(--r-sm); text-align: left; font-size: var(--t-md); color: var(--ink-2); }
  .match:hover { background: var(--surface-2); color: var(--ink-1); }
  h2 { margin: 26px 0 6px; }
  .changes { display: flex; flex-direction: column; }
  .change { display: flex; align-items: baseline; gap: 10px; height: 28px; padding: 0 10px; border-radius: var(--r-sm); font-size: var(--t-md); color: var(--ink-2); text-align: left; }
  .change:hover, .more:hover { background: var(--surface-2); color: var(--ink-1); }
  .d { flex: 1; font-size: var(--t-sm); color: var(--ink-3); }
  .k { font-size: var(--t-xs); text-transform: capitalize; }
  .k-modified, .k-renamed { color: var(--amber-ink); }
  .k-added, .k-untracked { color: var(--ok); }
  .k-deleted { color: var(--bad); }
  .more { height: 26px; padding: 0 10px; border-radius: var(--r-sm); font-size: var(--t-sm); color: var(--ink-3); text-align: left; }
</style>
