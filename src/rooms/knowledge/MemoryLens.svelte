<!--
  Team memory: what every agent reads at task start. Shared (this workspace and global), private
  (one agent) and task notes (passed to dependents). Who wrote each one and how we know.
-->
<script lang="ts">
  import type { Memory } from '../../lib/gen/Memory';
  import type { MemoryKind } from '../../lib/gen/MemoryKind';
  import type { MemoryScope } from '../../lib/gen/MemoryScope';
  import { addMemory, deleteMemory, togglePinMemory } from '../../lib/state/actions';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago } from '../../lib/util/format';
  import { fuzzy } from '../../lib/util/fuzzy';
  import { idVar, KIND_LABEL, SCOPE_LABEL, SCOPE_TIP, type Provenance as Prov } from '../../lib/util/labels';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import InlineText from '../../components/InlineText.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Provenance from '../../components/Provenance.svelte';

  type Filter = 'all' | 'shared' | 'private' | 'task';
  let filter = $state<Filter>('all');
  let q = $state('');
  const match = (m: Memory) =>
    filter === 'all' || (filter === 'shared' && (m.scope === 'project' || m.scope === 'global')) || (filter === 'private' && m.scope === 'agent') || (filter === 'task' && m.scope === 'task');
  const pinned = (m: Memory) => m.pinned || ui.pinned.includes(m.id);
  const memories = $derived(
    store.memories
      .filter((m) => !m.superseded_by && match(m) && (!q || fuzzy(q, m.text)))
      .sort((a, b) => Number(pinned(b)) - Number(pinned(a)) || b.created_at - a.created_at),
  );
  const counts = $derived({
    all: store.memories.length,
    shared: store.memories.filter((m) => m.scope === 'project' || m.scope === 'global').length,
    private: store.memories.filter((m) => m.scope === 'agent').length,
    task: store.memories.filter((m) => m.scope === 'task').length,
  });
  function prov(m: Memory): Prov {
    if (m.author === 'user') return 'provided';
    if (m.kind === 'summary') return 'model';
    if (m.author === 'codly') return m.source_event ? 'observed' : 'inferred';
    return 'model';
  }
  function scopeLabel(m: Memory) {
    if (m.scope === 'task' && m.scope_id) return `Task ${store.taskMap.get(m.scope_id)?.key ?? ''}`.trim();
    if (m.scope === 'agent' && m.scope_id) return `Private · ${store.agentMap.get(m.scope_id)?.name ?? ''}`;
    return SCOPE_LABEL[m.scope];
  }

  let adding = $state(false);
  let text = $state('');
  let scope = $state<MemoryScope>('project');
  let kind = $state<MemoryKind>('fact');
  let addEl: HTMLTextAreaElement | undefined = $state();
  async function add() {
    if (!text.trim()) return;
    const m = await addMemory(text.trim(), scope, kind);
    if (m) {
      text = '';
      adding = false;
    }
  }
  let confirmDel = $state<string | null>(null);
</script>

