<!--
  The knowledge graph: facts Codly derived from real events (who edited what, what depends on
  what) and facts agents added. Hover a node to see its neighbours; click to filter the list;
  double-click an agent or task to open it; a file highlights wherever it shows.
-->
<script lang="ts">
  import type { Triple } from '../../lib/gen/Triple';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { idVar } from '../../lib/util/labels';
  import { basename } from '../../lib/util/sanitize';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import Provenance from '../../components/Provenance.svelte';
  import { forceLayout } from './forceLayout';

  const triples = $derived(store.triples.filter((t) => t.valid_to == null));
  const type = (e: string) => e.split(':')[0];
  function label(e: string): string {
    const [t, ...rest] = e.split(':');
    const v = rest.join(':');
    if (t === 'agent') return store.agentMap.get(v)?.name ?? 'agent';
    if (t === 'task') return store.taskMap.get(v)?.key ?? 'task';
    if (t === 'file') return basename(v);
    return v || e;
  }
  function color(e: string): string {
    const t = type(e);
    if (t === 'agent') return idVar(store.agentMap.get(e.slice(6))?.color ?? 0);
    if (t === 'task') return 'var(--ink-2)';
    if (t === 'file') return 'var(--aurora-m)';
    return 'var(--pv-model)';
  }
  let box: HTMLDivElement | undefined = $state();
  let size = $state({ w: 700, h: 480 });
  $effect(() => {
    const el = box;
    if (!el) return;
    const m = () => (size = { w: Math.max(260, el.clientWidth), h: Math.max(220, el.clientHeight) });
    m();
    const ro = new ResizeObserver(m);
    ro.observe(el);
    return () => ro.disconnect();
  });
  const graph = $derived.by(() => {
    const ids = [...new Set(triples.flatMap((t) => [t.subject, t.object]))];
    return { ids, pos: forceLayout(ids, triples.map((t) => ({ s: t.subject, t: t.object })), size.w, size.h, { pad: 60, padBottom: 60 }) };
  });
  let hover = $state<string | null>(null);
  const focus = $derived(hover ?? ui.memoryEntity);
  const near = $derived.by(() => {
    if (!focus) return null;
    const s = new Set([focus]);
    for (const t of triples) {
      if (t.subject === focus) s.add(t.object);
      if (t.object === focus) s.add(t.subject);
    }
    return s;
  });
  const listed = $derived(ui.memoryEntity ? triples.filter((t) => t.subject === ui.memoryEntity || t.object === ui.memoryEntity) : triples);
  const prov = (t: Triple) => (t.source === 'observed' ? 'observed' : t.source === 'user' ? 'provided' : 'model') as 'observed' | 'provided' | 'model';
  function enter(id: string) {
    hover = id;
    const [t, ...rest] = id.split(':');
    const v = rest.join(':');
    ui.setHover(t === 'agent' ? { agentId: v } : t === 'task' ? { taskId: v } : t === 'file' ? { path: v } : null);
  }
  function leave() {
    hover = null;
    ui.setHover(null);
  }
  function open(id: string) {
    const [t, ...rest] = id.split(':');
    const v = rest.join(':');
    if (t === 'agent' && store.agentMap.has(v)) ui.selectAgent(v);
    else if (t === 'task' && store.taskMap.has(v)) ui.selectTask(v);
    else if (t === 'file') ui.selectFile(v);
  }
</script>

