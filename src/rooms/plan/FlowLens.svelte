<!--
  Flow: the plan's DAG left → right. Edges are dependencies; a live edge (into running work)
  carries the aurora; the critical path is drawn heavier. Cards are the tasks.
-->
<script lang="ts">
  import type { Task } from '../../lib/gen/Task';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { layoutDag } from '../../lib/util/dag';
  import { GLOSSARY } from '../../lib/util/labels';
  import TaskNode from './TaskNode.svelte';

  interface Props {
    tasks: Task[];
  }
  let { tasks }: Props = $props();
  const W = 290;
  const H = 150;
  const layout = $derived(layoutDag(tasks.map((t) => ({ id: t.id, deps: t.depends_on, done: t.status === 'done' })), { nodeW: W, nodeH: H, gapX: 96, gapY: 26 }));
  const live = (t: Task | undefined) => !!t && ['running', 'verifying', 'reviewing', 'merging'].includes(t.status);
  const edges = $derived(
    layout.edges.map((e) => {
      const a = layout.nodes.get(e.from)!;
      const b = layout.nodes.get(e.to)!;
      const x1 = a.x + W;
      const y1 = a.y + H / 2;
      const x2 = b.x;
      const y2 = b.y + H / 2;
      const mx = (x1 + x2) / 2;
      const from = store.taskMap.get(e.from);
      const to = store.taskMap.get(e.to);
      const lit = ui.hover.taskId === e.from || ui.hover.taskId === e.to;
      return {
        key: `${e.from}-${e.to}`,
        d: `M${x1 + 2} ${y1} C${mx} ${y1}, ${mx} ${y2}, ${x2 - 8} ${y2}`,
        tip: { x: x2 - 8, y: y2 },
        state: live(to) ? 'live' : from?.status === 'done' && to?.status === 'done' ? 'done' : from?.status === 'done' ? 'met' : 'wait',
        critical: e.critical,
        lit,
      };
    }),
  );
  const uid = $props.id();
</script>

<div class="flow">
  <div class="canvas" style:width="{layout.width}px" style:height="{layout.height}px">
    <svg width={layout.width} height={layout.height} aria-hidden="true">
      <defs>
        <linearGradient id="ag-{uid}" x1="0" x2="1" y1="0" y2="0">
          <stop offset="0" style="stop-color: var(--aurora-a)" /><stop offset="0.5" style="stop-color: var(--aurora-m)" /><stop offset="1" style="stop-color: var(--aurora-b)" />
        </linearGradient>
      </defs>
      {#each edges as e (e.key)}
        <path class="edge e-{e.state}" class:crit={e.critical} class:lit={e.lit} d={e.d} stroke={e.state === 'live' ? `url(#ag-${uid})` : undefined} />
        <circle class="tip e-{e.state}" cx={e.tip.x + 3} cy={e.tip.y} r="3" />
      {/each}
    </svg>
    {#each tasks as t (t.id)}
      {@const p = layout.nodes.get(t.id)}
      {#if p}<div class="slot" style:left="{p.x}px" style:top="{p.y}px" style:width="{W}px" style:height="{H}px"><TaskNode task={t} /></div>{/if}
    {/each}
  </div>
  {#if layout.critical.size > 1}<p class="legend" data-tip={GLOSSARY.critical}><span class="lc"></span>Critical path</p>{/if}
</div>

<style>
  /* a plotting chart: faint dots, like the grid of an observatory plate */
  .flow { position: relative; flex: 1; min-height: 0; overflow: auto; display: grid; place-items: center; padding: 32px 40px 56px; margin: 0 16px 16px; border-radius: var(--r-lg); background-image: radial-gradient(circle, var(--line-2) 1px, transparent 1.2px); background-size: 26px 26px; background-position: 13px 13px; }
  .canvas { position: relative; }
  svg { position: absolute; inset: 0; overflow: visible; }
  .edge { fill: none; stroke: var(--line-3); stroke-width: 1.5; transition: stroke var(--d-3), stroke-width var(--d-3); }
  .edge.crit { stroke-width: 2.5; }
  .edge.e-wait { stroke-dasharray: 4 5; }
  .edge.e-done, .edge.e-met { stroke: color-mix(in srgb, var(--ok) 55%, transparent); }
  .edge.e-live { stroke-width: 2.5; stroke-dasharray: 10 8; animation: flow-dash 1.2s linear infinite; }
  .edge.lit { stroke-width: 3; }
  .tip { fill: var(--line-3); }
  .tip.e-done, .tip.e-met { fill: color-mix(in srgb, var(--ok) 70%, transparent); }
  .tip.e-live { fill: var(--aurora-b); }
  @keyframes flow-dash { to { stroke-dashoffset: -36; } }
  .slot { position: absolute; display: flex; }
  .slot > :global(*) { height: 100%; }
  .legend { position: absolute; left: 28px; bottom: 16px; display: flex; align-items: center; gap: 8px; font-size: var(--t-sm); color: var(--ink-3); }
  .lc { width: 22px; height: 2.5px; border-radius: 2px; background: var(--line-3); }
</style>