<div class="ml">
  <div class="bar">
    <div class="lenses" role="radiogroup" aria-label="Scope">
      {#each [['all', 'All'], ['shared', 'Shared'], ['private', 'Private'], ['task', 'Task']] as [id, label] (id)}
        <button role="radio" aria-checked={filter === id} onclick={() => (filter = id as Filter)}>{label} <span class="n">{counts[id as Filter]}</span></button>
      {/each}
    </div>
    <label class="find"><Icon name="search" size={13} /><input placeholder="Filter memories" bind:value={q} spellcheck="false" /></label>
    <button class="btn sm" onclick={() => { adding = !adding; queueMicrotask(() => addEl?.focus()); }}><Icon name="plus" size={13} />Add a memory</button>
  </div>
  <div class="scroll">
    {#if adding}
      <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
        <textarea bind:this={addEl} bind:value={text} rows="2" class="input" placeholder="Something every agent should know — e.g. “Use pnpm, not npm.”"
          onkeydown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); e.stopPropagation(); add(); } else if (e.key === 'Escape') { e.stopPropagation(); adding = false; } }}></textarea>
        <div class="add-row">
          <select class="input" bind:value={scope} aria-label="Who reads it"><option value="project">Shared · this workspace</option><option value="global">Global · every project</option></select>
          <select class="input" bind:value={kind} aria-label="Kind">{#each ['fact', 'preference', 'decision', 'lesson', 'note'] as k (k)}<option value={k}>{KIND_LABEL[k as MemoryKind]}</option>{/each}</select>
          <span class="grow"></span>
          <button class="btn ghost sm" type="button" onclick={() => (adding = false)}>Cancel</button>
          <button class="btn primary sm" type="submit" disabled={!text.trim()}>Save</button>
        </div>
      </form>
    {/if}
    {#if !memories.length}
      <EmptyState compact title={q ? `Nothing matches “${q}”.` : 'Nothing remembered yet.'} body="Agents save lessons and notes as they work; add facts and preferences every agent will read." />
    {:else}
      <ul class="mems">
        {#each memories as m (m.id)}
          {@const a = store.agentMap.get(m.author)}
          <li class="m" class:pin={pinned(m)} style:--hue={a ? idVar(a.color) : 'var(--line-3)'}>
            <p class="text selectable"><InlineText text={m.text} /></p>
            <div class="meta">
              <span class="kind">{KIND_LABEL[m.kind]}</span>
              <span data-tip={SCOPE_TIP[m.scope]}>{scopeLabel(m)}</span>
              <span class="by">{#if a}<Portrait agent={a} size={16} />{a.name}{:else}{m.author === 'user' ? 'You' : 'Codly'}{/if}</span>
              <Provenance kind={prov(m)} />
              <span class="num when">{ago(m.created_at, ui.now)}</span>
              <span class="grow"></span>
              {#if confirmDel === m.id}
                <button class="btn sm danger" onclick={() => { confirmDel = null; deleteMemory(m.id); }}>Delete</button>
                <button class="btn sm ghost" onclick={() => (confirmDel = null)}>Keep</button>
              {:else}
                <button class="icon-btn sm act" class:on={pinned(m)} aria-pressed={pinned(m)} aria-label={pinned(m) ? 'Unpin' : 'Pin'} data-tip={pinned(m) ? 'Pinned: always in the memory pack' : 'Pin: always include it in the memory pack'} onclick={() => togglePinMemory(m)}><Icon name="pin" size={13} /></button>
                <button class="icon-btn sm act" aria-label="Delete memory" data-tip="Delete" onclick={() => (confirmDel = m.id)}><Icon name="trash" size={13} /></button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .ml { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .bar { display: flex; align-items: center; gap: 10px; padding: 0 28px 14px; flex-wrap: wrap; }
  .n { margin-left: 1px; }
  .find { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; margin-left: auto; border-radius: var(--r-full); background: var(--surface-1); color: var(--ink-3); }
  .find input { border: 0; outline: 0; background: transparent; width: 170px; font-size: var(--t-md); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 0 28px 28px; }
  .add { display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px; padding: 14px; border-radius: var(--r-lg); background: var(--surface-1); }
  .add-row { display: flex; gap: 8px; align-items: center; }
  .grow { flex: 1; }
  .mems { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 12px; align-items: start; }
  .m { display: flex; flex-direction: column; gap: 10px; padding: 14px 16px 12px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 2px 0 0 var(--hue); }
  .m.pin { background: var(--surface-2); }
  .text { font-size: var(--t-lg); line-height: 22px; color: var(--ink-1); }
  .meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; font-size: var(--t-xs); color: var(--ink-3); }
  .kind { font-weight: var(--w-semi); text-transform: uppercase; letter-spacing: 0.06em; color: var(--ink-2); }
  .by { display: inline-flex; align-items: center; gap: 5px; color: var(--ink-2); }
  .act { opacity: 0; transition: opacity var(--d-2); }
  .m:hover .act, .act:focus-visible, .act.on { opacity: 1; }
  .act.on { color: var(--ink-1); }
</style>
