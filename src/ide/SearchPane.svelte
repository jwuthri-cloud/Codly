<!-- Search rail: file names and text, both from the core (confined to the worktree). -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { IdeSearch } from '../lib/gen/IdeSearch';
  import { ide } from './ide.svelte';

  let query = $state('');
  let result = $state<IdeSearch | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let input: HTMLInputElement | undefined = $state();
  let seq = 0;

  // Debounced; a newer query makes an older answer irrelevant.
  $effect(() => {
    const q = query.trim();
    const id = ide.agentId;
    if (!id || q.length < 2) {
      result = null;
      error = null;
      return;
    }
    const mine = ++seq;
    busy = true;
    const t = setTimeout(async () => {
      try {
        const r = await api.ideSearch(id, q);
        if (mine === seq) (result = r), (error = null);
      } catch (e) {
        if (mine === seq) error = e instanceof Error ? e.message : String(e);
      } finally {
        if (mine === seq) busy = false;
      }
    }, 180);
    return () => clearTimeout(t);
  });

  $effect(() => {
    void ide.searchFocus;
    input?.focus();
  });
  onMount(() => input?.focus());

  const groups = $derived.by(() => {
    const m = new Map<string, NonNullable<typeof result>['hits']>();
    for (const h of result?.hits ?? []) m.set(h.path, [...(m.get(h.path) ?? []), h]);
    return [...m];
  });
</script>

<div class="pane">
  <input class="input" bind:this={input} bind:value={query} placeholder="Search files and text" spellcheck="false" aria-label="Search in the workspace" />
  <div class="results">
    {#if error}
      <p class="hint bad">{error}</p>
    {:else if query.trim().length < 2}
      <p class="hint">Type at least two characters. Case-insensitive; ignores .gitignore’d files, <code>node_modules</code> and <code>target</code>.</p>
    {:else if result && !result.files.length && !result.hits.length}
      <p class="hint">{busy ? 'Searching…' : 'Nothing found.'}</p>
    {:else if result}
      {#if result.files.length}
        <h3 class="eyebrow">Files · {result.files.length}</h3>
        {#each result.files as f (f)}
          <button class="file truncate" onclick={() => ide.openFile(f)}>{f}</button>
        {/each}
      {/if}
      {#if groups.length}
        <h3 class="eyebrow">In code · {result.hits.length}</h3>
        {#each groups as [path, hits] (path)}
          <p class="path truncate">{path}</p>
          {#each hits as h (h.line)}
            <button class="hit" onclick={() => ide.openFile(h.path, Number(h.line))}>
              <span class="ln num">{h.line}</span><span class="txt truncate">{h.text}</span>
            </button>
          {/each}
        {/each}
      {/if}
      {#if result.truncated}<p class="hint">Showing the first results; narrow the query to see more.</p>{/if}
    {/if}
  </div>
</div>

<style>
  .pane { display: flex; flex-direction: column; gap: 10px; min-height: 0; flex: 1; padding: 0 8px 8px; }
  .results { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; }
  h3 { margin: 10px 4px 4px; }
  .file, .hit { display: flex; gap: 8px; align-items: center; width: 100%; height: 24px; padding: 0 6px; border-radius: var(--r-sm); font-size: var(--t-md); color: var(--ink-2); text-align: left; }
  .file:hover, .hit:hover { background: var(--surface-2); color: var(--ink-1); }
  .path { margin: 8px 4px 2px; font-size: var(--t-sm); color: var(--ink-1); font-weight: var(--w-med); }
  .hit { font-family: var(--font-mono); font-size: var(--t-sm); }
  .ln { color: var(--ink-3); min-width: 28px; text-align: right; }
  .txt { flex: 1; }
  .hint { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); padding: 8px 4px; }
  .hint.bad { color: var(--bad); }
  code { font-size: var(--t-xs); }
</style>
