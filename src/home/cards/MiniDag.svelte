<!-- A plan's tasks as chips flowing left → right, joined by their dependencies. -->
<script lang="ts">
  import type { Task } from '../../lib/gen/Task';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { layoutDag } from '../../lib/util/dag';
  import { idVar, PROVIDER_LABEL } from '../../lib/util/labels';

  interface Props {
    tasks: Task[];
    width?: number;
  }
  let { tasks, width = 600 }: Props = $props();

  const GAP_X = 34;
  const layers = $derived.by(() => {
    const l = layoutDag(tasks.map((t) => ({ id: t.id, deps: t.depends_on, done: t.status === 'done' })), { nodeW: 1, nodeH: 1, gapX: 0, gapY: 0 });
    return Math.max(1, ...[...l.nodes.values()].map((n) => n.layer + 1));
  });
  const nodeW = $derived(Math.min(250, Math.floor((width - (layers - 1) * GAP_X) / layers)));
  const layout = $derived(
    layoutDag(tasks.map((t) => ({ id: t.id, deps: t.depends_on, done: t.status === 'done' })), { nodeW, nodeH: 46, gapX: GAP_X, gapY: 8 }),
  );
  const edges = $derived(
    layout.edges.map((e) => {
      const a = layout.nodes.get(e.from)!;
      const b = layout.nodes.get(e.to)!;
      const x1 = a.x + nodeW;
      const y1 = a.y + 23;
      const x2 = b.x;
      const y2 = b.y + 23;
      const mx = (x1 + x2) / 2;
      return { key: `${e.from}-${e.to}`, d: `M${x1} ${y1} C${mx} ${y1}, ${mx} ${y2}, ${x2 - 4} ${y2}` };
    }),
  );
</script>

<div class="dag" style:width="{layout.width}px" style:height="{layout.height}px">
  <svg width={layout.width} height={layout.height} aria-hidden="true">
    {#each edges as e (e.key)}<path d={e.d} />{/each}
  </svg>
  {#each tasks as t (t.id)}
    {@const p = layout.nodes.get(t.id)}
    {@const a = t.assignee ? store.agentMap.get(t.assignee) : null}
    {#if p}
      <button
        class="chip-t s-{t.status}"
        class:lit={ui.hover.taskId === t.id || (a && ui.hover.agentId === a.id)}
        style:left="{p.x}px"
        style:top="{p.y}px"
        style:width="{nodeW}px"
        style:--hue={a ? idVar(a.color) : 'var(--line-3)'}
        onclick={() => ui.selectTask(t.id)}
        onpointerenter={() => ui.setHover({ taskId: t.id })}
        onpointerleave={() => ui.setHover(null)}
      >
        <span class="top"><span class="key">{t.key}</span><span class="title truncate">{t.title}</span></span>
        <span class="who truncate">{a ? `${a.name} · ${PROVIDER_LABEL[a.provider]}` : t.agent_hint ? PROVIDER_LABEL[t.agent_hint] : 'Any agent'}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .dag { position: relative; max-width: 100%; }
  svg { position: absolute; inset: 0; overflow: visible; }
  path { fill: none; stroke: var(--line-3); stroke-width: 1.4; }
  .chip-t { position: absolute; height: 46px; display: flex; flex-direction: column; justify-content: center; gap: 2px; padding: 0 12px 0 14px; border-radius: 12px; text-align: left; background: var(--surface-2); box-shadow: inset 3px 0 0 var(--hue); transition: background-color var(--d-2) var(--ease-out), box-shadow var(--d-2) var(--ease-out); }
  .chip-t:hover, .chip-t.lit { background: var(--surface-3); }
  .top { display: flex; align-items: baseline; gap: 7px; min-width: 0; }
  .key { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .title { font-size: var(--t-md); font-weight: var(--w-med); color: var(--ink-1); }
  .who { font-size: var(--t-xs); color: var(--ink-3); }
  .s-done .title { color: var(--ink-2); }
  .s-running, .s-verifying, .s-merging { box-shadow: inset 3px 0 0 var(--hue), inset 0 0 0 1px color-mix(in srgb, var(--live) 35%, transparent); }
</style>
