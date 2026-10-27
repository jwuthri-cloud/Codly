<!--
  Find: one box over everything the team wrote down.
  - Exact: the literal words, case-insensitive, across the whiteboard, tasks and memories.
  - Meaning: ranked by relevance with the engines agents use (codly_memory_search, codly_search,
    codly_kg_query). Codly has no embeddings yet, so "meaning" is term weighting, not vectors.
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import type { ExactHit } from '../../lib/gen/ExactHit';
  import type { HitKind } from '../../lib/gen/HitKind';
  import type { Memory } from '../../lib/gen/Memory';
  import type { SearchResult } from '../../lib/gen/SearchResult';
  import type { Triple } from '../../lib/gen/Triple';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago } from '../../lib/util/format';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import Provenance from '../../components/Provenance.svelte';

  type Mode = 'exact' | 'meaning';
  let mode = $state<Mode>('exact');
  let q = $state('');
  let busy = $state(false);
  let failed = $state<string | null>(null);
  let exact = $state<ExactHit[] | null>(null);
  let meaning = $state<{ memories: Memory[]; code: SearchResult | null; facts: Triple[] } | null>(null);
  let el: HTMLInputElement | undefined = $state();
  let seq = 0;

  const KIND: Record<HitKind, string> = { board: 'Whiteboard', task: 'Task', memory: 'Memory' };
  const who = (id: string | null) => (id ? (store.agentMap.get(id)?.name ?? (id === 'user' ? 'You' : id === 'codly' ? 'Codly' : null)) : null);

  /** Facts of the knowledge graph that mention any word of the query (the same data as codly_kg_query). */
  function facts(query: string): Triple[] {
    const words = query.toLowerCase().split(/\W+/).filter((w) => w.length >= 3);
    if (!words.length) return [];
    return store.triples
      .filter((t) => t.valid_to == null && words.some((w) => `${t.subject} ${t.predicate} ${t.object}`.toLowerCase().includes(w)))
      .slice(0, 8);
  }
  function entity(e: string): string {
    const [t, ...rest] = e.split(':');
    const v = rest.join(':');
    return t === 'agent' ? (store.agentMap.get(v)?.name ?? v) : t === 'task' ? (store.taskMap.get(v)?.key ?? v) : v || e;
  }

  $effect(() => {
    const query = q.trim();
    const m = mode;
    if (!query) {
      exact = meaning = null;
      failed = null;
      busy = false;
      return;
    }
    const my = ++seq;
    busy = true;
    const t = setTimeout(async () => {
      try {
        if (m === 'exact') {
          const r = await api.exactSearch(query);
          if (my === seq) exact = r;
        } else {
          const [memories, code] = await Promise.all([api.memories(query), api.search(query).catch(() => null)]);
          if (my === seq) meaning = { memories: memories.slice(0, 12), code, facts: facts(query) };
        }
        if (my === seq) failed = null;
      } catch (e) {
        if (my === seq) failed = e instanceof Error ? e.message : String(e);
      } finally {
        if (my === seq) busy = false;
      }
    }, 180);
    return () => clearTimeout(t);
  });
  $effect(() => {
    el?.focus();
  });

  /** The snippet split around the match; offsets are in characters (code points), not UTF-16 units. */
  function parts(h: ExactHit): [string, string, string] {
    const c = Array.from(h.snippet);
    return [c.slice(0, h.at).join(''), c.slice(h.at, h.at + h.len).join(''), c.slice(h.at + h.len).join('')];
  }
  function openHit(h: ExactHit) {
    if (h.kind === 'task' && store.taskMap.has(h.id)) ui.selectTask(h.id);
  }
</script>

