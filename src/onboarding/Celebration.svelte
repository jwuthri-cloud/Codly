<!--
  A quest completed: a short banner (always announced through aria-live) and, unless the system
  asks for reduced motion, a CSS-only burst of sparkles.
-->
<script lang="ts">
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import { quest } from './quest.svelte';

  const BITS = Array.from({ length: 18 }, (_, i) => ({ a: (i / 18) * 360, d: 70 + ((i * 37) % 60), h: i % 3 }));
</script>

<div class="live sr-only" role="status" aria-live="polite">
  {#if quest.celebration}Quest complete: {quest.celebration.title}, +{quest.celebration.xp} XP{#if quest.celebration.level}. Level up: {quest.celebration.level}{/if}.{/if}
</div>

{#if quest.celebration}
  {#key quest.celebration.nonce}
    <div class="cel" aria-hidden="true">
      {#if !ui.reducedMotion}
        <div class="burst">
          {#each BITS as b, i (i)}<i style="--a:{b.a}deg; --d:{b.d}px" class="h{b.h}"></i>{/each}
        </div>
      {/if}
      <div class="banner glass" class:still={ui.reducedMotion}>
        <Icon name="sparkle" size={15} />
        <span><b>{quest.celebration.title}</b> <span class="xp num">+{quest.celebration.xp} XP</span></span>
        {#if quest.celebration.level}<span class="lvl">Level up · {quest.celebration.level}</span>{/if}
      </div>
    </div>
  {/key}
{/if}

<style>
  .cel { position: fixed; top: 64px; left: 50%; z-index: var(--z-modal); pointer-events: none; transform: translateX(-50%); display: grid; place-items: center; }
  .banner { display: inline-flex; align-items: center; gap: 10px; padding: 10px 16px; border-radius: var(--r-full); font-size: var(--t-md); white-space: nowrap; animation: pop-in var(--d-4) var(--spring) both; }
  .banner.still { animation: none; }
  .banner :global(svg) { color: var(--live); }
  .xp { color: var(--live); font-weight: var(--w-semi); }
  .lvl { padding: 3px 9px; border-radius: var(--r-full); background: var(--aurora); color: var(--on-aurora); font-size: var(--t-sm); font-weight: var(--w-semi); }
  .burst { position: absolute; width: 0; height: 0; }
  .burst i { position: absolute; width: 6px; height: 10px; border-radius: 2px; background: var(--aurora-a); opacity: 0; animation: burst 1100ms var(--ease-out) forwards; }
  .burst i.h1 { background: var(--aurora-m); width: 5px; height: 5px; border-radius: 50%; }
  .burst i.h2 { background: var(--aurora-b); }
  @keyframes burst {
    0% { opacity: 1; transform: rotate(var(--a)) translateY(0) scale(0.6); }
    70% { opacity: 1; }
    100% { opacity: 0; transform: rotate(var(--a)) translateY(calc(var(--d) * -1)) rotate(200deg) scale(1); }
  }
  @media (prefers-reduced-motion: reduce) { .burst { display: none; } .banner { animation: none; } }
</style>
