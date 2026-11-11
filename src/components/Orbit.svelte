<!--
  The orbit: an agent's todo-list progress drawn as a ring around its portrait, with a small dot
  where its drone is along the way. Live = aurora on a teal track, needs you = amber,
  blocked = red, done = settled. Moves only when progress changes; never loops.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Tone } from '../lib/narrate';

  interface Props {
    size: number;
    progress: number | null;
    tone: Tone;
    children: Snippet;
    stroke?: number;
  }
  let { size, progress, tone, children, stroke = 2 }: Props = $props();
  const uid = $props.id();
  const r = $derived(size / 2 - stroke / 2 - 0.5);
  const c = $derived(2 * Math.PI * r);
  const p = $derived(tone === 'wait' ? 1 : Math.max(0, Math.min(1, progress ?? (tone === 'done' ? 1 : 0))));
  const dot = $derived.by(() => {
    const a = -Math.PI / 2 + p * Math.PI * 2;
    return { x: size / 2 + r * Math.cos(a), y: size / 2 + r * Math.sin(a) };
  });
</script>

<span class="orbit t-{tone}" style:width="{size}px" style:height="{size}px">
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <defs>
      <linearGradient id="a-{uid}" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" style="stop-color: var(--aurora-a)" />
        <stop offset="0.5" style="stop-color: var(--aurora-m)" />
        <stop offset="1" style="stop-color: var(--aurora-b)" />
      </linearGradient>
    </defs>
    <circle class="track" cx={size / 2} cy={size / 2} {r} fill="none" stroke-width={stroke} />
    {#if p > 0}
      <circle
        class="arc"
        cx={size / 2}
        cy={size / 2}
        {r}
        fill="none"
        stroke-width={stroke}
        stroke-linecap="round"
        stroke={tone === 'live' ? `url(#a-${uid})` : undefined}
        stroke-dasharray="{c * p} {c}"
        transform="rotate(-90 {size / 2} {size / 2})"
      />
    {/if}
    {#if tone === 'live' && progress != null && p < 1}<circle class="drone" cx={dot.x} cy={dot.y} r={stroke + 0.6} />{/if}
  </svg>
  <span class="face">{@render children()}</span>
</span>

<style>
  .orbit { position: relative; display: inline-grid; place-items: center; flex-shrink: 0; }
  svg { position: absolute; inset: 0; overflow: visible; }
  .track { stroke: var(--line-2); }
  .arc { transition: stroke-dasharray var(--d-5) var(--spring); }
  .t-wait .arc { stroke: var(--amber); }
  .t-done .arc { stroke: var(--ok); opacity: 0.8; }
  .t-idle .arc, .t-off .arc { stroke: var(--ink-3); }
  .t-blocked .arc { stroke: var(--bad); }
  .t-wait .track { stroke: var(--amber-soft); }
  /* With no todo list there is no arc: the track alone tells working and blocked from idle.
     Tinted, not solid, so an arc drawn over it still shows how far the agent got. */
  .t-live .track { stroke: color-mix(in srgb, var(--live) 45%, transparent); }
  .t-blocked .track { stroke: color-mix(in srgb, var(--bad) 45%, transparent); }
  .drone { fill: var(--aurora-b); stroke: var(--night); stroke-width: 1.5; transition: cx var(--d-5) var(--spring), cy var(--d-5) var(--spring); }
  .face { display: grid; place-items: center; }
</style>
