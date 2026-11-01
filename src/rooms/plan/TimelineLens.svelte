<!--
  Timeline: one lane per agent in its hue. Bars are tool calls (writes solid, reads faint,
  commands striped), amber marks are approvals, small ticks are commits, checks and merges.
  The live edge (aurora) follows now until you scroll back. Click a bar → the agent's history
  at that moment. Stopped agents fold under a "Stopped" row below the ones still running.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { narrate } from '../../lib/state/narration';
  import { store, type Seg } from '../../lib/state/store.svelte';
  import { ui, wallNow } from '../../lib/state/ui.svelte';
  import { clock, clockSec, dur } from '../../lib/util/format';
  import { activityClass, idVar, pastVerb, phrase } from '../../lib/util/labels';
  import { oneLine } from '../../lib/util/sanitize';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Say from '../../components/Say.svelte';

  const LANE = 56;
  const FOLD = 36;
  const AXIS = 30;
  const RIGHT = 80;
  const ZOOMS = [0.5, 1, 2, 4, 6, 10, 16, 28];

  let scroller: HTMLDivElement | undefined = $state();
  let viewW = $state(800);
  let follow = $state(true);
  let fit = $state(true);
  let showStopped = $state(false);
  let clockNow = $state(wallNow());
  onMount(() => {
    const id = setInterval(() => document.visibilityState === 'visible' && (clockNow = wallNow()), 250);
    return () => clearInterval(id);
  });
  $effect(() => {
    const el = scroller;
    if (!el) return;
    viewW = el.clientWidth;
    const ro = new ResizeObserver(() => (viewW = el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  });

  const lanes = $derived.by(() => {
    void store.rev;
    void store.liveRev;
    return [...store.agents]
      .sort((a, b) => (a.role === 'lead' ? -1 : b.role === 'lead' ? 1 : a.created_at - b.created_at))
      .map((a) => ({ agent: a, segs: (store.live[a.id]?.segments ?? []).slice(), turns: (store.live[a.id]?.turns ?? []).slice() }));
  });
  // Running agents first; the stopped ones below a fold (open by itself when nobody runs).
  const alive = $derived(lanes.filter((l) => l.agent.status !== 'offline'));
  const stopped = $derived(lanes.filter((l) => l.agent.status === 'offline'));
  const open = $derived(showStopped || !alive.length);
  const foldY = $derived(AXIS + alive.length * LANE);
  const shown = $derived([
    ...alive.map((l, i) => ({ ...l, y: AXIS + i * LANE, off: false })),
    ...(open ? stopped.map((l, i) => ({ ...l, y: foldY + (stopped.length ? FOLD : 0) + i * LANE, off: true })) : []),
  ]);
  type Mark = { agentId: string; ts: number; end: number | null; kind: 'approval' | 'commit' | 'verified' | 'merged' | 'artifact'; ok: boolean; text: string };
  const marks = $derived.by((): Mark[] => {
    void store.rev;
    const out: Mark[] = [];
    for (const ap of store.approvals) {
      const r = store.events.find((e) => e.kind.type === 'approval_resolved' && e.kind.approval_id === ap.id);
      out.push({ agentId: ap.agent_id, ts: ap.created_at, end: r ? r.ts : ap.status === 'pending' ? null : ap.created_at, kind: 'approval', ok: ap.status === 'approved', text: ap.subject.kind === 'command' ? `Asked to run ${oneLine(ap.subject.command, 60)}` : 'Asked you' });
    }
    for (const e of store.events) {
      const k = e.kind;
      if (!e.agent_id) continue;
      if (k.type === 'committed') out.push({ agentId: e.agent_id, ts: e.ts, end: null, kind: 'commit', ok: true, text: `Codly committed ${k.sha.slice(0, 7)} · ${k.files.length} files` });
      else if (k.type === 'verified') out.push({ agentId: e.agent_id, ts: e.ts, end: null, kind: 'verified', ok: k.ok, text: `${k.ok ? 'Verified' : 'Failed'}: ${k.command}` });
      else if (k.type === 'merged') out.push({ agentId: e.agent_id, ts: e.ts, end: null, kind: 'merged', ok: true, text: `Merged into ${k.branch}` });
      else if (k.type === 'artifact_added') out.push({ agentId: e.agent_id, ts: e.ts, end: null, kind: 'artifact', ok: true, text: `Made ${k.artifact.title}` });
    }
    return out;
  });

  const dataT0 = $derived.by(() => {
    let min = Infinity;
    for (const l of lanes) {
      for (const s of l.segs) min = Math.min(min, s.start);
      for (const t of l.turns) min = Math.min(min, t.start);
    }
    return min === Infinity ? clockNow - 60_000 : min - 4000;
  });
  const fitPx = $derived(Math.max(0.05, (viewW - RIGHT - 16) / Math.max(10, (clockNow - dataT0) / 1000)));
  const pxs = $derived(fit ? fitPx : (ZOOMS[ui.timelineZoom] ?? 4));
  const tEnd = $derived(clockNow + (RIGHT / pxs) * 1000);
  const t0 = $derived(Math.min(dataT0, tEnd - (viewW / pxs) * 1000));
  const width = $derived(Math.max(viewW, ((tEnd - t0) / 1000) * pxs));
  const height = $derived(foldY + (stopped.length ? FOLD : 0) + (open ? stopped.length * LANE : 0) + 8);
  const x = (ts: number) => ((ts - t0) / 1000) * pxs;
  const ticks = $derived.by(() => {
    const steps = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
    const s = steps.find((v) => v * pxs >= 96) ?? 3600;
    const start = Math.ceil(t0 / (s * 1000)) * s * 1000;
    const out: number[] = [];
    for (let t = start; t <= tEnd; t += s * 1000) out.push(t);
    return { list: out, s };
  });
  $effect(() => {
    void clockNow;
    void width;
    if (follow && scroller) scroller.scrollLeft = scroller.scrollWidth;
  });
  function onScroll() {
    if (!scroller) return;
    const atEnd = scroller.scrollWidth - scroller.scrollLeft - scroller.clientWidth < 24;
    if (!atEnd && follow) follow = false;
    else if (atEnd && !follow) follow = true;
  }
  function zoom(d: number) {
    const cur = fit ? ZOOMS.findIndex((z) => z >= fitPx) : ui.timelineZoom;
    ui.timelineZoom = Math.max(0, Math.min(ZOOMS.length - 1, (cur < 0 ? ZOOMS.length - 1 : cur) + d));
    fit = false;
  }
  const segTip = (s: Seg) => `${s.end == null ? phrase(s.activity, s.label) : `${pastVerb(s.activity)} ${s.label}`} · ${clockSec(s.start)}${s.end ? ` · ${dur(s.end - s.start)}` : ''}${s.testsPassed != null ? ` · ${s.testsPassed} passed${s.testsFailed ? `, ${s.testsFailed} failed` : ''}` : ''}`;
</script>

{#snippet fold()}
  <button class="fold" style:height="{FOLD}px" aria-expanded={open} disabled={!alive.length} onclick={() => (showStopped = !showStopped)}>
    {#if alive.length}<Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} />{/if}Stopped · {stopped.length}
  </button>
{/snippet}

{#if !lanes.length}
  <EmptyState title="Nothing has happened yet." body="Each agent gets a lane here once it starts working." />
{:else}
  <div class="tl">
    <div class="lanes-h" style:padding-top="{AXIS}px">
      {#each shown as l, i (l.agent.id)}
        {@const n = narrate(l.agent.id)}
        {#if i === alive.length}{@render fold()}{/if}
        <button class="lh" class:off={l.off} class:lit={ui.hover.agentId === l.agent.id} style:height="{LANE}px" onclick={() => ui.selectAgent(l.agent.id)} onpointerenter={() => ui.setHover({ agentId: l.agent.id })} onpointerleave={() => ui.setHover(null)}>
          <Portrait agent={l.agent} size={24} />
          <span class="lt"><b>{l.agent.name}</b><span class="truncate">{#if n}<Say parts={n.parts} />{/if}</span></span>
        </button>
      {/each}
      {#if shown.length === alive.length && stopped.length}{@render fold()}{/if}
    </div>
    <div class="scroller" bind:this={scroller} onscroll={onScroll}>
      <svg {width} {height} role="img" aria-label="Agent activity over time">
        {#each ticks.list as t (t)}
          <line class="grid" x1={x(t)} x2={x(t)} y1={AXIS - 6} y2={height} />
          <text class="tick" x={x(t) + 4} y={AXIS - 12}>{ticks.s < 60 ? clockSec(t).replace(/\s?[AP]M$/i, '') : clock(t)}</text>
        {/each}
        {#if stopped.length}<line class="fold-line" x1="0" x2={width} y1={foldY + FOLD / 2} y2={foldY + FOLD / 2} />{/if}
        {#each shown as l (l.agent.id)}
          {@const y = l.y}
          <g class="lane" class:off={l.off} class:lit={ui.hover.agentId === l.agent.id} style:--hue={idVar(l.agent.color)}>
            <rect class="lane-bg" x="0" {y} {width} height={LANE} />
            {#each l.turns as t, ti (ti)}<rect class="turn" x={x(t.start)} y={y + 10} width={Math.max(2, x(t.end ?? clockNow) - x(t.start))} height={LANE - 20} rx="6" />{/each}
            {#each l.segs as s (s.callId)}
              {@const w = Math.max(3, x(s.end ?? clockNow) - x(s.start))}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <rect class="seg c-{activityClass(s.activity)}" class:open={s.end == null} class:fail={s.ok === false} x={x(s.start)} y={y + 16} width={w} height={LANE - 32} rx="3" role="button" tabindex="-1" data-tip={segTip(s)} onclick={() => ui.selectAgent(s.agentId, s.eventId)} />
            {/each}
            {#each marks.filter((m) => m.agentId === l.agent.id) as m, mi (mi)}
              {#if m.kind === 'approval'}
                <rect class="ap" class:ok={m.ok} x={x(m.ts)} y={y + 6} width={Math.max(3, x(m.end ?? clockNow) - x(m.ts))} height="4" rx="2" data-tip={m.text} />
              {:else}
                <circle class="mk m-{m.kind}" class:bad={!m.ok} cx={x(m.ts)} cy={y + LANE - 8} r="3.5" data-tip={m.text} />
              {/if}
            {/each}
          </g>
        {/each}
        <line class="now" x1={x(clockNow)} x2={x(clockNow)} y1={AXIS - 8} y2={height} />
      </svg>
    </div>
    <div class="tools">
      <button class="icon-btn sm" aria-label="Zoom out" onclick={() => zoom(-1)}><Icon name="minus" size={13} /></button>
      <button class="btn sm ghost" aria-pressed={fit} onclick={() => ((fit = true), (follow = true))}>Fit</button>
      <button class="icon-btn sm" aria-label="Zoom in" onclick={() => zoom(1)}><Icon name="plus" size={13} /></button>
      {#if !follow}<button class="btn sm" onclick={() => (follow = true)}>Back to now</button>{/if}
    </div>
  </div>
{/if}

<style>
  .tl { position: relative; flex: 1; min-height: 0; display: flex; padding: 4px 24px 24px; }
  .lanes-h { width: 240px; flex-shrink: 0; display: flex; flex-direction: column; }
  .lh { display: flex; align-items: center; gap: 10px; padding: 0 12px 0 6px; border-radius: var(--r-md) 0 0 var(--r-md); text-align: left; min-width: 0; }
  .lh:hover, .lh.lit { background: var(--surface-1); }
  .lt { display: flex; flex-direction: column; min-width: 0; font-size: var(--t-sm); color: var(--ink-3); line-height: 16px; }
  .lt b { color: var(--ink-1); font-size: var(--t-md); }
  .lh.off, .lane.off { opacity: 0.55; }
  .lh.off:hover, .lh.off.lit { opacity: 1; }
  .fold { display: flex; align-items: center; gap: 6px; padding: 0 12px 0 8px; font-size: var(--t-sm); font-weight: 600; color: var(--ink-3); text-align: left; }
  .fold:not(:disabled):hover { color: var(--ink-1); }
  .fold-line { stroke: var(--line); stroke-dasharray: 3 4; }
  .scroller { flex: 1; min-width: 0; overflow-x: auto; overflow-y: hidden; }
  svg { display: block; }
  .grid { stroke: var(--line); }
  .tick { fill: var(--ink-3); font-size: 10.5px; font-family: var(--font-ui); font-variant-numeric: tabular-nums; }
  .lane-bg { fill: transparent; }
  .lane.lit .lane-bg { fill: var(--surface-1); }
  .turn { fill: color-mix(in srgb, var(--hue) 7%, transparent); }
  .seg { fill: var(--hue); cursor: pointer; transition: opacity var(--d-2); }
  .seg.c-read, .seg.c-think { opacity: 0.38; }
  .seg.c-exec { fill: color-mix(in srgb, var(--hue) 70%, var(--ink-1)); opacity: 0.85; }
  .seg.c-wait { fill: var(--amber); }
  .seg.fail { fill: var(--bad); opacity: 0.9; }
  .seg.open { opacity: 1; }
  .seg:hover { opacity: 1; }
  .ap { fill: var(--amber); }
  .ap.ok { fill: color-mix(in srgb, var(--amber) 55%, transparent); }
  .mk { fill: var(--ink-2); }
  .mk.m-merged { fill: var(--ok); }
  .mk.m-verified { fill: var(--ok); }
  .mk.m-artifact { fill: var(--aurora-m); }
  .mk.bad { fill: var(--bad); }
  .now { stroke: var(--live); stroke-width: 1.5; }
  .tools { position: absolute; right: 30px; bottom: 30px; display: flex; gap: 4px; align-items: center; padding: 4px; border-radius: var(--r-full); background: var(--pop); box-shadow: var(--pop-shadow); }
</style>
