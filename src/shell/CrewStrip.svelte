<!--
  The crew strip: everyone across your requests. Each request's Lead is a small pill first
  (hover: the request it serves; click: that request), then one pill per active worker.
  Status changes are mirrored to a polite live region for screen readers.
-->
<script lang="ts">
  import { narrate } from '../lib/state/narration';
  import { store } from '../lib/state/store.svelte';
  import CrewPill from './CrewPill.svelte';

  interface Props {
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  const leads = $derived(store.leads.filter((a) => a.status !== 'offline'));
  const crew = $derived(
    // The team first (always there, even idle), then everyone else by arrival.
    store.agents
      .filter((a) => a.role !== 'lead' && (a.status !== 'offline' || a.member))
      .sort((a, b) => Number(!a.member) - Number(!b.member) || a.created_at - b.created_at),
  );

  // aria-live: announce sentence changes (timer excluded), at most one per agent per change.
  let said = new Map<string, string>();
  let announce = $state('');
  $effect(() => {
    const lines: string[] = [];
    for (const a of crew) {
      const n = narrate(a.id);
      if (!n) continue;
      if (said.get(a.id) !== n.key) {
        if (said.has(a.id)) lines.push(`${a.name}: ${n.key.replace(/`/g, '')}`);
        said.set(a.id, n.key);
      }
    }
    if (lines.length) announce = lines.join('. ');
  });
</script>

<div class="strip" role="group" aria-label="Your crew">
  {#each leads as a (a.id)}
    <CrewPill agent={a} compact />
  {/each}
  {#if leads.length && crew.length}<span class="sep" aria-hidden="true"></span>{/if}
  {#each crew as a (a.id)}
    <CrewPill agent={a} {compact} />
  {/each}
</div>
<p class="sr-only" aria-live="polite">{announce}</p>

<style>
  .strip { display: flex; align-items: center; justify-content: center; gap: 8px; min-width: 0; padding: 4px 2px; overflow-x: auto; scrollbar-width: none; mask-image: linear-gradient(90deg, transparent 0, #000 12px, #000 calc(100% - 12px), transparent 100%); }
  .strip::-webkit-scrollbar { display: none; }
  .sep { width: 1px; height: 22px; background: var(--line-2); flex-shrink: 0; margin: 0 2px; }
</style>
