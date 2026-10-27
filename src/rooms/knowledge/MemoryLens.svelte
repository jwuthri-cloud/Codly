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
  import { memoryView } from '../../lib/util/memory';
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
  let kindFilter = $state<MemoryKind | 'all'>('all');
  const KINDS: MemoryKind[] = ['decision', 'lesson', 'fact', 'preference', 'note', 'summary'];
  const KIND_HUE: Record<MemoryKind, number> = { decision: 1, lesson: 3, fact: 2, preference: 5, note: 7, summary: 4 };
  const kindCount = $derived(Object.fromEntries(KINDS.map((k) => [k, store.memories.filter((m) => !m.superseded_by && m.kind === k).length])) as Record<MemoryKind, number>);
  const pinned = (m: Memory) => m.pinned || ui.pinned.includes(m.id);
  const memories = $derived(
    store.memories
      .filter((m) => !m.superseded_by && match(m) && (kindFilter === 'all' || m.kind === kindFilter) && (!q || fuzzy(q, `${m.title} ${m.tags.join(' ')} ${m.text}`)))
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

  /** A graph entity id as a short label: task:<id> → its key, file:<path> → file name. */
  function refLabel(r: string) {
    const [type, ...rest] = r.split(':');
    const v = rest.join(':');
    if (type === 'task') return store.taskMap.get(v)?.key ?? 'task';
    if (type === 'agent') return store.agentMap.get(v)?.name ?? 'agent';
    if (type === 'file') return v.split('/').pop() || v;
    return v || r;
  }
  let open = $state<Set<string>>(new Set());
  const toggle = (id: string) => { const n = new Set(open); n.has(id) ? n.delete(id) : n.add(id); open = n; };

  let adding = $state(false);
  let title = $state('');
  let tagText = $state('');
  let text = $state('');
  let scope = $state<MemoryScope>('project');
  let kind = $state<MemoryKind>('fact');
  let addEl: HTMLTextAreaElement | undefined = $state();
  async function add() {
    if (!text.trim()) return;
    const tags = tagText.split(',').map((t) => t.trim()).filter(Boolean);
    const m = await addMemory(text.trim(), scope, kind, title.trim() || null, tags.length ? tags : null);
    if (m) {
      text = '';
      title = '';
      tagText = '';
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
  <div class="kinds" role="radiogroup" aria-label="Kind">
    <button class="kchip" role="radio" aria-checked={kindFilter === 'all'} onclick={() => (kindFilter = 'all')}>Everything</button>
    {#each KINDS.filter((k) => kindCount[k]) as k (k)}
      <button class="kchip" role="radio" aria-checked={kindFilter === k} style:--hue="var(--id-{KIND_HUE[k]})" onclick={() => (kindFilter = kindFilter === k ? 'all' : k)}>
        <i class="dot"></i>{KIND_LABEL[k]} <span class="n">{kindCount[k]}</span>
      </button>
    {/each}
  </div>
  <div class="scroll">
    {#if adding}
      <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
        <input class="input add-title" bind:value={title} placeholder="Headline (optional) — e.g. “Use pnpm”" maxlength="120" />
        <textarea bind:this={addEl} bind:value={text} rows="3" class="input" placeholder="Something every agent should know — e.g. “Use pnpm, not npm.”"
          onkeydown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); e.stopPropagation(); add(); } else if (e.key === 'Escape') { e.stopPropagation(); adding = false; } }}></textarea>
        <div class="add-row">
          <select class="input" bind:value={scope} aria-label="Who reads it"><option value="project">Shared · this workspace</option><option value="global">Global · every project</option></select>
          <select class="input" bind:value={kind} aria-label="Kind">{#each ['fact', 'preference', 'decision', 'lesson', 'note'] as k (k)}<option value={k}>{KIND_LABEL[k as MemoryKind]}</option>{/each}</select>
          <input class="input grow" bind:value={tagText} placeholder="Tags, comma separated" aria-label="Tags" />
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
          {@const v = memoryView(m)}
          {@const refs = m.refs.map(refLabel)}
          {@const tags = [...new Set([v.key, ...m.tags].filter((t): t is string => !!t))].filter((t) => !refs.includes(t))}
          <li class="m" class:pin={pinned(m)} style:--hue="var(--id-{KIND_HUE[m.kind]})">
            <div class="top">
              <span class="kind"><i class="dot"></i>{KIND_LABEL[m.kind]}</span>
              <span class="scope" data-tip={SCOPE_TIP[m.scope]}>{scopeLabel(m)}</span>
              <span class="grow"></span>
              {#if confirmDel === m.id}
                <button class="btn sm danger" onclick={() => { confirmDel = null; deleteMemory(m.id); }}>Delete</button>
                <button class="btn sm ghost" onclick={() => (confirmDel = null)}>Keep</button>
              {:else}
                <button class="icon-btn sm act" class:on={pinned(m)} aria-pressed={pinned(m)} aria-label={pinned(m) ? 'Unpin' : 'Pin'} data-tip={pinned(m) ? 'Pinned: always in the memory pack' : 'Pin: always include it in the memory pack'} onclick={() => togglePinMemory(m)}><Icon name="pin" size={13} /></button>
                <button class="icon-btn sm act" aria-label="Delete memory" data-tip="Delete" onclick={() => (confirmDel = m.id)}><Icon name="trash" size={13} /></button>
              {/if}
              <span class="num when">{ago(m.created_at, ui.now)}</span>
            </div>
            <h3 class="title selectable"><InlineText text={v.title} /></h3>
            {#if v.body}
              <p class="body selectable" class:clamp={!open.has(m.id)}><InlineText text={v.body} /></p>
              {#if v.body.length > 220}<button class="more" onclick={() => toggle(m.id)}>{open.has(m.id) ? 'Show less' : 'Show more'}</button>{/if}
            {/if}
            <div class="foot">
              <div class="chips">
                {#each tags as t (t)}<span class="chip">{t}</span>{/each}
                {#each refs as r, i (m.refs[i])}<span class="chip ref" data-tip={m.refs[i]}>{r}</span>{/each}
              </div>
              <span class="by">{#if a}<Portrait agent={a} size={16} />{a.name}{:else}{m.author === 'user' ? 'You' : 'Codly'}{/if}</span>
              <Provenance kind={prov(m)} compact />
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .ml { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .bar { display: flex; align-items: center; gap: 10px; padding: 0 28px 12px; flex-wrap: wrap; }
  .n { margin-left: 1px; opacity: 0.7; }
  .find { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; margin-left: auto; border-radius: var(--r-full); background: var(--surface-1); color: var(--ink-3); }
  .find input { border: 0; outline: 0; background: transparent; width: 170px; font-size: var(--t-md); }
  .kinds { display: flex; gap: 6px; flex-wrap: wrap; padding: 0 28px 16px; }
  .kchip { --hue: var(--ink-3); display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 11px; border: 0; border-radius: var(--r-full); background: transparent; color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); cursor: pointer; transition: background var(--d-2), color var(--d-2); }
  .kchip:hover { color: var(--ink-1); background: var(--surface-1); }
  .kchip[aria-checked='true'] { color: var(--ink-1); background: var(--surface-2); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--hue); flex-shrink: 0; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 0 28px 28px; }
  .add { display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px; padding: 14px; border-radius: var(--r-lg); background: var(--surface-1); }
  .add-title { font-family: var(--font-display); font-size: 20px; }
  .add-row { display: flex; gap: 8px; align-items: center; }
  .grow { flex: 1; }
  .mems { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 14px; align-items: start; }
  .m { display: flex; flex-direction: column; gap: 8px; padding: 14px 16px 12px; border-radius: var(--r-lg); background: var(--surface-1); transition: background var(--d-2); }
  .m:hover, .m.pin { background: var(--surface-2); }
  .top { display: flex; align-items: center; gap: 10px; min-height: 22px; font-size: var(--t-xs); color: var(--ink-3); }
  .kind { display: inline-flex; align-items: center; gap: 6px; font-weight: var(--w-semi); text-transform: uppercase; letter-spacing: 0.06em; color: var(--ink-2); }
  .when { margin-left: 2px; }
  .title { margin: 2px 0 0; font-family: var(--font-display); font-weight: 400; font-size: 21px; line-height: 26px; letter-spacing: -0.01em; color: var(--ink-1); text-wrap: balance; }
  .body { margin: 0; font-size: var(--t-md); line-height: 20px; color: var(--ink-2); }
  .body.clamp { display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .more { align-self: flex-start; padding: 0; border: 0; background: none; color: var(--ink-3); font-size: var(--t-xs); cursor: pointer; }
  .more:hover { color: var(--ink-1); }
  .foot { display: flex; align-items: center; gap: 8px; margin-top: auto; padding-top: 6px; font-size: var(--t-xs); color: var(--ink-3); }
  .chips { display: flex; flex-wrap: wrap; gap: 5px; flex: 1; min-width: 0; }
  .chip { padding: 2px 8px; border-radius: var(--r-full); background: color-mix(in srgb, var(--ink-3) 14%, transparent); color: var(--ink-2); font-size: var(--t-xs); line-height: 16px; }
  .chip.ref { font-family: var(--font-mono); font-size: 11px; background: color-mix(in srgb, var(--hue) 16%, transparent); }
  .by { display: inline-flex; align-items: center; gap: 5px; color: var(--ink-2); white-space: nowrap; }
  .act { opacity: 0; transition: opacity var(--d-2); }
  .m:hover .act, .act:focus-visible, .act.on { opacity: 1; }
  .act.on { color: var(--ink-1); }
</style>
