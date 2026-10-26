<!-- "Why?" progressive disclosure: each line carries its provenance badge. -->
<script lang="ts" module>
  import type { Provenance } from '../lib/util/labels';
  export type WhyLine = { prov: Provenance; text: string; meta?: string };
</script>

<script lang="ts">
  import Icon from './Icon.svelte';
  import ProvBadge from './Provenance.svelte';

  interface Props {
    lines: WhyLine[];
    label?: string;
    open?: boolean;
  }
  let { lines, label = 'Why?', open = $bindable(false) }: Props = $props();
</script>

{#if lines.length}
  <div class="why">
    <button class="why-btn" aria-expanded={open} onclick={() => (open = !open)}>
      {label}<span class="tri" class:open><Icon name="chevron-right" size={11} stroke={2} /></span>
    </button>
    {#if open}
      <ul class="lines">
        {#each lines as l, i (i)}
          <li>
            <ProvBadge kind={l.prov} compact />
            <span class="t selectable">{l.text}</span>
            {#if l.meta}<span class="meta num">{l.meta}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .why { display: flex; flex-direction: column; gap: 8px; }
  .why-btn { align-self: flex-start; display: inline-flex; align-items: center; gap: 3px; height: 22px; padding: 0 8px; margin-left: -8px; border-radius: var(--r-full); color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); }
  .why-btn:hover { background: var(--surface-2); color: var(--ink-1); }
  .tri { display: inline-grid; transition: transform var(--d-2) var(--ease-out); }
  .tri.open { transform: rotate(90deg); }
  .lines { display: flex; flex-direction: column; gap: 7px; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); animation: fade-in var(--d-2) var(--ease-out); }
  li { display: flex; align-items: flex-start; gap: 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  li :global(.pv) { margin-top: -1px; }
  .t { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .meta { color: var(--ink-3); white-space: nowrap; }
</style>
