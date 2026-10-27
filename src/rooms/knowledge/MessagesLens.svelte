<!--
  Who talks to whom. Nodes are agents (plus you and the whiteboard); an edge is a kind of message
  between two of them, coloured by its act and thickened by how many there were. Built from the
  chats and notes Codly already records. The act is a keyword guess (Inferred), not model prose.
  Click a legend chip to hide an act; hover a node to follow its traffic.
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import type { MessageGraph } from '../../lib/gen/MessageGraph';
  import type { MsgAct } from '../../lib/gen/MsgAct';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import { circleLayout, edgeOffsets, edgePath } from '../../lib/util/msgLayout';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import InlineText from '../../components/InlineText.svelte';
  import Provenance from '../../components/Provenance.svelte';
  import { ACT, ACTS } from './acts';

  const R = 22;
  let graph = $state<MessageGraph>({ nodes: [], edges: [] });
  let loaded = $state(false);
  let busy = $state(false);
  let failed = $state<string | null>(null);
  let hidden = $state<MsgAct[]>([]);
  let hover = $state<string | null>(null);
  let box: HTMLDivElement | undefined = $state();
  let size = $state({ w: 640, h: 440 });

  let seq = 0;
  async function refresh() {
    const my = ++seq;
    busy = true;
    try {
      const g = await api.messageGraph();
      if (my === seq) {
        graph = g;
        failed = null;
      }
    } catch (e) {
      if (my === seq) failed = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === seq) {
        busy = false;
        loaded = true;
      }
    }
  }
  // Load on open, and again (debounced) whenever the team talks or writes a note.
  $effect(() => {
    void store.chats.length;
    void store.memories.length;
    const t = setTimeout(refresh, loaded ? 400 : 0);
    return () => clearTimeout(t);
  });
  $effect(() => {
    const el = box;
    if (!el) return;
    const m = () => (size = { w: Math.max(300, el.clientWidth), h: Math.max(260, el.clientHeight) });
    m();
    const ro = new ResizeObserver(m);
    ro.observe(el);
    return () => ro.disconnect();
  });

  const pos = $derived(circleLayout(graph.nodes.map((n) => n.id), size.w, size.h));
  const shown = $derived(graph.edges.filter((e) => !hidden.includes(e.act) && pos.has(e.from) && pos.has(e.to)));
  const geo = $derived.by(() => {
    const off = edgeOffsets(shown);
    return shown.map((e, i) => ({ e, ...edgePath(pos.get(e.from)!, pos.get(e.to)!, off[i], R) }));
  });
  const counts = $derived(Object.fromEntries(ACTS.map((a) => [a.id, graph.edges.filter((e) => e.act === a.id).reduce((n, e) => n + e.count, 0)])) as Record<MsgAct, number>);
  const total = $derived(graph.edges.reduce((n, e) => n + e.count, 0));
  const label = (id: string) => graph.nodes.find((n) => n.id === id)?.label ?? id;
  const latest = $derived([...shown].sort((a, b) => b.last_ts - a.last_ts));
  const touches = (e: { from: string; to: string }) => !hover || e.from === hover || e.to === hover;
  const toggle = (a: MsgAct) => (hidden = hidden.includes(a) ? hidden.filter((x) => x !== a) : [...hidden, a]);
  function open(id: string, kind: string) {
    if (kind === 'agent' && store.agentMap.has(id)) ui.selectAgent(id);
  }
</script>

