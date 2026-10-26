<script lang="ts">
  import { drag } from '../lib/state/dnd.svelte';
  import Icon from './Icon.svelte';
</script>

{#if drag.payload}
  <div
    class="ghost"
    class:ok={drag.over && drag.overOk}
    class:no={drag.over && !drag.overOk}
    style:transform="translate({drag.x + 12}px, {drag.y + 10}px)"
    style:--c={drag.payload.color ?? 'var(--aurora-m)'}
  >
    <Icon name={drag.payload.kind === 'agent' ? 'user' : 'board'} size={14} />
    <span class="truncate">{drag.payload.label}</span>
    {#if drag.hint}<span class="hint">{drag.hint}</span>{/if}
  </div>
{/if}

<style>
  .ghost { position: fixed; left: 0; top: 0; z-index: var(--z-pop); display: inline-flex; align-items: center; gap: 7px; max-width: 280px; height: 30px; padding: 0 12px; border-radius: var(--r-full); background: var(--pop); color: var(--ink-1); font-size: var(--t-sm); font-weight: var(--w-med); box-shadow: var(--pop-shadow), inset 3px 0 0 var(--c); pointer-events: none; white-space: nowrap; transform-origin: 0 0; rotate: -2deg; }
  .ghost.ok { box-shadow: var(--pop-shadow), inset 0 0 0 1.5px var(--live); rotate: 0deg; }
  .ghost.no { opacity: 0.75; }
  .hint { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