<div class="fl">
  <div class="top">
    <label class="q"><Icon name="search" size={16} /><input bind:this={el} bind:value={q} placeholder={mode === 'exact' ? 'The exact words — a name, a flag, a sentence' : 'Describe what you need — “how do we handle theme tokens”'} spellcheck="false" />{#if busy}<span class="muted">Searching…</span>{/if}</label>
    <div class="lenses" role="radiogroup" aria-label="Search mode">
      <button role="radio" aria-checked={mode === 'exact'} data-tip="Literal text, case-insensitive" onclick={() => (mode = 'exact')}>Exact</button>
      <button role="radio" aria-checked={mode === 'meaning'} data-tip="Ranked by relevance, like the agents’ own search" onclick={() => (mode = 'meaning')}>Meaning</button>
    </div>
  </div>
  <div class="scroll">
    {#if failed}
      <EmptyState compact title="Search failed." body={failed} />
    {:else if mode === 'exact'}
      {#if !exact}
        <EmptyState compact title="Search the board, tasks and memory." body="Finds the words you remember, wherever the team wrote them: whiteboard notes, task goals and summaries, memories." />
      {:else if !exact.length}
        <EmptyState compact title={`Nothing says “${q.trim()}”.`} body="Exact search matches the whole phrase. Try fewer words, or switch to Meaning." />
      {:else}
        <p class="head num">{exact.length} {exact.length === 1 ? 'match' : 'matches'}</p>
        <ul class="hits">
          {#each exact as h (h.kind + h.id)}
            {@const [a, m, b] = parts(h)}
            <li>
              <button class="hit" class:static={h.kind !== 'task'} onclick={() => openHit(h)}>
                <span class="kind k-{h.kind}">{KIND[h.kind]}</span>
                <span class="body">
                  <span class="title truncate">{h.title}</span>
                  <span class="snip selectable">{a}<mark>{m}</mark>{b}</span>
                  <span class="meta"><span>in {h.field}</span>{#if who(h.who)}<span>{who(h.who)}</span>{/if}<span class="num">{ago(h.ts, ui.now)}</span></span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if !meaning}
      <EmptyState compact title="Ask what the team knows." body="Memories, code and knowledge-graph facts, ranked the way agents see them when they search." />
    {:else}
      <section>
        <h2 class="eyebrow">Memories <span class="num">{meaning.memories.length}</span></h2>
        {#each meaning.memories as m (m.id)}
          <p class="mem selectable"><span class="kind k-{m.kind === 'note' ? 'board' : 'memory'}">{m.kind}</span>{m.text}<span class="meta"><span>{who(m.author) ?? m.author}</span><span class="num">{ago(m.created_at, ui.now)}</span></span></p>
        {:else}
          <p class="none">Nothing remembered about this.</p>
        {/each}
      </section>
      <section>
        <h2 class="eyebrow">Code {#if meaning.code}<span class="num">{meaning.code.citations.length}</span><Provenance kind="observed" compact />{/if}</h2>
        {#each meaning.code?.citations ?? [] as c (c.id)}
          <button class="cite" onpointerenter={() => ui.setHover({ path: c.path })} onpointerleave={() => ui.setHover(null)} onclick={() => ui.selectFile(c.path)}>
            <code>{c.path}<span class="muted">:{c.start_line}–{c.end_line}</span></code>{#if c.symbol}<span class="sym">{c.symbol}</span>{/if}
          </button>
        {:else}
          <p class="none">No code hits{meaning.code ? ` (${meaning.code.confidence} confidence)` : ' — the index is not ready'}.</p>
        {/each}
      </section>
      <section>
        <h2 class="eyebrow">Facts <span class="num">{meaning.facts.length}</span></h2>
        {#each meaning.facts as t (t.id)}
          <p class="fact"><span>{entity(t.subject)}</span><span class="pred">{t.predicate.replace(/_/g, ' ')}</span><span>{entity(t.object)}</span></p>
        {:else}
          <p class="none">No facts mention it.</p>
        {/each}
      </section>
    {/if}
  </div>
</div>

<style>
  .fl { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 0 28px 24px; }
  .top { display: flex; align-items: center; gap: 12px; margin-bottom: 18px; }
  .q { flex: 1; display: flex; align-items: center; gap: 10px; height: 48px; padding: 0 18px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-3); }
  .q input { flex: 1; border: 0; outline: 0; background: transparent; font-size: 15px; color: var(--ink-1); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .head { margin-bottom: 10px; font-size: var(--t-sm); color: var(--ink-3); }
  .hits { display: flex; flex-direction: column; gap: 4px; }
  .hit { display: flex; align-items: flex-start; gap: 14px; width: 100%; padding: 10px 14px; border-radius: var(--r-md); text-align: left; }
  .hit:hover { background: var(--surface-1); }
  .hit.static { cursor: default; }
  .kind { flex-shrink: 0; min-width: 76px; padding-top: 2px; font-size: var(--t-xs); font-weight: var(--w-semi); text-transform: uppercase; letter-spacing: 0.06em; }
  .k-board { color: var(--amber-ink); }
  .k-task { color: var(--aurora-m); }
  .k-memory { color: var(--ink-2); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .title { font-size: var(--t-md); font-weight: var(--w-med); color: var(--ink-1); }
  .snip { font-size: var(--t-sm); line-height: 18px; color: var(--ink-2); overflow-wrap: anywhere; }
  mark { background: var(--amber-soft); color: var(--ink-1); border-radius: 3px; padding: 0 1px; }
  .meta { display: flex; gap: 10px; font-size: var(--t-xs); color: var(--ink-3); }
  section { margin-bottom: 22px; }
  section .eyebrow { margin-bottom: 8px; display: flex; align-items: center; gap: 8px; }
  .mem { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 12px; padding: 8px 14px; font-size: var(--t-md); line-height: 20px; color: var(--ink-1); }
  .mem .kind { min-width: 0; padding: 0; }
  .mem .meta { margin-left: auto; }
  .cite { display: flex; align-items: baseline; gap: 12px; width: 100%; padding: 7px 14px; border-radius: var(--r-md); text-align: left; }
  .cite:hover { background: var(--surface-1); }
  .cite code { font-size: 12.5px; color: var(--ink-1); }
  .sym { font-size: var(--t-sm); color: var(--aurora-m); }
  .fact { display: flex; gap: 8px; padding: 6px 14px; font-size: var(--t-md); color: var(--ink-1); }
  .pred { color: var(--ink-3); }
  .none { padding: 4px 14px; font-size: var(--t-sm); color: var(--ink-3); }
</style>