<div class="mg">
  <div class="bar">
    <div class="legend" role="group" aria-label="Message acts">
      {#each ACTS as a (a.id)}
        <button class="chip" class:off={hidden.includes(a.id)} aria-pressed={!hidden.includes(a.id)} data-tip={a.tip} onclick={() => toggle(a.id)}>
          <span class="sw" style:background={a.color}></span>{a.label}<span class="n num">{counts[a.id]}</span>
        </button>
      {/each}
    </div>
    <span class="grow"></span>
    <span class="muted">Acts are keyword guesses</span><Provenance kind="inferred" />
    <button class="btn sm" onclick={refresh} disabled={busy} aria-label="Refresh the graph"><Icon name="refresh" size={13} />{busy ? 'Refreshing…' : 'Refresh'}</button>
  </div>

  {#if failed}
    <EmptyState compact title="Couldn’t build the graph." body={failed} />
  {:else if loaded && !total}
    <EmptyState compact title="No messages yet." body="When you talk to the Lead, workers ask it questions, or agents post notes, the traffic shows here." />
  {:else}
    <div class="body">
      <div class="canvas" bind:this={box}>
        <svg viewBox="0 0 {size.w} {size.h}" width={size.w} height={size.h} role="img" aria-label="Messages between agents, {total} in all">
          <defs>
            {#each ACTS as a (a.id)}
              <marker id="arrow-{a.id}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto" markerUnits="userSpaceOnUse"><path d="M0,0.5 L8,4 L0,7.5 z" style:fill={a.color} /></marker>
            {/each}
          </defs>
          {#each geo as g (g.e.from + g.e.to + g.e.act)}
            <path class="edge" class:dim={!touches(g.e)} d={g.d} style:stroke={ACT[g.e.act].color} stroke-width={1.4 + Math.min(g.e.count, 6) * 0.45} marker-end="url(#arrow-{g.e.act})"><title>{label(g.e.from)} → {label(g.e.to)} · {ACT[g.e.act].label} ×{g.e.count}: {g.e.sample}</title></path>
            {#if g.e.count > 1}<text class="cnt num" class:dim={!touches(g.e)} x={g.mid.x} y={g.mid.y - 4} text-anchor="middle">{g.e.count}</text>{/if}
          {/each}
          {#each graph.nodes as n (n.id)}
            {@const p = pos.get(n.id)}
            {#if p}
              <g class="node" class:dim={hover && hover !== n.id && !shown.some((e) => (e.from === hover && e.to === n.id) || (e.to === hover && e.from === n.id))} role="button" tabindex="0" aria-label={n.label}
                onpointerenter={() => (hover = n.id)} onpointerleave={() => (hover = null)} onfocus={() => (hover = n.id)} onblur={() => (hover = null)}
                ondblclick={() => open(n.id, n.kind)} onkeydown={(e) => e.key === 'Enter' && open(n.id, n.kind)}>
                {#if n.kind === 'board'}
                  <rect x={p.x - R} y={p.y - R * 0.7} width={R * 2} height={R * 1.4} rx="6" class="board" />
                {:else}
                  <circle cx={p.x} cy={p.y} r={R} class:user={n.kind === 'user'} style:fill={n.color != null ? idVar(n.color) : 'var(--surface-3)'} />
                {/if}
                <text class="init" class:ink={n.color == null} x={p.x} y={p.y + 4.5} text-anchor="middle">{n.kind === 'board' ? '' : n.label.slice(0, 1).toUpperCase()}</text>
                <text class="name" x={p.x} y={p.y + R + 15} text-anchor="middle">{n.label}</text>
                {#if n.title}<text class="role" x={p.x} y={p.y + R + 28} text-anchor="middle">{n.title}</text>{/if}
              </g>
            {/if}
          {/each}
        </svg>
      </div>
      <aside class="side" aria-label="Latest messages">
        <h2 class="eyebrow">Latest</h2>
        <ul>
          {#each latest as e (e.from + e.to + e.act)}
            <li class:dim={!touches(e)}>
              <p class="who"><span class="sw" style:background={ACT[e.act].color}></span>{label(e.from)} → {label(e.to)}<span class="muted">{ACT[e.act].label}{e.count > 1 ? ` ×${e.count}` : ''}</span></p>
              <p class="sample selectable"><InlineText text={e.sample} /></p>
              <p class="when num">{ago(e.last_ts, ui.now)}</p>
            </li>
          {/each}
        </ul>
      </aside>
    </div>
  {/if}
</div>

<style>
  .mg { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 0 28px 24px; }
  .bar { display: flex; align-items: center; gap: 10px; padding-bottom: 14px; flex-wrap: wrap; }
  .grow { flex: 1; }
  .legend { display: flex; gap: 6px; flex-wrap: wrap; }
  .chip { display: inline-flex; align-items: center; gap: 7px; height: 26px; padding: 0 11px; border-radius: var(--r-full); background: var(--surface-1); font-size: var(--t-sm); color: var(--ink-2); transition: opacity var(--d-2); }
  .chip:hover { background: var(--surface-2); }
  .chip.off { opacity: 0.45; }
  .chip .n { color: var(--ink-3); }
  .sw { width: 9px; height: 9px; border-radius: 3px; flex-shrink: 0; }
  .body { flex: 1; min-height: 0; display: flex; gap: 16px; }
  .canvas { flex: 1; min-width: 0; min-height: 0; position: relative; border-radius: var(--r-lg); background: var(--surface-1); overflow: hidden; }
  svg { position: absolute; inset: 0; display: block; }
  .edge { fill: none; stroke-linecap: round; opacity: 0.85; transition: opacity var(--d-2); }
  .edge.dim, .cnt.dim, .node.dim { opacity: 0.18; }
  .cnt { font-size: 10px; fill: var(--ink-2); paint-order: stroke; stroke: var(--night); stroke-width: 3px; }
  .node { cursor: default; outline: none; transition: opacity var(--d-2); }
  .node circle { stroke: var(--night); stroke-width: 2px; }
  .node circle.user { stroke: var(--line-3); stroke-dasharray: 3 3; }
  .node:focus-visible circle, .node:focus-visible rect { stroke: var(--focus); }
  .board { fill: var(--amber-soft); stroke: var(--amber); stroke-width: 1.5px; }
  .init { font-size: 13px; font-weight: var(--w-semi); fill: var(--on-aurora); pointer-events: none; }
  .init.ink { fill: var(--ink-2); }
  .name { font-size: 12px; font-weight: var(--w-med); fill: var(--ink-1); pointer-events: none; }
  .role { font-size: 10.5px; fill: var(--ink-3); pointer-events: none; }
  .side { width: 300px; flex-shrink: 0; min-height: 0; overflow-y: auto; }
  .side ul { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
  .side li { padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); transition: opacity var(--d-2); }
  .side li.dim { opacity: 0.35; }
  .who { display: flex; align-items: center; gap: 7px; font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-1); flex-wrap: wrap; }
  .who .muted { margin-left: auto; font-weight: var(--w-reg); }
  .sample { margin-top: 4px; font-size: var(--t-sm); line-height: 17px; color: var(--ink-2); display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .when { margin-top: 4px; font-size: var(--t-xs); color: var(--ink-3); }
  @media (max-width: 900px) { .body { flex-direction: column; } .side { width: auto; max-height: 200px; } }
</style>
