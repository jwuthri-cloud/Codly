<!--
  An agent's portrait: its character as the office draws it (office/portrait.ts), or — while the
  office has none (it returns '') — a small monogram glyph in the agent's identity hue.
-->
<script lang="ts" module>
  import { agentPortrait } from '../office/portrait';
  import { kenneyReady, loadKenney } from '../office/gfx/kenney';
  import type { Agent } from '../lib/gen/Agent';
  const cache = new Map<string, string>();
  // Portraits are drawn from the Kenney characters: monograms until they've loaded.
  let ready = $state(kenneyReady());
  loadKenney().then(() => (ready = kenneyReady()));
  function portraitOf(a: Pick<Agent, 'id' | 'color' | 'look_seed' | 'provider' | 'role'>, px: number): string {
    const key = `${a.id}:${a.color}:${a.look_seed}:${a.provider}:${a.role}:${px}`;
    let v = cache.get(key);
    if (v == null) {
      try {
        v = agentPortrait(a, px) || '';
      } catch {
        v = '';
      }
      if (v) cache.set(key, v);
    }
    return v;
  }
</script>

<script lang="ts">
  import { idVar } from '../lib/util/labels';

  interface Props {
    agent: Pick<Agent, 'id' | 'color' | 'look_seed' | 'provider' | 'role' | 'name'> | null | undefined;
    size?: number;
    dim?: boolean;
  }
  let { agent, size = 24, dim = false }: Props = $props();
  const uid = $props.id();
  const src = $derived(agent && ready ? portraitOf(agent, size) : '');
  const hue = $derived(idVar(agent?.color ?? 0));
  const lead = $derived(agent?.role === 'lead');
</script>

{#if src}
  <img class="portrait" class:dim {src} width={size} height={size} alt="" aria-hidden="true" style:width="{size}px" style:height="{size}px" />
{:else}
  <svg class="portrait" class:dim width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" style:--hue={hue}>
    <defs>
      <radialGradient id="b-{uid}" cx="38%" cy="30%" r="75%">
        <stop offset="0" style="stop-color: color-mix(in oklab, var(--hue) 55%, white)" />
        <stop offset="0.55" style="stop-color: var(--hue)" />
        <stop offset="1" style="stop-color: color-mix(in oklab, var(--hue) 62%, black)" />
      </radialGradient>
      <radialGradient id="g-{uid}" cx="50%" cy="55%" r="50%">
        <stop offset="0" style="stop-color: var(--hue); stop-opacity: 0.45" />
        <stop offset="1" style="stop-color: var(--hue); stop-opacity: 0" />
      </radialGradient>
    </defs>
    <circle cx="16" cy="17" r="15" fill="url(#g-{uid})" />
    {#if lead}<path d="M9.5 9.2a8 8 0 0 1 13 0" fill="none" style="stroke: var(--hue)" stroke-width="1.6" stroke-linecap="round" opacity="0.9" />{/if}
    <circle cx="16" cy="17" r="9.5" fill="url(#b-{uid})" />
    <rect x="9.6" y="14.2" width="12.8" height="5.6" rx="2.8" fill="#0b1018" opacity="0.88" />
    <circle cx="13.4" cy="17" r="1.25" style="fill: color-mix(in oklab, var(--hue) 35%, white)" />
    <circle cx="18.6" cy="17" r="1.25" style="fill: color-mix(in oklab, var(--hue) 35%, white)" />
  </svg>
{/if}

<style>
  .portrait { flex-shrink: 0; display: block; border-radius: 50%; }
  .dim { filter: saturate(0.4); opacity: 0.6; }
</style>