{#if !triples.length}
  <EmptyState title="No knowledge yet." body="Codly records facts from real events — who edited what, which task depends on which — and agents add their own." />
{:else}
  <div class="gl">
    <div class="gbox" bind:this={box}>
      <svg width={size.w} height={size.h} role="img" aria-label="Knowledge graph">
        {#each triples as t (t.id)}
          {@const a = graph.pos.get(t.subject)}
          {@const b = graph.pos.get(t.object)}
          {#if a && b}
            {@const lit = !!near && (t.subject === focus || t.object === focus)}
            <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} class="edge" class:lit class:faded={!!near && !lit} class:model={t.source !== 'observed'} />
            {#if lit || triples.length <= 12}<text x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 5} class="el" class:lit text-anchor="middle">{t.predicate.replace(/_/g, ' ')}</text>{/if}
          {/if}
        {/each}
        {#each graph.ids as id (id)}
          {@const p = graph.pos.get(id)}
          {#if p}
            <g class="node t-{type(id)}" class:faded={!!near && !near.has(id)} class:sel={ui.memoryEntity === id} transform="translate({p.x} {p.y})" style:--c={color(id)}
              role="button" tabindex="0" aria-label={label(id)}
              onpointerenter={() => enter(id)} onpointerleave={leave}
              onclick={() => (ui.memoryEntity = ui.memoryEntity === id ? null : id)} ondblclick={() => open(id)} onkeydown={(e) => e.key === 'Enter' && open(id)}>
              {#if type(id) === 'agent'}<circle r="9" class="shape" />{:else if type(id) === 'task'}<rect x="-8" y="-8" width="16" height="16" rx="4" class="shape" />{:else if type(id) === 'file'}<circle r="5.5" class="shape" />{:else}<path d="M0 -8 L8 0 L0 8 L-8 0Z" class="shape" />{/if}
              <text y="22" text-anchor="middle" class="nl">{label(id)}</text>
            </g>
          {/if}
        {/each}
      </svg>
      <p class="legend"><span><i class="lg-a"></i>agent</span><span><i class="lg-t"></i>task</span><span><i class="lg-f"></i>file</span><span><i class="lg-c"></i>idea</span><span class="hint">Double-click to open</span></p>
    </div>
    <aside class="facts">
      <p class="eyebrow">{listed.length} facts{#if ui.memoryEntity} about <button class="chip" onclick={() => (ui.memoryEntity = null)}>{label(ui.memoryEntity)} <Icon name="x" size={10} /></button>{/if}</p>
      <ul>
        {#each listed as t (t.id)}
          <li><Provenance kind={prov(t)} compact /><span class="s">{label(t.subject)}</span><em>{t.predicate.replace(/_/g, ' ')}</em><span class="s">{label(t.object)}</span></li>
        {/each}
      </ul>
    </aside>
  </div>
{/if}

<style>
  .gl { flex: 1; min-height: 0; display: flex; gap: 8px; padding: 0 20px 20px 28px; }
  .gbox { position: relative; flex: 1; min-width: 0; border-radius: var(--r-lg); background: var(--surface-1); overflow: hidden; }
  svg { display: block; }
  .edge { stroke: var(--line-3); stroke-width: 1.2; transition: opacity var(--d-2); }
  .edge.model { stroke-dasharray: 3 4; }
  .edge.lit { stroke: var(--ink-2); stroke-width: 1.6; }
  .edge.faded, .node.faded { opacity: 0.18; }
  .el { font-size: 10px; fill: var(--ink-3); pointer-events: none; }
  .el.lit { fill: var(--ink-1); }
  .node { cursor: pointer; transition: opacity var(--d-2); outline: none; }
  .shape { fill: var(--c); stroke: var(--glass-solid); stroke-width: 2; }
  .t-file .shape, .t-concept .shape { fill: color-mix(in srgb, var(--c) 80%, transparent); }
  .node.sel .shape, .node:focus-visible .shape { stroke: var(--ink-1); }
  .nl { font-size: 11px; fill: var(--ink-2); font-family: var(--font-ui); pointer-events: none; }
  .t-file .nl { font-family: var(--font-mono); font-size: 10.5px; }
  .legend { position: absolute; left: 14px; bottom: 12px; display: flex; gap: 14px; font-size: var(--t-xs); color: var(--ink-3); }
  .legend span { display: inline-flex; align-items: center; gap: 5px; }
  .legend i { display: inline-block; width: 9px; height: 9px; border-radius: 50%; }
  .lg-a { background: var(--id-7); }
  .lg-t { background: var(--ink-2); border-radius: 2px !important; }
  .lg-f { background: var(--aurora-m); }
  .lg-c { background: var(--pv-model); transform: rotate(45deg); border-radius: 1px !important; }
  .hint { color: var(--ink-4); }
  .facts { width: 300px; flex-shrink: 0; overflow-y: auto; padding: 4px 8px; }
  .facts .eyebrow { display: flex; align-items: center; gap: 6px; margin-bottom: 10px; }
  .facts ul { display: flex; flex-direction: column; gap: 8px; }
  .facts li { display: flex; align-items: center; gap: 6px; font-size: var(--t-sm); flex-wrap: wrap; }
  .s { color: var(--ink-1); }
  em { font-style: normal; color: var(--ink-3); }
</style>
